//! US-003: `make chrome-boxes FILE=<design.json>` exports a design as HTML with
//! the CSS of `docs/formats/layout.md`, loads the bundled fonts through
//! `@font-face`, renders it in headless Chrome at a device pixel ratio of 2, and
//! prints every node's box as JSON. Every expected value of the layout tests
//! comes from this tool, so these tests check that it prints those values.
//!
//! The tool prints one JSON object on standard output and nothing else: each
//! key is a node ID, and its value is the node's box in page coordinates, as
//! Chrome's `getBoundingClientRect` gives it, `{"x": 0, "y": 0, "width": 60,
//! "height": 30}`. A hidden node and every node below it get an empty box at
//! the top-left corner of their parent, as `docs/formats/layout.md` says, and a
//! page gets an empty box at its origin. Anything else that the tool says
//! goes to standard error.
//!
//! The tests need Chrome: headless Chrome 155 (the version the expected
//! values were measured with) at its usual place on the machine.

mod common;
mod design;

use design::cases::{self, Case};
use design::{at, board, fixed, flex, rect};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

/// Where tests of this file write the designs they print the boxes of.
const SCRATCH: &str = env!("CARGO_TARGET_TMPDIR");

/// The tool may keep a profile or other files in fixed places, so one Chrome
/// runs at a time.
static ONE_CHROME_AT_A_TIME: Mutex<()> = Mutex::new(());

/// How long `make chrome-boxes` may take to render a design and exit.
const TOOL_TIMEOUT: Duration = Duration::from_secs(180);

/// The boxes the tool printed, `[x, y, width, height]` by node ID.
type Boxes = BTreeMap<String, [f64; 4]>;

/// Writes `design` to a file called `name` and returns its path.
fn write_design(name: &str, design: &Value) -> PathBuf {
    let directory = Path::new(SCRATCH).join("chrome-boxes");
    fs::create_dir_all(&directory).expect("the scratch directory can be created");
    let path = directory.join(format!("{name}.json"));
    fs::write(&path, design.to_string()).expect("the design can be written");
    path
}

/// Runs `make chrome-boxes FILE=<file>` at the repository root, detached from
/// any `make` that started this test run, and kills it and the Chrome it started
/// if it takes longer than [`TOOL_TIMEOUT`].
fn make_chrome_boxes(file: &str) -> Output {
    let mut child = Command::new("make")
        .args(["chrome-boxes", &format!("FILE={file}")])
        .current_dir(common::repo_root())
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env_remove("MAKELEVEL")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group, so that a timeout can kill Chrome with it.
        .process_group(0)
        .spawn()
        .expect("make can be started");
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let read = |pipe: &mut dyn Read| {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes).ok();
        bytes
    };
    let stdout_reader = thread::spawn(move || read(&mut stdout));
    let stderr_reader = thread::spawn(move || read(&mut stderr));
    let deadline = Instant::now() + TOOL_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("make can be waited for") {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let group = format!("-{}", child.id());
                Command::new("kill").args(["-KILL", &group]).status().ok();
                child.kill().ok();
                panic!(
                    "`make chrome-boxes FILE={file}` must render the design and exit within {TOOL_TIMEOUT:?}, but it was still running"
                );
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    };
    Output {
        status,
        stdout: stdout_reader.join().expect("stdout can be read"),
        stderr: stderr_reader.join().expect("stderr can be read"),
    }
}

