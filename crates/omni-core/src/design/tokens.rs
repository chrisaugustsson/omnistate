//! Design tokens: the token tree of a project's design system doc, in the W3C
//! Design Tokens (DTCG) format, with one value per mode
//! (docs/formats/tokens.md).

use super::value::{AttachmentRef, Color, FontFamily, TokenRef};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::str::FromStr;

/// A checked token tree: every reference resolves, no reference loops, and
/// every token has a value for each of the design system's modes.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenTree {
    json: Value,
    modes: Vec<String>,
    collections: Vec<Collection>,
    font_files: Vec<AttachmentRef>,
    tokens: BTreeMap<String, Token>,
}

/// A top-level group and the modes its tokens have values in: its own list,
/// or the root list when the group states none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub modes: Vec<String>,
}

/// One mode per collection, such as `color` to `dark`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModeChoice(pub BTreeMap<String, String>);

impl ModeChoice {
    /// The mode chosen for `collection`.
    pub fn get(&self, collection: &str) -> Option<&str> {
        self.0.get(collection).map(String::as_str)
    }

    /// Reads a choice against `tree`: a bare mode such as `dark` selects that
    /// mode in every collection that lists it, and `color:dark,density:compact`
    /// selects a mode per collection. A collection the text leaves out takes
    /// its first mode, so the choice names every collection that has modes.
    pub fn parse(text: &str, tree: &TokenTree) -> Result<ModeChoice, TokenError> {
        let mut chosen: BTreeMap<String, String> = BTreeMap::new();
        if text.contains(':') || text.contains(',') {
            for pair in text.split(',') {
                let Some((name, mode)) = pair
                    .split_once(':')
                    .filter(|(n, m)| !n.is_empty() && !m.is_empty())
                else {
                    return refuse(format!(
                        "`{pair}` is not a collection and a mode: write `color:dark,density:compact`"
                    ));
                };
                let Some(collection) = tree.collections.iter().find(|c| c.name == name) else {
                    let names: Vec<&str> =
                        tree.collections.iter().map(|c| c.name.as_str()).collect();
                    return refuse(format!(
                        "`{name}` is not a collection ({})",
                        names.join(", ")
                    ));
                };
                if collection.modes.is_empty() {
                    return refuse(format!("`{name}` has no modes"));
                }
                if !collection.modes.iter().any(|listed| listed == mode) {
                    return refuse(format!(
                        "`{mode}` is not a mode of `{name}` ({})",
                        collection.modes.join(", ")
                    ));
                }
                if chosen.insert(name.to_string(), mode.to_string()).is_some() {
                    return refuse(format!("`{name}` is chosen twice"));
                }
            }
        } else {
            for collection in &tree.collections {
                if collection.modes.iter().any(|listed| listed == text) {
                    chosen.insert(collection.name.clone(), text.to_string());
                }
            }
            if chosen.is_empty() {
                return refuse(format!("`{text}` is not a mode of any collection"));
            }
        }
        for collection in &tree.collections {
            if let Some(first) = collection.modes.first() {
                chosen
                    .entry(collection.name.clone())
                    .or_insert_with(|| first.clone());
            }
        }
        Ok(ModeChoice(chosen))
    }
}

/// What a token states about itself beyond its value: the fields it may bind
/// to and its name per platform.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Token {
    scopes: Option<Vec<Scope>>,
    code_syntax: BTreeMap<String, String>,
}

impl Token {
    /// The fields the token may bind to, as written; `None` when it states none.
    pub fn scopes(&self) -> Option<&[Scope]> {
        self.scopes.as_deref()
    }

    /// The token's name per platform, such as `css` to `--is-open`.
    pub fn code_syntax(&self) -> &BTreeMap<String, String> {
        &self.code_syntax
    }
}

/// A field that a token may bind to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Fill,
    Stroke,
    TextColor,
    Content,
    Visible,
    Opacity,
    Width,
    Height,
    Gap,
    Pad,
    Radius,
    FontSize,
    LineHeight,
    LetterSpacing,
    StrokeWidth,
}

const SCOPES: [(&str, Scope); 15] = [
    ("fill", Scope::Fill),
    ("stroke", Scope::Stroke),
    ("textColor", Scope::TextColor),
    ("content", Scope::Content),
    ("visible", Scope::Visible),
    ("opacity", Scope::Opacity),
    ("width", Scope::Width),
    ("height", Scope::Height),
    ("gap", Scope::Gap),
    ("pad", Scope::Pad),
    ("radius", Scope::Radius),
    ("fontSize", Scope::FontSize),
    ("lineHeight", Scope::LineHeight),
    ("letterSpacing", Scope::LetterSpacing),
    ("strokeWidth", Scope::StrokeWidth),
];

