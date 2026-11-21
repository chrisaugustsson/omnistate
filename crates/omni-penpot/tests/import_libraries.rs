//! Copies of the components of a shared library (`omni import penpot`, docs/formats/penpot-export.md,
//! "Libraries"): a Penpot file that uses a library published apart from it
//! imports as instances of the library's components, not as plain shapes.
//!
//! The contract these tests pin down:
//!
//! - Every imported component keeps its Penpot file and component in its
//!   `origin`, with the node each layer of Penpot's component became, and the
//!   origin survives the trip through `PUT .../components`, which names the
//!   nodes anew.
//! - Imported onto a design system that has a library's components from
//!   Penpot, a file whose copies name that library (`componentFile`) places
//!   them as instances of those components, with the overrides keyed by their
//!   nodes, and imports no component of its own for them.
//! - Importing a file whose components the design system already has adds
//!   none again; a component without a Penpot origin stays as it is. A file
//!   duplicated from the library keeps the library's component ids: a
//!   component of it with the same id and the same content is the design
//!   system's, and one that changed is imported as the file's own.
//! - An export that bundles the library ("Export file + libraries") and goes
//!   onto a design system without it imports each library component its
//!   copies place once, and no other; the manifest may list the library first.
//! - A copy of a library that is neither bundled nor in the design system is
//!   drawn as the plain shapes it holds, and the report names the library.

mod common;

use common::convert::{FILE_ID, paragraph, run, shape, text_content, uid};
use common::import::{
    C_BELL, C_PRIMARY, C_SECONDARY, C_STAR, button_copy, components, find_json, import, json,
    library_entries_all, library_export, n, named, page, page_of, set, solid,
};
use common::{export_entries, stored_zip, write_temp};
use omni_core::design::NodeId;
use omni_core::design::tags;
use omni_penpot::import::{Ids, Import, import_file_onto};
use serde_json::{Value, json};
use std::path::PathBuf;

const AREA_ID: &str = "f0000000-0000-4000-8000-0000000000a1";
const AREA_PAGE: &str = "a0000000-0000-4000-8000-0000000000a1";

/// The shape numbers of the area file.
const KASSA: u32 = 200;
const KOP: u32 = 210;
const KOP_IGEN: u32 = 220;

// The server --------------------------------------------------------------------------------

/// What `PUT .../components` does with the tags it is sent: it keeps the IDs
/// the tags give and names every other node, and answers with the components
/// as JSON.
struct Server {
    next: u64,
}

impl Server {
    fn new() -> Server {
        Server { next: 0 }
    }

    fn id(&mut self) -> NodeId {
        const ALPHABET: &[u8] = b"0123456789abcdefghjkmnpqrstvwxyz";
        self.next += 1;
        let mut left = self.next;
        let mut id = *b"s0000000";
        for slot in id[1..].iter_mut().rev() {
            *slot = ALPHABET[(left % 32) as usize];
            left /= 32;
        }
        String::from_utf8_lossy(&id).parse().expect("a node ID")
    }

    /// Sends the components of `import` as the CLI does, as often as it takes
    /// for everything to be named, and returns them as the server stored them.
    fn put_components(&mut self, import: &Import, ids: &mut Ids) -> Vec<Value> {
        for _ in 0..8 {
            let rendered = import.component_tags(ids).expect("the components render");
            let stored: Vec<Value> = tags::parse_all(&rendered.tags)
                .unwrap_or_else(|error| panic!("the server refuses the tags: {error}"))
                .into_iter()
                .map(|root| {
                    let node = root
                        .try_assign_ids(|| self.id())
                        .expect("the server names the nodes");
                    serde_json::to_value(node).expect("a node serializes")
                })
                .collect();
            let answer = json!({ "components": stored }).to_string();
            import
                .learn_components(ids, &rendered, &answer)
                .expect("the answer is understood");
            if rendered.complete {
                return stored;
            }
        }
        panic!("the components were not complete after 8 passes");
    }
}

/// The import onto a design system whose head holds `kept` and no tokens.
fn onto(import: &Import, kept: &[Value], ids: &mut Ids) -> Import {
    import
        .onto_design_system(
            &json!({ "tokens": {} }).to_string(),
            &json!({ "version": { "components": kept } }).to_string(),
            ids,
        )
        .expect("the import goes onto the design system")
}

