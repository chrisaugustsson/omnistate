//! Helpers for the US-051 import tests: a synthetic export with what an
//! "Export file + libraries" export of TireBud holds (design tokens with two
//! themes, library colors and typographies, components with variants, copies
//! with overrides, an image and a shape that cannot be imported), and readers
//! for the imported design and design system.
//!
//! The Penpot fields follow tirebud-ordrar.penpot (docs/formats/penpot-export.md):
//!
//! - `files/<file>/tokens.json` holds the token sets (`Bas`, `Mörkt`, `Ljust`),
//!   each a tree of groups down to tokens `{$value, $type, $description}`, and
//!   `$themes`, a list of `{id, name, group, selectedTokenSets}`, and
//!   `$metadata` with the `activeThemes`. A shape names the tokens applied to it
//!   in `appliedTokens`, such as `{fill: "color.surface", strokeColor:
//!   "color.border"}`, and still stores the resolved values in its fills and
//!   strokes.
//! - `files/<file>/colors/<id>.json` is a library color `{id, name, color,
//!   opacity}` and `files/<file>/typographies/<id>.json` a typography with its
//!   font values as text.
//! - `files/<file>/components/<id>.json` is a component `{id, name, path,
//!   mainInstanceId, mainInstancePage}`, plus `variantId` and
//!   `variantProperties` (`[{name, value}]`) for the variants of one set.
//! - The main instance of a component is a frame with `componentRoot`,
//!   `mainInstance` and `componentId`; variants sit in a frame with
//!   `isVariantContainer` and carry `variantId` and `variantName`. A copy
//!   placed on a board is a frame with `componentRoot`, `componentId` and
//!   `shapeRef` (the main instance's id), and every shape inside it has a
//!   `shapeRef` to the shape of the main it copies. A copy nested inside a
//!   component has `componentId` and `shapeRef` but no `componentRoot`.
//! - A copy's shape lists what the designer changed in `touched`:
//!   `text-content-text` (the text), `fill-group` (the fills),
//!   `visibility-group` (`hidden`), `swap-slot-<id of the main's nested copy>`
//!   (another component swapped in, whose id is then the shape's
//!   `componentId`) and `layer-effects-group` (the shadows), and holds the
//!   changed values itself.
#![allow(dead_code)]

use std::path::PathBuf;

use omni_core::design::Node;
use omni_core::design::tokens::TokenTree;
use omni_penpot::import::{Import, ImportEntry, import_file};
use serde_json::{Value, json};

use super::convert::{
    FILE_ID, S, flex, image_fill, media_entries, paragraph, run, shape, text_content, uid,
};
use super::{SyntheticPage, export_entries, stored_zip, write_temp};

pub const PAGE_K: &str = "a0000000-0000-4000-8000-00000000000a";
pub const PAGE_S: &str = "a0000000-0000-4000-8000-00000000000b";

pub const C_PRIMARY: &str = "c0000000-0000-4000-8000-000000000001";
pub const C_SECONDARY: &str = "c0000000-0000-4000-8000-000000000002";
pub const C_BELL: &str = "c0000000-0000-4000-8000-000000000003";
pub const C_STAR: &str = "c0000000-0000-4000-8000-000000000004";
pub const V_BUTTON: &str = "d0000000-0000-4000-8000-000000000001";

pub const MEDIA: &str = "e0000000-0000-4000-8000-000000000001";
pub const STORED: &str = "e0000000-0000-4000-8000-000000000002";

