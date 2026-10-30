//! How deep a design may nest (`omni_render::nesting`). Reading, resolving,
//! laying out, painting and dropping a design recurse into its nodes, so the
//! stack they need grows with every level. The WebAssembly module crashed the
//! design viewer on the Penpot import tirebud-ordrar with "memory access out
//! of bounds": Emscripten's default stack of 64 KB holds about 10 levels, and
//! an overflow overwrote the allocator's state below the stack.
//!
//! - The module's stack (`STACK_SIZE` in `build.rs`) holds
//!   `nesting::MAX_NESTING` levels with room to spare, and an overflow aborts
//!   with "stack overflow" (`STACK_OVERFLOW_CHECK=2`) instead of corrupting
//!   memory.
//! - Resolving instances cuts a page off at `MAX_NESTING` levels below it:
//!   the node at the last level that holds more is drawn as an error
//!   placeholder without its children, and `Scene::errors` lists it with
//!   `Missing::Nesting`. So a scene never nests deeper, whatever its design
//!   system, and builds within a bounded stack.
//! - A design read from JSON that nests too deeply to read fails with a
//!   reason that says so.
//! - In the browser, `make render-wasm`'s module loads a page 62 levels deep
//!   (the deepest JSON page it reads), refuses a design whose instances nest
//!   deeper than `MAX_NESTING` with that reason, and loads the next page
//!   after it.
//! - The same browser page runs the module's timings of a change of design
//!   system, which `make bench-ordrar` charges to its frames.

mod common;

use omni_core::design::Node;
use omni_render::nesting::{self, MAX_NESTING};
use omni_render::paint::Assets;
use omni_render::select::Layer;
use omni_render::system::{DesignSystem, Missing};
use omni_render::view::Scene;
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{OnceLock, mpsc};
use std::thread;
use std::time::Duration;

/// How much stack each level may take in the WebAssembly module: measured at
/// about 5 KB in a release build, so this leaves three times that.
const STACK_PER_LEVEL: usize = 16 * 1024;

/// The stack that a scene of the deepest instances is built on in these
/// tests. Without the cut-off at `MAX_NESTING`, the instance chain of
/// [`instance_chain`] resolves almost 1,000 levels deep, and building it
/// overflows this stack.
const TEST_STACK: usize = 16 * 1024 * 1024;

/// A node ID that is unique for every `n` (and fits the ID alphabet).
fn node_id(prefix: char, n: usize) -> String {
    format!("{prefix}{n:07}")
}

/// A frame that hugs `child`, in a flex layout like a Penpot auto layout.
fn hugging_frame(id: String, level: usize, child: Value) -> Value {
    json!({
        "id": id,
        "type": "frame",
        "name": format!("Level {level}"),
        "width": "hug",
        "height": "hug",
        "layout": {
            "mode": "flex",
            "direction": if level % 2 == 0 { "column" } else { "row" },
            "padding": {"top": 1, "right": 1, "bottom": 1, "left": 1}
        },
        "fills": [{"type": "solid", "color": "#eeeeee"}],
        "children": [child]
    })
}

fn leaf_text() -> Value {
    json!({
        "id": node_id('t', 0),
        "type": "text",
        "name": "Leaf",
        "width": "hug",
        "height": "hug",
        "content": "deep",
        "fontFamily": "Inter",
        "fontSize": 16
    })
}

fn page_of(board: Value) -> Value {
    json!({"id": "pg000001", "type": "page", "name": "Deep", "children": [board]})
}

/// A page whose one board holds `levels - 2` frames, each inside the one
/// before, around a text: `levels` (2 or more) levels below the page.
fn nested_page(levels: usize) -> Value {
    let mut node = leaf_text();
    for level in (2..levels).rev() {
        node = hugging_frame(node_id('f', level), level, node);
    }
    node = hugging_frame(node_id('b', 1), 1, node);
    node["type"] = json!("board");
    node["x"] = json!(0);
    node["y"] = json!(0);
    page_of(node)
}

