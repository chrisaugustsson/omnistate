//! Expressions of prototype actions: the value a `setVariable` takes from
//! `expr` and the `condition` of a conditional block, and the checks that a
//! write makes of them against the design system's tokens.
//!
//! The JSON tree and its names are FigCraft's (`crates/doc/src/proto_expr.rs`,
//! MIT OR Apache-2.0; see `NOTICE`), with `{mode}` and `{inMode}` added.

use super::{Action, describe};
use crate::design::tokens::{TokenTree, TokenType};
use crate::design::value::Color;
use serde_json::{Map, Value};
use std::fmt;
use std::str::FromStr;

/// The deepest an expression nests; the root is level 1.
const MAX_DEPTH: usize = 16;

/// The operators of `{op, args}`, with their number of arguments.
const OPS: &[(&str, usize)] = &[
    ("add", 2),
    ("subtract", 2),
    ("multiply", 2),
    ("divide", 2),
    ("equals", 2),
    ("notEqual", 2),
    ("less", 2),
    ("lessOrEqual", 2),
    ("greater", 2),
    ("greaterOrEqual", 2),
    ("and", 2),
    ("or", 2),
    ("not", 1),
    ("negate", 1),
];

/// A refused expression or variable action, in words for the person who wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExprError {
    message: String,
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExprError {}

fn refuse<T>(message: impl Into<String>) -> Result<T, ExprError> {
    Err(ExprError {
        message: message.into(),
    })
}

/// An expression: a literal, a variable, an operator on expressions, the mode
/// of a collection or a variable's value in another mode.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Value(Value),
    Variable(String),
    Op { op: String, args: Vec<Expr> },
    Mode(String),
    InMode { variable: String, mode: String },
}

impl Expr {
    /// Reads an expression from its JSON tree.
    pub fn parse(value: &Value) -> Result<Expr, ExprError> {
        parse_at(value, 1)
    }

    /// The type the expression yields, or why it is refused: a name that is
    /// no token, collection or mode of `tree`, or operands of the wrong types.
    pub fn check(&self, tree: &TokenTree) -> Result<TokenType, ExprError> {
        match self {
            Expr::Value(value) => match value {
                Value::Number(_) => Ok(TokenType::Number),
                Value::String(_) => Ok(TokenType::String),
                Value::Bool(_) => Ok(TokenType::Boolean),
                _ => refuse("a `value` in an expression is a number, string or boolean"),
            },
            Expr::Variable(path) => token_type(tree, path),
            Expr::Mode(collection) => {
                modes_of(tree, collection)?;
                Ok(TokenType::String)
            }
            Expr::InMode { variable, mode } => {
                let ty = token_type(tree, variable)?;
                let collection = variable.split('.').next().unwrap_or_default();
                let modes = modes_of(tree, collection)?;
                known_mode(collection, mode, &modes)?;
                Ok(ty)
            }
            Expr::Op { op, args } => {
                let types = args
                    .iter()
                    .map(|arg| arg.check(tree))
                    .collect::<Result<Vec<_>, _>>()?;
                op_type(op, &types)
            }
        }
    }
}