/// The shape numbers of the library export ([`library_export`]).
pub mod n {
    // Page Komponenter.
    pub const KNAPPAR: u32 = 1;
    pub const CONTAINER: u32 = 2;
    pub const PRIMARY: u32 = 3;
    pub const PRIMARY_LABEL: u32 = 4;
    pub const PRIMARY_DOT: u32 = 5;
    pub const PRIMARY_HINT: u32 = 6;
    pub const PRIMARY_ICON: u32 = 7;
    pub const PRIMARY_ICON_GLYPH: u32 = 8;
    pub const SECONDARY: u32 = 9;
    pub const SECONDARY_LABEL: u32 = 10;
    pub const SECONDARY_DOT: u32 = 11;
    pub const SECONDARY_HINT: u32 = 12;
    pub const SECONDARY_ICON: u32 = 13;
    pub const SECONDARY_ICON_GLYPH: u32 = 14;
    pub const IKONER: u32 = 19;
    pub const BELL: u32 = 20;
    pub const BELL_GLYPH: u32 = 21;
    pub const STAR: u32 = 22;
    pub const STAR_GLYPH: u32 = 23;
    // Page Start.
    pub const HEM: u32 = 30;
    pub const RUBRIK: u32 = 31;
    pub const BUY: u32 = 32;
    pub const PAY: u32 = 40;
    pub const PAY_LABEL: u32 = 41;
    pub const PAY_DOT: u32 = 42;
    pub const PAY_HINT: u32 = 43;
    pub const PAY_ICON: u32 = 44;
    pub const PAY_ICON_GLYPH: u32 = 45;
    pub const SHADOWED: u32 = 50;
    pub const SHADOWED_DOT: u32 = 52;
    pub const PICTURE: u32 = 60;
    pub const UNION: u32 = 61;
    pub const NOTE: u32 = 70;
}

/// A page of a synthetic export with `top_level` directly on it.
pub fn page_of(id: &str, name: &str, index: u32, top_level: Vec<S>) -> SyntheticPage {
    let ids = top_level.iter().map(S::id).collect();
    let mut shapes = Vec::new();
    for shape in top_level {
        flatten(shape, super::ROOT_ID, &mut shapes);
    }
    SyntheticPage {
        id: id.to_string(),
        name: name.to_string(),
        index,
        top_level: ids,
        shapes,
    }
}

fn flatten(shape: S, parent: &str, out: &mut Vec<Value>) {
    let S { mut json, kids } = shape;
    json["parentId"] = json!(parent);
    json["frameId"] = json!(parent);
    let id = json["id"].as_str().expect("an id").to_string();
    if !kids.is_empty() {
        json["shapes"] = json!(kids.iter().map(S::id).collect::<Vec<_>>());
    }
    out.push(json);
    for kid in kids {
        flatten(kid, &id, out);
    }
}

pub fn solid(hex: &str) -> Value {
    json!([{ "fillColor": hex, "fillOpacity": 1 }])
}

/// The token sets and themes of TireBud, cut down: `Bas` holds a spacing, a
/// radius and a font size, `Mörkt` and `Ljust` the same three colors with a
/// value each, one of them written as `rgba(...)`.
pub fn tokens_json() -> Value {
    json!({
        "Bas": {
            "space": {
                "4": { "$value": "16", "$type": "spacing", "$description": "" }
            },
            "radius": {
                "md": { "$value": "8", "$type": "borderRadius", "$description": "" }
            },
            "font": {
                "size": {
                    "base": { "$value": "14", "$type": "fontSizes", "$description": "" }
                }
            }
        },
        "Mörkt": {
            "color": {
                "surface": { "$value": "#1C1C21", "$type": "color", "$description": "" },
                "border": { "$value": "rgba(229,72,77,0.40)", "$type": "color", "$description": "" },
                "text": { "$value": "#F4F4F5", "$type": "color", "$description": "" }
            }
        },
        "Ljust": {
            "color": {
                "surface": { "$value": "#FFFFFF", "$type": "color", "$description": "" },
                "border": { "$value": "rgba(220,38,38,0.40)", "$type": "color", "$description": "" },
                "text": { "$value": "#18181B", "$type": "color", "$description": "" }
            }
        },
        "$themes": [
            {
                "id": "f1000000-0000-4000-8000-000000000001",
                "name": "Mörkt",
                "group": "Tema",
                "description": "",
                "isSource": false,
                "selectedTokenSets": { "Mörkt": "enabled", "Bas": "enabled" }
            },
            {
                "id": "f1000000-0000-4000-8000-000000000002",
                "name": "Ljust",
                "group": "Tema",
                "description": "",
                "isSource": false,
                "selectedTokenSets": { "Ljust": "enabled", "Bas": "enabled" }
            }
        ],
        "$metadata": {
            "tokenSetOrder": ["Bas", "Mörkt", "Ljust"],
            "activeThemes": ["Tema/Mörkt"],
            "activeSets": ["Mörkt", "Bas"]
        }
    })
}

