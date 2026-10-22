//! US-051: `omni_penpot::import::import_file` on the real exports
//! fixtures/penpot/tirebud-idag.penpot and tirebud-ordrar.penpot. The expected
//! numbers are read from the exports' own JSON in the tests, outside the
//! crate. See import_synthetic.rs for the rules of the import.
//!
//! - Both exports import. Their design system passes OmniState's checks, with
//!   the modes Mörkt and Ljust (Penpot's themes), and every token reference and
//!   every instance in the design finds its token or component.
//! - tirebud-idag.penpot: every board of the converter is a board of the page
//!   Idag; every Penpot color token is a token of the same path with Penpot's
//!   value in each theme's mode, and every spacing and radius token is a token
//!   too; every token linked to a fill or a stroke stays a reference. Its boards
//!   draw as close to Penpot's PNGs as the comparison of the converted boards
//!   showed (`compare::compare_import` beside `compare::compare_file`).
//! - tirebud-ordrar.penpot: each of its 322 components is one component of the
//!   design system, the variants with their properties; its copies are
//!   instances with the override kinds TireBud uses (text, fills, visibility
//!   and swapped components); its library colors and typographies are tokens.
//! - Every entry of the report about a shape names the page and board the
//!   shape is on, and a reason.
//! - tirebud-ordrar.penpot (US-057): its texts use Geist and Geist Mono, whose
//!   files sit beside the exports in fixtures/penpot/fonts/<family>/. They are
//!   attachments of the import, and every typography token of either family
//!   lists files of its family in `$extensions.omnistate.fontFiles`.
//!
//! The tests skip, with a message, when fixtures/penpot/ is empty.

mod common;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::OnceLock;

use omni_core::design::TokenRef;
use omni_core::design::tokens::TokenTree;
use omni_penpot::compare::{compare_file, compare_import};
use omni_penpot::import::Import;
use omni_penpot::{PenpotFile, convert_file, read_file};
use omni_render::system::DesignSystem;
use serde_json::Value;
use zip::ZipArchive;

use common::import::{
    component_by_id, components, import, is_named, json, resolve, rgba_of, token_tree, tokens_of,
    variant_default,
};
use common::{fixture, page_shapes};

const MODES: [&str; 2] = ["Mörkt", "Ljust"];

fn imported(name: &str) -> Option<Import> {
    fixture(name).map(|path| import(&path))
}

fn idag() -> Option<&'static Import> {
    static CELL: OnceLock<Option<Import>> = OnceLock::new();
    CELL.get_or_init(|| imported("tirebud-idag.penpot"))
        .as_ref()
}

fn ordrar() -> Option<&'static Import> {
    static CELL: OnceLock<Option<Import>> = OnceLock::new();
    CELL.get_or_init(|| imported("tirebud-ordrar.penpot"))
        .as_ref()
}

/// The JSON entries of the export at `path` whose names start with `prefix`
/// and end in `.json`, by name, as long as `keep` says so of the entry's name.
fn entries(path: &Path, prefix: &str, keep: impl Fn(&str) -> bool) -> BTreeMap<String, Value> {
    let mut archive = ZipArchive::new(File::open(path).expect("the export opens"))
        .expect("the export is a zip archive");
    let names: Vec<String> = archive
        .file_names()
        .filter_map(|name| name.ok().map(|name| name.to_string()))
        .filter(|name| name.starts_with(prefix) && name.ends_with(".json") && keep(name))
        .collect();
    let mut found = BTreeMap::new();
    for name in names {
        let mut text = String::new();
        archive
            .by_name(&name)
            .expect("a listed entry")
            .read_to_string(&mut text)
            .expect("the entry reads");
        found.insert(
            name.clone(),
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}")),
        );
    }
    found
}

/// The one JSON entry of the export whose name ends in `suffix`.
fn entry(path: &Path, suffix: &str) -> Value {
    let found = entries(path, "files/", |name| name.ends_with(suffix));
    assert_eq!(found.len(), 1, "one entry ends in {suffix}");
    found.into_values().next().expect("the entry")
}

