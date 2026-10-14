//! Prints the box of every node of a design as the native build lays it out:
//! `cargo run -p omni-render --example layout-boxes -- <design.json>`, or
//! `make nodes-boxes` for the design of the nodes test page. The page compares
//! them with the boxes of the WebAssembly build.

use omni_render::sandbox;
use omni_render::text::Fonts;
use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: layout-boxes <design.json>");
        return ExitCode::from(2);
    };
    let design = match std::fs::read_to_string(&path) {
        Ok(design) => design,
        Err(error) => {
            eprintln!("cannot read {path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    match sandbox::layout_json(&design, &Fonts::bundled()) {
        Ok(boxes) => {
            println!("{boxes}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{path} is not a design: {error}");
            ExitCode::FAILURE
        }
    }
}