/// The archive entries of the design tokens.
pub fn token_entries(tokens: &Value) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            format!("files/{FILE_ID}/tokens.json"),
            tokens.to_string().into_bytes(),
        ),
        (
            format!("files/{FILE_ID}/tokens-status.json"),
            json!({ "activeThemes": ["Tema/Mörkt"], "activeSets": ["Mörkt", "Bas"] })
                .to_string()
                .into_bytes(),
        ),
    ]
}

/// The archive entries of the library colors `brand-accent` (#FF5500) and
/// `overlay` (black at half opacity), and the typography `Body` (Geist, 16,
/// 400, line height 1.2, letter spacing 0.5).
pub fn library_entries() -> Vec<(String, Vec<u8>)> {
    let color = |id: &str, name: &str, hex: &str, opacity: f64| {
        (
            format!("files/{FILE_ID}/colors/{id}.json"),
            json!({
                "id": id,
                "name": name,
                "color": hex,
                "opacity": opacity,
                "modifiedAt": "2026-10-02T09:39:17.433Z"
            })
            .to_string()
            .into_bytes(),
        )
    };
    let body = "b2000000-0000-4000-8000-000000000001";
    vec![
        color(
            "b1000000-0000-4000-8000-000000000001",
            "brand-accent",
            "#FF5500",
            1.0,
        ),
        color(
            "b1000000-0000-4000-8000-000000000002",
            "overlay",
            "#000000",
            0.5,
        ),
        (
            format!("files/{FILE_ID}/typographies/{body}.json"),
            json!({
                "lineHeight": "1.2",
                "path": "TireBud",
                "fontStyle": "normal",
                "fontId": "gfont-geist",
                "fontSize": "16",
                "fontWeight": "400",
                "name": "Body",
                "modifiedAt": "2026-10-02T10:37:00.964Z",
                "fontVariantId": "regular",
                "id": body,
                "letterSpacing": "0.5",
                "fontFamily": "Geist",
                "textTransform": "none"
            })
            .to_string()
            .into_bytes(),
        ),
    ]
}

/// The archive entry of a component.
pub fn component_entry(
    id: &str,
    name: &str,
    path: &str,
    main: u32,
    variant: Option<(&str, &str, &str)>,
) -> (String, Vec<u8>) {
    let mut component = json!({
        "path": path,
        "mainInstanceId": uid(main),
        "name": name,
        "modifiedAt": "2026-10-02T11:05:25.919Z",
        "mainInstancePage": PAGE_K,
        "id": id
    });
    if let Some((variant_id, property, value)) = variant {
        component["variantId"] = json!(variant_id);
        component["variantProperties"] = json!([{ "name": property, "value": value }]);
    }
    (
        format!("files/{FILE_ID}/components/{id}.json"),
        component.to_string().into_bytes(),
    )
}

/// A text shape with one run of `text` in Inter 14, filled `hex`.
pub fn label(n: u32, name: &str, bounds: [f64; 4], text: &str, hex: &str) -> S {
    shape(n, "text", name, bounds).with(json!({
        "growType": "fixed",
        "content": text_content(vec![paragraph(vec![run(
            text,
            json!({ "fontSize": "14", "fills": [{ "fillColor": hex, "fillOpacity": 1 }] }),
        )])])
    }))
}

