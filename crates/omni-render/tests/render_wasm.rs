//! US-001: `make render-wasm` builds omni-render for the Emscripten target, and
//! the render-sandbox page loads the resulting module onto a WebGL canvas.

mod common;

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::OnceLock;

/// `\0asm` followed by binary format version 1.
const WASM_HEADER: &[u8] = b"\0asm\x01\0\0\0";
/// The same header, base64-encoded, as Emscripten embeds it with `-sSINGLE_FILE`.
const WASM_HEADER_BASE64: &str = "AGFzbQEAAAA";

/// Runs `make render-wasm` once for every test in this file.
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

fn sandbox_dir() -> PathBuf {
    common::repo_root().join("render-sandbox")
}

fn sandbox_page() -> PathBuf {
    sandbox_dir().join("index.html")
}

fn read_text(path: &Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn is_path_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'/' | b'_' | b'-')
}

/// Relative paths written as quoted string literals that end in one of `extensions`.
fn quoted_paths(text: &str, extensions: &[&str]) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut paths = Vec::new();
    for (end, &quote) in bytes.iter().enumerate() {
        if !matches!(quote, b'"' | b'\'' | b'`') {
            continue;
        }
        let mut start = end;
        while start > 0 && is_path_byte(bytes[start - 1]) {
            start -= 1;
        }
        if start == 0 || start == end || bytes[start - 1] != quote {
            continue;
        }
        let path = &text[start..end];
        if extensions.iter().any(|extension| path.ends_with(extension)) {
            paths.push(path.to_string());
        }
    }
    paths
}

/// Resolves `reference`, as written in `from`, to an existing file inside render-sandbox/.
fn resolve(from: &Path, reference: &str) -> Option<PathBuf> {
    let root = sandbox_dir().canonicalize().ok()?;
    let candidate = match reference.strip_prefix('/') {
        Some(rooted) => root.join(rooted),
        None => from.parent()?.join(reference),
    };
    let path = candidate.canonicalize().ok()?;
    (path.is_file() && path.starts_with(&root)).then_some(path)
}

/// The local scripts the page loads, directly or through other scripts.
fn loader_chain(page: &Path) -> Vec<PathBuf> {
    let mut scripts: Vec<PathBuf> = Vec::new();
    let mut queue = VecDeque::from([page.to_path_buf()]);
    while let Some(file) = queue.pop_front() {
        for reference in quoted_paths(&read_text(&file), &[".js", ".mjs"]) {
            if let Some(script) = resolve(&file, &reference) {
                if !scripts.contains(&script) {
                    scripts.push(script.clone());
                    queue.push_back(script);
                }
            }
        }
    }
    scripts
}

/// Where the page's scripts load a WebAssembly module from, if they load one.
fn loaded_module(page: &Path, scripts: &[PathBuf]) -> Option<String> {
    for script in scripts {
        let text = read_text(script);
        for reference in quoted_paths(&text, &[".wasm"]) {
            for from in [script.as_path(), page] {
                let Some(module) = resolve(from, &reference) else {
                    continue;
                };
                let bytes = fs::read(&module).unwrap_or_default();
                if bytes.starts_with(WASM_HEADER) {
                    return Some(format!("{} ({} bytes)", module.display(), bytes.len()));
                }
            }
        }
        if text.contains(WASM_HEADER_BASE64) {
            return Some(format!("embedded in {}", script.display()));
        }
    }
    None
}

#[test]
fn make_render_wasm_builds_omni_render_for_wasm32_unknown_emscripten() {
    render_wasm();
    let commands = common::make_dry_run("render-wasm");
    assert!(
        commands.contains("wasm32-unknown-emscripten"),
        "`make render-wasm` must build for the wasm32-unknown-emscripten target; it runs:\n{commands}"
    );
    assert!(
        commands.contains("omni-render") || commands.contains("omni_render"),
        "`make render-wasm` must build the omni-render crate; it runs:\n{commands}"
    );
}

#[test]
fn sandbox_page_loads_the_wasm_module_built_by_make_render_wasm() {
    render_wasm();
    let page = sandbox_page();
    let scripts = loader_chain(&page);
    assert!(
        !scripts.is_empty(),
        "render-sandbox/index.html must load the module's JavaScript from render-sandbox/"
    );
    let module = loaded_module(&page, &scripts);
    assert!(
        module.is_some(),
        "none of the scripts render-sandbox/index.html loads ({scripts:?}) loads a WebAssembly module that exists after `make render-wasm`"
    );
}

#[test]
fn sandbox_page_shows_a_webgl_canvas() {
    render_wasm();
    let page = sandbox_page();
    let html = read_text(&page);
    assert!(
        html.to_lowercase().contains("<canvas"),
        "render-sandbox/index.html must contain a <canvas> element:\n{html}"
    );
    let scripts = loader_chain(&page);
    let uses_webgl = std::iter::once(html)
        .chain(scripts.iter().map(|script| read_text(script)))
        .any(|source| source.to_lowercase().contains("webgl"));
    assert!(
        uses_webgl,
        "the page or the scripts it loads ({scripts:?}) must draw through a WebGL context"
    );
}