impl Scope {
    pub fn name(self) -> &'static str {
        SCOPES
            .iter()
            .find(|(_, scope)| *scope == self)
            .map_or("", |(name, _)| name)
    }

    fn from_name(name: &str) -> Option<Self> {
        SCOPES
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, scope)| *scope)
    }
}

/// Why a token tree was refused; the message names the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenError {
    message: String,
}

/// The groups at the top of a tree and the type of the tokens each holds.
const GROUPS: [(&str, TokenType); 5] = [
    ("color", TokenType::Color),
    ("spacing", TokenType::Dimension),
    ("radius", TokenType::Dimension),
    ("typography", TokenType::Typography),
    ("shadow", TokenType::Shadow),
];

/// The deepest chain of references that `resolve` follows.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TokenType {
    Color,
    Dimension,
    Typography,
    Shadow,
    Number,
    String,
    Boolean,
}

const TYPES: [TokenType; 7] = [
    TokenType::Color,
    TokenType::Dimension,
    TokenType::Typography,
    TokenType::Shadow,
    TokenType::Number,
    TokenType::String,
    TokenType::Boolean,
];

impl TokenType {
    fn name(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Dimension => "dimension",
            Self::Typography => "typography",
            Self::Shadow => "shadow",
            Self::Number => "number",
            Self::String => "string",
            Self::Boolean => "boolean",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        TYPES.into_iter().find(|ty| ty.name() == name)
    }

    /// Whether a group without a `$type` may hold tokens of this type.
    fn is_state(self) -> bool {
        matches!(self, Self::Number | Self::String | Self::Boolean)
    }
}

impl fmt::Display for TokenType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

fn refuse<T>(message: String) -> Result<T, TokenError> {
    Err(TokenError { message })
}

/// A reference that a value holds, with the type that the place needs.
struct Reference {
    target: String,
    needs: TokenType,
}

/// A token of the tree, with what the checks need of it.
struct Entry<'a> {
    ty: TokenType,
    object: &'a Map<String, Value>,
}

type Tokens<'a> = HashMap<String, Entry<'a>>;

impl TokenTree {
    /// Reads and checks a DTCG token tree.
    pub fn from_json(json: &Value) -> Result<TokenTree, TokenError> {
        let Some(root) = json.as_object() else {
            return refuse("a token file is a JSON object of token groups".to_string());
        };
        let modes = read_modes(root)?;
        let mut collections: Vec<Collection> = Vec::new();
        let mut tokens = Tokens::new();
        for (name, child) in root {
            match name.as_str() {
                "$extensions" | "$description" => {}
                name if name.starts_with('$') => {
                    return refuse(format!(
                        "`{name}` is not a property of a token file: the root has `$extensions` and `$description`, and the groups {}",
                        group_names()
                    ));
                }
                name => {
                    let Some(group) = child.as_object() else {
                        return refuse(format!("{name} is a group, so it is a JSON object"));
                    };
                    collections.push(Collection {
                        name: name.to_string(),
                        modes: read_group_modes(name, group)?.unwrap_or_else(|| modes.clone()),
                    });
                    match GROUPS.iter().find(|(group, _)| *group == name) {
                        Some((_, ty)) => collect(name, group, Some(*ty), Some(*ty), &mut tokens)?,
                        None => {
                            if !is_name(name) {
                                return refuse(format!(
                                    "`{name}` is not a group name: a design writes `${name}.x`, so names have letters, digits, - and _"
                                ));
                            }
                            collect(name, group, None, None, &mut tokens)?;
                        }
                    }
                }
            }
        }

        let mut font_files = Vec::new();
        let mut metadata = BTreeMap::new();
        let mut references: Vec<(&str, ModeReferences)> = Vec::new();
        let mut paths: Vec<&String> = tokens.keys().collect();
        paths.sort();
        for path in paths {
            let token = &tokens[path];
            let top = path.split('.').next().unwrap_or_default();
            let collection = collections
                .iter()
                .find(|collection| collection.name == top)
                .expect("every token is in a collection");
            let (checked, meta) = check_token(path, token, collection, &mut font_files)?;
            references.push((path.as_str(), checked));
            metadata.insert(path.clone(), meta);
        }

        for (path, values) in &references {
            for (_, found) in values {
                for reference in found {
                    check_reference(path, reference, &tokens)?;
                }
            }
        }
        check_loops(&references, &collections)?;

        Ok(TokenTree {
            json: json.clone(),
            modes,
            collections,
            font_files,
            tokens: metadata,
        })
    }

    /// What the token at the dotted `path`, such as `state.count`, states
    /// about itself.
    pub fn token(&self, path: &str) -> Option<&Token> {
        self.tokens.get(path)
    }

