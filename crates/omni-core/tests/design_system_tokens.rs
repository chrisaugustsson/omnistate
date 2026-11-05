//! US-028: the token tree of a project's design system doc, in the W3C Design
//! Tokens (DTCG) format, with one value per mode (Q27).
//!
//! The contract these tests pin down (`omni_core::design::tokens`; the rest is
//! the green step's choice):
//!
//! - `TokenTree::from_json(&Value)` reads and checks a token tree and
//!   `to_json()` writes it back exactly as it was read. A refusal is a
//!   `TokenError` whose message names the token (its dot path, such as
//!   `color.surface`) or the mode that is wrong.
//! - Tokens are grouped by type in the top-level groups `color` (`$type`
//!   `color`), `spacing` and `radius` (`dimension`), `typography`
//!   (`typography`) and `shadow` (`shadow`). A group may state `$type` for its
//!   tokens, or each token states its own; a type that does not fit the group
//!   is refused, and so is a `$value` that does not fit the type. Colors are
//!   `#rrggbb` or `#rrggbbaa`, dimensions are strings such as `"16px"`,
//!   typography values are objects of `fontFamily`, `fontSize`, `fontWeight`,
//!   `lineHeight` and `letterSpacing`, and shadow values objects of `color`,
//!   `offsetX`, `offsetY`, `blur` and `spread`. Token and group names are
//!   what a design can write after `$`: letters, digits, `-` and `_`.
//! - Inside a token file a token refers to another as `{color.surface}`, as a
//!   whole `$value`, a whole mode value or a field of a composite value. In
//!   design tags the same token is `$color.surface`, and
//!   `resolve(&TokenRef, mode)` gives the value of that token in that mode
//!   with every reference followed (composite fields included). `$color.ink`
//!   inside a token file is not a reference and is refused.
//! - The design system lists its modes at the root, in
//!   `$extensions.omnistate.modes` (an array of distinct names), and
//!   `modes()` returns them in order. A token's value per mode is the object
//!   `$extensions.omnistate.modes` on the token, which gives every listed mode
//!   and no other; a token without it has its `$value` in every mode. Other
//!   `$extensions` and `$description` are kept.
//! - A reference to a token that does not exist (or to a group) and a
//!   reference loop, in any mode, are refused; a loop that only one mode has
//!   is refused with a message that names that mode. A loop between `$value`s
//!   is refused too when a token on it has mode values that end the chain.
//! - A typography token may list the font files it uses in
//!   `$extensions.omnistate.fontFiles`, each `attachment:<id>`;
//!   `font_files()` lists every one in the tree. A URL or anything else is
//!   refused.
//! - Every typography value has a definite `lineHeight` (a number, a
//!   multiple of the font size), in every mode (Q44).
//! - `for_mode(mode)` writes the tree of one mode, for one file per mode: each
//!   token's `$value` is its value in that mode (references stay references),
//!   no token has `$extensions.omnistate.modes`, and the root lists that one
//!   mode. It is a valid tree. An unknown mode gives `None`.
//! - docs/formats/tokens.md describes the format, with a complete example that
//!   these rules accept.

mod common;

use common::{with, without};
use omni_core::design::tags;
use omni_core::design::tokens::TokenTree;
use omni_core::design::{AttachmentRef, Node, TokenRef};
use serde_json::{Value, json};
use std::path::Path;

const INTER: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd";
const INTER_BOLD: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abce";