/// The four layers of a Button: Label, Dot, Hint and the nested copy of the
/// bell icon (with its Glyph), numbered from `first`, at the top left of `at`.
/// In a copy of the main `main`, each layer points at the main's layer it
/// copies.
fn button_layers(first: u32, at: [f64; 2], main: Option<u32>) -> Vec<S> {
    let [x, y] = at;
    let mut layers = vec![
        label(
            first,
            "Label",
            [x + 8.0, y + 12.0, 60.0, 16.0],
            "Köp",
            "#F4F4F5",
        ),
        shape(first + 1, "rect", "Dot", [x + 76.0, y + 16.0, 8.0, 8.0])
            .with(json!({ "fills": solid("#DC2626") })),
        label(
            first + 2,
            "Hint",
            [x + 92.0, y + 12.0, 30.0, 16.0],
            "Tips",
            "#A1A1AA",
        ),
        shape(
            first + 3,
            "frame",
            "Ikon / bell",
            [x + 132.0, y + 12.0, 16.0, 16.0],
        )
        .with(json!({
            "componentId": C_BELL,
            "componentFile": FILE_ID,
            "shapeRef": uid(n::BELL),
            "showContent": true
        }))
        .kids(vec![
            shape(
                first + 4,
                "rect",
                "Glyph",
                [x + 132.0, y + 12.0, 16.0, 16.0],
            )
            .with(json!({ "fills": solid("#F2B33D"), "shapeRef": uid(n::BELL_GLYPH) })),
        ]),
    ];
    if let Some(main) = main {
        for (layer, copied) in layers.iter_mut().zip(main + 1..) {
            set(layer, json!({ "shapeRef": uid(copied) }));
        }
        set(&mut layers[3].kids[0], json!({ "shapeRef": uid(main + 5) }));
    }
    layers
}

/// Sets the fields of `extra` on a shape, replacing those it has.
pub fn set(shape: &mut S, extra: Value) {
    for (key, value) in extra.as_object().expect("`set` takes an object") {
        shape.json[key] = value.clone();
    }
}

/// The main instance of a Button variant.
fn button_main(main: u32, component: &str, variant: &str, fill: &str, at: [f64; 2]) -> S {
    let [x, y] = at;
    shape(main, "frame", "Button", [x, y, 160.0, 40.0])
        .with(flex("row"))
        .with(json!({
            "fills": solid(fill),
            "componentRoot": true,
            "mainInstance": true,
            "componentId": component,
            "componentFile": FILE_ID,
            "variantId": V_BUTTON,
            "variantName": variant,
            "touched": []
        }))
        .kids(button_layers(main + 1, at, None))
}

/// A copy of the Button whose main is `main`, numbered `copy` with its layers
/// numbered from `copy + 1`.
pub fn button_copy(
    copy: u32,
    main: u32,
    component: &str,
    fill: &str,
    name: &str,
    at: [f64; 2],
) -> S {
    let [x, y] = at;
    shape(copy, "frame", name, [x, y, 160.0, 40.0])
        .with(flex("row"))
        .with(json!({
            "fills": solid(fill),
            "componentRoot": true,
            "componentId": component,
            "componentFile": FILE_ID,
            "shapeRef": uid(main),
            "touched": []
        }))
        .kids(button_layers(copy + 1, at, Some(main)))
}

/// The main instance of an icon: a 16 by 16 frame holding a Glyph.
fn icon_main(main: u32, component: &str, name: &str, glyph: &str, at: [f64; 2]) -> S {
    let [x, y] = at;
    shape(main, "frame", name, [x, y, 16.0, 16.0])
        .with(json!({
            "componentRoot": true,
            "mainInstance": true,
            "componentId": component,
            "componentFile": FILE_ID,
            "showContent": true,
            "touched": []
        }))
        .kids(vec![
            shape(main + 1, "rect", "Glyph", [x, y, 16.0, 16.0])
                .with(json!({ "fills": solid(glyph) })),
        ])
}

/// The page Komponenter: the board Knappar holds the variant set Button
/// (Primär and Sekundär, each a Label, a Dot, a Hint and a nested copy of the
/// bell icon), and the board Ikoner the icons bell and star.
pub fn components_page() -> SyntheticPage {
    let knappar = shape(n::KNAPPAR, "frame", "Knappar", [0.0, 0.0, 400.0, 300.0])
        .with(json!({ "fills": solid("#131316") }))
        .kids(vec![
            shape(n::CONTAINER, "frame", "Button", [20.0, 20.0, 360.0, 120.0])
                .with(flex("row"))
                .with(json!({ "isVariantContainer": true }))
                .kids(vec![
                    button_main(n::PRIMARY, C_PRIMARY, "Primär", "#01B597", [20.0, 20.0]),
                    button_main(
                        n::SECONDARY,
                        C_SECONDARY,
                        "Sekundär",
                        "#2A2A31",
                        [200.0, 20.0],
                    ),
                ]),
        ]);
    let ikoner = shape(n::IKONER, "frame", "Ikoner", [500.0, 0.0, 200.0, 100.0])
        .with(json!({ "fills": solid("#131316") }))
        .kids(vec![
            icon_main(n::BELL, C_BELL, "bell", "#F2B33D", [520.0, 20.0]),
            icon_main(n::STAR, C_STAR, "star", "#60A5FA", [560.0, 20.0]),
        ]);
    page_of(PAGE_K, "Komponenter", 0, vec![knappar, ikoner])
}