    /// The tree as DTCG JSON, as it was read.
    pub fn to_json(&self) -> Value {
        self.json.clone()
    }

    /// The design system's modes, in the order the tree lists them.
    pub fn modes(&self) -> &[String] {
        &self.modes
    }

    /// The collections of the tree, one per top-level group, in the order of
    /// the file. A collection with no list of its own has the root list.
    pub fn collections(&self) -> Vec<Collection> {
        self.collections.clone()
    }

    /// Whether `choice` names only collections of the tree and modes they list.
    fn allows(&self, choice: &ModeChoice) -> bool {
        choice.0.iter().all(|(name, mode)| {
            self.collections
                .iter()
                .any(|c| c.name == *name && c.modes.iter().any(|listed| listed == mode))
        })
    }

    /// The mode that `choice` gives the collection `name`: the chosen one, or
    /// the collection's first; `None` for a collection without modes.
    fn mode_in<'a>(&'a self, choice: &'a ModeChoice, name: &str) -> Option<&'a str> {
        let collection = self.collections.iter().find(|c| c.name == name)?;
        choice
            .get(name)
            .or_else(|| collection.modes.first().map(String::as_str))
    }

    /// The value of the token that a design refers to, such as
    /// `$color.surface`, in the modes of `choice`, with every reference
    /// followed; each token is read in the mode its own collection has in the
    /// choice.
    pub fn resolve(&self, token: &TokenRef, choice: &ModeChoice) -> Option<Value> {
        if !self.allows(choice) {
            return None;
        }
        self.resolve_path(token.path(), choice, 0)
    }

    /// The tree for a choice of modes: each token's `$value` is its value in
    /// the mode its collection has in `choice`.
    pub fn for_mode(&self, choice: &ModeChoice) -> Option<Value> {
        if !self.allows(choice) {
            return None;
        }
        let mut json = self.json.clone();
        let root = json.as_object_mut()?;
        let mut root_list_mode: Option<String> = None;
        for (name, child) in root.iter_mut() {
            if name.starts_with('$') {
                continue;
            }
            let mode = self.mode_in(choice, name);
            if let Some(mode) = mode {
                flatten(child, mode);
            }
            let own = child
                .get_mut("$extensions")
                .and_then(|extensions| extensions.get_mut("omnistate"))
                .and_then(Value::as_object_mut)
                .and_then(|omnistate| omnistate.get_mut("modes"));
            match own {
                Some(listed) => {
                    *listed = Value::Array(mode.into_iter().map(Value::from).collect());
                }
                None => {
                    if root_list_mode.is_none() {
                        root_list_mode = mode.map(str::to_string);
                    }
                }
            }
        }
        if let Some(mode) = root_list_mode {
            let omnistate = root
                .get_mut("$extensions")?
                .get_mut("omnistate")?
                .as_object_mut()?;
            omnistate.insert("modes".to_string(), Value::Array(vec![mode.into()]));
        }
        Some(json)
    }

    /// Every font file that the typography tokens use.
    pub fn font_files(&self) -> Vec<AttachmentRef> {
        self.font_files.clone()
    }
}

impl fmt::Display for TokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TokenError {}

// Reading -------------------------------------------------------------------------

fn group_names() -> String {
    let names: Vec<&str> = GROUPS.iter().map(|(name, _)| *name).collect();
    names.join(", ")
}

/// Whether `name` can be written after `$` in a design: letters, digits, `-`
/// and `_`.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Whether `name` can name a mode: letters (of any script), digits, `-` and
/// `_`, as Penpot's theme names `Mörkt` and `Ljust` need.
fn is_mode_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

fn is_property(name: &str) -> bool {
    matches!(
        name,
        "$type" | "$description" | "$extensions" | "$deprecated"
    )
}

/// The design system's modes, from `$extensions.omnistate.modes` at the root.
fn read_modes(root: &Map<String, Value>) -> Result<Vec<String>, TokenError> {
    let missing = || {
        refuse(
            "the design system lists no modes: list them at the root in $extensions.omnistate.modes, such as [\"light\", \"dark\"]"
                .to_string(),
        )
    };
    let Some(listed) = root
        .get("$extensions")
        .and_then(|extensions| extensions.get("omnistate"))
        .and_then(|omnistate| omnistate.get("modes"))
    else {
        return missing();
    };
    let modes = mode_names(listed, "$extensions.omnistate.modes")?;
    if modes.is_empty() {
        return missing();
    }
    Ok(modes)
}

