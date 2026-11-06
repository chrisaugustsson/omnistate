//! Play-time variables: the values a prototype's `setVariable` gives tokens
//! and the modes its `setVariableMode` gives boards, for one play session,
//! and the evaluation of the expressions that read them (D30, D33, D34).
//! Nothing here is written to the design system.

use omni_core::design::proto::Expr;
use omni_core::design::tokens::{ModeChoice, TokenTree};
use omni_core::design::{Node, NodeId, TokenRef};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;

pub use omni_core::design::tokens::TokenValue;

/// What a play session has changed: the value of each token that
/// `setVariable` set, by dotted path, and the modes of each board that
/// `setVariableMode` set, which start from the board's own `modes`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayVariables {
    pub values: BTreeMap<String, TokenValue>,
    pub modes: BTreeMap<NodeId, ModeChoice>,
}

impl PlayVariables {
    /// The modes `board` is resolved in: the first mode of each collection
    /// that has modes, then the board's own `modes`, then those that play
    /// set on it.
    pub fn modes_of(&self, tree: &TokenTree, board: &Node) -> ModeChoice {
        let mut choice: BTreeMap<String, String> = tree
            .collections()
            .into_iter()
            .filter_map(|collection| Some((collection.name, collection.modes.first()?.clone())))
            .collect();
        if let Some(own) = board.modes() {
            choice.extend(own.iter().map(|(name, mode)| (name.clone(), mode.clone())));
        }
        if let Some(set) = self.modes.get(board.id()) {
            choice.extend(
                set.0
                    .iter()
                    .map(|(name, mode)| (name.clone(), mode.clone())),
            );
        }
        ModeChoice(choice)
    }
}

/// An expression that cannot be evaluated, in words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalError {
    message: String,
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for EvalError {}

fn fail<T>(message: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError {
        message: message.into(),
    })
}

/// Evaluating an expression of the design (`omni_core::design::proto::Expr`).
pub trait Eval {
    /// The value of the expression for `board`, the board that is playing:
    /// a variable is the value play set, or else the design system's value in
    /// the board's modes; `{mode}` is the board's mode for a collection and
    /// `{inMode}` the design system's value in another mode, whatever play
    /// set. A division by zero is 0.
    fn eval(
        &self,
        vars: &PlayVariables,
        tree: &TokenTree,
        board: &Node,
    ) -> Result<TokenValue, EvalError>;
}

impl Eval for Expr {
    fn eval(
        &self,
        vars: &PlayVariables,
        tree: &TokenTree,
        board: &Node,
    ) -> Result<TokenValue, EvalError> {
        match self {
            Expr::Value(value) => match TokenValue::from_json(value) {
                Some(value) => Ok(value),
                None => fail("a value is a number, string or boolean"),
            },
            Expr::Variable(path) => match vars.values.get(path) {
                Some(value) => Ok(value.clone()),
                None => designed(tree, path, &vars.modes_of(tree, board)),
            },
            Expr::Mode(collection) => match vars.modes_of(tree, board).get(collection) {
                Some(mode) => Ok(TokenValue::String(mode.to_string())),
                None => fail(format!("`{collection}` has no modes")),
            },
            Expr::InMode { variable, mode } => {
                let collection = variable.split('.').next().unwrap_or_default();
                let mut choice = vars.modes_of(tree, board);
                choice.0.insert(collection.to_string(), mode.clone());
                designed(tree, variable, &choice)
            }
            Expr::Op { op, args } => {
                let values = args
                    .iter()
                    .map(|arg| arg.eval(vars, tree, board))
                    .collect::<Result<Vec<_>, _>>()?;
                operate(op, &values)
            }
        }
    }
}

/// The design system's value of the token at `path` in `choice`.
fn designed(tree: &TokenTree, path: &str, choice: &ModeChoice) -> Result<TokenValue, EvalError> {
    let value = format!("${path}")
        .parse::<TokenRef>()
        .ok()
        .and_then(|reference| tree.resolve(&reference, choice))
        .as_ref()
        .and_then(TokenValue::from_json);
    match value {
        Some(value) => Ok(value),
        None => fail(format!("`{path}` has no value to read")),
    }
}

fn operate(op: &str, values: &[TokenValue]) -> Result<TokenValue, EvalError> {
    use TokenValue::{Boolean, Number};
    let compare = |order: fn(f64, f64) -> bool| match values {
        [Number(left), Number(right)] => Ok(Boolean(order(*left, *right))),
        _ => fail(format!("`{op}` takes two numbers")),
    };
    let arithmetic = |apply: fn(f64, f64) -> f64| match values {
        [Number(left), Number(right)] => Ok(Number(apply(*left, *right))),
        _ => fail(format!("`{op}` takes two numbers")),
    };
    match (op, values) {
        ("add", _) => arithmetic(|a, b| a + b),
        ("subtract", _) => arithmetic(|a, b| a - b),
        ("multiply", _) => arithmetic(|a, b| a * b),
        ("divide", _) => arithmetic(|a, b| if b == 0.0 { 0.0 } else { a / b }),
        ("less", _) => compare(|a, b| a < b),
        ("lessOrEqual", _) => compare(|a, b| a <= b),
        ("greater", _) => compare(|a, b| a > b),
        ("greaterOrEqual", _) => compare(|a, b| a >= b),
        ("equals" | "notEqual", [left, right]) => {
            if std::mem::discriminant(left) != std::mem::discriminant(right) {
                return fail(format!("`{op}` compares values of one type"));
            }
            Ok(Boolean((left == right) == (op == "equals")))
        }
        ("and", [Boolean(left), Boolean(right)]) => Ok(Boolean(*left && *right)),
        ("or", [Boolean(left), Boolean(right)]) => Ok(Boolean(*left || *right)),
        ("not", [Boolean(flag)]) => Ok(Boolean(!flag)),
        ("negate", [Number(number)]) => Ok(Number(-number)),
        _ => fail(format!("`{op}` cannot take these values")),
    }
}

/// Reads an expression from its JSON tree and evaluates it.
pub(crate) fn evaluate(
    json: &Value,
    vars: &PlayVariables,
    tree: &TokenTree,
    board: &Node,
) -> Result<TokenValue, EvalError> {
    match Expr::parse(json) {
        Ok(expr) => expr.eval(vars, tree, board),
        Err(error) => fail(error.to_string()),
    }
}
