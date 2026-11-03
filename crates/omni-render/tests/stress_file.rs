//! US-004: `make stress-file` writes a large, realistic design file that every
//! renderer story is tested on (the bench of US-008, the layout parity check of
//! US-009 and the selection index of US-007).
//!
//! The file is one page, as JSON in the format of `omni_core::design`, whose
//! children are the boards. The make target takes two optional variables:
//!
//! - `OUT=<path>`: where to write the file. A relative path is relative to the
//!   repository root. Without it the file goes to
//!   `render-sandbox/build/stress-file.json`, where the sandbox page can load it.
//! - `SEED=<number>`: the seed of the random generator. Without it the target
//!   uses its own fixed seed. The same seed always gives the same file, byte
//!   for byte, and different seeds give different files.
//!
//! Whatever the seed, the file has 100 boards and at least 30,000 nodes (the
//! page, the boards and everything inside them all count), and at least 20
//! image nodes or image fills. Exactly one board is text-heavy: more than half
//! of its nodes are texts, a larger share than on any other board, and it has
//! exactly 2,000 nodes (the board itself and every node inside it). The file
//! holds every node type except instances (US-032 covers them), uses the `none`,
//! `flex` and `grid` layout modes, and has literal values only: no token
//! references, which are strings that start with `$`.

mod common;

use omni_core::design::Node;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

const SCRATCH: &str = env!("CARGO_TARGET_TMPDIR");

/// Where the target writes the file when `OUT` is not given.
const DEFAULT_OUT: &str = "render-sandbox/build/stress-file.json";

/// Seeds the tests generate files with.
const SEED: &str = "42";
const OTHER_SEED: &str = "43";

/// Properties whose strings are free text, not values that could be token
/// references: a text may well start with `$`.
const FREE_TEXT: [&str; 2] = ["content", "name"];

/// A file the target wrote, as JSON.
struct Stress {
    bytes: Vec<u8>,
    design: Value,
}

/// Files generated so far, by seed, so that each seed is generated once. The
/// lock is held while generating, which also keeps `make` runs one at a time.
static GENERATED: Mutex<Option<HashMap<&'static str, &'static Stress>>> = Mutex::new(None);

/// Runs `make stress-file` with `variables` (such as `SEED=42`).
fn make_stress_file(variables: &[String]) -> Output {
    let variables: Vec<&str> = variables.iter().map(String::as_str).collect();
    common::make_stress_file(&variables)
}

/// Runs `make stress-file` with `variables`, asserts that it succeeds, and
/// returns what it wrote at `path`.
fn generate(variables: &[String], path: &Path) -> Stress {
    let _ = fs::remove_file(path);
    let output = make_stress_file(variables);
    assert!(
        output.status.success(),
        "`make stress-file {}` must write the design file and exit successfully:\n{}",
        variables.join(" "),
        common::describe(&output)
    );
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!(
            "`make stress-file {}` must write the design file to {}: {error}",
            variables.join(" "),
            path.display()
        )
    });
    let design = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "the design file at {} must be JSON, but: {error}",
            path.display()
        )
    });
    Stress { bytes, design }
}

fn out_path(name: &str) -> PathBuf {
    let directory = Path::new(SCRATCH).join("stress-file");
    fs::create_dir_all(&directory).expect("the scratch directory can be created");
    directory.join(name)
}

/// The file `make stress-file SEED=<seed> OUT=<scratch path>` writes. Each seed
/// is generated once per test run.
fn stress(seed: &'static str) -> &'static Stress {
    let mut generated = GENERATED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let generated = generated.get_or_insert_with(HashMap::new);
    generated.entry(seed).or_insert_with(|| {
        let path = out_path(&format!("seed-{seed}.json"));
        let variables = [format!("SEED={seed}"), format!("OUT={}", path.display())];
        Box::leak(Box::new(generate(&variables, &path)))
    })
}

fn children(node: &Value) -> &[Value] {
    node.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// `node` and every node below it, parents before children.
fn subtree<'a>(node: &'a Value, into: &mut Vec<&'a Value>) {
    into.push(node);
    for child in children(node) {
        subtree(child, into);
    }
}

fn nodes_of(root: &Value) -> Vec<&Value> {
    let mut nodes = Vec::new();
    subtree(root, &mut nodes);
    nodes
}

fn kind(node: &Value) -> &str {
    node["type"].as_str().expect("every node has a type")
}

fn boards(design: &Value) -> &[Value] {
    assert_eq!(
        kind(design),
        "page",
        "the design file must be one page whose children are the boards"
    );
    children(design)
}

/// Image nodes plus image fills.
fn images_in(nodes: &[&Value]) -> usize {
    nodes
        .iter()
        .map(|node| {
            let node_itself = usize::from(kind(node) == "image");
            let fills = node
                .get("fills")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter(|fill| fill["type"] == "image")
                .count();
            node_itself + fills
        })
        .sum()
}

fn text_share(board: &Value) -> f64 {
    let nodes = nodes_of(board);
    let texts = nodes.iter().filter(|node| kind(node) == "text").count();
    texts as f64 / nodes.len() as f64
}

/// The first string that is a token reference, as a path to it.
fn find_token_reference(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::String(text) if text.starts_with('$') => Some(format!("{path} = {text:?}")),
        Value::Object(object) => object
            .iter()
            .filter(|(key, _)| !FREE_TEXT.contains(&key.as_str()))
            .find_map(|(key, value)| find_token_reference(value, &format!("{path}/{key}"))),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| find_token_reference(item, &format!("{path}/{index}"))),
        _ => None,
    }
}