fn parse_at(value: &Value, level: usize) -> Result<Expr, ExprError> {
    if level > MAX_DEPTH {
        return refuse(format!("an expression nests at most {MAX_DEPTH} levels"));
    }
    let Some(map) = value.as_object() else {
        return refuse(format!(
            "an expression is an object, but this one is {}: write `{{value: 1}}`, `{{variable: \"state.count\"}}` or `{{op: \"add\", args: [...]}}`",
            describe(value)
        ));
    };
    const FIELDS: [&str; 6] = ["value", "variable", "op", "args", "mode", "inMode"];
    let not_a_field = |extra: &str| {
        refuse(format!(
            "`{extra}` is not a field of an expression: write one of {}",
            FIELDS.join(", ")
        ))
    };
    if let Some(extra) = map.keys().find(|key| !FIELDS.contains(&key.as_str())) {
        return not_a_field(extra);
    }
    let (kind, allowed): (&str, &[&str]) = if map.contains_key("op") {
        ("op", &["op", "args"])
    } else if map.contains_key("value") {
        ("value", &["value"])
    } else if map.contains_key("variable") {
        ("variable", &["variable"])
    } else if map.contains_key("mode") {
        ("mode", &["mode"])
    } else if map.contains_key("inMode") {
        ("inMode", &["inMode"])
    } else {
        return refuse(
            "an expression has `value`, `variable`, `op` with `args`, `mode` or `inMode`",
        );
    };
    if let Some(extra) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        return not_a_field(extra);
    }
    Ok(match kind {
        "value" => Expr::Value(map["value"].clone()),
        "variable" => Expr::Variable(name(map, "variable")?),
        "mode" => Expr::Mode(name(map, "mode")?),
        "inMode" => {
            let Some(inner) = map["inMode"].as_object() else {
                return refuse(
                    "`inMode` is an object: write `inMode: {variable: \"color.surface\", mode: \"dark\"}`",
                );
            };
            if let Some(extra) = inner
                .keys()
                .find(|key| !matches!(key.as_str(), "variable" | "mode"))
            {
                return refuse(format!(
                    "`{extra}` is not a field of `inMode`: it takes `variable` and `mode`"
                ));
            }
            Expr::InMode {
                variable: name(inner, "variable")?,
                mode: name(inner, "mode")?,
            }
        }
        _ => {
            let Some(op) = map["op"].as_str() else {
                return refuse("`op` is the name of an operator, such as `op: \"add\"`");
            };
            if !OPS.iter().any(|(known, _)| *known == op) {
                return refuse(format!(
                    "`{op}` is not an operator: write one of {}",
                    OPS.iter()
                        .map(|(known, _)| *known)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            let Some(items) = map.get("args").and_then(Value::as_array) else {
                return refuse(format!(
                    "`{op}` needs `args`, a list of expressions: write `args: [{{value: 1}}]`"
                ));
            };
            Expr::Op {
                op: op.to_string(),
                args: items
                    .iter()
                    .map(|item| parse_at(item, level + 1))
                    .collect::<Result<_, _>>()?,
            }
        }
    })
}

fn name(map: &Map<String, Value>, field: &str) -> Result<String, ExprError> {
    match map.get(field).and_then(Value::as_str) {
        Some(text) => Ok(text.to_string()),
        None => refuse(format!("`{field}` is a name, written as text")),
    }
}

fn token_type(tree: &TokenTree, path: &str) -> Result<TokenType, ExprError> {
    match tree.token(path) {
        Some(token) => Ok(token.ty()),
        None => refuse(format!("`{path}` is not a token")),
    }
}

/// The modes of the collection `name`.
fn modes_of(tree: &TokenTree, name: &str) -> Result<Vec<String>, ExprError> {
    let Some(collection) = tree.collections().into_iter().find(|c| c.name == name) else {
        return refuse(format!("`{name}` is not a collection of the design system"));
    };
    if collection.modes.is_empty() {
        return refuse(format!("`{name}` has no modes"));
    }
    Ok(collection.modes)
}

fn known_mode(collection: &str, mode: &str, modes: &[String]) -> Result<(), ExprError> {
    if modes.iter().any(|listed| listed == mode) {
        return Ok(());
    }
    refuse(format!(
        "`{mode}` is not a mode of `{collection}` ({})",
        modes.join(", ")
    ))
}

/// The type that the operator `op` yields on arguments of the types `types`.
fn op_type(op: &str, types: &[TokenType]) -> Result<TokenType, ExprError> {
    let arity = OPS
        .iter()
        .find(|(known, _)| *known == op)
        .map_or(0, |(_, arity)| *arity);
    if types.len() != arity {
        return refuse(format!(
            "`{op}` takes {}",
            if arity == 1 {
                "one argument"
            } else {
                "two arguments"
            }
        ));
    }
    let all = |ty: TokenType| types.iter().all(|found| *found == ty);
    match op {
        "add" | "subtract" | "multiply" | "divide" if all(TokenType::Number) => {
            Ok(TokenType::Number)
        }
        "less" | "lessOrEqual" | "greater" | "greaterOrEqual" if all(TokenType::Number) => {
            Ok(TokenType::Boolean)
        }
        "negate" if all(TokenType::Number) => Ok(TokenType::Number),
        "and" | "or" | "not" if all(TokenType::Boolean) => Ok(TokenType::Boolean),
        "equals" | "notEqual" if types[0] == types[1] => Ok(TokenType::Boolean),
        "equals" | "notEqual" => refuse(format!("`{op}` compares values of one type")),
        "not" => refuse("`not` takes a boolean"),
        "negate" => refuse("`negate` takes a number"),
        "and" | "or" => refuse(format!("`{op}` takes booleans")),
        _ => refuse(format!("`{op}` takes numbers")),
    }
}

/// How a JSON value reads in a refusal: "a string", "null".
fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

impl Action {
    /// Checks the variables, collections, modes and expressions that the
    /// action names (and those of the actions in its conditional blocks)
    /// against the design system's tokens.
    pub fn check_variables(&self, tree: &TokenTree) -> Result<(), ExprError> {
        match self {
            Action::SetVariable {
                variable,
                value,
                expr,
            } => {
                let expected = match (value, expr) {
                    (Some(_), Some(_)) => {
                        return refuse("setVariable takes `value` or `expr`, not both");
                    }
                    (None, None) => return refuse("setVariable needs `value` or `expr`"),
                    _ => token_type(tree, variable)?,
                };
                if !matches!(
                    expected,
                    TokenType::Number | TokenType::String | TokenType::Boolean | TokenType::Color
                ) {
                    return refuse(format!(
                        "`{variable}` is a {expected} token; set variable takes number, string, boolean or color tokens"
                    ));
                }
                if let Some(value) = value {
                    let fits = match (expected, value) {
                        (TokenType::Number, Value::Number(_)) => true,
                        (TokenType::String, Value::String(_)) => true,
                        (TokenType::Boolean, Value::Bool(_)) => true,
                        (TokenType::Color, Value::String(text)) => Color::from_str(text).is_ok(),
                        _ => false,
                    };
                    if !fits {
                        let found = match (expected, value) {
                            (TokenType::Color, Value::String(_)) => "not a color",
                            _ => kind_of(value),
                        };
                        return refuse(format!(
                            "`{variable}` is a {expected} token; the value is {found}"
                        ));
                    }
                }
                if let Some(expr) = expr {
                    let yields = Expr::parse(expr)?.check(tree)?;
                    let fits = yields == expected
                        || (expected == TokenType::Color && yields == TokenType::String);
                    if !fits {
                        return refuse(format!(
                            "`{variable}` is a {expected} token; the expression yields a {yields}"
                        ));
                    }
                }
                Ok(())
            }
            Action::SetVariableMode { collection, mode } => {
                let modes = modes_of(tree, collection)?;
                known_mode(collection, mode, &modes)
            }
            Action::Conditional { blocks } => {
                if blocks.is_empty() {
                    return refuse("a conditional needs at least one block");
                }
                for (index, block) in blocks.iter().enumerate() {
                    match &block.condition {
                        None if index + 1 < blocks.len() => {
                            return refuse(
                                "only the last block of a conditional may have no condition",
                            );
                        }
                        None => {}
                        Some(condition) => {
                            if Expr::parse(condition)?.check(tree)? != TokenType::Boolean {
                                return refuse("a condition yields a boolean");
                            }
                        }
                    }
                    for action in &block.actions {
                        action.check_variables(tree)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