/// The page Start: the board Hem, filled with the token `color.surface` and
/// stroked with `color.border`, holds
///
/// - Rubrik, a text filled with the token `color.text`;
/// - Köp-knapp, an untouched copy of Button / Primär;
/// - Betala-knapp, a copy of Button / Sekundär whose Label says "Betala", whose
///   Dot is green (#16A34A), whose Hint is hidden and whose icon is swapped
///   for the star;
/// - Skugg-knapp, a copy of Button / Primär whose Dot got a shadow, which no
///   override can hold;
/// - Bild, a rect filled with an image;
/// - Union, a boolean shape, which the converter leaves out.
///
/// The text Anteckning sits on the page outside every board.
pub fn start_page() -> SyntheticPage {
    let mut pay = button_copy(
        n::PAY,
        n::SECONDARY,
        C_SECONDARY,
        "#2A2A31",
        "Betala-knapp",
        [20.0, 120.0],
    );
    set(
        &mut pay.kids[0],
        json!({
            "touched": ["text-content-text"],
            "content": text_content(vec![paragraph(vec![run(
                "Betala",
                json!({ "fontSize": "14", "fills": [{ "fillColor": "#F4F4F5", "fillOpacity": 1 }] }),
            )])])
        }),
    );
    set(
        &mut pay.kids[1],
        json!({ "touched": ["fill-group"], "fills": solid("#16A34A") }),
    );
    set(
        &mut pay.kids[2],
        json!({ "touched": ["visibility-group"], "hidden": true }),
    );
    set(
        &mut pay.kids[3],
        json!({
            "name": "Ikon / star",
            "componentId": C_STAR,
            "shapeRef": uid(n::STAR),
            "touched": [format!("swap-slot-{}", uid(n::SECONDARY_ICON))]
        }),
    );
    set(
        &mut pay.kids[3].kids[0],
        json!({ "fills": solid("#60A5FA"), "shapeRef": uid(n::STAR_GLYPH) }),
    );
    let mut shadowed = button_copy(
        n::SHADOWED,
        n::PRIMARY,
        C_PRIMARY,
        "#01B597",
        "Skugg-knapp",
        [20.0, 180.0],
    );
    set(
        &mut shadowed.kids[1],
        json!({
            "touched": ["layer-effects-group"],
            "shadow": [{
                "id": "f2000000-0000-4000-8000-000000000001",
                "style": "drop-shadow",
                "color": { "color": "#000000", "opacity": 0.5 },
                "offsetX": 0,
                "offsetY": 2,
                "blur": 4,
                "spread": 0,
                "hidden": false
            }]
        }),
    );
    let hem = shape(n::HEM, "frame", "Hem", [0.0, 0.0, 400.0, 600.0])
        .with(json!({
            "fills": solid("#1C1C21"),
            "strokes": [{
                "strokeColor": "#E5484D",
                "strokeOpacity": 0.4,
                "strokeWidth": 1,
                "strokeStyle": "solid",
                "strokeAlignment": "inner"
            }],
            "appliedTokens": { "fill": "color.surface", "strokeColor": "color.border" }
        }))
        .kids(vec![
            label(
                n::RUBRIK,
                "Rubrik",
                [20.0, 20.0, 200.0, 24.0],
                "Välkommen",
                "#F4F4F5",
            )
            .with(json!({ "appliedTokens": { "fill": "color.text" } })),
            button_copy(
                n::BUY,
                n::PRIMARY,
                C_PRIMARY,
                "#01B597",
                "Köp-knapp",
                [20.0, 60.0],
            ),
            pay,
            shadowed,
            shape(n::PICTURE, "rect", "Bild", [220.0, 60.0, 100.0, 100.0])
                .with(json!({ "fills": [image_fill(MEDIA)] })),
            shape(n::UNION, "bool", "Union", [220.0, 180.0, 40.0, 40.0])
                .with(json!({ "fills": solid("#FF0000") })),
        ]);
    let note = label(
        n::NOTE,
        "Anteckning",
        [500.0, 20.0, 120.0, 16.0],
        "Utkast",
        "#FFFFFF",
    );
    page_of(PAGE_S, "Start", 1, vec![hem, note])
}

