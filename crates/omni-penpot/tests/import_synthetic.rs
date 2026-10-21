//! US-051: `omni_penpot::import::import_file` on small exports that the tests
//! build, with what an "Export file + libraries" export of TireBud holds (see
//! tests/common/import.rs for the Penpot fields). The import reuses the reader
//! and the converter, and gives:
//!
//! - `pages`: the design doc, one `page` node per Penpot page, named as the
//!   page and in Penpot's page order, holding the boards of the page and, when
//!   shapes sit outside every board, the board "Unplaced" (US-054). A board
//!   without components or tokens is the board that `convert_file` gives, node
//!   IDs apart.
//! - `design_system`: the design system doc, a DTCG token tree
//!   (docs/formats/tokens.md) with the components under `$components`.
//!   Penpot's token themes become its modes, named as the themes (`Mörkt`,
//!   `Ljust`) in Penpot's order; a token of a set that every theme enables has
//!   one value, a token of a set that one theme enables has that theme's value
//!   in its mode. A Penpot color token keeps its path (`color.surface`), and
//!   `rgba(r, g, b, a)` becomes `#rrggbbaa`. Spacing (`spacing`) and radius
//!   (`borderRadius`) tokens go into the groups `spacing` and `radius`, their
//!   Penpot path at the end of their path, with `px` lengths. A Penpot token
//!   that has no group in OmniState's tree is either still a token or listed in
//!   the report by its path. Library colors become `color` tokens and library
//!   typographies `typography` tokens whose paths end in their names.
//! - A token linked to a fill or a stroke (`appliedTokens`) stays a reference:
//!   the fill, the stroke or the text's color is `$<path of the token>`.
//! - Each Penpot component becomes one component, named as the component (with
//!   its path in front as `<path> / <name>` when it has one). The variants of a
//!   set share the set's name and its variant properties, with every value of
//!   the set, and each variant's defaults are its own values. A copy nested in
//!   a component is an instance there.
//! - A copy on a board becomes an instance that shows the component (or the
//!   variant) it copies, with its overrides: a changed text is a `content`
//!   override, changed fills a `fills` override, a hidden layer a `visible`
//!   override and a swapped component a `component` override, each keyed by the
//!   node ID of the layer in the component it shows. An untouched copy has no
//!   overrides.
//! - Images become attachments that the design refers to as `attachment:<id>`.
//! - `report`: everything that could not be imported, with its page, board,
//!   shape and reason. Nothing is dropped silently.

mod common;

use std::collections::BTreeSet;

use omni_core::design::NodeKind;
use omni_penpot::convert_file;
use omni_penpot::import::import_file;
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::Scene;
use serde_json::{Value, json};

use common::convert::{
    PNG, attachment_refs, board_node, flex, paragraph, run, shape, text_content, uid, write_export,
    write_export_with,
};
use common::import::*;

/// A copy of `value` without the `id` of any node in it.
fn without_ids(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "id")
                .map(|(key, item)| (key.clone(), without_ids(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_ids).collect()),
        other => other.clone(),
    }
}