/// A design system of `count` components, each `frames` hugging frames deep
/// around an instance of the one before (the first around a text), and a page
/// with one instance of the last: 24 components of 40 frames resolve almost
/// 1,000 levels deep, and every instance is within the 24 that may be inside
/// one another.
fn instance_chain(count: usize, frames: usize) -> (Value, Value) {
    let mut ids = 0;
    let mut next = |prefix| {
        ids += 1;
        node_id(prefix, ids)
    };
    let mut components: Vec<Value> = Vec::new();
    for index in 0..count {
        let mut inner = match components.last() {
            None => leaf_text(),
            Some(previous) => json!({
                "id": next('n'),
                "type": "instance",
                "name": format!("Inner {index}"),
                "component": previous["id"]
            }),
        };
        for level in 0..frames {
            inner = hugging_frame(next('f'), level, inner);
        }
        components.push(json!({
            "id": next('c'),
            "type": "component",
            "name": format!("Component {index}"),
            "width": "hug",
            "height": "hug",
            "layout": {"mode": "flex"},
            "children": [inner]
        }));
    }
    let top = json!({
        "id": "n9999999",
        "type": "instance",
        "name": "Top",
        "component": components.last().expect("the chain has a component")["id"]
    });
    let board = json!({
        "id": "b9999999",
        "type": "board",
        "name": "Board",
        "x": 0,
        "y": 0,
        "width": "hug",
        "height": "hug",
        "layout": {"mode": "flex"},
        "children": [top]
    });
    let system = json!({
        "$extensions": {"omnistate": {"modes": ["light"]}},
        "$components": components
    });
    (system, page_of(board))
}

fn read_node(json: &Value) -> Node {
    serde_json::from_value(json.clone())
        .unwrap_or_else(|error| panic!("the fixture must be a design node, but: {error}"))
}

/// How many levels the deepest of `layers` is below the page.
fn layer_depth(layers: &[Layer]) -> usize {
    let mut deepest = 0;
    let mut pending: Vec<(&Layer, usize)> = layers.iter().map(|layer| (layer, 1)).collect();
    while let Some((layer, level)) = pending.pop() {
        deepest = deepest.max(level);
        pending.extend(layer.children.iter().map(|child| (child, level + 1)));
    }
    deepest
}

/// Runs `work` on a thread with a stack of `size` bytes, where an overflow
/// aborts the test run.
fn on_stack<T: Send + 'static>(size: usize, work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(size)
        .spawn(work)
        .expect("a thread can be started")
        .join()
        .unwrap_or_else(|_| panic!("the work on a bounded stack panicked"))
}

#[test]
fn resolving_cuts_a_design_off_at_the_nesting_limit() {
    let (system, page) = instance_chain(24, 40);
    let (errors, depth) = on_stack(TEST_STACK, move || {
        let system = DesignSystem::from_json(&system)
            .unwrap_or_else(|error| panic!("the design system must be read, but: {error}"));
        let scene = Scene::with_system(read_node(&page), Assets::new(), system);
        (scene.errors().to_vec(), layer_depth(&scene.layers()))
    });
    assert_eq!(
        depth, MAX_NESTING,
        "the scene must be cut off at {MAX_NESTING} levels, the node at the limit drawn as a placeholder"
    );
    let nesting: Vec<String> = errors
        .iter()
        .filter(|error| error.missing == Missing::Nesting)
        .map(|error| error.node.to_string())
        .collect();
    assert_eq!(
        nesting.len(),
        1,
        "the one node at the limit that holds more must be listed as too deep, but the errors are {errors:?}"
    );
    assert!(
        nesting[0].starts_with("n9999999/"),
        "the node at the limit is inside the page's instance, so it is addressed from it: {}",
        nesting[0]
    );
}