/// The library imported and sent to a design system of its own: the
/// components as the server stored them.
fn library_in_design_system(test: &str) -> Vec<Value> {
    let library = import(&library_export(test));
    Server::new().put_components(&library, &mut Ids::default())
}

/// The component of `components` that came from the Penpot component `id`.
fn from_penpot<'a>(components: &'a [Value], id: &str) -> &'a Value {
    components
        .iter()
        .find(|component| component["origin"]["component"] == id)
        .unwrap_or_else(|| panic!("no component came from {id}"))
}

/// The node the layer `n` of a Penpot component became in `component`.
fn layer(component: &Value, n: u32) -> String {
    component["origin"]["layers"][uid(n)]
        .as_str()
        .unwrap_or_else(|| panic!("the origin names no node for layer {n}: {component}"))
        .to_string()
}

// The area file -----------------------------------------------------------------------------

/// The page Kassa of the area file: the board Kassa holds two copies of the
/// library's Button / Primär, Köp untouched and Köp igen whose Label says
/// "Kör igen".
fn area_page() -> common::SyntheticPage {
    let kop = button_copy(KOP, n::PRIMARY, C_PRIMARY, "#01B597", "Köp", [20.0, 20.0]);
    let mut again = button_copy(
        KOP_IGEN,
        n::PRIMARY,
        C_PRIMARY,
        "#01B597",
        "Köp igen",
        [20.0, 80.0],
    );
    set(
        &mut again.kids[0],
        json!({
            "touched": ["text-content-text"],
            "content": text_content(vec![paragraph(vec![run(
                "Kör igen",
                json!({ "fontSize": "14", "fills": [{ "fillColor": "#F4F4F5", "fillOpacity": 1 }] }),
            )])])
        }),
    );
    let kassa = shape(KASSA, "frame", "Kassa", [0.0, 0.0, 400.0, 200.0])
        .with(json!({ "fills": solid("#131316") }))
        .kids(vec![kop, again]);
    page_of(AREA_PAGE, "Kassa", 0, vec![kassa])
}

/// The archive entries of the area file alone.
fn area_entries() -> Vec<(String, Vec<u8>)> {
    export_entries(AREA_ID, "TireBud - Kassa", &[area_page()])
}

/// The area file with the board Kassa holding `copies`, exported with its
/// library.
fn bundled_with(test: &str, copies: Vec<common::convert::S>) -> PathBuf {
    let kassa = shape(KASSA, "frame", "Kassa", [0.0, 0.0, 400.0, 200.0])
        .with(json!({ "fills": solid("#131316") }))
        .kids(copies);
    let page = page_of(AREA_PAGE, "Kassa", 0, vec![kassa]);
    let mut entries = export_entries(AREA_ID, "TireBud - Kassa", &[page]);
    entries.retain(|(name, _)| name != "manifest.json");
    entries.extend(
        library_entries_all()
            .into_iter()
            .filter(|(name, _)| name != "manifest.json"),
    );
    let manifest = json!({
        "type": "penpot/export-files", "version": 1,
        "files": [
            { "id": AREA_ID, "name": "TireBud - Kassa", "features": [] },
            { "id": FILE_ID, "name": "TireBud - Syntetisk", "features": [] }
        ],
        "relations": [[AREA_ID, FILE_ID]]
    });
    entries.insert(
        0,
        (
            "manifest.json".to_string(),
            manifest.to_string().into_bytes(),
        ),
    );
    write_temp(
        &format!("bundled-with-{test}.penpot"),
        &stored_zip(&entries),
    )
}

/// A copy of Button / Primär numbered from `copy`, at `y`.
fn primary_copy(copy: u32, name: &str, y: f64) -> common::convert::S {
    button_copy(copy, n::PRIMARY, C_PRIMARY, "#01B597", name, [20.0, y])
}

/// The area file exported alone.
fn area_export(test: &str) -> PathBuf {
    write_temp(&format!("area-{test}.penpot"), &stored_zip(&area_entries()))
}