/// Every token of a Penpot token set: (path, type, value).
fn penpot_tokens(set: &Value) -> Vec<(String, String, Value)> {
    let mut found = Vec::new();
    walk(set, "", &mut found);
    found
}

fn walk(node: &Value, path: &str, found: &mut Vec<(String, String, Value)>) {
    let Some(map) = node.as_object() else { return };
    if let Some(value) = map.get("$value") {
        let kind = map["$type"].as_str().unwrap_or("").to_string();
        found.push((path.to_string(), kind, value.clone()));
        return;
    }
    for (name, child) in map {
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        walk(child, &path, found);
    }
}

/// A Penpot color, `#rrggbb`, `#rgb` or `rgba(r, g, b, a)`, as `[r, g, b, a]`.
fn penpot_rgba(text: &str) -> [u8; 4] {
    if let Some(inner) = text.strip_prefix("rgba(").and_then(|t| t.strip_suffix(')')) {
        let parts: Vec<f64> = inner
            .split(',')
            .map(|part| part.trim().parse().expect("a number"))
            .collect();
        return [
            parts[0] as u8,
            parts[1] as u8,
            parts[2] as u8,
            (parts[3] * 255.0).round() as u8,
        ];
    }
    let hex = text.strip_prefix('#').expect("a hex color").to_lowercase();
    let hex: String = match hex.len() {
        3 => hex.chars().flat_map(|digit| [digit, digit]).collect(),
        _ => hex,
    };
    let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex");
    [byte(0), byte(2), byte(4), 255]
}

/// Every `$` reference of `value` in a place that holds a style value: not
/// in text content or names, so that "$5.00" is not taken for one.
fn references(value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if text.starts_with('$') => found.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| references(item, found)),
        Value::Object(map) => {
            for (key, item) in map {
                let skipped = [
                    "id",
                    "type",
                    "name",
                    "content",
                    "component",
                    "props",
                    "bind",
                    "path",
                    "children",
                ];
                if !skipped.contains(&key.as_str()) {
                    references(item, found);
                }
            }
            for child in map
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                references(child, found);
            }
        }
        _ => {}
    }
}

/// Every node of type `kind` in `value` and below it.
fn nodes_of<'a>(value: &'a Value, kind: &str, found: &mut Vec<&'a Value>) {
    if value["type"] == kind {
        found.push(value);
    }
    for child in value["children"].as_array().into_iter().flatten() {
        nodes_of(child, kind, found);
    }
}

/// Checks that the design system passes OmniState's checks with the modes of
/// TireBud, and that every instance and token reference finds its component
/// and token in every mode.
fn assert_resolves(import: &Import, what: &str) {
    DesignSystem::from_json(&import.design_system)
        .unwrap_or_else(|error| panic!("the design system of {what} is refused: {error}"));
    let tree: TokenTree = token_tree(import);
    assert_eq!(tree.modes(), MODES, "{what}");
    let mut found = Vec::new();
    let mut instances = Vec::new();
    let pages: Vec<Value> = import.pages.iter().map(json).collect();
    for page in &pages {
        references(page, &mut found);
        nodes_of(page, "instance", &mut instances);
    }
    for component in components(import) {
        references(component, &mut found);
        nodes_of(component, "instance", &mut instances);
    }
    for reference in &found {
        let token: TokenRef = reference
            .parse()
            .unwrap_or_else(|e| panic!("{what}: {reference} is no token reference: {e}"));
        for mode in MODES {
            assert!(
                tree.resolve(&token, mode).is_some(),
                "{what}: {reference} is no token in {mode}"
            );
        }
    }
    let ids: HashSet<&str> = components(import)
        .into_iter()
        .filter_map(|component| component["id"].as_str())
        .collect();
    for instance in instances {
        let component = instance["component"].as_str().unwrap_or("");
        assert!(
            ids.contains(component),
            "{what}: {instance} places no component"
        );
        for change in instance["overrides"]
            .as_object()
            .into_iter()
            .flat_map(|o| o.values())
        {
            if let Some(swapped) = change["component"].as_str() {
                assert!(
                    ids.contains(swapped),
                    "{what}: {instance} swaps in no component"
                );
            }
        }
    }
}