/// The modes a top-level group lists in its own `$extensions.omnistate.modes`;
/// `None` when it lists none, and then it has the root list. An empty list is
/// a collection without modes.
fn read_group_modes(
    name: &str,
    group: &Map<String, Value>,
) -> Result<Option<Vec<String>>, TokenError> {
    let Some(extensions) = group.get("$extensions") else {
        return Ok(None);
    };
    let Some(extensions) = extensions.as_object() else {
        return refuse(format!("{name} has $extensions, which is a JSON object"));
    };
    let Some(omnistate) = extensions.get("omnistate") else {
        return Ok(None);
    };
    let Some(omnistate) = omnistate.as_object() else {
        return refuse(format!(
            "{name} has $extensions.omnistate, which is a JSON object"
        ));
    };
    omnistate
        .get("modes")
        .map(|listed| mode_names(listed, &format!("$extensions.omnistate.modes of {name}")))
        .transpose()
}

/// The names in a list of modes, written at `place`.
fn mode_names(listed: &Value, place: &str) -> Result<Vec<String>, TokenError> {
    let Some(listed) = listed.as_array() else {
        return refuse(format!(
            "the modes in {place} are an array of names, such as [\"light\", \"dark\"]"
        ));
    };
    let mut modes: Vec<String> = Vec::new();
    for mode in listed {
        let Some(mode) = mode.as_str().filter(|mode| is_mode_name(mode)) else {
            return refuse(format!(
                "{mode} is not a mode name: a mode is named with letters, digits, - and _, such as light"
            ));
        };
        if modes.iter().any(|known| known == mode) {
            return refuse(format!("the mode {mode} is listed twice in {place}"));
        }
        modes.push(mode.to_string());
    }
    Ok(modes)
}

/// Walks a group, checking its names and types, and collects its tokens.
fn collect<'a>(
    path: &str,
    group: &'a Map<String, Value>,
    holds: Option<TokenType>,
    inherited: Option<TokenType>,
    tokens: &mut Tokens<'a>,
) -> Result<(), TokenError> {
    let mut holds = holds;
    let mut inherited = inherited;
    if let Some(ty) = group.get("$type") {
        let stated = stated_type(path, ty, holds, "the group")?;
        inherited = Some(stated);
        holds = Some(stated);
    }
    for (name, child) in group {
        if name.starts_with('$') {
            if !is_property(name) {
                return refuse(format!(
                    "{path} has `{name}`, which is no group property: a name that starts with $ is a DTCG property"
                ));
            }
            continue;
        }
        if !is_name(name) {
            return refuse(format!(
                "`{name}` in {path} is not a token name: a design writes `${path}.{name}`, so names have letters, digits, - and _"
            ));
        }
        let child_path = format!("{path}.{name}");
        let Some(object) = child.as_object() else {
            return refuse(format!(
                "{child_path} is a token or a group, so it is a JSON object"
            ));
        };
        if object.contains_key("$value") {
            let ty = match (object.get("$type"), inherited) {
                (Some(ty), _) => stated_type(&child_path, ty, holds, "the token")?,
                (None, Some(ty)) => ty,
                (None, None) => {
                    return refuse(format!(
                        "`{child_path}`: a token in a group without a `$type` states its own"
                    ));
                }
            };
            tokens.insert(child_path, Entry { ty, object });
        } else {
            collect(&child_path, object, holds, inherited, tokens)?;
        }
    }
    Ok(())
}

/// The type that a token or group states with `$type`, which has to be the
/// type of the group it is in.
fn stated_type(
    path: &str,
    ty: &Value,
    holds: Option<TokenType>,
    what: &str,
) -> Result<TokenType, TokenError> {
    let Some(stated) = ty.as_str().and_then(TokenType::from_name) else {
        return refuse(format!(
            "{path} has the type {ty}, which is none of color, dimension, typography, shadow, number, string and boolean"
        ));
    };
    match holds {
        Some(holds) if stated != holds => {
            return refuse(format!(
                "{path} is {what} of type {stated}, but its group holds {holds} tokens"
            ));
        }
        None if what == "the group" && !stated.is_state() => {
            return refuse(format!(
                "`{path}`: a group without a `$type` holds tokens of its own types, and states only number, string or boolean for them all"
            ));
        }
        _ => {}
    }
    Ok(stated)
}

/// What one value of a token refers to, with the mode it is the value in
/// (`None` for the `$value`).
type ModeReferences<'a> = Vec<(Option<&'a str>, Vec<Reference>)>;

