//! US-032: the WebAssembly module draws instances and tokens for the host
//! page and reports what is missing, a sample page in `render-sandbox/` shows
//! a design with instances in light and in dark mode for the human check, and
//! the README documents instances and tokens.
//!
//! The logic is native Rust and covered by `instances.rs`; these are the
//! exports that forward it to the browser, which `make render-wasm` must
//! build:
//!
//! - `omni_render_scene_set_system(system, len) -> bool`: reads a design
//!   system version (`DesignSystem::from_json`: a DTCG token tree with the
//!   components under `$components`) and makes the loaded scene, and every
//!   scene loaded after it, resolve its instances and token references
//!   against it, in its first mode. `false` if it is not a design system
//!   version (`omni_render_error` has the reason).
//! - `omni_render_scene_set_mode(mode, len) -> bool`: `Scene::set_mode` for the
//!   scene of the page; `false` for a mode the design system does not list.
//! - `omni_render_scene_errors() -> *const c_char`: `Scene::errors` as a
//!   NUL-ended JSON array that stays until the next call: `[{"node":
//!   "<address>", "component": "<id>"}, {"node": "<address>", "token":
//!   "$color.surface"}]`, one entry per node drawn as an error placeholder.
//!
//! The sample page is `render-sandbox/instances.html`. It draws the sample
//! design `render-sandbox/instances.json` (a page) with the design system
//! `render-sandbox/instances-system.json` through those exports and
//! `omni_render_scene_load` and `omni_render_scene_draw`, lets the reviewer
//! switch between light and dark mode, and shows the errors the module
//! reports. The sample uses every kind of property (variant, boolean, text
//! and slot) and overrides, and draws in both modes with nothing missing.
//!
//! What no test can see, the human check covers: that the sample design looks
//! right in light and in dark mode.

mod common;

use omni_core::design::Node;
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::{Scene, Viewport};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::OnceLock;

fn sandbox_dir() -> PathBuf {
    common::repo_root().join("render-sandbox")
}

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"))
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&read_text(path))
        .unwrap_or_else(|error| panic!("{path:?} must be JSON, but: {error}"))
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

/// The values of the `src` attributes in `html`.
fn script_sources(html: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("src=") {
        rest = &rest[at + 4..];
        let Some(quote) = rest.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
            continue;
        };
        if let Some(end) = rest[1..].find(quote) {
            sources.push(rest[1..1 + end].to_string());
        }
    }
    sources
}

/// Relative paths written as quoted strings in `text` that end in one of
/// `extensions`.
fn quoted_paths(text: &str, extensions: &[&str]) -> Vec<String> {
    let mut paths = Vec::new();
    for quote in ['"', '\'', '`'] {
        for (index, piece) in text.split(quote).enumerate() {
            if index % 2 == 1
                && !piece.is_empty()
                && !piece.contains(char::is_whitespace)
                && extensions
                    .iter()
                    .any(|extension| piece.ends_with(extension))
            {
                paths.push(piece.to_string());
            }
        }
    }
    paths
}

/// The text of `instances.html` and of every local script it loads, directly
/// or through other scripts, joined.
fn page_source() -> String {
    let page = sandbox_dir().join("instances.html");
    let html = read_text(&page);
    let mut all = html.clone();
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue: Vec<(PathBuf, String)> = script_sources(&html)
        .into_iter()
        .map(|source| (page.clone(), source))
        .collect();
    while let Some((from, reference)) = queue.pop() {
        let Some(directory) = from.parent() else {
            continue;
        };
        let Ok(path) = directory.join(&reference).canonicalize() else {
            continue;
        };
        if !path.is_file() || !path.starts_with(sandbox_dir()) || !seen.insert(path.clone()) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for next in quoted_paths(&text, &[".js", ".mjs"]) {
            queue.push((path.clone(), next));
        }
        all.push('\n');
        all.push_str(&text);
    }
    all
}

#[test]
fn the_wasm_module_exports_the_design_system_functions_the_sample_page_calls() {
    render_wasm();
    let loader = read_text(&sandbox_dir().join("build/omni_render.js"));
    for export in [
        "_omni_render_scene_set_system",
        "_omni_render_scene_set_mode",
        "_omni_render_scene_errors",
    ] {
        assert!(
            loader.contains(export),
            "render-sandbox/build/omni_render.js must export `{export}` after `make render-wasm`"
        );
    }
}

#[test]
fn the_sample_page_draws_the_sample_design_in_light_and_dark_mode_and_shows_errors() {
    let source = page_source();
    for (needle, why) in [
        ("<canvas", "draw on a canvas"),
        ("instances.json", "load the sample design"),
        ("instances-system.json", "load the sample design system"),
        (
            "_omni_render_scene_load",
            "load the sample design into the scene",
        ),
        (
            "_omni_render_scene_set_system",
            "hand the design system to the module",
        ),
        ("_omni_render_scene_draw", "draw the scene"),
        ("_omni_render_scene_set_mode", "switch the mode"),
        ("light", "offer light mode"),
        ("dark", "offer dark mode"),
        (
            "_omni_render_scene_errors",
            "pick up the errors the module reports, and show them",
        ),
    ] {
        assert!(
            source.contains(needle),
            "render-sandbox/instances.html or the scripts it loads must {why} (`{needle}`)"
        );
    }
}

