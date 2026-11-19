//! US-327: the new build runs as its own compose project, `omnistate-next`,
//! with volumes of its own, so the human's `omnistate` stack keeps its data.
//! `make next-up` rebuilds and starts it, `make next-down` stops it and keeps
//! its volumes, and `docs/delivery-stack.md` gains a short section of its own on
//! it. These read the Makefile (through `make --dry-run`) and the doc, so they
//! need no Docker and run everywhere.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// The commands `make <target>` runs, printed without running them.
fn make_dry_run(target: &str) -> String {
    let output = Command::new("make")
        .args(["--dry-run", "--always-make", target])
        .current_dir(repo_root())
        .env_remove("MAKEFLAGS")
        .env_remove("MFLAGS")
        .env_remove("MAKELEVEL")
        .output()
        .expect("make can be started");
    assert!(
        output.status.success(),
        "`make {target}` is not a working Makefile target:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The stack's compose commands, leaving out the edge's own.
fn stack_compose_lines(commands: &str) -> Vec<&str> {
    commands
        .lines()
        .filter(|line| line.contains("docker compose") && !line.contains("docker/edge"))
        .collect()
}

fn delivery_stack_doc() -> String {
    let path = repo_root().join("docs/delivery-stack.md");
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

#[test]
fn make_next_up_rebuilds_and_starts_only_the_project_omnistate_next() {
    let commands = make_dry_run("next-up");
    let lines = stack_compose_lines(&commands);
    assert!(
        lines
            .iter()
            .any(|line| line.contains(" up ") && line.contains("--build")),
        "`make next-up` must start the stack with --build:\n{commands}"
    );
    for line in &lines {
        assert!(
            line.contains("-p omnistate-next"),
            "every compose command of `make next-up` must name the project omnistate-next:\n{line}"
        );
        assert!(
            line.contains("OMNI_PORT=") && !line.contains("OMNI_PORT=8420"),
            "`make next-up` must publish a host port other than the omnistate stack's 8420:\n{line}"
        );
    }
}

#[test]
fn make_next_down_stops_omnistate_next_and_keeps_its_volumes() {
    let commands = make_dry_run("next-down");
    let lines = stack_compose_lines(&commands);
    assert!(
        lines
            .iter()
            .any(|line| line.contains("-p omnistate-next") && line.contains(" down")),
        "`make next-down` must take the project omnistate-next down:\n{commands}"
    );
    for line in &lines {
        assert!(
            line.contains("-p omnistate-next"),
            "`make next-down` must touch only the project omnistate-next:\n{line}"
        );
        let words: Vec<&str> = line.split_whitespace().collect();
        assert!(
            !words.contains(&"--volumes") && !words.contains(&"-v"),
            "`make next-down` must keep the volumes:\n{line}"
        );
    }
}

#[test]
fn the_next_stack_section_leaves_the_test_stack_subsections_under_their_own_heading() {
    let doc = delivery_stack_doc();
    let mut top_level = None;
    let mut next_stack_seen = false;
    for line in doc.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            top_level = Some(heading.trim());
            next_stack_seen |= heading.trim() == "The next stack";
        } else if let Some(heading) = line.strip_prefix("### ") {
            if heading.to_lowercase().contains("test stack") {
                assert_eq!(
                    top_level,
                    Some("The test stack"),
                    "the subsection `### {heading}` must sit under `## The test stack`"
                );
            }
        }
    }
    assert!(
        next_stack_seen,
        "docs/delivery-stack.md must have a `## The next stack` section"
    );
}

#[test]
fn the_next_stack_section_says_how_to_start_stop_and_sign_in() {
    let doc = delivery_stack_doc();
    let section: String = doc
        .lines()
        .skip_while(|line| line.trim() != "## The next stack")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect::<Vec<_>>()
        .join("\n");
    for needle in [
        "make next-up",
        "make next-down",
        "http://omnistate-next.localhost",
    ] {
        assert!(
            section.contains(needle),
            "the `## The next stack` section must mention `{needle}`:\n{section}"
        );
    }
    assert!(
        section.contains("omni-api setup") || section.contains("make next-seed"),
        "the `## The next stack` section must say how the first member is created \
         (omni-api setup or make next-seed):\n{section}"
    );
}