/// Checks a token's properties, its value and its value in each mode; returns
/// the references that those values hold.
fn check_token<'a>(
    path: &str,
    token: &'a Entry<'a>,
    collection: &'a Collection,
    font_files: &mut Vec<AttachmentRef>,
) -> Result<(ModeReferences<'a>, Token), TokenError> {
    for name in token.object.keys() {
        if name != "$value" && !is_property(name) {
            return refuse(format!(
                "{path} has `{name}`, which is no token property: a token has $value, $type, $description and $extensions"
            ));
        }
    }
    let mut meta = Token::default();
    let mut found = Vec::new();
    let mut references = Vec::new();
    check_value(path, token.ty, &token.object["$value"], &mut references)?;
    found.push((None, references));

    if let Some(extensions) = token.object.get("$extensions") {
        let Some(extensions) = extensions.as_object() else {
            return refuse(format!("{path} has $extensions, which is a JSON object"));
        };
        if let Some(omnistate) = extensions.get("omnistate") {
            let Some(omnistate) = omnistate.as_object() else {
                return refuse(format!(
                    "{path} has $extensions.omnistate, which is a JSON object"
                ));
            };
            for (name, value) in omnistate {
                match name.as_str() {
                    "modes" => check_modes(path, token, value, collection, &mut found)?,
                    "fontFiles" => check_font_files(path, token.ty, value, font_files)?,
                    "scopes" => meta.scopes = Some(read_scopes(path, value)?),
                    "codeSyntax" => meta.code_syntax = read_code_syntax(path, value)?,
                    other => {
                        return refuse(format!(
                            "{path} has `{other}` in $extensions.omnistate; a token has `modes`, `fontFiles`, `scopes` and `codeSyntax` there"
                        ));
                    }
                }
            }
        }
    }
    Ok((found, meta))
}

fn read_scopes(path: &str, value: &Value) -> Result<Vec<Scope>, TokenError> {
    let Some(listed) = value.as_array() else {
        return refuse(format!("`{path}`: scopes is an array of scope names"));
    };
    listed
        .iter()
        .map(|scope| {
            scope
                .as_str()
                .and_then(Scope::from_name)
                .ok_or_else(|| TokenError {
                    message: format!(
                        "`{path}`: `{}` is not a scope: write some of {}",
                        scope
                            .as_str()
                            .map_or_else(|| scope.to_string(), str::to_string),
                        SCOPES.map(|(name, _)| name).join(", ")
                    ),
                })
        })
        .collect()
}

fn read_code_syntax(path: &str, value: &Value) -> Result<BTreeMap<String, String>, TokenError> {
    let names = value.as_object().and_then(|platforms| {
        platforms
            .iter()
            .map(|(platform, name)| Some((platform.clone(), name.as_str()?.to_string())))
            .collect()
    });
    match names {
        Some(names) => Ok(names),
        None => refuse(format!(
            "`{path}`: codeSyntax maps a platform name to a string"
        )),
    }
}

fn check_modes<'a>(
    path: &str,
    token: &Entry<'_>,
    value: &Value,
    collection: &'a Collection,
    found: &mut ModeReferences<'a>,
) -> Result<(), TokenError> {
    let Some(per_mode) = value.as_object() else {
        return refuse(format!(
            "{path} has $extensions.omnistate.modes, which is an object with a value for each mode"
        ));
    };
    let Collection { name: group, modes } = collection;
    if modes.is_empty() {
        return refuse(format!(
            "`{path}`: `{group}` has no modes, so its tokens have no mode values"
        ));
    }
    for name in per_mode.keys() {
        if !modes.iter().any(|mode| mode == name) {
            return refuse(format!(
                "`{path}`: `{name}` is not a mode of `{group}` ({})",
                modes.join(", ")
            ));
        }
    }
    for mode in modes {
        let Some(value) = per_mode.get(mode) else {
            return refuse(format!(
                "{path} has no value for the {mode} mode: a token with mode values has one for every mode"
            ));
        };
        let mut references = Vec::new();
        check_value(
            &format!("{path} in the {mode} mode"),
            token.ty,
            value,
            &mut references,
        )?;
        found.push((Some(mode), references));
    }
    Ok(())
}

fn check_font_files(
    path: &str,
    ty: TokenType,
    value: &Value,
    font_files: &mut Vec<AttachmentRef>,
) -> Result<(), TokenError> {
    if ty != TokenType::Typography {
        return refuse(format!(
            "{path} lists font files, but only typography tokens use them"
        ));
    }
    let Some(files) = value.as_array() else {
        return refuse(format!(
            "{path} has fontFiles, which is an array of attachment:<id> entries"
        ));
    };
    for file in files {
        let file = file
            .as_str()
            .and_then(|text| AttachmentRef::from_str(text).ok());
        let Some(file) = file else {
            return refuse(format!(
                "{path} has a font file that is no attachment: fontFiles entries are files uploaded to the workspace, written attachment:<id>"
            ));
        };
        if !font_files.contains(&file) {
            font_files.push(file);
        }
    }
    Ok(())
}

/// The path that a whole string refers to when it is written `{color.surface}`.
fn reference_in(text: &str) -> Option<&str> {
    let path = text.strip_prefix('{')?.strip_suffix('}')?;
    path.split('.').all(is_name).then_some(path)
}

