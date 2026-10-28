//! US-021: the document of the operation engine.
//!
//! - docs/formats/operations.md defines the JSON of the five operations. Every
//!   ```json code block in it is an operation or a batch (an array) of
//!   operations that `omni-core` reads; between them the examples use all five
//!   operations. Results and errors are shown in other kinds of code block
//!   (```text).

use omni_core::ops::Operation;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

fn read(relative: &str, what: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{relative} must {what}, but it cannot be read: {error}"))
}

/// The contents of the ```json fenced code blocks of a Markdown file.
fn json_fences(markdown: &str) -> Vec<String> {
    let mut fences = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        let fence_line = line.trim_start().starts_with("```");
        match current.as_mut() {
            None if line.trim() == "```json" => current = Some(String::new()),
            None => {}
            Some(_) if fence_line => fences.extend(current.take()),
            Some(fence) => {
                fence.push_str(line);
                fence.push('\n');
            }
        }
    }
    fences
}

#[test]
fn the_operations_are_documented_with_valid_examples_of_each() {
    let doc = read(
        "docs/formats/operations.md",
        "define the JSON shape of each operation",
    );
    let examples = json_fences(&doc);
    assert!(
        !examples.is_empty(),
        "docs/formats/operations.md must show its examples in ```json code blocks"
    );
    let mut failures = Vec::new();
    let mut kinds = HashSet::new();
    for example in &examples {
        let value: Value = match serde_json::from_str(example) {
            Ok(value) => value,
            Err(error) => {
                failures.push(format!("not JSON ({error}): {example}"));
                continue;
            }
        };
        let ops = match &value {
            Value::Array(ops) => ops.clone(),
            op => vec![op.clone()],
        };
        for op in ops {
            if let Err(error) = serde_json::from_value::<Operation>(op.clone()) {
                failures.push(format!("not an operation ({error}): {op}"));
            }
            if let Some(kind) = op.get("op").and_then(Value::as_str) {
                kinds.insert(kind.to_string());
            }
        }
    }
    assert!(
        failures.is_empty(),
        "every ```json example in docs/formats/operations.md must be an operation or a batch of operations: {failures:?}"
    );
    for kind in ["insert", "replace", "set", "move", "delete"] {
        assert!(
            kinds.contains(kind),
            "docs/formats/operations.md must show a `{kind}` operation in a ```json example"
        );
    }
}