/// Every node of `json` of type `instance`, at any depth.
fn instances_in(json: &Value) -> Vec<&Value> {
    let mut found = Vec::new();
    let mut pending = vec![json];
    while let Some(node) = pending.pop() {
        if node["type"] == "instance" {
            found.push(node);
        }
        if let Some(children) = node.get("children").and_then(Value::as_array) {
            pending.extend(children);
        }
    }
    found
}

#[test]
fn the_sample_design_uses_every_kind_of_property_and_overrides() {
    let page = read_json(&sandbox_dir().join("instances.json"));
    let system = read_json(&sandbox_dir().join("instances-system.json"));
    let components: HashMap<&str, &Value> = system["$components"]
        .as_array()
        .expect("the sample design system lists its components under `$components`")
        .iter()
        .filter_map(|component| Some((component["id"].as_str()?, component)))
        .collect();
    let instances = instances_in(&page);
    assert!(!instances.is_empty(), "the sample design places instances");
    let mut kinds = BTreeSet::new();
    for instance in &instances {
        let component = instance["component"].as_str().unwrap_or_default();
        let definitions = components
            .get(component)
            .unwrap_or_else(|| panic!("the sample design system has the component {component}"));
        if let Some(props) = instance.get("props").and_then(Value::as_object) {
            for name in props.keys() {
                if let Some(kind) = definitions["props"][name]["type"].as_str() {
                    kinds.insert(kind.to_string());
                }
            }
        }
    }
    for kind in ["variant", "boolean", "text", "slot"] {
        assert!(
            kinds.contains(kind),
            "an instance of the sample design sets a {kind} property; the kinds set are {kinds:?}"
        );
    }
    assert!(
        instances
            .iter()
            .any(|instance| instance.get("overrides").is_some()),
        "an instance of the sample design has overrides"
    );
}

#[test]
fn the_sample_design_draws_in_light_and_dark_mode_with_nothing_missing() {
    let page: Node = serde_json::from_value(read_json(&sandbox_dir().join("instances.json")))
        .unwrap_or_else(|error| panic!("instances.json must be a design page, but: {error}"));
    let system = DesignSystem::from_json(&read_json(&sandbox_dir().join("instances-system.json")))
        .unwrap_or_else(|error| {
            panic!("instances-system.json must be a design system version, but: {error}")
        });
    let modes = system.modes().to_vec();
    for mode in ["light", "dark"] {
        assert!(
            modes.iter().any(|listed| listed == mode),
            "the sample design system has a {mode} mode; it has {modes:?}"
        );
    }
    let mut scene = Scene::with_system(page, Assets::new(), system);
    let bounds = scene.bounds();
    let viewport = Viewport::at(
        bounds.x,
        bounds.y,
        1.0,
        bounds.width.clamp(1.0, 2000.0),
        bounds.height.clamp(1.0, 2000.0),
    );
    let mut frames = Vec::new();
    for mode in ["light", "dark"] {
        scene
            .set_mode(mode)
            .unwrap_or_else(|error| panic!("{mode} is a mode, but: {error}"));
        let (_, rgba) = scene.render_rgba(&viewport, 1.0);
        assert!(
            scene.errors().is_empty(),
            "the sample design misses nothing in {mode} mode, but the renderer reports {:?}",
            scene.errors()
        );
        frames.push(rgba);
    }
    let differing = frames[0]
        .chunks_exact(4)
        .zip(frames[1].chunks_exact(4))
        .filter(|(light, dark)| light != dark)
        .count();
    assert!(
        differing * 10 > frames[0].len() / 4,
        "the sample design looks different in dark mode: at least a tenth of its pixels change, but {differing} of {} do",
        frames[0].len() / 4
    );
}

/// The lines of the README section whose heading contains `title` (case
/// insensitive), up to the next heading of the same or a higher level.
fn readme_section(readme: &str, title: &str) -> Option<String> {
    let mut section: Option<(usize, Vec<&str>)> = None;
    for line in readme.lines() {
        let level = line.chars().take_while(|&c| c == '#').count();
        let is_heading = level > 0 && line[level..].starts_with(' ');
        match (&mut section, is_heading) {
            (None, true) if line.to_lowercase().contains(title) => {
                section = Some((level, vec![line]));
            }
            (Some((start, _)), true) if level <= *start => break,
            (Some((_, lines)), _) => lines.push(line),
            _ => {}
        }
    }
    section.map(|(_, lines)| lines.join("\n"))
}

#[test]
fn the_readme_documents_instances_and_tokens() {
    let readme = read_text(&common::repo_root().join("crates/omni-render/README.md"));
    assert!(
        !readme.contains("out of scope until US-032"),
        "the README must no longer say that instances and tokens are out of scope"
    );
    let section = readme_section(&readme, "instances and tokens").unwrap_or_else(|| {
        panic!("crates/omni-render/README.md needs an \"Instances and tokens\" section")
    });
    for (needle, why) in [
        ("variant", "how a variant value picks the variant of a set"),
        ("override", "how overrides apply"),
        ("mode", "how the mode is chosen and switched"),
        ("placeholder", "the error placeholder"),
        (
            "<instanceId>/<componentNodeId>",
            "addresses inside instances",
        ),
        ("instances.html", "the sample page for the human check"),
    ] {
        assert!(
            section.contains(needle),
            "the Instances and tokens section must describe {why} (`{needle}`):\n{section}"
        );
    }
}