/// Runs `make chrome-boxes FILE=<file>` and parses what it prints: one JSON
/// object of boxes by node ID.
fn chrome_boxes(file: &str) -> Boxes {
    let _chrome = ONE_CHROME_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let output = make_chrome_boxes(file);
    assert!(
        output.status.success(),
        "`make chrome-boxes FILE={file}` must render the design in headless Chrome and print its boxes, but it failed:\n{}",
        common::describe(&output)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let printed: Value = serde_json::from_str(&stdout).unwrap_or_else(|error| {
        panic!(
            "`make chrome-boxes FILE={file}` must print the boxes as one JSON object and nothing else on standard output ({error}), but it printed:\n{stdout}"
        )
    });
    let nodes = printed.as_object().unwrap_or_else(|| {
        panic!("`make chrome-boxes` must print a JSON object with a box for each node ID, but it printed:\n{stdout}")
    });
    nodes
        .iter()
        .map(|(id, node_box)| {
            let number = |key: &str| {
                node_box[key].as_f64().unwrap_or_else(|| {
                    panic!(
                        "the box of node {id} must be an object with the numbers x, y, width and height, but it is {node_box}"
                    )
                })
            };
            (
                id.clone(),
                [number("x"), number("y"), number("width"), number("height")],
            )
        })
        .collect()
}

/// The IDs of `design` and of every node below it.
fn ids_of(design: &Value) -> BTreeSet<String> {
    design::ids_in(design)
}

/// A page of the boards of `cases`, each node renamed to an ID of its own so
/// that the cases can share a page, and, for each case, the new ID of every
/// node by the ID the case knows it by.
fn page_of(cases: &[Case]) -> (Value, Vec<HashMap<String, String>>) {
    fn rename(node: &mut Value, next: &mut u32, renamed: &mut HashMap<String, String>) {
        let new_id = format!("n{next:07}");
        *next += 1;
        let old_id = node["id"].as_str().expect("nodes have an ID").to_string();
        renamed.insert(old_id, new_id.clone());
        node["id"] = json!(new_id);
        if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
            for child in children {
                rename(child, next, renamed);
            }
        }
    }
    let mut next = 1;
    let mut boards = Vec::new();
    let mut renamings = Vec::new();
    for case in cases {
        let mut board = case.design.clone();
        let mut renamed = HashMap::new();
        rename(&mut board, &mut next, &mut renamed);
        boards.push(board);
        renamings.push(renamed);
    }
    let page = json!({"id": "page0001", "type": "page", "name": "Page", "children": boards});
    (page, renamings)
}

/// The boxes that `printed` gets wrong: every box that a case lists, which the
/// tool must print within the case's tolerance, where `renamed` gives the ID the
/// page has for each node of the case.
fn wrong_boxes(
    cases: &[Case],
    renamings: &[HashMap<String, String>],
    printed: &Boxes,
) -> Vec<String> {
    let mut wrong = Vec::new();
    for (case, renamed) in cases.iter().zip(renamings) {
        for (id, [x, y, width, height]) in &case.boxes {
            let new_id = &renamed[*id];
            let Some(actual) = printed.get(new_id) else {
                wrong.push(format!("{}: node {id} has no box in the output", case.name));
                continue;
            };
            let near =
                |actual: f64, expected: f64| (actual - expected).abs() <= f64::from(case.tolerance);
            if !(near(actual[0], *x)
                && near(actual[1], *y)
                && near(actual[2], *width)
                && near(actual[3], *height))
            {
                wrong.push(format!(
                    "{}: node {id} must have the box x {x}, y {y}, {width} x {height} (give or take {} px), but the tool printed x {}, y {}, {} x {}",
                    case.name, case.tolerance, actual[0], actual[1], actual[2], actual[3]
                ));
            }
        }
    }
    wrong
}