/// The area file exported with its library ("Export file + libraries"); the
/// manifest lists the library first.
fn bundled_export(test: &str) -> PathBuf {
    let mut entries: Vec<(String, Vec<u8>)> = area_entries()
        .into_iter()
        .filter(|(name, _)| name != "manifest.json")
        .collect();
    entries.extend(
        library_entries_all()
            .into_iter()
            .filter(|(name, _)| name != "manifest.json"),
    );
    let manifest = json!({
        "type": "penpot/export-files",
        "version": 1,
        "generatedBy": "penpot/2.19.0-RC1",
        "referer": "penpot",
        "files": [
            { "id": FILE_ID, "name": "TireBud - Syntetisk", "features": [] },
            { "id": AREA_ID, "name": "TireBud - Kassa", "features": [] }
        ],
        "relations": [[AREA_ID, FILE_ID]]
    });
    entries.insert(
        0,
        (
            "manifest.json".to_string(),
            manifest.to_string().into_bytes(),
        ),
    );
    write_temp(&format!("bundled-{test}.penpot"), &stored_zip(&entries))
}

fn import_onto(path: &PathBuf, design_system: &[Value]) -> Import {
    import_file_onto(path, design_system)
        .unwrap_or_else(|error| panic!("cannot import {path:?}: {error}"))
}

fn kassa(import: &Import) -> Value {
    named(&json(page(import, "Kassa")), "Kassa").clone()
}

// The origin --------------------------------------------------------------------------------

