//! US-051: `omni_penpot::import::import_file` on the real exports
//! fixtures/penpot/tirebud-idag.penpot and tirebud-ordrar.penpot. The expected
//! numbers are read from the exports' own JSON in the tests, outside the
//! crate. See import_synthetic.rs for the rules of the import.
//!
//! - tirebud-idag.penpot: its boards draw as close to Penpot's PNGs as the
//!   comparison of the converted boards showed (`compare::compare_import`
//!   beside `compare::compare_file`).
//! - tirebud-ordrar.penpot imports. Its design system passes OmniState's
//!   checks, with the modes Mörkt and Ljust (Penpot's themes), and every token
//!   reference and every instance in the design finds its token or component.
//!   Each of its 322 components is one component of the design system, the
//!   variants with their properties; its copies are instances with the
//!   override kinds TireBud uses (text, fills, visibility and swapped
//!   components).
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
use omni_core::design::tokens::{ModeChoice, TokenTree};
use omni_penpot::compare::{compare_file, compare_import};
use omni_penpot::import::Import;
use omni_render::system::DesignSystem;
use serde_json::Value;
use zip::ZipArchive;

use common::fixture;
use common::import::{
    component_by_id, components, import, is_named, json, token_tree, variant_default,
};

const MODES: [&str; 2] = ["Mörkt", "Ljust"];

fn imported(name: &str) -> Option<Import> {
    fixture(name).map(|path| import(&path))
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
                ModeChoice::parse(mode, &tree)
                    .ok()
                    .and_then(|choice| tree.resolve(&token, &choice))
                    .is_some(),
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