/// A small design system with a light and a dark mode and a token of every
/// group.
fn sample() -> Value {
    json!({
        "$extensions": {"omnistate": {"modes": ["light", "dark"]}},
        "color": {
            "$type": "color",
            "ink": {"$value": "#0f172a"},
            "paper": {"$value": "#ffffff"},
            "surface": {
                "$value": "#ffffff",
                "$description": "Cards and sheets",
                "$extensions": {
                    "omnistate": {"modes": {"light": "#ffffff", "dark": "#0f172a"}},
                    "com.example.tool": {"locked": true}
                }
            },
            "text": {
                "$value": "{color.ink}",
                "$extensions": {"omnistate": {"modes": {"light": "{color.ink}", "dark": "{color.paper}"}}}
            },
            "shadow": {"$value": "#0f172a33"},
            "brand": {"500": {"$value": "#3b82f6"}}
        },
        "spacing": {
            "$type": "dimension",
            "sm": {"$value": "8px"},
            "md": {"$value": "16px"},
            "gutter": {"$value": "{spacing.md}"}
        },
        "radius": {
            "$type": "dimension",
            "md": {"$value": "8px"},
            "card": {"$value": "{radius.md}"}
        },
        "typography": {
            "$type": "typography",
            "body": {
                "$value": {
                    "fontFamily": "Inter",
                    "fontSize": "16px",
                    "fontWeight": 400,
                    "lineHeight": 1.5,
                    "letterSpacing": "0px"
                },
                "$extensions": {"omnistate": {"fontFiles": [INTER]}}
            }
        },
        "shadow": {
            "$type": "shadow",
            "card": {
                "$value": {
                    "color": "{color.shadow}",
                    "offsetX": "0px",
                    "offsetY": "4px",
                    "blur": "12px",
                    "spread": "-2px"
                }
            }
        }
    })
}

fn accept(json: &Value) -> TokenTree {
    TokenTree::from_json(json).unwrap_or_else(|error| {
        panic!("this token tree must be accepted, but it was refused: {error}\n{json:#}")
    })
}

/// Asserts that `json` is refused with a message that has every needle in
/// it, and returns the message.
fn refuse(json: &Value, why: &str, needles: &[&str]) -> String {
    match TokenTree::from_json(json) {
        Ok(_) => panic!("{why}, but this token tree was accepted:\n{json:#}"),
        Err(error) => {
            let message = error.to_string();
            for needle in needles {
                assert!(
                    message.contains(needle),
                    "{why}: the message should name {needle:?}, but it is {message:?}"
                );
            }
            message
        }
    }
}

fn token(reference: &str) -> TokenRef {
    reference
        .parse()
        .unwrap_or_else(|_| panic!("`{reference}` is a token reference"))
}

fn resolve(tree: &TokenTree, reference: &str, mode: &str) -> Value {
    tree.resolve(&token(reference), mode)
        .unwrap_or_else(|| panic!("`{reference}` resolves in the {mode} mode"))
}

/// Every token of a tree (an object with `$value`), with its dot path.
fn tokens_of(json: &Value, path: &str, found: &mut Vec<(String, Value)>) {
    let Some(object) = json.as_object() else {
        return;
    };
    if object.contains_key("$value") {
        found.push((path.to_string(), json.clone()));
        return;
    }
    for (name, child) in object {
        if name.starts_with('$') {
            continue;
        }
        let child_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}.{name}")
        };
        tokens_of(child, &child_path, found);
    }
}

// ---------------------------------------------------------------------------
// The DTCG format, grouped by type
// ---------------------------------------------------------------------------

#[test]
fn a_design_system_in_the_dtcg_format_is_accepted_and_written_back_as_read() {
    let json = sample();
    let tree = accept(&json);
    assert_eq!(
        tree.to_json(),
        json,
        "the tree is written back exactly as it was read, $description and other $extensions included"
    );
    assert_eq!(accept(&tree.to_json()), tree, "the written tree reads back");
}

#[test]
fn tokens_are_grouped_by_type_in_the_five_groups() {
    for (group, odd) in [
        ("color", json!({"$type": "dimension", "$value": "16px"})),
        ("spacing", json!({"$type": "color", "$value": "#ffffff"})),
        ("radius", json!({"$type": "color", "$value": "#ffffff"})),
        (
            "typography",
            json!({"$type": "dimension", "$value": "16px"}),
        ),
        ("shadow", json!({"$type": "color", "$value": "#000000"})),
    ] {
        refuse(
            &with(&sample(), &format!("/{group}/odd"), odd),
            &format!("a token in the {group} group has that group's type"),
            &[&format!("{group}.odd")],
        );
    }
    refuse(
        &with(&sample(), "/color/$type", json!("dimension")),
        "the color group holds colors",
        &["color"],
    );
}

#[test]
fn a_token_may_state_its_type_instead_of_its_group() {
    let json = without(&sample(), "/spacing/$type");
    let mut json = json;
    for name in ["sm", "md", "gutter"] {
        json = with(&json, &format!("/spacing/{name}/$type"), json!("dimension"));
    }
    let tree = accept(&json);
    assert_eq!(resolve(&tree, "$spacing.gutter", "light"), json!("16px"));
}