fn board_names(page: &Value) -> Vec<&str> {
    page["children"]
        .as_array()
        .map(|boards| {
            boards
                .iter()
                .filter_map(|board| board["name"].as_str())
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn pages_and_boards_become_one_design_doc() {
    let import = import_library("pages");
    let names: Vec<&str> = import.pages.iter().map(|page| page.name()).collect();
    assert_eq!(
        names,
        ["Komponenter", "Start"],
        "the pages, in Penpot's order"
    );
    for page in &import.pages {
        assert_eq!(page.kind(), NodeKind::Page, "{}", page.name());
        for board in page.children() {
            assert_eq!(
                board.kind(),
                NodeKind::Board,
                "{} on {}",
                board.name(),
                page.name()
            );
        }
    }
    assert_eq!(
        board_names(&json(page(&import, "Komponenter"))),
        ["Knappar", "Ikoner"]
    );
    // Anteckning sits on the page outside every board (US-054).
    assert_eq!(
        board_names(&json(page(&import, "Start"))),
        ["Hem", "Unplaced"]
    );
    let start = json(page(&import, "Start"));
    let hem = named(&start, "Hem");
    assert_eq!(
        (hem["x"].as_f64(), hem["y"].as_f64()),
        (Some(0.0), Some(0.0))
    );
    assert_eq!(hem["width"]["fixed"].as_f64(), Some(400.0), "{hem}");
    assert_eq!(hem["height"]["fixed"].as_f64(), Some(600.0), "{hem}");
    assert!(
        find_json(named(&start, "Unplaced"), "Anteckning").is_some(),
        "{start}"
    );
}

/// One line of a text's `positionData`: a run of `text` at `y`.
fn line(y: f64, text: &str) -> Value {
    json!({
        "x": 10, "y": y, "width": 100, "height": 18.2, "text": text,
        "fontFamily": "Inter", "fontSize": "14px", "fontWeight": "400",
        "fontStyle": "normal", "lineHeight": "1.45", "letterSpacing": "0px",
        "textAlign": "left", "textDecoration": "none", "textTransform": "none",
        "direction": "ltr",
        "fills": [{ "fillColor": "#102030", "fillOpacity": 1 }]
    })
}

#[test]
fn shapes_without_components_or_tokens_import_as_the_converter_converts_them() {
    // The rules of US-054: the line pitch of positionData, a filling child with
    // a min size of 0, a text that hugs following Chrome, and a loose text in
    // the board Unplaced.
    let style = json!({ "fontSize": "14", "lineHeight": "1.45" });
    let path = write_export(
        "import-as-converted",
        vec![
            shape(1, "frame", "Plain", [0.0, 0.0, 300.0, 200.0])
                .with(flex("column"))
                .with(json!({ "fills": solid("#FFFFFF") }))
                .kids(vec![
                    shape(4, "frame", "Filler", [0.0, 60.0, 300.0, 40.0])
                        .with(flex("row"))
                        .with(json!({ "layoutItemHSizing": "fill", "layoutItemVSizing": "fix" }))
                        .kids(vec![shape(5, "rect", "Wide", [0.0, 60.0, 400.0, 40.0])]),
                    shape(2, "text", "Two lines", [0.0, 0.0, 300.0, 41.0]).with(json!({
                        "growType": "auto-height",
                        "layoutItemHSizing": "fix",
                        "layoutItemVSizing": "auto",
                        "content": text_content(vec![
                            paragraph(vec![run("One", style.clone())]),
                            paragraph(vec![run("Two", style.clone())]),
                        ]),
                        "positionData": [line(0.0, "One"), line(20.296875, "Two")]
                    })),
                ]),
            shape(3, "text", "Loose", [400.0, 10.0, 60.0, 20.0]).with(json!({
                "growType": "auto-width",
                "content": text_content(vec![paragraph(vec![run("Hej", json!({}))])]),
                "positionData": [line(29.2, "Hej")]
            })),
        ],
    );
    let conversion = convert_file(&path).expect("the export converts");
    let import = import(&path);
    assert_eq!(import.pages.len(), 1);
    let page = json(&import.pages[0]);
    assert_eq!(page["name"], common::convert::PAGE_NAME);
    assert_eq!(board_names(&page), ["Plain", "Unplaced"]);
    for converted in &conversion.boards {
        let expected = without_ids(&json(board_node(&converted.design)));
        let imported = without_ids(named(&page, &converted.name));
        assert_eq!(
            imported, expected,
            "the board {} imports as it converts",
            converted.name
        );
    }
}

/// The token tree of an export of one plain board with the TireBud tokens.
fn tokens_import(test: &str) -> omni_penpot::import::Import {
    let path = write_export_with(
        test,
        vec![
            shape(1, "frame", "Hem", [0.0, 0.0, 100.0, 100.0])
                .with(json!({ "fills": solid("#1C1C21") })),
        ],
        token_entries(&tokens_json()),
    );
    import(&path)
}

#[test]
fn token_themes_become_the_modes_of_a_dtcg_token_tree() {
    let import = tokens_import("import-themes");
    let tree = token_tree(&import);
    assert_eq!(tree.modes(), ["Mörkt", "Ljust"]);
    for (path, dark, light) in [
        (
            "color.surface",
            [0x1c, 0x1c, 0x21, 255],
            [0xff, 0xff, 0xff, 255],
        ),
        (
            "color.border",
            [0xe5, 0x48, 0x4d, 0x66],
            [0xdc, 0x26, 0x26, 0x66],
        ),
        (
            "color.text",
            [0xf4, 0xf4, 0xf5, 255],
            [0x18, 0x18, 0x1b, 255],
        ),
    ] {
        for (mode, expected) in [("Mörkt", dark), ("Ljust", light)] {
            let value = resolve(&tree, path, mode).unwrap_or_else(|| {
                panic!("{path} is a token in {mode}: {:#}", import.design_system)
            });
            assert_eq!(rgba_of(&value), Some(expected), "{path} in {mode}: {value}");
        }
    }
    let all = tokens_of(&import.design_system);
    for (group, tail, expected) in [("spacing", "space.4", "16px"), ("radius", "md", "8px")] {
        let found: Vec<&String> = all
            .iter()
            .map(|(path, _)| path)
            .filter(|path| path.starts_with(&format!("{group}.")) && path.ends_with(tail))
            .collect();
        assert_eq!(found.len(), 1, "one {group} token ends in {tail}: {all:?}");
        for mode in ["Mörkt", "Ljust"] {
            assert_eq!(
                resolve(&tree, found[0], mode),
                Some(json!(expected)),
                "{} in {mode}",
                found[0]
            );
        }
    }
}

#[test]
fn penpot_tokens_without_a_dtcg_group_are_kept_or_reported() {
    let import = tokens_import("import-font-size");
    let kept = tokens_of(&import.design_system)
        .iter()
        .any(|(path, _)| path.ends_with("font.size.base") || path.ends_with("size.base"));
    let reported = import
        .report
        .iter()
        .any(|entry| entry.name.contains("font.size.base") && !entry.reason.is_empty());
    assert!(
        kept || reported,
        "the font size token font.size.base is neither a token nor in the report: {:#}\n{:?}",
        import.design_system,
        import.report
    );
}

#[test]
fn token_links_on_fills_and_strokes_stay_token_references() {
    let import = import_library("token-links");
    let start = json(page(&import, "Start"));
    let hem = named(&start, "Hem");
    assert_eq!(hem["fills"][0]["color"], "$color.surface", "{hem}");
    assert_eq!(hem["strokes"][0]["color"], "$color.border", "{hem}");
    assert_eq!(hem["strokes"][0]["width"].as_f64(), Some(1.0), "{hem}");
    let rubrik = named(&start, "Rubrik");
    assert_eq!(rubrik["color"], "$color.text", "{rubrik}");
    // A fill that no token is linked to keeps its color.
    let knappar = json(page(&import, "Komponenter"));
    let board = named(&knappar, "Knappar");
    assert_eq!(
        rgba_of(&board["fills"][0]["color"]),
        Some([0x13, 0x13, 0x16, 255]),
        "{board}"
    );
}

#[test]
fn library_colors_and_typographies_become_tokens() {
    let import = import_library("library");
    let tree = token_tree(&import);
    let all = tokens_of(&import.design_system);
    for (name, expected) in [
        ("brand-accent", [0xff, 0x55, 0x00, 255]),
        ("overlay", [0x00, 0x00, 0x00, 0x80]),
    ] {
        let found: Vec<&String> = all
            .iter()
            .map(|(path, _)| path)
            .filter(|path| path.starts_with("color.") && path.to_lowercase().ends_with(name))
            .collect();
        assert_eq!(
            found.len(),
            1,
            "one color token for the library color {name}: {all:?}"
        );
        for mode in ["Mörkt", "Ljust"] {
            let value = resolve(&tree, found[0], mode).expect("the token resolves");
            let rgba = rgba_of(&value).unwrap_or_else(|| panic!("{value} is a color"));
            assert_eq!(rgba[..3], expected[..3], "{name} in {mode}: {value}");
            assert!(
                rgba[3].abs_diff(expected[3]) <= 1,
                "{name} in {mode}: {value}"
            );
        }
    }
    let body: Vec<&String> = all
        .iter()
        .map(|(path, _)| path)
        .filter(|path| path.starts_with("typography.") && path.to_lowercase().ends_with("body"))
        .collect();
    assert_eq!(body.len(), 1, "one typography token for Body: {all:?}");
    for mode in ["Mörkt", "Ljust"] {
        let value = resolve(&tree, body[0], mode).expect("the typography resolves");
        assert_eq!(value["fontFamily"], "Geist", "{value}");
        assert_eq!(value["fontSize"], "16px", "{value}");
        assert_eq!(
            value["fontWeight"]
                .as_f64()
                .or_else(|| value["fontWeight"].as_str()?.parse().ok()),
            Some(400.0),
            "{value}"
        );
        assert_eq!(value["lineHeight"].as_f64(), Some(1.2), "{value}");
        assert_eq!(value["letterSpacing"], "0.5px", "{value}");
    }
}

#[test]
fn components_and_their_variants_become_design_system_components() {
    let import = import_library("components");
    assert_eq!(
        components(&import).len(),
        4,
        "Button / Primär, Button / Sekundär, bell and star: {:#}",
        import.design_system["$components"]
    );
    for component in components(&import) {
        assert_eq!(component["type"], "component", "{component}");
    }
    let buttons = components_named(&import, "Button", "");
    assert_eq!(buttons.len(), 2, "the variant set Button has two variants");
    let mut defaults = BTreeSet::new();
    for button in &buttons {
        let typ = &button["props"]["Typ"];
        assert_eq!(typ["type"], "variant", "{button}");
        let values: BTreeSet<&str> = typ["values"]
            .as_array()
            .expect("variant values")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(values, BTreeSet::from(["Primär", "Sekundär"]), "{button}");
        defaults.insert(typ["default"].as_str().expect("a default").to_string());
    }
    assert_eq!(
        defaults,
        BTreeSet::from(["Primär".to_string(), "Sekundär".to_string()]),
        "each variant's default is its own value"
    );
    let bell = components_named(&import, "bell", "Ikon");
    let star = components_named(&import, "star", "Ikon");
    assert_eq!((bell.len(), star.len()), (1, 1), "the icons bell and star");
    for icon in [bell[0], star[0]] {
        assert!(
            icon["props"]
                .as_object()
                .is_none_or(|props| props.values().all(|def| def["type"] != "variant")),
            "an icon is no variant: {icon}"
        );
        assert!(find_json(icon, "Glyph").is_some(), "{icon}");
    }
    let primary = buttons
        .iter()
        .find(|button| variant_default(button, "Typ") == Some("Primär"))
        .expect("the variant Primär");
    assert_eq!(
        rgba_of(&primary["fills"][0]["color"]),
        Some([0x01, 0xb5, 0x97, 255]),
        "{primary}"
    );
    assert_eq!(named(primary, "Label")["content"], "Köp", "{primary}");
    for layer in ["Dot", "Hint"] {
        named(primary, layer);
    }
    let nested = named(primary, "Ikon / bell");
    assert_eq!(
        nested["type"], "instance",
        "a nested copy is an instance: {nested}"
    );
    assert_eq!(nested["component"], bell[0]["id"], "{nested}");
}

#[test]
fn copies_become_instances_that_keep_their_overrides() {
    let import = import_library("copies");
    let start = json(page(&import, "Start"));
    let hem = named(&start, "Hem");

    let buy = named(hem, "Köp-knapp");
    assert_eq!(buy["type"], "instance", "{buy}");
    assert_eq!(
        variant_default(shown(&import, buy), "Typ"),
        Some("Primär"),
        "{buy}"
    );
    assert!(
        buy["overrides"].is_null() || buy["overrides"] == json!({}),
        "an untouched copy has no overrides: {buy}"
    );

    let pay = named(hem, "Betala-knapp");
    assert_eq!(pay["type"], "instance", "{pay}");
    assert_eq!(
        (pay["x"].as_f64(), pay["y"].as_f64()),
        (Some(20.0), Some(120.0)),
        "{pay}"
    );
    let secondary = shown(&import, pay);
    assert_eq!(variant_default(secondary, "Typ"), Some("Sekundär"), "{pay}");
    let id_of = |name: &str| {
        named(secondary, name)["id"]
            .as_str()
            .expect("a node ID")
            .to_string()
    };
    let overrides = &pay["overrides"];
    assert_eq!(
        overrides[id_of("Label")]["content"],
        "Betala",
        "text: {pay}"
    );
    assert_eq!(
        rgba_of(&overrides[id_of("Dot")]["fills"][0]["color"]),
        Some([0x16, 0xa3, 0x4a, 255]),
        "fills: {pay}"
    );
    assert_eq!(
        overrides[id_of("Hint")]["visible"],
        false,
        "visibility: {pay}"
    );
    let star = components_named(&import, "star", "Ikon");
    assert_eq!(star.len(), 1);
    assert_eq!(
        overrides[id_of("Ikon / bell")]["component"],
        star[0]["id"],
        "the swapped icon: {pay}"
    );
    for (key, change) in overrides.as_object().expect("overrides") {
        for field in change.as_object().expect("an override").keys() {
            assert!(
                ["content", "fills", "visible", "component"].contains(&field.as_str()),
                "{key}: {field} is no override field"
            );
        }
    }
}

#[test]
fn the_imported_pages_draw_with_every_component_and_token_found_in_both_modes() {
    let import = import_library("draw");
    for page in &import.pages {
        let system = DesignSystem::from_json(&import.design_system)
            .unwrap_or_else(|error| panic!("the design system is refused: {error}"));
        let mut scene = Scene::with_system(page.clone(), Assets::new(), system);
        assert!(
            scene.errors().is_empty(),
            "{} in Mörkt: {:?}",
            page.name(),
            scene.errors()
        );
        scene.set_mode("Ljust").expect("Ljust is a mode");
        assert!(
            scene.errors().is_empty(),
            "{} in Ljust: {:?}",
            page.name(),
            scene.errors()
        );
    }
}

#[test]
fn images_become_attachments() {
    let import = import_library("images");
    let start = json(page(&import, "Start"));
    let picture = named(&start, "Bild");
    let refs = attachment_refs(picture);
    assert_eq!(refs.len(), 1, "{picture}");
    let id = refs[0].strip_prefix("attachment:").expect("a reference");
    let attachment = import
        .attachments
        .iter()
        .find(|attachment| attachment.id == id)
        .unwrap_or_else(|| panic!("no attachment {id}"));
    assert_eq!(attachment.bytes, PNG);
    assert_eq!(attachment.extension, "png");
    let mut everything = Vec::new();
    for page in &import.pages {
        everything.extend(attachment_refs(&json(page)));
    }
    everything.extend(attachment_refs(&import.design_system));
    for reference in everything {
        let id = reference.strip_prefix("attachment:").expect("a reference");
        assert!(
            import
                .attachments
                .iter()
                .any(|attachment| attachment.id == id),
            "{reference} is an attachment of the import"
        );
    }
}

#[test]
fn what_cannot_be_imported_is_reported_with_its_page_board_shape_and_reason() {
    let import = import_library("report");
    let union = entries_about(&import, n::UNION);
    assert!(
        union.iter().any(|entry| entry.property.is_none()
            && entry.page == "Start"
            && entry.board == "Hem"
            && entry.board_id == uid(n::HEM)
            && entry.name == "Union"
            && !entry.reason.is_empty()),
        "the boolean shape Union: {:?}",
        import.report
    );
    // The shadow on the Dot of Skugg-knapp is an override no instance can
    // hold: it is listed, for the layer or for its copy.
    let mut shadowed = entries_about(&import, n::SHADOWED_DOT);
    shadowed.extend(entries_about(&import, n::SHADOWED));
    assert!(
        shadowed.iter().any(|entry| entry.page == "Start"
            && entry.board == "Hem"
            && entry.board_id == uid(n::HEM)
            && !entry.reason.is_empty()),
        "the shadow override of Skugg-knapp: {:?}",
        import.report
    );
    let start = json(page(&import, "Start"));
    assert!(
        find_json(&start, "Skugg-knapp").is_some(),
        "the copy itself is still imported: {start}"
    );
    for entry in &import.report {
        if !entry.shape_id.is_empty() {
            assert!(
                !entry.page.is_empty() && !entry.board.is_empty() && !entry.reason.is_empty(),
                "{entry:?}"
            );
        }
    }
}

#[test]
fn nothing_is_dropped_silently() {
    let import = import_library("dropped");
    let mut names = Vec::new();
    for page in &import.pages {
        all_names(&json(page), &mut names);
    }
    for page in [components_page(), start_page()] {
        // The layers of main instances are in the components, and the layers
        // of copies in the instances.
        let mut inside: BTreeSet<String> = BTreeSet::new();
        let roots: Vec<&Value> = page
            .shapes
            .iter()
            .filter(|shape| shape["mainInstance"] == true || shape["componentRoot"] == true)
            .collect();
        for root in &roots {
            inside.insert(root["id"].as_str().unwrap().to_string());
        }
        let mut grew = true;
        while grew {
            grew = false;
            for shape in &page.shapes {
                let id = shape["id"].as_str().unwrap();
                if !inside.contains(id)
                    && shape["parentId"]
                        .as_str()
                        .is_some_and(|parent| inside.contains(parent))
                {
                    inside.insert(id.to_string());
                    grew = true;
                }
            }
        }
        for shape in &page.shapes {
            let id = shape["id"].as_str().unwrap();
            let is_copy = shape["componentRoot"] == true && shape["mainInstance"] != true;
            if inside.contains(id) && !is_copy {
                continue;
            }
            let name = shape["name"].as_str().unwrap();
            let reported = import
                .report
                .iter()
                .any(|entry| entry.shape_id == id && entry.property.is_none());
            assert!(
                names.iter().any(|known| known == name) || reported,
                "{name} on {} is neither a node nor in the report",
                page.name
            );
        }
    }
}

#[test]
fn a_file_that_is_not_an_export_is_an_error() {
    let path = common::write_temp("import-not-an-export.penpot", b"not a zip archive");
    let error = import_file(&path).expect_err("a file that is no zip archive");
    assert!(
        error.to_string().contains("import-not-an-export.penpot"),
        "{error}"
    );
}