#[test]
fn a_design_exactly_at_the_limit_resolves_whole() {
    let (system, page) = instance_chain(9, 13);
    let (errors, depth) = on_stack(TEST_STACK, move || {
        let system = DesignSystem::from_json(&system)
            .unwrap_or_else(|error| panic!("the design system must be read, but: {error}"));
        let scene = Scene::with_system(read_node(&page), Assets::new(), system);
        (scene.errors().to_vec(), layer_depth(&scene.layers()))
    });
    assert!(errors.is_empty(), "nothing is missing, but: {errors:?}");
    // The board; for each of the 9 components its instance, drawn as the
    // component's frame, and the 13 frames inside it; and the text.
    assert_eq!(depth, 1 + 9 * 14 + 1, "every level must be drawn");
    assert_eq!(depth, MAX_NESTING);
}

#[test]
fn a_design_too_deep_to_read_says_so() {
    let (deepest, too_deep) = on_stack(TEST_STACK, || {
        let deepest = nesting::read(&nested_page(62).to_string()).map(|node| nesting::depth(&node));
        (
            deepest,
            nesting::read(&nested_page(80).to_string()).map(|_| ()),
        )
    });
    assert_eq!(
        deepest,
        Ok(63),
        "a page 62 levels deep, the deepest JSON page, must be read"
    );
    let error = too_deep.expect_err("a page 80 levels deep is deeper than JSON is read");
    assert!(
        error.contains(&format!("more than {MAX_NESTING} levels deep")),
        "the reason must say that the design nests too deeply, but it is: {error}"
    );
}

#[test]
fn the_webassembly_stack_holds_the_deepest_design() {
    let path = common::repo_root().join("crates/omni-render/build.rs");
    let build = fs::read_to_string(&path).expect("crates/omni-render/build.rs can be read");
    let stack = build
        .split('"')
        .find_map(|setting| setting.strip_prefix("STACK_SIZE="))
        .unwrap_or_else(|| {
            panic!(
                "build.rs must set STACK_SIZE: Emscripten's default of 64 KB holds about 10 levels"
            )
        });
    let stack: usize = stack
        .parse()
        .unwrap_or_else(|_| panic!("STACK_SIZE must be a number of bytes, not `{stack}`"));
    assert!(
        stack >= MAX_NESTING * STACK_PER_LEVEL,
        "STACK_SIZE={stack} must hold {MAX_NESTING} levels of {STACK_PER_LEVEL} bytes"
    );
    assert!(
        build.contains("\"STACK_OVERFLOW_CHECK=2\""),
        "build.rs must set STACK_OVERFLOW_CHECK=2, so that an overflow aborts instead of corrupting memory"
    );
}

// ---------------------------------------------------------------------------
// In the browser
// ---------------------------------------------------------------------------

/// How long the page may take to load the module and the designs. A module
/// whose memory a stack overflow corrupted may hang, so this bounds the test.
const BROWSER_TIMEOUT: Duration = Duration::from_secs(120);

const CHROME_MACOS: &str = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const CHROME_NAMES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
];

/// The start of a page that runs steps in `make render-wasm`'s module and posts
/// the `result` they fill in to `/result`. Nothing is drawn, so it needs no
/// WebGL.
const PAGE_START: &str = r#"<!doctype html>
<meta charset="utf-8">
<script type="module">
const report = (result) => fetch("/result", { method: "POST", body: JSON.stringify(result) });
try {
  const { default: createOmniRender } = await import("./omni_render.js");
  const module = await createOmniRender();
  const encoder = new TextEncoder();
  const withText = (text, use) => {
    const bytes = encoder.encode(text);
    const pointer = module._omni_render_alloc(bytes.length);
    module.HEAPU8.set(bytes, pointer);
    try { return use(pointer, bytes.length); } finally { module._omni_render_free(pointer, bytes.length); }
  };
  const fetched = async (name) => (await fetch(name)).text();
  const failure = () => module.UTF8ToString(module._omni_render_error());
  const setSystem = (text) =>
    withText(text, (pointer, length) => module._omni_render_scene_set_system(pointer, length)) ? "set" : failure();
  const load = async (name) =>
    withText(await fetched(name), (pointer, length) => module._omni_render_scene_load(pointer, length)) ? "loaded" : failure();
  const result = {};
"#;

const PAGE_END: &str = r#"  report(result);
} catch (error) {
  report({ crash: String(error) });
}
</script>
"#;