/// Checks that `value` fits `ty` and collects the references in it.
fn check_value(
    path: &str,
    ty: TokenType,
    value: &Value,
    references: &mut Vec<Reference>,
) -> Result<(), TokenError> {
    match ty {
        TokenType::Color => check_color(path, value, references),
        TokenType::Dimension => check_dimension(path, value, references).map(|_| ()),
        TokenType::Typography => check_typography(path, value, references),
        TokenType::Shadow => check_shadow(path, value, references),
        TokenType::Number => check_state(
            path,
            ty,
            value,
            references,
            |v| v.is_number(),
            "takes a JSON number",
        ),
        TokenType::String => check_state(
            path,
            ty,
            value,
            references,
            |v| v.is_string(),
            "takes a JSON string",
        ),
        TokenType::Boolean => check_state(
            path,
            ty,
            value,
            references,
            |v| v.is_boolean(),
            "takes true or false",
        ),
    }
}

/// A number, string or boolean: a literal that `fits`, or a reference to a
/// token of the same type.
fn check_state(
    path: &str,
    ty: TokenType,
    value: &Value,
    references: &mut Vec<Reference>,
    fits: fn(&Value) -> bool,
    takes: &str,
) -> Result<(), TokenError> {
    if let Some(target) = value.as_str().and_then(reference_in) {
        references.push(Reference {
            target: target.to_string(),
            needs: ty,
        });
        return Ok(());
    }
    if fits(value) {
        return Ok(());
    }
    refuse(format!("`{path}`: a {ty} token {takes}"))
}

fn check_color(
    path: &str,
    value: &Value,
    references: &mut Vec<Reference>,
) -> Result<(), TokenError> {
    let text = value.as_str();
    if let Some(target) = text.and_then(reference_in) {
        references.push(Reference {
            target: target.to_string(),
            needs: TokenType::Color,
        });
        return Ok(());
    }
    match text {
        Some(text) if Color::from_str(text).is_ok() => Ok(()),
        Some(text) if text.starts_with('$') => refuse(format!(
            "{path} has {value}, which is no color: a token file refers to another token in braces, as {{{}}}",
            &text[1..]
        )),
        _ => refuse(format!(
            "{path} has {value}, which is no color: write #rrggbb or #rrggbbaa, or refer to a color token as {{color.surface}}"
        )),
    }
}

/// The pixels of a dimension such as `16px`.
fn pixels(text: &str) -> Option<f64> {
    let number = text.strip_suffix("px")?;
    let digits = number.strip_prefix('-').unwrap_or(number);
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !all_digits(whole) || fraction.is_some_and(|part| !all_digits(part)) {
        return None;
    }
    number
        .parse()
        .ok()
        .filter(|pixels: &f64| pixels.is_finite())
}

/// Checks a dimension, which is a length in pixels written `16px`, and
/// returns the pixels of a literal.
fn check_dimension(
    path: &str,
    value: &Value,
    references: &mut Vec<Reference>,
) -> Result<Option<f64>, TokenError> {
    let text = value.as_str();
    if let Some(target) = text.and_then(reference_in) {
        references.push(Reference {
            target: target.to_string(),
            needs: TokenType::Dimension,
        });
        return Ok(None);
    }
    match text.and_then(pixels) {
        Some(pixels) => Ok(Some(pixels)),
        None => refuse(format!(
            "{path} has {value}, which is no dimension: write a length in pixels such as \"16px\", or refer to a dimension token as {{spacing.md}}"
        )),
    }
}

/// The fields of a composite value; each is checked by the caller.
fn fields_of<'a>(
    path: &str,
    value: &'a Value,
    what: &str,
    known: &[&str],
) -> Result<&'a Map<String, Value>, TokenError> {
    let Some(fields) = value.as_object() else {
        return refuse(format!(
            "{path} has {value}, which is no {what} value: write an object of {}",
            known.join(", ")
        ));
    };
    for name in fields.keys() {
        if !known.contains(&name.as_str()) {
            return refuse(format!(
                "{path} has `{name}`, which is no field of a {what} value ({})",
                known.join(", ")
            ));
        }
    }
    Ok(fields)
}