/// Checks that every report entry about a shape of `file` names the page and
/// board the shape is on, and a reason.
fn assert_report_places(import: &Import, file: &PenpotFile, what: &str) {
    let mut page_of: HashMap<&str, &str> = HashMap::new();
    for page in &file.pages {
        for shape in page_shapes(page) {
            page_of.insert(&shape.id, &page.name);
        }
    }
    let mut about_shapes = 0;
    for entry in &import.report {
        let Some(page) = page_of.get(entry.shape_id.as_str()) else {
            continue;
        };
        about_shapes += 1;
        assert_eq!(&entry.page, page, "{what}: {entry:?}");
        assert!(!entry.board.is_empty(), "{what}: {entry:?}");
        assert!(!entry.reason.is_empty(), "{what}: {entry:?}");
    }
    assert!(about_shapes > 0, "{what} lists shapes it leaves out");
}

#[test]
fn idag_imports_every_board_onto_the_page_idag() {
    let (Some(import), Some(path)) = (idag(), fixture("tirebud-idag.penpot")) else {
        return;
    };
    let conversion = convert_file(&path).expect("the export converts");
    assert_eq!(import.pages.len(), 1);
    let page = json(&import.pages[0]);
    assert_eq!(page["name"], "Idag");
    let boards: Vec<&str> = page["children"]
        .as_array()
        .expect("boards")
        .iter()
        .filter_map(|board| board["name"].as_str())
        .collect();
    let converted: Vec<&str> = conversion
        .boards
        .iter()
        .map(|board| board.name.as_str())
        .collect();
    assert_eq!(boards, converted);
    assert_eq!(boards.len(), 16, "15 boards and Unplaced");
    assert!(
        components(import).is_empty(),
        "the idag export has no components"
    );
    assert_resolves(import, "idag");
}

#[test]
fn idag_tokens_keep_their_values_in_the_modes_of_their_themes() {
    let (Some(import), Some(path)) = (idag(), fixture("tirebud-idag.penpot")) else {
        return;
    };
    let tokens = entry(&path, "/tokens.json");
    let tree = token_tree(import);
    assert_eq!(tree.modes(), MODES);
    let mut colors = 0;
    for mode in MODES {
        for (path, kind, value) in penpot_tokens(&tokens[mode]) {
            assert_eq!(kind, "color", "{mode} holds colors");
            let found = resolve(&tree, &path, mode)
                .unwrap_or_else(|| panic!("{path} is a token in {mode}"));
            assert_eq!(
                rgba_of(&found),
                Some(penpot_rgba(value.as_str().expect("a color"))),
                "{path} in {mode}: Penpot says {value}, the import {found}"
            );
            colors += 1;
        }
    }
    assert_eq!(colors, 2 * 68);
    let all = tokens_of(&import.design_system);
    let mut dimensions = 0;
    for (path, kind, value) in penpot_tokens(&tokens["Bas"]) {
        let group = match kind.as_str() {
            "spacing" => "spacing",
            "borderRadius" => "radius",
            _ => continue,
        };
        let found: Vec<&String> = all
            .iter()
            .map(|(tree_path, _)| tree_path)
            .filter(|tree_path| {
                tree_path.starts_with(&format!("{group}.")) && tree_path.ends_with(&path)
            })
            .collect();
        assert_eq!(found.len(), 1, "one {group} token for {path}");
        let expected = format!("{}px", value.as_str().expect("a number as text"));
        for mode in MODES {
            assert_eq!(
                resolve(&tree, found[0], mode),
                Some(Value::String(expected.clone())),
                "{path} in {mode}"
            );
        }
        dimensions += 1;
    }
    assert_eq!(dimensions, 13 + 6, "13 spacings and 6 radii");
}