#[test]
fn imported_components_keep_their_penpot_origin_through_the_server() {
    let stored = library_in_design_system("origin");
    assert_eq!(stored.len(), 4, "{stored:#?}");
    for id in [C_PRIMARY, C_SECONDARY, C_BELL, C_STAR] {
        let component = from_penpot(&stored, id);
        assert_eq!(component["origin"]["tool"], "penpot");
        assert_eq!(component["origin"]["file"], FILE_ID);
    }
    // Each layer names the node the server gave it.
    let primary = from_penpot(&stored, C_PRIMARY);
    let label = layer(primary, n::PRIMARY_LABEL);
    let node = primary["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|child| child["id"] == label.as_str())
        .unwrap_or_else(|| panic!("the Label layer {label} is no child of {primary:#}"));
    assert_eq!(node["name"], "Label");
    assert_eq!(layer(primary, n::PRIMARY), primary["id"].as_str().unwrap());
}

#[test]
fn the_origin_names_the_servers_nodes_when_no_component_places_another() {
    // The library without its Buttons: two icons, which place nothing, so
    // the components go in one pass unless the origin waits for the IDs.
    let entries: Vec<(String, Vec<u8>)> = library_entries_all()
        .into_iter()
        .filter(|(name, _)| {
            !name.ends_with(&format!("components/{C_PRIMARY}.json"))
                && !name.ends_with(&format!("components/{C_SECONDARY}.json"))
        })
        .collect();
    let icons = import(&write_temp("icons.penpot", &stored_zip(&entries)));
    let stored = Server::new().put_components(&icons, &mut Ids::default());
    assert_eq!(stored.len(), 2, "{stored:#?}");
    let bell = from_penpot(&stored, C_BELL);
    let glyph = layer(bell, n::BELL_GLYPH);
    assert_eq!(bell["children"][0]["id"], glyph.as_str(), "{bell:#}");
}

// A library imported before -----------------------------------------------------------------

#[test]
fn a_file_that_links_a_library_imported_before_places_its_components() {
    let stored = library_in_design_system("linked");
    let area = import_onto(&area_export("linked"), &stored);
    assert_eq!(
        area.component_count(),
        0,
        "the area imports no component of its own: {:#}",
        area.design_system["$components"]
    );
    let primary = from_penpot(&stored, C_PRIMARY);
    let board = kassa(&area);
    let kop = find_json(&board, "Köp").expect("the copy Köp is there");
    assert_eq!(kop["type"], "instance", "{kop:#}");
    assert_eq!(kop["component"], primary["id"]);
    assert!(kop.get("overrides").is_none(), "{kop:#}");
    let again = find_json(&board, "Köp igen").expect("the copy Köp igen is there");
    assert_eq!(again["component"], primary["id"]);
    assert_eq!(
        again["overrides"],
        json!({ layer(primary, n::PRIMARY_LABEL): { "content": "Kör igen" } }),
        "the override is keyed by the design system's Label node"
    );
    for entry in &area.report {
        assert!(
            !entry.reason.contains("plain shapes"),
            "no copy is drawn as plain shapes: {entry:?}"
        );
    }

    // Sent onto the design system, the head keeps its four components and the
    // design names them.
    let mut ids = Ids::default();
    let merged = onto(&area, &stored, &mut ids);
    assert_eq!(merged.component_count(), 4);
    let tags = merged.design_tags(&ids).expect("the design renders");
    let placed = format!("component=\"{}\"", primary["id"].as_str().unwrap());
    assert_eq!(tags.matches(&placed).count(), 2, "{tags}");
}

#[test]
fn importing_the_same_file_twice_adds_no_components() {
    let mut stored = library_in_design_system("twice");
    // A component made in OmniState, which the import leaves alone.
    let hand_made = json!({
        "id": "hand0001", "type": "component", "name": "Chip",
        "width": { "fixed": 40 }, "height": { "fixed": 20 },
        "layout": { "mode": "none" }, "children": []
    });
    stored.push(hand_made.clone());

    let again = import_onto(&library_export("twice-again"), &stored);
    assert_eq!(again.component_count(), 0, "{:#}", again.design_system);
    // The copies on the library's own page Start place the stored components.
    let start = json(page(&again, "Start"));
    let buy = named(&start, "Köp-knapp");
    assert_eq!(buy["type"], "instance");
    assert_eq!(buy["component"], from_penpot(&stored, C_PRIMARY)["id"]);

    let mut ids = Ids::default();
    let merged = onto(&again, &stored, &mut ids);
    let kept = components(&merged);
    assert_eq!(kept.len(), 5);
    assert!(
        kept.contains(&&hand_made),
        "the hand-made component stays as it was"
    );
}

#[test]
fn a_component_the_design_system_has_is_not_added_even_when_read_without_it() {
    let stored = library_in_design_system("defensive");
    // Read without the design system, the area's bundled library components
    // are converted, but going onto the head reuses the head's.
    let area = import(&bundled_export("defensive"));
    assert!(area.component_count() > 0);
    let mut ids = Ids::default();
    let merged = onto(&area, &stored, &mut ids);
    assert_eq!(merged.component_count(), 4);
    let tags = merged.design_tags(&ids).expect("the design renders");
    let primary = from_penpot(&stored, C_PRIMARY);
    let placed = format!("component=\"{}\"", primary["id"].as_str().unwrap());
    assert_eq!(tags.matches(&placed).count(), 2, "{tags}");
    let label = format!(
        "{}: {{content: \"Kör igen\"}}",
        layer(primary, n::PRIMARY_LABEL)
    );
    assert!(
        tags.contains(&label),
        "the override names the head's Label: {tags}"
    );
}

/// The library as a file duplicated from it: the same pages, shapes and
/// component ids, under the file id `file`. With `glyph`, the bell's Glyph is
/// that color instead.
fn duplicate_export(test: &str, file: &str, glyph: Option<&str>) -> PathBuf {
    let entries: Vec<(String, Vec<u8>)> = library_entries_all()
        .into_iter()
        .map(|(name, bytes)| {
            let mut text = String::from_utf8(bytes).unwrap_or_default();
            if name.ends_with(&format!("{}.json", uid(n::BELL + 1)))
                && let Some(glyph) = glyph
            {
                text = text.replace("#F2B33D", glyph);
            }
            (
                name.replace(FILE_ID, file),
                text.replace(FILE_ID, file).into_bytes(),
            )
        })
        .collect();
    write_temp(&format!("duplicate-{test}.penpot"), &stored_zip(&entries))
}

#[test]
fn a_file_duplicated_from_the_library_reuses_its_unchanged_components() {
    const COPY: &str = "f0000000-0000-4000-8000-0000000000d1";
    let stored = library_in_design_system("duplicate");
    let copy = import_onto(&duplicate_export("same", COPY, None), &stored);
    assert_eq!(
        copy.component_count(),
        4,
        "read onto the head, the file's own are converted"
    );
    let mut ids = Ids::default();
    let merged = onto(&copy, &stored, &mut ids);
    assert_eq!(
        merged.component_count(),
        4,
        "the same Penpot ids with the same content are the head's"
    );
    let tags = merged.design_tags(&ids).expect("the design renders");
    let primary = from_penpot(&stored, C_PRIMARY);
    let placed = format!("component=\"{}\"", primary["id"].as_str().unwrap());
    assert!(tags.contains(&placed), "{tags}");

    // A component that changed in the copy is the file's own.
    let changed = import_onto(&duplicate_export("changed", COPY, Some("#000000")), &stored);
    let mut ids = Ids::default();
    let merged = onto(&changed, &stored, &mut ids);
    let added: Vec<&Value> = components(&merged)[4..].to_vec();
    let mut names: Vec<&str> = added.iter().map(|c| c["name"].as_str().unwrap()).collect();
    names.sort();
    // The bell, and the Buttons that hold a copy of it.
    assert_eq!(names, ["Button", "Button", "Ikon / bell"], "{added:#?}");
    assert!(added.iter().all(|c| c["origin"]["file"] == COPY));
}

// A bundled library -------------------------------------------------------------------------

#[test]
fn an_export_that_bundles_the_library_imports_its_placed_components_once() {
    let area = import_onto(&bundled_export("bundled"), &[]);
    assert_eq!(
        page(&area, "Kassa").name(),
        "Kassa",
        "the area file is the one read"
    );
    assert!(
        area.pages.iter().all(|page| page.name() == "Kassa"),
        "the library's pages are not imported"
    );
    let imported = components(&area);
    let mut origins: Vec<&str> = imported
        .iter()
        .map(|component| component["origin"]["component"].as_str().unwrap())
        .collect();
    origins.sort();
    // Button / Primär, and the bell its layers hold; not Sekundär or star.
    assert_eq!(origins, [C_PRIMARY, C_BELL], "{imported:#?}");
    for component in &imported {
        assert_eq!(component["origin"]["file"], FILE_ID);
    }
    let primary = imported
        .iter()
        .find(|component| component["origin"]["component"] == C_PRIMARY)
        .unwrap();
    let board = kassa(&area);
    for name in ["Köp", "Köp igen"] {
        let copy = find_json(&board, name).unwrap();
        assert_eq!(copy["type"], "instance", "{copy:#}");
        assert_eq!(copy["component"], primary["id"]);
    }
    // The library's tokens come along, for the components to refer to.
    assert!(
        area.design_system["color"]["surface"].is_object(),
        "{:#}",
        area.design_system
    );
}

#[test]
fn a_bundled_library_the_design_system_has_is_not_imported_again() {
    let stored = library_in_design_system("bundled-known");
    let area = import_onto(&bundled_export("bundled-known"), &stored);
    assert_eq!(area.component_count(), 0);
    let board = kassa(&area);
    let copy = find_json(&board, "Köp").unwrap();
    assert_eq!(copy["component"], from_penpot(&stored, C_PRIMARY)["id"]);
}

// Changes no override holds --------------------------------------------------------------

#[test]
fn a_change_marked_but_to_the_mains_values_keeps_the_copy_an_instance() {
    // Penpot marks the Dot's size and strokes as changed, but they are the
    // main's again.
    let mut copy = primary_copy(KOP, "Köp", 20.0);
    set(
        &mut copy.kids[1],
        json!({ "touched": ["geometry-group", "stroke-group"] }),
    );
    let area = import_onto(&bundled_with("no-op", vec![copy]), &[]);
    let board = kassa(&area);
    let kop = find_json(&board, "Köp").unwrap();
    assert_eq!(kop["type"], "instance", "{kop:#}");
    let about: Vec<_> = area
        .report
        .iter()
        .filter(|entry| entry.shape_id == uid(KOP) || entry.shape_id == uid(KOP + 2))
        .collect();
    assert!(about.is_empty(), "{about:#?}");
}

#[test]
fn a_copy_that_changes_what_no_override_holds_is_drawn_as_its_shapes() {
    // The Dot gets a stroke, which no override holds.
    let mut copy = primary_copy(KOP, "Köp", 20.0);
    set(
        &mut copy.kids[1],
        json!({
            "touched": ["stroke-group"],
            "strokes": [{ "strokeColor": "#FFFFFF", "strokeOpacity": 1, "strokeWidth": 2, "strokeAlignment": "inner", "strokeStyle": "solid" }]
        }),
    );
    let area = import_onto(&bundled_with("strokes", vec![copy]), &[]);
    let board = kassa(&area);
    let kop = find_json(&board, "Köp").unwrap();
    assert_eq!(
        kop["type"], "frame",
        "the copy is drawn as Penpot shows it: {kop:#}"
    );
    let dot = find_json(kop, "Dot").unwrap();
    assert_eq!(dot["strokes"][0]["color"], "#ffffff", "{dot:#}");
    // The bell inside it is a copy of its own, untouched: an instance.
    let bell = find_json(kop, "Ikon / bell").unwrap();
    assert_eq!(bell["type"], "instance", "{bell:#}");
    let entry = area
        .report
        .iter()
        .find(|entry| entry.shape_id == uid(KOP))
        .unwrap_or_else(|| panic!("the copy is reported: {:#?}", area.report));
    assert!(entry.reason.contains("its strokes"), "{entry:?}");
}

#[test]
fn a_change_inside_a_nested_copy_is_kept_by_that_copy() {
    // The Glyph of the bell inside the Button turns black: the Button cannot
    // hold that, the bell can.
    let mut copy = primary_copy(KOP, "Köp", 20.0);
    set(
        &mut copy.kids[3].kids[0],
        json!({ "touched": ["fill-group"], "fills": solid("#000000") }),
    );
    let area = import_onto(&bundled_with("nested", vec![copy]), &[]);
    let board = kassa(&area);
    let kop = find_json(&board, "Köp").unwrap();
    assert_eq!(kop["type"], "frame", "{kop:#}");
    let bell = find_json(kop, "Ikon / bell").unwrap();
    assert_eq!(bell["type"], "instance", "{bell:#}");
    let component = components(&area)
        .into_iter()
        .find(|component| component["id"] == bell["component"])
        .expect("the bell is imported");
    assert_eq!(component["origin"]["component"], C_BELL);
    // Keyed by the bell's own Glyph, which the Button's Glyph copies.
    let glyph = layer(component, n::BELL_GLYPH);
    assert_eq!(
        bell["overrides"][glyph.as_str()]["fills"][0]["color"],
        "#000000",
        "{bell:#}"
    );
}

// Neither -----------------------------------------------------------------------------------

#[test]
fn a_copy_of_a_library_neither_bundled_nor_imported_is_drawn_plain_and_reported() {
    let area = import_onto(&area_export("unlinked"), &[]);
    assert_eq!(area.component_count(), 0);
    let board = kassa(&area);
    let copy = find_json(&board, "Köp").unwrap();
    assert_eq!(copy["type"], "frame", "{copy:#}");
    let entry = area
        .report
        .iter()
        .find(|entry| entry.shape_id == uid(KOP))
        .unwrap_or_else(|| panic!("the copy is reported: {:#?}", area.report));
    assert!(entry.reason.contains(FILE_ID), "{entry:?}");
    assert!(entry.reason.contains("library"), "{entry:?}");
}

// The TireBud exports -------------------------------------------------------------------------

#[test]
fn the_ordrar_export_imported_twice_adds_no_components() {
    let Some(path) = common::fixture("tirebud-ordrar.penpot") else {
        return;
    };
    let first = import(&path);
    let count = first.component_count();
    assert!(count > 0);
    let stored = Server::new().put_components(&first, &mut Ids::default());
    assert_eq!(stored.len(), count);
    assert!(
        stored
            .iter()
            .all(|component| component["origin"]["tool"] == "penpot"),
        "every component keeps its origin"
    );

    let again = import_onto(&path, &stored);
    assert_eq!(again.component_count(), 0, "nothing is imported twice");
    let mut ids = Ids::default();
    let merged = onto(&again, &stored, &mut ids);
    assert_eq!(merged.component_count(), count);
    // Every instance of the second import places a stored component.
    let tags = merged.design_tags(&ids).expect("the design renders");
    let first_tags = first
        .design_tags(&{
            let mut ids = Ids::default();
            Server::new().put_components(&first, &mut ids);
            ids
        })
        .expect("the first design renders");
    assert_eq!(
        tags.matches("<Instance").count(),
        first_tags.matches("<Instance").count(),
        "the second import places as many instances as the first"
    );
}