#[test]
fn make_stress_file_writes_a_design_file_of_100_boards_in_json() {
    let default_path = common::repo_root().join(DEFAULT_OUT);
    let first = generate(&[], &default_path);
    assert_eq!(
        boards(&first.design).len(),
        100,
        "the stress file must have 100 boards"
    );
    assert!(
        boards(&first.design)
            .iter()
            .all(|board| kind(board) == "board"),
        "every child of the page must be a board"
    );

    // Without SEED the target uses one fixed seed, so a second run writes the
    // same file, wherever it is told to write it.
    let path = out_path("default-seed.json");
    let second = generate(&[format!("OUT={}", path.display())], &path);
    assert_eq!(
        first.bytes, second.bytes,
        "`make stress-file` without SEED must write the same file every time, and OUT must change only where it writes"
    );
}

#[test]
fn the_file_is_a_valid_design_page_with_unique_node_ids() {
    let stress = stress(SEED);
    let page: Node = serde_json::from_slice(&stress.bytes).unwrap_or_else(|error| {
        panic!("the stress file must be accepted by the design node model: {error}")
    });
    let mut ids = BTreeSet::new();
    let mut count = 0;
    let mut pending = vec![&page];
    while let Some(node) = pending.pop() {
        count += 1;
        assert!(
            ids.insert(node.id().to_string()),
            "node ID {} is used twice: IDs are unique within a file",
            node.id()
        );
        pending.extend(node.children());
    }
    assert_eq!(count, nodes_of(&stress.design).len());
}

#[test]
fn the_file_has_at_least_30000_nodes() {
    let stress = stress(SEED);
    let total = nodes_of(&stress.design).len();
    assert!(
        total >= 30_000,
        "the stress file must have at least 30,000 nodes in total, but it has {total}"
    );
}

#[test]
fn the_file_has_at_least_20_image_nodes_or_image_fills() {
    let stress = stress(SEED);
    let images = images_in(&nodes_of(&stress.design));
    assert!(
        images >= 20,
        "the stress file must have at least 20 image nodes or image fills, but it has {images}"
    );
}

#[test]
fn exactly_one_board_is_text_heavy_and_has_exactly_2000_nodes() {
    let stress = stress(SEED);
    let boards = boards(&stress.design);
    let sizes: Vec<(usize, usize)> = boards
        .iter()
        .enumerate()
        .map(|(index, board)| (index, nodes_of(board).len()))
        .collect();
    let of_2000: Vec<usize> = sizes
        .iter()
        .filter(|(_, size)| *size == 2_000)
        .map(|(index, _)| *index)
        .collect();
    assert_eq!(
        of_2000.len(),
        1,
        "exactly one board must have exactly 2,000 nodes (the board and every node in it); board sizes: {sizes:?}"
    );

    let heavy = &boards[of_2000[0]];
    let share = text_share(heavy);
    assert!(
        share > 0.5,
        "the 2,000-node board must be text-heavy, with more than half of its nodes texts, but {:.0}% are",
        share * 100.0
    );
    let busiest_other = boards
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != of_2000[0])
        .map(|(_, board)| text_share(board))
        .fold(0.0, f64::max);
    assert!(
        share > busiest_other,
        "the 2,000-node board must have a larger share of texts ({:.0}%) than any other board ({:.0}%)",
        share * 100.0,
        busiest_other * 100.0
    );
}

#[test]
fn the_file_uses_every_node_type_except_instance() {
    let stress = stress(SEED);
    let found: BTreeSet<&str> = nodes_of(&stress.design).into_iter().map(kind).collect();
    for expected in ["page", "board", "frame", "text", "image", "vector"] {
        assert!(
            found.contains(expected),
            "the stress file must use {expected} nodes; it uses {found:?}"
        );
    }
    assert!(
        !found.contains("instance"),
        "the stress file must not use instances (US-032 covers them); it uses {found:?}"
    );
}

#[test]
fn the_file_uses_every_layout_mode() {
    let stress = stress(SEED);
    let modes: BTreeSet<&str> = nodes_of(&stress.design)
        .into_iter()
        .filter_map(|node| node.pointer("/layout/mode").and_then(Value::as_str))
        .collect();
    for expected in ["none", "flex", "grid"] {
        assert!(
            modes.contains(expected),
            "the stress file must use the {expected} layout mode; it uses {modes:?}"
        );
    }
}

#[test]
fn the_file_holds_literal_values_only() {
    let stress = stress(SEED);
    if let Some(reference) = find_token_reference(&stress.design, "") {
        panic!(
            "the stress file must hold literal values only, but it has a token reference at {reference}"
        );
    }
}

#[test]
fn another_seed_gives_another_file() {
    let first = stress(SEED);
    let other = stress(OTHER_SEED);
    assert_ne!(
        first.bytes, other.bytes,
        "SEED={SEED} and SEED={OTHER_SEED} must give different files"
    );
}