/// Prints the boxes of the page of `cases` and checks every box that a case
/// lists against what the tool printed.
fn check_cases(test: &str, cases: &[Case]) {
    let (page, renamings) = page_of(cases);
    let file = write_design(test, &page);
    let printed = chrome_boxes(&file.to_string_lossy());
    let wrong = wrong_boxes(cases, &renamings, &printed);
    assert!(
        wrong.is_empty(),
        "`make chrome-boxes` must print the boxes Chrome gives these designs, but {} of them differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn it_prints_the_box_chrome_gives_every_node_of_every_case_and_an_empty_one_for_the_page() {
    // The only test of the export rules: the page holds the board of every case
    // of `cases::all()`, some 950 nodes, plus a board at a place of its own, and
    // the tool must print the box Chrome gives every node of every case. One run
    // checks the CSS of every layout property of docs/formats/layout.md; text in
    // the bundled Inter Regular through `@font-face`, whatever its family,
    // weight and style, with newlines, tabs and spaces kept (`pre-wrap`); line
    // heights written in px, rounded down to a 64th of a pixel (Q44), rendered
    // at a device pixel ratio of 2; the style of nodes left out and `clip` as
    // `overflow: clip`, so neither changes a box; the content of texts escaped,
    // never written as markup; boards that hug or fill their width as
    // `width: max-content` wherever they sit; and hidden boards and the nodes
    // below them with empty boxes at the board's own x and y.
    let second = cases::case(
        "a board at (300, 40)",
        at(
            &board(
                "brd00001",
                fixed(120.0),
                fixed(80.0),
                flex(json!({"padding": {"top": 5, "left": 7}})),
                vec![rect("a0000001", 30.0, 20.0)],
            ),
            300.0,
            40.0,
        ),
        &[
            ("brd00001", [300.0, 40.0, 120.0, 80.0]),
            ("a0000001", [307.0, 45.0, 30.0, 20.0]),
        ],
    );
    let mut cases = cases::all();
    cases.push(second);
    let (page, renamings) = page_of(&cases);
    let file = write_design("page-of-every-case", &page);
    let printed = chrome_boxes(&file.to_string_lossy());
    assert_eq!(
        printed.keys().cloned().collect::<BTreeSet<_>>(),
        ids_of(&page),
        "`make chrome-boxes` must print one box for each node of the design, the page and the nodes below hidden nodes included, and no other"
    );
    assert_eq!(
        printed["page0001"],
        [0.0, 0.0, 0.0, 0.0],
        "a page is a canvas without a size, so its box is empty at its origin"
    );
    let wrong = wrong_boxes(&cases, &renamings, &printed);
    assert!(
        wrong.is_empty(),
        "`make chrome-boxes` must print the box Chrome gives every node of every case, with all of them on one page: boards sit at their own x and y, and a hidden node and every node below it get an empty box at the top-left corner of its parent, or at the x and y of a hidden board; but {} boxes differ:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn it_exports_margins_as_css_margins() {
    // US-053: a margin on each side is `margin-top`, `margin-right`,
    // `margin-bottom` and `margin-left`, in px. Without them, Penpot's Punkt dot
    // sits at y 0 instead of y 5, and every other case loses its margins.
    check_cases("margins", &cases::margins());
}

#[test]
fn it_exports_the_self_alignment_of_grid_items_as_justify_self_and_align_self() {
    // US-053: `justifySelf` and `alignSelf` are written as `justify-self` and
    // `align-self` in place of the ones that the item's sizing gives, so the
    // icon button sits at the center of its 40 x 40 area instead of its start.
    check_cases("grid-self-alignment", &cases::grid_self_alignment());
}

#[test]
fn it_prints_the_boxes_of_a_board_that_is_not_on_a_page_and_takes_a_path_from_the_repository_root()
{
    // `make` runs at the repository root, where a path like `target/...` is
    // relative. The design is a board, which sits at its own x and y.
    let case = cases::case(
        "a board at (20, 10) that hugs a row of two boxes",
        at(
            &board(
                "brd00001",
                json!("hug"),
                json!("hug"),
                flex(json!({"gap": {"column": 6}})),
                vec![rect("a0000001", 50.0, 20.0), rect("b0000001", 30.0, 40.0)],
            ),
            20.0,
            10.0,
        ),
        &[
            ("brd00001", [20.0, 10.0, 86.0, 40.0]),
            ("a0000001", [20.0, 10.0, 50.0, 20.0]),
            ("b0000001", [76.0, 10.0, 30.0, 40.0]),
        ],
    );
    let path = write_design("board-root", &case.design);
    let root = common::repo_root();
    let file = path
        .strip_prefix(&root)
        .map_or_else(|_| path.clone(), Path::to_path_buf);
    let printed = chrome_boxes(&file.to_string_lossy());
    assert_eq!(
        printed.keys().cloned().collect::<BTreeSet<_>>(),
        ids_of(&case.design),
        "`make chrome-boxes` must print one box for each node of the design"
    );
    for (id, expected) in &case.boxes {
        let actual = printed[*id];
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(actual, expected)| (actual - expected).abs() <= f64::from(case.tolerance)),
            "{}: node {id} must have the box {expected:?}, but the tool printed {actual:?}",
            case.name
        );
    }
}