fn check_typography(
    path: &str,
    value: &Value,
    references: &mut Vec<Reference>,
) -> Result<(), TokenError> {
    const FIELDS: [&str; 5] = [
        "fontFamily",
        "fontSize",
        "fontWeight",
        "lineHeight",
        "letterSpacing",
    ];
    let fields = fields_of(path, value, "typography", &FIELDS)?;
    for required in ["fontFamily", "fontSize", "lineHeight"] {
        if !fields.contains_key(required) {
            let why = if required == "lineHeight" {
                ": a typography token always has a line height (a number, a multiple of the font size), so text never falls back to the font's normal line height"
            } else {
                ""
            };
            return refuse(format!("{path} has no {required}{why}"));
        }
    }
    let family = fields["fontFamily"].as_str();
    if family.is_none_or(|family| family.starts_with('{') || FontFamily::from_str(family).is_err())
    {
        return refuse(format!(
            "{path} has the fontFamily {}, which is no font family: write its name, such as \"Inter\"",
            fields["fontFamily"]
        ));
    }
    let size = check_dimension(&format!("{path} fontSize"), &fields["fontSize"], references)?;
    if size.is_some_and(|size| size < 0.0) {
        return refuse(format!("{path} has a negative fontSize"));
    }
    if let Some(weight) = fields.get("fontWeight") {
        if !weight
            .as_f64()
            .is_some_and(|weight| (1.0..=1000.0).contains(&weight))
        {
            return refuse(format!(
                "{path} has the fontWeight {weight}, which is a number from 1 to 1000"
            ));
        }
    }
    if !fields["lineHeight"]
        .as_f64()
        .is_some_and(|height| height >= 0.0)
    {
        return refuse(format!(
            "{path} has the lineHeight {}, which is no definite line height: write a number, a multiple of the font size, never \"normal\"",
            fields["lineHeight"]
        ));
    }
    if let Some(spacing) = fields.get("letterSpacing") {
        check_dimension(&format!("{path} letterSpacing"), spacing, references)?;
    }
    Ok(())
}

fn check_shadow(
    path: &str,
    value: &Value,
    references: &mut Vec<Reference>,
) -> Result<(), TokenError> {
    const FIELDS: [&str; 5] = ["color", "offsetX", "offsetY", "blur", "spread"];
    let fields = fields_of(path, value, "shadow", &FIELDS)?;
    for required in ["color", "offsetX", "offsetY", "blur"] {
        if !fields.contains_key(required) {
            return refuse(format!("{path} has no {required}"));
        }
    }
    check_color(&format!("{path} color"), &fields["color"], references)?;
    for name in ["offsetX", "offsetY", "blur", "spread"] {
        let Some(field) = fields.get(name) else {
            continue;
        };
        let pixels = check_dimension(&format!("{path} {name}"), field, references)?;
        if name == "blur" && pixels.is_some_and(|blur| blur < 0.0) {
            return refuse(format!("{path} has a negative blur"));
        }
    }
    Ok(())
}

// References ----------------------------------------------------------------------

fn check_reference(path: &str, reference: &Reference, tokens: &Tokens) -> Result<(), TokenError> {
    let target = &reference.target;
    let Some(found) = tokens.get(target) else {
        let is_group = tokens
            .keys()
            .any(|token| token.starts_with(&format!("{target}.")));
        return refuse(if is_group {
            format!("{path} refers to {target}, which is a group, not a token")
        } else {
            format!("{path} refers to {target}, which is no token")
        });
    };
    if found.ty != reference.needs {
        return refuse(format!(
            "{path} refers to {target}, a {} token, where a {} is needed",
            found.ty, reference.needs
        ));
    }
    Ok(())
}

/// Refuses a loop of references in any mode: a token that, through the tokens
/// it refers to, refers back to itself. Each token is read in the mode of its
/// own collection; the passes are every collection in each of its modes (the
/// others in their first), then the `$value`s alone.
fn check_loops(
    references: &[(&str, ModeReferences)],
    collections: &[Collection],
) -> Result<(), TokenError> {
    let first: BTreeMap<&str, &str> = collections
        .iter()
        .filter_map(|c| Some((c.name.as_str(), c.modes.first()?.as_str())))
        .collect();
    let mut passes: Vec<Option<BTreeMap<&str, &str>>> = vec![Some(first.clone())];
    for collection in collections {
        for mode in collection.modes.iter().skip(1) {
            let mut pass = first.clone();
            pass.insert(collection.name.as_str(), mode.as_str());
            passes.push(Some(pass));
        }
    }
    passes.push(None);
    for pass in passes {
        let mode_of = |path: &str| {
            let top = path.split('.').next().unwrap_or_default();
            pass.as_ref().and_then(|pass| pass.get(top).copied())
        };
        // What each token refers to in this pass: its mode value when it has
        // one, else its $value.
        let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
        for (path, values) in references {
            let mode = mode_of(path);
            let chosen = values
                .iter()
                .find(|(of, _)| mode.is_some() && *of == mode)
                .or_else(|| values.iter().find(|(of, _)| of.is_none()));
            let targets = chosen
                .map(|(_, found)| found.iter().map(|r| r.target.as_str()).collect())
                .unwrap_or_default();
            edges.insert(*path, targets);
        }
        let mut done: HashMap<&str, bool> = HashMap::new();
        let mut starts: Vec<&str> = edges.keys().copied().collect();
        starts.sort_unstable();
        for start in starts {
            let mut chain = Vec::new();
            if let Some(cycle) = find_loop(start, &edges, &mut done, &mut chain) {
                return refuse(loop_message(&cycle, mode_of(cycle[0])));
            }
        }
    }
    Ok(())
}