#[test]
fn values_must_fit_their_type() {
    for (pointer, bad, path) in [
        ("/color/ink/$value", json!("16px"), "color.ink"),
        ("/color/ink/$value", json!(12), "color.ink"),
        ("/spacing/sm/$value", json!("#ffffff"), "spacing.sm"),
        ("/radius/md/$value", json!("large"), "radius.md"),
        (
            "/shadow/card/$value",
            json!("0 4px 12px #0f172a33"),
            "shadow.card",
        ),
        ("/shadow/card/$value/color", json!("16px"), "shadow.card"),
        (
            "/typography/body/$value",
            json!("Inter 16px"),
            "typography.body",
        ),
    ] {
        refuse(
            &with(&sample(), pointer, bad.clone()),
            &format!("{pointer} = {bad} does not fit the token's type"),
            &[path],
        );
    }
}

#[test]
fn token_names_are_what_a_design_can_write_after_a_dollar() {
    for name in ["brand 500", "a.b", "{x}", "", "$weird"] {
        let json = with(
            &sample(),
            &format!("/color/{name}"),
            json!({"$value": "#000000"}),
        );
        refuse(
            &json,
            &format!("`{name}` cannot be written as `$color.{name}` in a design"),
            &[],
        );
    }
}

// ---------------------------------------------------------------------------
// References: `{color.surface}` in token files, `$color.surface` in designs
// ---------------------------------------------------------------------------

#[test]
fn token_files_refer_to_other_tokens_in_braces() {
    let tree = accept(&sample());
    assert_eq!(resolve(&tree, "$color.text", "light"), json!("#0f172a"));
    assert_eq!(resolve(&tree, "$spacing.gutter", "light"), json!("16px"));
    assert_eq!(resolve(&tree, "$radius.card", "dark"), json!("8px"));
    assert_eq!(
        resolve(&tree, "$shadow.card", "light")["color"],
        json!("#0f172a33"),
        "a reference inside a composite value is followed"
    );
    assert_eq!(resolve(&tree, "$color.brand.500", "dark"), json!("#3b82f6"));
}

#[test]
fn design_syntax_is_no_reference_inside_a_token_file() {
    refuse(
        &with(&sample(), "/color/text/$value", json!("$color.ink")),
        "token files write references as `{color.ink}`, not `$color.ink`",
        &["color.text"],
    );
    refuse(
        &with(
            &sample(),
            "/shadow/card/$value/color",
            json!("$color.shadow"),
        ),
        "token files write references as `{color.shadow}`, not `$color.shadow`",
        &["shadow.card"],
    );
}