#[test]
fn idag_token_links_on_fills_and_strokes_stay_references() {
    let (Some(import), Some(path)) = (idag(), fixture("tirebud-idag.penpot")) else {
        return;
    };
    let left_out: HashSet<&str> = import
        .report
        .iter()
        .filter(|entry| entry.property.is_none())
        .map(|entry| entry.shape_id.as_str())
        .collect();
    let mut links = 0;
    let shapes = entries(&path, "files/", |name| {
        name.contains("/pages/") && name.matches('/').count() == 4
    });
    for shape in shapes.values() {
        let id = shape["id"].as_str().unwrap_or("");
        if left_out.contains(id) {
            continue;
        }
        let applied = &shape["appliedTokens"];
        let has = |key: &str| !shape[key].as_array().is_none_or(Vec::is_empty);
        if applied.get("fill").is_some() && (shape["type"] == "text" || has("fills")) {
            links += 1;
        }
        if applied.get("strokeColor").is_some() && has("strokes") {
            links += 1;
        }
    }
    assert_eq!(links, 1_779, "798 text fills, 458 fills and 523 strokes");
    let mut found = Vec::new();
    for page in &import.pages {
        references(&json(page), &mut found);
    }
    let colors = found
        .iter()
        .filter(|reference| reference.starts_with("$color."))
        .count();
    assert!(
        colors >= links,
        "{colors} color references for {links} token links on fills and strokes"
    );
}

#[test]
fn idag_boards_match_penpot_pngs_as_closely_as_the_comparison_showed() {
    let Some(path) = fixture("tirebud-idag.penpot") else {
        return;
    };
    if !common::repo_root()
        .join("fixtures/penpot/tirebud-idag")
        .is_dir()
    {
        eprintln!("SKIPPED: fixtures/penpot/tirebud-idag/ (Penpot's PNGs) is not there");
        return;
    }
    let converted = compare_file(&path).expect("the converted boards compare");
    let imported = compare_import(&path).expect("the imported boards compare");
    let by_id: HashMap<&str, _> = imported
        .boards
        .iter()
        .map(|board| (board.board_id.as_str(), board))
        .collect();
    assert_eq!(imported.boards.len(), converted.boards.len());
    let mut compared = 0;
    for before in &converted.boards {
        let after = by_id
            .get(before.board_id.as_str())
            .unwrap_or_else(|| panic!("the board {} is compared", before.board));
        assert!(
            after.layout.mismatches.len() <= before.layout.mismatches.len(),
            "{}: {} layout mismatches imported, {} converted",
            before.board,
            after.layout.mismatches.len(),
            before.layout.mismatches.len()
        );
        let Some(pixel) = &before.pixel else { continue };
        let imported_pixel = after
            .pixel
            .as_ref()
            .unwrap_or_else(|| panic!("{} has a PNG", before.board));
        assert!(
            imported_pixel.share() <= pixel.share() + 0.001,
            "{}: {:.4} of the pixels differ imported, {:.4} converted",
            before.board,
            imported_pixel.share(),
            pixel.share()
        );
        compared += 1;
    }
    assert_eq!(compared, 15, "the 15 boards with a PNG");
}