fn loop_message(cycle: &[&str], mode: Option<&str>) -> String {
    let first = cycle[0];
    let place = match mode {
        Some(mode) => format!("in the {mode} mode"),
        None => "through $value".to_string(),
    };
    if cycle.len() == 1 {
        format!("{first} refers to itself, {place}")
    } else {
        format!(
            "{first} refers back to itself {place}, through {}",
            cycle[1..].join(" and ")
        )
    }
}

/// Depth-first search for a loop from `at`; `done` holds the tokens known to
/// lead to no loop (true) or to be on the chain being walked (false).
fn find_loop<'a>(
    at: &'a str,
    edges: &HashMap<&'a str, Vec<&'a str>>,
    done: &mut HashMap<&'a str, bool>,
    chain: &mut Vec<&'a str>,
) -> Option<Vec<&'a str>> {
    match done.get(at) {
        Some(true) => return None,
        Some(false) => {
            let from = chain.iter().position(|token| *token == at)?;
            return Some(chain[from..].to_vec());
        }
        None => {}
    }
    done.insert(at, false);
    chain.push(at);
    for next in edges.get(at).into_iter().flatten() {
        if let Some(cycle) = find_loop(next, edges, done, chain) {
            return Some(cycle);
        }
    }
    chain.pop();
    done.insert(at, true);
    None
}

// Resolving and writing a mode ------------------------------------------------------

/// The token at `path`, an object with a `$value`.
fn token_at<'a>(json: &'a Value, path: &str) -> Option<&'a Map<String, Value>> {
    let mut at = json;
    for name in path.split('.') {
        at = at.as_object()?.get(name)?;
    }
    let token = at.as_object()?;
    token.contains_key("$value").then_some(token)
}

impl TokenTree {
    fn resolve_path(&self, path: &str, choice: &ModeChoice, depth: usize) -> Option<Value> {
        let top = path.split('.').next()?;
        if depth > MAX_DEPTH || top.starts_with('$') {
            return None;
        }
        let token = token_at(&self.json, path)?;
        let mode = self.mode_in(choice, top);
        let value = mode
            .and_then(|mode| {
                token
                    .get("$extensions")
                    .and_then(|extensions| extensions.get("omnistate"))
                    .and_then(|omnistate| omnistate.get("modes"))
                    .and_then(|modes| modes.get(mode))
            })
            .or_else(|| token.get("$value"))?;
        self.follow(value, choice, depth)
    }

    /// `value` with each `{token}` in it replaced by that token's value.
    fn follow(&self, value: &Value, choice: &ModeChoice, depth: usize) -> Option<Value> {
        match value {
            Value::String(text) => match reference_in(text) {
                Some(path) => self.resolve_path(path, choice, depth + 1),
                None => Some(value.clone()),
            },
            Value::Object(fields) => {
                let mut resolved = Map::new();
                for (name, field) in fields {
                    resolved.insert(name.clone(), self.follow(field, choice, depth)?);
                }
                Some(Value::Object(resolved))
            }
            other => Some(other.clone()),
        }
    }
}

/// Rewrites every token under `node` to have its value in `mode` only.
fn flatten(node: &mut Value, mode: &str) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    if object.contains_key("$value") {
        let own = object
            .get_mut("$extensions")
            .and_then(|extensions| extensions.get_mut("omnistate"))
            .and_then(Value::as_object_mut)
            .and_then(|omnistate| omnistate.remove("modes"))
            .and_then(|mut modes| modes.get_mut(mode).map(Value::take));
        if let Some(own) = own {
            object.insert("$value".to_string(), own);
        }
        drop_empty_extensions(object);
        return;
    }
    for (name, child) in object.iter_mut() {
        if !name.starts_with('$') {
            flatten(child, mode);
        }
    }
}

/// Removes the `omnistate` object and `$extensions` once nothing is left in them.
fn drop_empty_extensions(token: &mut Map<String, Value>) {
    let Some(extensions) = token.get_mut("$extensions").and_then(Value::as_object_mut) else {
        return;
    };
    if extensions
        .get("omnistate")
        .and_then(Value::as_object)
        .is_some_and(Map::is_empty)
    {
        extensions.remove("omnistate");
    }
    if extensions.is_empty() {
        token.remove("$extensions");
    }
}