/// The steps that load the designs of the nesting test, one after another.
const NESTING_STEPS: &str = r#"  const depth = () => {
    let deepest = 0;
    const walk = (layers, level) => { for (const layer of layers) { deepest = Math.max(deepest, level); walk(layer.children, level + 1); } };
    walk(JSON.parse(module.UTF8ToString(module._omni_render_scene_layers())), 1);
    return deepest;
  };
  result.deep = await load("deep.json");
  result.deepDepth = depth();
  result.system = setSystem(await fetched("chain-system.json"));
  result.chain = await load("chain-page.json");
  result.chainErrors = JSON.parse(module.UTF8ToString(module._omni_render_scene_errors()));
  result.next = await load("next.json");
  result.nextDepth = depth();
"#;

/// The steps that read the timings of a load and of the change of design
/// system after it.
const TIMINGS_STEPS: &str = r#"  const timings = () => JSON.parse(module.UTF8ToString(module._omni_render_scene_timings()));
  setSystem(JSON.stringify({ $components: [] }));
  await load("page.json");
  result.loadTimings = timings();
  result.system = setSystem(await fetched("system.json"));
  result.systemTimings = timings();
"#;

/// The Chrome binary: `CHROME`, or Chrome where it usually is.
fn chrome() -> PathBuf {
    if let Some(named) = std::env::var_os("CHROME") {
        return named.into();
    }
    if Path::new(CHROME_MACOS).exists() {
        return CHROME_MACOS.into();
    }
    std::env::var_os("PATH")
        .iter()
        .flat_map(std::env::split_paths)
        .flat_map(|directory| CHROME_NAMES.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.exists())
        .expect("the test needs Chrome: name its binary in the CHROME variable")
}

/// Serves the files of `root` and sends the body of each POST to `/result`.
fn serve(listener: TcpListener, root: PathBuf, results: mpsc::Sender<String>) {
    for stream in listener.incoming().flatten() {
        let (root, results) = (root.clone(), results.clone());
        thread::spawn(move || answer(stream, &root, &results));
    }
}

fn answer(stream: TcpStream, root: &Path, results: &mpsc::Sender<String>) {
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    if reader.read_line(&mut request).is_err() {
        return;
    }
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut parts = request.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let (status, kind, body) = if method == "POST" && path == "/result" {
        let mut body = vec![0; length];
        if reader.read_exact(&mut body).is_ok() {
            results
                .send(String::from_utf8_lossy(&body).into_owned())
                .ok();
        }
        ("200 OK", "text/plain", Vec::new())
    } else {
        let name = path.trim_start_matches('/');
        let name = if name.is_empty() { "index.html" } else { name };
        let kind = match name.rsplit('.').next() {
            Some("html") => "text/html",
            Some("js") => "text/javascript",
            Some("wasm") => "application/wasm",
            _ => "application/json",
        };
        match fs::read(root.join(name)) {
            Ok(body) if !name.contains("..") => ("200 OK", kind, body),
            _ => ("404 Not Found", "text/plain", Vec::new()),
        }
    };
    let mut stream = &stream;
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).ok();
    stream.write_all(&body).ok();
}

/// Runs `make render-wasm` once for every test in this file that needs it.
fn render_wasm() -> &'static Output {
    static BUILD: OnceLock<Output> = OnceLock::new();
    let output = BUILD.get_or_init(|| common::make(&["render-wasm"]));
    assert!(
        output.status.success(),
        "`make render-wasm` failed:\n{}",
        common::describe(output)
    );
    output
}