#[test]
fn designs_write_the_same_token_with_a_dollar() {
    let node: Node = tags::parse(
        r#"<Frame id="k3f9x2qa" name="Card" dir="col" gap="$spacing.md" fill="$color.surface"></Frame>"#,
    )
    .unwrap_or_else(|error| panic!("the tags parse: {error}"))
    .assign_ids(|| panic!("every tag has an id"));
    let json = serde_json::to_value(&node).expect("a node serializes");
    let fill = json["fills"][0]["color"]
        .as_str()
        .unwrap_or_else(|| panic!("the frame's fill is a token reference: {json}"));
    assert_eq!(fill, "$color.surface");
    let tree = accept(&sample());
    assert_eq!(resolve(&tree, fill, "light"), json!("#ffffff"));
    assert_eq!(resolve(&tree, fill, "dark"), json!("#0f172a"));
    assert_eq!(
        tree.resolve(&token("$color.missing"), "light"),
        None,
        "a token the tree does not have resolves to nothing"
    );
    assert_eq!(
        tree.resolve(&token("$color"), "light"),
        None,
        "a group is not a token"
    );
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

#[test]
fn the_design_system_lists_its_modes() {
    let tree = accept(&sample());
    assert_eq!(tree.modes(), ["light".to_string(), "dark".to_string()]);

    let brands = with(
        &with(
            &sample(),
            "/$extensions/omnistate/modes",
            json!(["acme", "globex"]),
        ),
        "/color/surface/$extensions/omnistate/modes",
        json!({"acme": "#fef3c7", "globex": "#dbeafe"}),
    );
    let brands = with(
        &brands,
        "/color/text/$extensions/omnistate/modes",
        json!({"acme": "{color.ink}", "globex": "{color.ink}"}),
    );
    let tree = accept(&brands);
    assert_eq!(tree.modes(), ["acme".to_string(), "globex".to_string()]);
    assert_eq!(resolve(&tree, "$color.surface", "acme"), json!("#fef3c7"));
    assert_eq!(resolve(&tree, "$color.surface", "globex"), json!("#dbeafe"));
}

#[test]
fn a_token_has_one_value_per_mode() {
    let tree = accept(&sample());
    assert_eq!(resolve(&tree, "$color.surface", "light"), json!("#ffffff"));
    assert_eq!(resolve(&tree, "$color.surface", "dark"), json!("#0f172a"));
    assert_eq!(
        resolve(&tree, "$color.text", "dark"),
        json!("#ffffff"),
        "a mode value may be a reference, followed in that mode"
    );
    for mode in ["light", "dark"] {
        assert_eq!(
            resolve(&tree, "$color.ink", mode),
            json!("#0f172a"),
            "a token without mode values has its $value in every mode"
        );
    }
    assert_eq!(
        tree.resolve(&token("$color.surface"), "sepia"),
        None,
        "a mode the design system does not list resolves to nothing"
    );
}

#[test]
fn mode_values_give_exactly_the_listed_modes() {
    refuse(
        &with(
            &sample(),
            "/color/surface/$extensions/omnistate/modes/sepia",
            json!("#f5e6c8"),
        ),
        "sepia is not a mode of the design system",
        &["color.surface", "sepia"],
    );
    refuse(
        &without(&sample(), "/color/surface/$extensions/omnistate/modes/dark"),
        "a token with mode values has one for every mode",
        &["color.surface", "dark"],
    );
    refuse(
        &with(
            &sample(),
            "/color/surface/$extensions/omnistate/modes/dark",
            json!("16px"),
        ),
        "a mode value fits the token's type",
        &["color.surface"],
    );
    refuse(
        &with(
            &sample(),
            "/$extensions/omnistate/modes",
            json!(["light", "dark", "light"]),
        ),
        "the modes are distinct",
        &["light"],
    );
    refuse(
        &with(&sample(), "/$extensions/omnistate/modes", json!("light")),
        "the modes are a list of names",
        &[],
    );
}

// ---------------------------------------------------------------------------
// Unknown references and loops
// ---------------------------------------------------------------------------

#[test]
fn references_to_unknown_tokens_are_refused() {
    for (pointer, reference, missing) in [
        ("/color/text/$value", "{color.missing}", "color.missing"),
        (
            "/color/text/$extensions/omnistate/modes/dark",
            "{color.night}",
            "color.night",
        ),
        ("/shadow/card/$value/color", "{color.nope}", "color.nope"),
        ("/spacing/gutter/$value", "{space.md}", "space.md"),
        ("/color/text/$value", "{color.brand}", "color.brand"),
    ] {
        refuse(
            &with(&sample(), pointer, json!(reference)),
            &format!("{pointer} = {reference} names no token"),
            &[missing],
        );
    }
}

#[test]
fn reference_loops_are_refused() {
    refuse(
        &with(&sample(), "/color/ink/$value", json!("{color.ink}")),
        "a token cannot refer to itself",
        &["color.ink"],
    );

    let two = with(
        &with(&sample(), "/color/ink/$value", json!("{color.paper}")),
        "/color/paper/$value",
        json!("{color.ink}"),
    );
    refuse(
        &two,
        "two tokens cannot refer to each other",
        &["color.ink"],
    );

    let three = with(
        &with(
            &with(&sample(), "/spacing/sm/$value", json!("{spacing.md}")),
            "/spacing/md/$value",
            json!("{spacing.gutter}"),
        ),
        "/spacing/gutter/$value",
        json!("{spacing.sm}"),
    );
    refuse(&three, "a longer loop is a loop too", &["spacing."]);

    // In the light mode the chain ends; in the dark mode it goes round.
    let dark_only = with(
        &with(
            &sample(),
            "/color/text/$extensions/omnistate/modes/dark",
            json!("{color.surface}"),
        ),
        "/color/surface/$extensions/omnistate/modes/dark",
        json!("{color.text}"),
    );
    refuse(
        &dark_only,
        "a loop in one mode is a loop",
        &["color.", "dark"],
    );
}

#[test]
fn a_loop_through_the_value_of_a_token_with_mode_values_is_refused() {
    // color.surface has a literal value in every mode, so no mode goes round;
    // its $value, which tools without modes read, does.
    let looped = with(
        &with(&sample(), "/color/surface/$value", json!("{color.ink}")),
        "/color/ink/$value",
        json!("{color.surface}"),
    );
    let message = refuse(
        &looped,
        "a loop between $values is a loop, even when one of the tokens has mode values",
        &["color."],
    );
    assert!(
        message.contains("color.surface") || message.contains("color.ink"),
        "the message names a token of the loop: {message:?}"
    );
}

// ---------------------------------------------------------------------------
// Typography: font files and line heights
// ---------------------------------------------------------------------------

#[test]
fn typography_tokens_can_use_attached_font_files() {
    let tree = accept(&sample());
    assert_eq!(
        tree.font_files(),
        vec![INTER.parse::<AttachmentRef>().unwrap()]
    );

    let heading = json!({
        "$value": {
            "fontFamily": "Inter",
            "fontSize": "32px",
            "fontWeight": 700,
            "lineHeight": 1.25,
            "letterSpacing": "-0.5px"
        },
        "$extensions": {"omnistate": {"fontFiles": [INTER_BOLD]}}
    });
    let tree = accept(&with(&sample(), "/typography/heading", heading));
    let mut files: Vec<String> = tree
        .font_files()
        .iter()
        .map(|file| file.as_str().to_string())
        .collect();
    files.sort();
    assert_eq!(files, vec![INTER.to_string(), INTER_BOLD.to_string()]);

    for bad in [
        "https://fonts.example.test/inter.woff2",
        "attachment:inter",
        "/fonts/inter.woff2",
    ] {
        refuse(
            &with(
                &sample(),
                "/typography/body/$extensions/omnistate/fontFiles",
                json!([bad]),
            ),
            &format!("`{bad}` is no attached font file"),
            &["typography.body"],
        );
    }
}

#[test]
fn typography_tokens_always_have_a_line_height() {
    let tree = accept(&sample());
    assert_eq!(
        resolve(&tree, "$typography.body", "light")["lineHeight"],
        json!(1.5)
    );
    refuse(
        &without(&sample(), "/typography/body/$value/lineHeight"),
        "a typography token never falls back to the font's normal line height (Q44)",
        &["typography.body", "lineHeight"],
    );
    refuse(
        &with(
            &sample(),
            "/typography/body/$value/lineHeight",
            json!("normal"),
        ),
        "`normal` is no definite line height (Q44)",
        &["typography.body", "lineHeight"],
    );

    let per_mode = json!({
        "light": {"fontFamily": "Inter", "fontSize": "16px", "fontWeight": 400, "lineHeight": 1.5, "letterSpacing": "0px"},
        "dark": {"fontFamily": "Inter", "fontSize": "16px", "fontWeight": 400, "letterSpacing": "0px"}
    });
    refuse(
        &with(
            &sample(),
            "/typography/body/$extensions/omnistate/modes",
            per_mode,
        ),
        "every mode's typography value has a line height",
        &["typography.body", "lineHeight"],
    );
}

// ---------------------------------------------------------------------------
// One file per mode
// ---------------------------------------------------------------------------

#[test]
fn the_tree_can_be_written_as_one_file_per_mode() {
    let tree = accept(&sample());
    let dark = tree.for_mode("dark").expect("dark is a mode");
    assert_eq!(dark["color"]["surface"]["$value"], json!("#0f172a"));
    assert_eq!(
        dark["color"]["text"]["$value"],
        json!("{color.paper}"),
        "references stay references in the file of a mode"
    );
    assert_eq!(dark["color"]["ink"]["$value"], json!("#0f172a"));
    assert_eq!(
        dark["$extensions"]["omnistate"]["modes"],
        json!(["dark"]),
        "the file of a mode lists that mode alone"
    );
    let mut found = Vec::new();
    tokens_of(&dark, "", &mut found);
    let mut all = Vec::new();
    tokens_of(&sample(), "", &mut all);
    let paths = |tokens: &[(String, Value)]| -> Vec<String> {
        tokens.iter().map(|(path, _)| path.clone()).collect()
    };
    assert_eq!(
        paths(&found),
        paths(&all),
        "the file of a mode has every token: {dark:#}"
    );
    for (path, token) in &found {
        assert!(
            token.pointer("/$extensions/omnistate/modes").is_none(),
            "{path} has its dark value only, no mode values: {token}"
        );
    }
    let back = accept(&dark);
    assert_eq!(resolve(&back, "$color.surface", "dark"), json!("#0f172a"));
    assert_eq!(resolve(&back, "$color.text", "dark"), json!("#ffffff"));

    let light = tree.for_mode("light").expect("light is a mode");
    assert_eq!(light["color"]["surface"]["$value"], json!("#ffffff"));
    assert_eq!(light["color"]["text"]["$value"], json!("{color.ink}"));
    assert_eq!(tree.for_mode("sepia"), None, "sepia is no mode");
}

// ---------------------------------------------------------------------------
// Number, string and boolean tokens, scopes and codeSyntax
// ---------------------------------------------------------------------------

fn with_state() -> Value {
    let mut tree = sample();
    tree["state"] = json!({
        "isOpen": {"$type": "boolean", "$value": false},
        "count": {"$type": "number", "$value": 2},
        "cta": {"$type": "string", "$value": "Pay now"},
        "total": {"$type": "number", "$value": "{state.count}"}
    });
    tree
}

#[test]
fn a_group_without_a_type_holds_number_string_and_boolean_tokens() {
    let tree = accept(&with_state());
    assert_eq!(resolve(&tree, "$state.total", "light"), json!(2));
    assert_eq!(resolve(&tree, "$state.cta", "dark"), json!("Pay now"));
    assert_eq!(resolve(&tree, "$state.isOpen", "light"), json!(false));
}

#[test]
fn state_tokens_refuse_values_of_another_type() {
    for (token, value, message) in [
        (
            "count",
            json!("2"),
            "`state.count`: a number token takes a JSON number",
        ),
        (
            "isOpen",
            json!("false"),
            "`state.isOpen`: a boolean token takes true or false",
        ),
        (
            "cta",
            json!(3),
            "`state.cta`: a string token takes a JSON string",
        ),
    ] {
        let tree = with(&with_state(), &format!("/state/{token}/$value"), value);
        refuse(&tree, token, &[message]);
    }
    refuse(
        &with(&with_state(), "/state/x", json!({"$value": 1})),
        "a token without a type",
        &["`state.x`: a token in a group without a `$type` states its own"],
    );
    refuse(
        &with(
            &with_state(),
            "/state/bg",
            json!({"$type": "color", "$value": "#ffffff"}),
        ),
        "a color in an untyped group",
        &["`state.bg`: a token in a group without a `$type` is a number, string or boolean"],
    );
    refuse(
        &with(
            &with_state(),
            "/state/label",
            json!({"$type": "string", "$value": "{state.count}"}),
        ),
        "a string alias to a number",
        &["state.count", "number"],
    );
}

#[test]
fn a_token_keeps_its_scopes_and_code_syntax() {
    let mut json = with_state();
    json["state"]["cta"]["$extensions"] = json!({"omnistate": {"scopes": ["content"]}});
    json["state"]["isOpen"]["$extensions"] =
        json!({"omnistate": {"codeSyntax": {"css": "--is-open", "swift": "isOpen"}}});
    let tree = accept(&json);
    assert_eq!(tree.to_json(), json);
    let cta = tree.token("state.cta").expect("a token");
    assert_eq!(cta.scopes().map(|s| s.len()), Some(1));
    assert_eq!(cta.scopes().unwrap()[0].name(), "content");
    assert!(cta.code_syntax().is_empty());
    assert_eq!(tree.token("state.count").unwrap().scopes(), None);
    let open = tree.token("state.isOpen").unwrap();
    assert_eq!(open.code_syntax()["css"], "--is-open");
    assert_eq!(open.code_syntax()["swift"], "isOpen");
}

#[test]
fn unknown_scopes_and_malformed_code_syntax_are_refused() {
    let with_omnistate = |token: &str, omnistate: Value| {
        with(
            &with_state(),
            &format!("/state/{token}/$extensions"),
            json!({"omnistate": omnistate}),
        )
    };
    refuse(
        &with_omnistate("cta", json!({"scopes": ["banana"]})),
        "banana is no scope",
        &[
            "`state.cta`: `banana` is not a scope: write some of fill, stroke, textColor, content, visible, opacity, width, height, gap, pad, radius, fontSize, lineHeight, letterSpacing, strokeWidth",
        ],
    );
    refuse(
        &with_omnistate("cta", json!({"scopes": "content"})),
        "scopes is no array",
        &["`state.cta`: scopes is an array of scope names"],
    );
    refuse(
        &with_omnistate("isOpen", json!({"codeSyntax": {"css": 1}})),
        "a codeSyntax name is no string",
        &["`state.isOpen`: codeSyntax maps a platform name to a string"],
    );
    refuse(
        &with_omnistate("isOpen", json!({"codeSyntax": ["css"]})),
        "codeSyntax is no object",
        &["codeSyntax maps a platform name to a string"],
    );
}

#[test]
fn a_new_top_level_group_that_states_number_holds_only_numbers() {
    let json = with(
        &sample(),
        "/limits",
        json!({"$type": "number", "max": {"$value": 3}}),
    );
    assert_eq!(resolve(&accept(&json), "$limits.max", "light"), json!(3));
    refuse(
        &with(
            &json,
            "/limits/label",
            json!({"$type": "string", "$value": "three"}),
        ),
        "a string in a group of numbers",
        &["limits.label"],
    );
    refuse(
        &with(&sample(), "/limits", json!({"$type": "color"})),
        "a new top-level group states number, string or boolean",
        &["`limits`", "number, string or boolean"],
    );
}

#[test]
fn scopes_are_kept_as_written_on_a_token_of_any_type() {
    let mut json = with_state();
    json["color"]["ink"]["$extensions"] = json!({"omnistate": {"scopes": ["content", "content"]}});
    json["state"]["isOpen"]["$extensions"] = json!({"omnistate": {"scopes": ["fill"]}});
    json["state"]["count"]["$extensions"] = json!({"omnistate": {"scopes": []}});
    let tree = accept(&json);
    let names = |path: &str| -> Vec<&str> {
        let token = tree.token(path).expect("a token");
        token
            .scopes()
            .expect("scopes")
            .iter()
            .map(|s| s.name())
            .collect()
    };
    assert_eq!(names("color.ink"), ["content", "content"]);
    assert_eq!(names("state.isOpen"), ["fill"]);
    assert_eq!(names("state.count"), Vec::<&str>::new());
}

// ---------------------------------------------------------------------------
// docs/formats/tokens.md
// ---------------------------------------------------------------------------

#[test]
fn the_format_doc_has_an_example_that_the_rules_accept() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/formats/tokens.md");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("docs/formats/tokens.md describes the token format ({e})"));
    for needle in [
        "$extensions",
        "omnistate",
        "modes",
        "{color.surface}",
        "$color.surface",
    ] {
        assert!(
            text.contains(needle),
            "docs/formats/tokens.md mentions `{needle}`"
        );
    }
    let example = text
        .split("```json")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .expect("docs/formats/tokens.md has a ```json example");
    let json: Value = serde_json::from_str(example)
        .unwrap_or_else(|e| panic!("the example in docs/formats/tokens.md is JSON: {e}"));
    let tree = accept(&json);
    assert!(
        tree.modes().len() >= 2,
        "the example lists at least two modes"
    );
    for group in ["color", "spacing", "radius", "typography", "shadow"] {
        assert!(
            json.get(group).is_some(),
            "the example has the {group} group"
        );
    }
    let mut found = Vec::new();
    tokens_of(&json, "", &mut found);
    assert!(
        found
            .iter()
            .any(|(_, token)| token.pointer("/$extensions/omnistate/modes").is_some()),
        "a token of the example has a value per mode in $extensions.omnistate.modes"
    );
    assert!(
        example.contains("\"{"),
        "a token of the example refers to another in braces"
    );
}