#[test]
fn ordrar_components_and_their_variants_become_design_system_components() {
    let (Some(import), Some(path)) = (ordrar(), fixture("tirebud-ordrar.penpot")) else {
        return;
    };
    let penpot: Vec<Value> = entries(&path, "files/", |name| name.contains("/components/"))
        .into_values()
        .collect();
    assert_eq!(penpot.len(), 322);
    assert_eq!(
        components(import).len(),
        322,
        "one component per Penpot component"
    );
    let mut sets: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for component in &penpot {
        if let Some(set) = component["variantId"].as_str() {
            sets.entry(set.to_string()).or_default().push(component);
        }
    }
    assert_eq!(sets.len(), 41, "41 variant sets");
    for component in &penpot {
        let name = component["name"].as_str().expect("a name");
        let path = component["path"].as_str().unwrap_or("");
        let properties: Vec<(&str, &str)> = component["variantProperties"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|property| {
                (
                    property["name"].as_str().expect("a property name"),
                    property["value"].as_str().expect("a property value"),
                )
            })
            .collect();
        let matching: Vec<&Value> = components(import)
            .into_iter()
            .filter(|candidate| is_named(candidate, name, path))
            .filter(|candidate| {
                properties
                    .iter()
                    .all(|(property, value)| variant_default(candidate, property) == Some(value))
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "one component is {path}/{name} {properties:?}"
        );
        let Some(set) = component["variantId"].as_str() else {
            continue;
        };
        for (property, _) in &properties {
            let all: BTreeSet<&str> = sets[set]
                .iter()
                .flat_map(|variant| {
                    variant["variantProperties"]
                        .as_array()
                        .into_iter()
                        .flatten()
                })
                .filter(|other| other["name"] == *property)
                .filter_map(|other| other["value"].as_str())
                .collect();
            let values: BTreeSet<&str> = matching[0]["props"][*property]["values"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            assert!(
                values.is_superset(&all),
                "{name}: {property} offers {values:?}, the set has {all:?}"
            );
        }
    }
}

#[test]
fn ordrar_copies_become_instances_with_the_override_kinds_tirebud_uses() {
    let Some(import) = ordrar() else { return };
    assert_resolves(import, "ordrar");
    let mut instances = Vec::new();
    let pages: Vec<Value> = import.pages.iter().map(json).collect();
    for page in &pages {
        nodes_of(page, "instance", &mut instances);
    }
    assert!(
        instances.len() > 1_000,
        "only {} instances",
        instances.len()
    );
    let mut kinds = BTreeSet::new();
    for instance in &instances {
        component_by_id(import, instance["component"].as_str().unwrap_or(""));
        for change in instance["overrides"]
            .as_object()
            .into_iter()
            .flat_map(|o| o.values())
        {
            for field in change.as_object().into_iter().flat_map(|c| c.keys()) {
                kinds.insert(field.clone());
            }
        }
    }
    assert_eq!(
        kinds,
        BTreeSet::from(["component", "content", "fills", "visible"].map(String::from)),
        "text, fills, visibility and swapped components"
    );
    let variants = components(import)
        .into_iter()
        .filter(|component| {
            component["props"]
                .as_object()
                .is_some_and(|props| props.values().any(|def| def["type"] == "variant"))
        })
        .count();
    assert_eq!(variants, 172, "the 172 variants keep their properties");
}

#[test]
fn ordrar_library_colors_and_typographies_become_tokens() {
    let (Some(import), Some(path)) = (ordrar(), fixture("tirebud-ordrar.penpot")) else {
        return;
    };
    let tree = token_tree(import);
    let all = tokens_of(&import.design_system);
    let tokens = entry(&path, "/tokens.json");
    let token_colors: HashSet<String> = penpot_tokens(&tokens["Mörkt"])
        .into_iter()
        .map(|(path, _, _)| path)
        .collect();
    let colors = entries(&path, "files/", |name| name.contains("/colors/"));
    assert_eq!(colors.len(), 33);
    for color in colors.values() {
        let name = color["name"].as_str().expect("a name");
        let opacity = color["opacity"].as_f64().unwrap_or(1.0);
        let mut expected = penpot_rgba(color["color"].as_str().expect("a color"));
        expected[3] = (opacity * 255.0).round() as u8;
        let token = all.iter().map(|(path, _)| path).find(|path| {
            path.starts_with("color.")
                && path.to_lowercase().ends_with(&name.to_lowercase())
                && MODES.iter().all(|mode| {
                    resolve(&tree, path, mode).and_then(|value| rgba_of(&value)) == Some(expected)
                })
        });
        // A library color named like a color token that differs between the
        // themes may be listed instead, but never dropped silently.
        let collides = token_colors.contains(&format!("color.{name}"));
        let reported = import
            .report
            .iter()
            .any(|entry| entry.name == name && !entry.reason.is_empty());
        assert!(
            token.is_some() || (collides && reported),
            "the library color {name} is a token with its value in every mode"
        );
    }
    let typographies = entries(&path, "files/", |name| name.contains("/typographies/"));
    assert_eq!(typographies.len(), 13);
    let styles: Vec<Value> = all
        .iter()
        .filter(|(path, _)| path.starts_with("typography."))
        .filter_map(|(path, _)| resolve(&tree, path, MODES[0]))
        .collect();
    for typography in typographies.values() {
        let number = |key: &str| -> f64 {
            typography[key]
                .as_str()
                .and_then(|text| text.parse().ok())
                .expect("a number as text")
        };
        let px =
            |value: &Value| -> Option<f64> { value.as_str()?.strip_suffix("px")?.parse().ok() };
        let weight = |value: &Value| -> Option<f64> {
            value.as_f64().or_else(|| value.as_str()?.parse().ok())
        };
        let found = styles.iter().any(|style| {
            style["fontFamily"] == typography["fontFamily"]
                && px(&style["fontSize"]) == Some(number("fontSize"))
                && weight(&style["fontWeight"]) == Some(number("fontWeight"))
                && style["lineHeight"].as_f64() == Some(number("lineHeight"))
                && px(&style["letterSpacing"]).unwrap_or(0.0) == number("letterSpacing")
        });
        assert!(found, "a typography token for {}", typography["name"]);
    }
}

#[test]
fn every_report_entry_about_a_shape_names_its_page_board_and_reason() {
    for name in ["tirebud-idag.penpot", "tirebud-ordrar.penpot"] {
        let Some(path) = fixture(name) else { continue };
        let import = match name {
            "tirebud-idag.penpot" => idag(),
            _ => ordrar(),
        }
        .expect("the export is there");
        let file = read_file(&path).expect("the export reads");
        assert_report_places(import, &file, name);
    }
}

#[test]
fn ordrar_typography_tokens_list_the_geist_fonts_beside_the_export() {
    let Some(path) = fixture("tirebud-ordrar.penpot") else {
        return;
    };
    let import = ordrar().expect("the export is there");
    let fonts = path.with_file_name("fonts");
    let files_of = |family: &str| -> Vec<Vec<u8>> {
        let mut files: Vec<_> = std::fs::read_dir(fonts.join(family))
            .unwrap_or_else(|error| panic!("fixtures/penpot/fonts/{family} is there: {error}"))
            .flatten()
            .map(|entry| entry.path())
            .filter(|file| file.extension().is_some_and(|ext| ext == "ttf"))
            .collect();
        files.sort();
        files
            .iter()
            .map(|file| std::fs::read(file).expect("the font file reads"))
            .collect()
    };
    let tree = token_tree(import);
    let all = tokens_of(&import.design_system);
    for family in ["Geist", "Geist Mono"] {
        let files = files_of(family);
        let typographies: Vec<&(String, Value)> = all
            .iter()
            .filter(|(path, _)| path.starts_with("typography."))
            .filter(|(path, _)| {
                resolve(&tree, path, MODES[0]).is_some_and(|value| value["fontFamily"] == family)
            })
            .collect();
        assert!(
            !typographies.is_empty(),
            "Ordrar has typography tokens in {family}"
        );
        for (path, token) in typographies {
            let listed = token["$extensions"]["omnistate"]["fontFiles"]
                .as_array()
                .unwrap_or_else(|| panic!("{path} lists its font files: {token}"));
            assert!(!listed.is_empty(), "{path} lists its font files: {token}");
            for reference in listed {
                let id = reference
                    .as_str()
                    .and_then(|text| text.strip_prefix("attachment:"))
                    .unwrap_or_else(|| panic!("{path} lists {reference}, no attachment"));
                let attachment = import
                    .attachments
                    .iter()
                    .find(|attachment| attachment.id == id)
                    .unwrap_or_else(|| {
                        panic!("{reference} of {path} is an attachment of the import")
                    });
                assert!(
                    files.contains(&attachment.bytes),
                    "{reference} of {path} is a file of fixtures/penpot/fonts/{family}"
                );
            }
        }
    }
}