/// Runs `steps` in a page in Chrome, with `files` served beside it in a folder
/// of this test's own, and gives back the `result` they filled in.
fn run_in_chrome(folder: &str, steps: &str, files: &[(&str, String)]) -> Value {
    render_wasm();
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(folder);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("the page's folder can be made");
    let build = common::repo_root().join("render-sandbox/build");
    for file in ["omni_render.js", "omni_render.wasm"] {
        fs::copy(build.join(file), root.join(file))
            .unwrap_or_else(|error| panic!("render-sandbox/build/{file} can be copied: {error}"));
    }
    fs::write(
        root.join("index.html"),
        format!("{PAGE_START}{steps}{PAGE_END}"),
    )
    .expect("the page can be written");
    for (name, content) in files {
        fs::write(root.join(name), content).expect("the page's files can be written");
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("a local port can be opened");
    let port = listener.local_addr().expect("the port is known").port();
    let (send, results) = mpsc::channel();
    let served = root.clone();
    thread::spawn(move || serve(listener, served, send));
    let mut browser = Command::new(chrome())
        .args([
            "--headless=new",
            "--no-first-run",
            "--no-default-browser-check",
            &format!("--user-data-dir={}", root.join("profile").display()),
            &format!("http://127.0.0.1:{port}/index.html"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("Chrome can be started");
    let result = results.recv_timeout(BROWSER_TIMEOUT);
    Command::new("kill")
        .args(["-KILL", &format!("-{}", browser.id())])
        .status()
        .ok();
    browser.wait().ok();

    let result = result.unwrap_or_else(|_| {
        panic!("the page must load the designs within {BROWSER_TIMEOUT:?}: a module whose memory was corrupted hangs")
    });
    let result: Value = serde_json::from_str(&result).expect("the page reports JSON");
    assert_eq!(
        result["crash"],
        Value::Null,
        "the module must not crash: {result}"
    );
    result
}

#[test]
fn the_webassembly_module_loads_deep_designs_and_refuses_deeper_ones() {
    let (system, chain) = instance_chain(24, 40);
    let result = run_in_chrome(
        "nesting-wasm",
        NESTING_STEPS,
        &[
            ("deep.json", nested_page(62).to_string()),
            ("chain-system.json", system.to_string()),
            ("chain-page.json", chain.to_string()),
            ("next.json", nested_page(12).to_string()),
        ],
    );
    assert_eq!(
        result["deep"], "loaded",
        "a page 62 levels deep, the deepest JSON page, must load: {result}"
    );
    assert_eq!(
        result["deepDepth"], 62,
        "every level of it must be there: {result}"
    );
    assert_eq!(result["system"], "set", "{result}");
    let refused = result["chain"].as_str().unwrap_or_default();
    assert!(
        refused.contains(&format!("more than {MAX_NESTING} levels deep")),
        "instances that nest almost 1,000 levels deep must be refused with a reason that says so: {result}"
    );
    assert!(
        result["chainErrors"]
            .as_array()
            .is_some_and(|errors| errors.iter().any(|error| error["nesting"] == MAX_NESTING)),
        "the errors must list the node where the design was cut off: {result}"
    );
    assert_eq!(
        result["next"], "loaded",
        "the module must load the next page after refusing one: {result}"
    );
    assert_eq!(result["nextDepth"], 12, "{result}");
}

#[test]
fn the_webassembly_module_reports_the_layout_of_a_change_of_design_system() {
    let (system, page) = instance_chain(2, 40);
    let result = run_in_chrome(
        "timings-wasm",
        TIMINGS_STEPS,
        &[
            ("system.json", system.to_string()),
            ("page.json", page.to_string()),
        ],
    );
    assert_eq!(result["system"], "set", "{result}");
    let (loaded, changed) = (&result["loadTimings"], &result["systemTimings"]);
    assert!(
        changed["submitMs"].is_number(),
        "the module must report the time submitted to the GPU, which `make bench-ordrar` reads to split painting from uploading: {result}"
    );
    let layout = changed["layoutMs"].as_f64().unwrap_or_else(|| {
        panic!("the module must report how long resolving a design system took: {result}")
    });
    assert!(
        layout > 0.0,
        "resolving the instances of a chain 80 frames deep takes time: {result}"
    );
    assert_ne!(
        changed["layoutMs"], loaded["layoutMs"],
        "the module must report the layout of the new design system, not of the load before it: {result}"
    );
}