/// The archive entries of the library export: both pages, the tokens, the
/// library colors and typography, the four components and the image.
pub fn library_entries_all() -> Vec<(String, Vec<u8>)> {
    let mut entries = export_entries(
        FILE_ID,
        "TireBud - Syntetisk",
        &[components_page(), start_page()],
    );
    entries.extend(token_entries(&tokens_json()));
    entries.extend(library_entries());
    entries.push(component_entry(
        C_PRIMARY,
        "Button",
        "",
        n::PRIMARY,
        Some((V_BUTTON, "Typ", "Primär")),
    ));
    entries.push(component_entry(
        C_SECONDARY,
        "Button",
        "",
        n::SECONDARY,
        Some((V_BUTTON, "Typ", "Sekundär")),
    ));
    entries.push(component_entry(C_BELL, "bell", "Ikon", n::BELL, None));
    entries.push(component_entry(C_STAR, "star", "Ikon", n::STAR, None));
    entries.extend(media_entries(MEDIA, STORED));
    entries
}

/// The path of the library export, written to the temporary directory.
pub fn library_export(test: &str) -> PathBuf {
    write_temp(
        &format!("import-{test}.penpot"),
        &stored_zip(&library_entries_all()),
    )
}

/// Imports the export at `path`; the import must succeed.
pub fn import(path: &PathBuf) -> Import {
    import_file(path).unwrap_or_else(|error| panic!("cannot import {path:?}: {error}"))
}

/// Imports the library export.
pub fn import_library(test: &str) -> Import {
    import(&library_export(test))
}

/// The page of the import named `name`.
pub fn page<'a>(import: &'a Import, name: &str) -> &'a Node {
    import
        .pages
        .iter()
        .find(|page| page.name() == name)
        .unwrap_or_else(|| {
            let names: Vec<&str> = import.pages.iter().map(|page| page.name()).collect();
            panic!("the import has no page {name:?}; it has {names:?}")
        })
}

/// The JSON of a node.
pub fn json(node: &Node) -> Value {
    serde_json::to_value(node).expect("a node serializes")
}

/// The JSON node named `name` in `root` or below it, the first in document
/// order.
pub fn find_json<'a>(root: &'a Value, name: &str) -> Option<&'a Value> {
    if root["name"] == name {
        return Some(root);
    }
    root["children"]
        .as_array()?
        .iter()
        .find_map(|child| find_json(child, name))
}

pub fn named<'a>(root: &'a Value, name: &str) -> &'a Value {
    find_json(root, name).unwrap_or_else(|| panic!("no node named {name:?} in {root}"))
}

/// The components of the design system doc, as JSON nodes.
pub fn components(import: &Import) -> Vec<&Value> {
    import.design_system["$components"]
        .as_array()
        .map(|list| list.iter().collect())
        .unwrap_or_default()
}

/// The component with the node ID `id`.
pub fn component_by_id<'a>(import: &'a Import, id: &str) -> &'a Value {
    components(import)
        .into_iter()
        .find(|component| component["id"] == id)
        .unwrap_or_else(|| panic!("no component has the ID {id}"))
}

/// The component of Penpot's `name` (with its `path`): named `name`, or
/// `<path> / <name>` (or `<path>/<name>`) when the Penpot component has a path.
pub fn is_named(component: &Value, name: &str, path: &str) -> bool {
    let actual = component["name"].as_str().unwrap_or("");
    actual == name
        || (!path.is_empty()
            && (actual == format!("{path} / {name}") || actual == format!("{path}/{name}")))
}

/// The components of the import that are Penpot's `name` with `path`.
pub fn components_named<'a>(import: &'a Import, name: &str, path: &str) -> Vec<&'a Value> {
    components(import)
        .into_iter()
        .filter(|component| is_named(component, name, path))
        .collect()
}

/// The default of the variant property `property` of a component.
pub fn variant_default<'a>(component: &'a Value, property: &str) -> Option<&'a str> {
    let def = &component["props"][property];
    (def["type"] == "variant")
        .then(|| def["default"].as_str())
        .flatten()
}

/// The component an instance shows: the one it names, or, when its props pick
/// another variant of that component's set, that variant (the rule of
/// docs/formats/design-tags.md: same name, same variant properties, defaults
/// equal to the instance's values).
pub fn shown<'a>(import: &'a Import, instance: &Value) -> &'a Value {
    let id = instance["component"]
        .as_str()
        .expect("an instance names a component");
    let base = component_by_id(import, id);
    let Some(defs) = base["props"].as_object() else {
        return base;
    };
    let variants: Vec<(&String, &Value)> = defs
        .iter()
        .filter(|(_, def)| def["type"] == "variant")
        .collect();
    if variants.is_empty() {
        return base;
    }
    let wanted = |name: &str| -> Option<String> {
        instance["props"][name]
            .as_str()
            .map(str::to_string)
            .or_else(|| variant_default(base, name).map(str::to_string))
    };
    let shows = |candidate: &Value| {
        variants
            .iter()
            .all(|(name, _)| variant_default(candidate, name).map(str::to_string) == wanted(name))
    };
    if shows(base) {
        return base;
    }
    components(import)
        .into_iter()
        .find(|candidate| {
            candidate["name"] == base["name"]
                && variants
                    .iter()
                    .all(|(name, def)| candidate["props"][name.as_str()]["values"] == def["values"])
                && shows(candidate)
        })
        .unwrap_or(base)
}

/// The token tree of the design system doc, read by OmniState's own checks.
pub fn token_tree(import: &Import) -> TokenTree {
    let mut tree = import.design_system.clone();
    tree.as_object_mut()
        .expect("the design system is a JSON object")
        .remove("$components");
    TokenTree::from_json(&tree)
        .unwrap_or_else(|error| panic!("the imported token tree is refused: {error}\n{tree:#}"))
}

/// Every token of a DTCG tree, by path: (path, token object).
pub fn tokens_of(tree: &Value) -> Vec<(String, Value)> {
    let mut found = Vec::new();
    walk_tokens(tree, "", &mut found);
    found
}

fn walk_tokens(node: &Value, path: &str, found: &mut Vec<(String, Value)>) {
    let Some(map) = node.as_object() else { return };
    if map.contains_key("$value") {
        found.push((path.to_string(), node.clone()));
        return;
    }
    for (name, child) in map {
        if name.starts_with('$') {
            continue;
        }
        let path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        walk_tokens(child, &path, found);
    }
}

/// The value of the token at `path` in `mode`, references followed.
pub fn resolve(tree: &TokenTree, path: &str, mode: &str) -> Option<Value> {
    let reference = format!("${path}").parse().ok()?;
    tree.resolve(&reference, mode)
}

/// A color as `[r, g, b, a]`, from `#rrggbb` or `#rrggbbaa` in either case.
pub fn rgba_of(value: &Value) -> Option<[u8; 4]> {
    let text = value.as_str()?.strip_prefix('#')?;
    let byte = |at: usize| u8::from_str_radix(text.get(at..at + 2)?, 16).ok();
    match text.len() {
        6 => Some([byte(0)?, byte(2)?, byte(4)?, 255]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

/// The entries of the report about the shape number `n`.
pub fn entries_about(import: &Import, n: u32) -> Vec<&ImportEntry> {
    let id = uid(n);
    import
        .report
        .iter()
        .filter(|entry| entry.shape_id == id)
        .collect()
}
