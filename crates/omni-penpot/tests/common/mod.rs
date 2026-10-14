//! Helpers shared by the omni-penpot integration tests.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use omni_penpot::{Page, Shape};
use serde_json::{Value, json};

/// The id of the root frame that every page of an export has, the parent of the
/// shapes that sit directly on the page.
pub const ROOT_ID: &str = "00000000-0000-0000-0000-000000000000";

/// The repository root, where the Cargo workspace and the Makefile live.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root exists")
}

/// The export `name` in fixtures/penpot/, or None, with a message on standard
/// error, when it is not there. The folder is gitignored until the product
/// owner decides that the designs may live in the repository, so the tests that
/// need it skip rather than fail on a clean checkout.
pub fn fixture(name: &str) -> Option<PathBuf> {
    let path = repo_root().join("fixtures/penpot").join(name);
    if path.is_file() {
        return Some(path);
    }
    eprintln!(
        "SKIPPED: fixtures/penpot/{name} is not there; copy the Penpot exports into fixtures/penpot/ to run this test"
    );
    None
}

/// Whether `actual` is within a billionth of `expected`.
pub fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{what}: expected {expected}, got {actual}"
    );
}

/// `shape` and everything inside it, parents before children.
pub fn flatten(shape: &Shape) -> Vec<&Shape> {
    let mut all = vec![shape];
    for child in &shape.children {
        all.extend(flatten(child));
    }
    all
}

/// Every shape of `page`, parents before children, the root frame not included.
pub fn page_shapes(page: &Page) -> Vec<&Shape> {
    page.shapes.iter().flat_map(flatten).collect()
}

/// The shape with `id` on `page`.
pub fn find<'a>(page: &'a Page, id: &str) -> Option<&'a Shape> {
    page_shapes(page).into_iter().find(|shape| shape.id == id)
}

/// A page of a synthetic export.
pub struct SyntheticPage {
    pub id: String,
    pub name: String,
    pub index: u32,
    /// The ids that the root frame lists, bottom of the stacking order first.
    pub top_level: Vec<String>,
    /// The shape objects, in the order their entries go into the archive.
    pub shapes: Vec<Value>,
}

/// A shape object as the export stores it, with the fields the reader needs.
pub fn shape_json(
    id: &str,
    kind: &str,
    name: &str,
    parent: &str,
    bounds: [f64; 4],
    children: &[&str],
) -> Value {
    let [x, y, width, height] = bounds;
    let mut shape = json!({
        "id": id,
        "name": name,
        "type": kind,
        "x": x,
        "y": y,
        "width": width,
        "height": height,
        "rotation": 0,
        "selrect": {
            "x": x, "y": y, "width": width, "height": height,
            "x1": x, "y1": y, "x2": x + width, "y2": y + height
        },
        "parentId": parent,
        "frameId": parent,
        "strokes": [],
        "fills": []
    });
    if !children.is_empty() {
        shape["shapes"] = json!(children);
    }
    shape
}

/// The bytes of a `.penpot` export holding one file with `pages`. Pages and
/// shapes go into the archive in the order given, which tests use to check that
/// the reader does not depend on it.
pub fn export(file_id: &str, file_name: &str, pages: &[SyntheticPage]) -> Vec<u8> {
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let manifest = json!({
        "type": "penpot/export-files",
        "version": 1,
        "generatedBy": "penpot/2.19.0-RC1",
        "referer": "penpot",
        "files": [{ "id": file_id, "name": file_name, "features": [] }],
        "relations": []
    });
    entries.push((
        "manifest.json".to_string(),
        manifest.to_string().into_bytes(),
    ));
    entries.push((
        format!("files/{file_id}.json"),
        json!({ "id": file_id, "name": file_name, "version": 67 })
            .to_string()
            .into_bytes(),
    ));
    for page in pages {
        let page_dir = format!("files/{file_id}/pages/{}", page.id);
        entries.push((
            format!("{page_dir}.json"),
            json!({ "id": page.id, "name": page.name, "index": page.index })
                .to_string()
                .into_bytes(),
        ));
        let mut root = shape_json(
            ROOT_ID,
            "frame",
            "Root Frame",
            ROOT_ID,
            [0.0, 0.0, 0.01, 0.01],
            &[],
        );
        root["shapes"] = json!(page.top_level);
        entries.push((
            format!("{page_dir}/{ROOT_ID}.json"),
            root.to_string().into_bytes(),
        ));
        for shape in &page.shapes {
            let id = shape["id"].as_str().expect("a synthetic shape has an id");
            entries.push((
                format!("{page_dir}/{id}.json"),
                shape.to_string().into_bytes(),
            ));
        }
    }
    stored_zip(&entries)
}

/// Writes `bytes` to a file of its own in the temporary directory and returns
/// its path. `name` keeps the files of different tests apart.
pub fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("omni-penpot-tests-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("the temporary directory can be created");
    let path = dir.join(name);
    fs::write(&path, bytes).expect("the temporary file can be written");
    path
}

/// A zip archive of `entries` with every file stored uncompressed.
pub fn stored_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in entries {
        let offset = archive.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        // Local file header.
        archive.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        archive.extend_from_slice(&20u16.to_le_bytes());
        archive.extend_from_slice(&[0; 2 + 2 + 2]);
        archive.extend_from_slice(&33u16.to_le_bytes());
        archive.extend_from_slice(&crc.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&size.to_le_bytes());
        archive.extend_from_slice(&(name.len() as u16).to_le_bytes());
        archive.extend_from_slice(&0u16.to_le_bytes());
        archive.extend_from_slice(name.as_bytes());
        archive.extend_from_slice(data);
        // Central directory header.
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&[0; 2 + 2 + 2]);
        directory.extend_from_slice(&33u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&[0; 2 + 2 + 2 + 2 + 4]);
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = archive.len() as u32;
    let directory_size = directory.len() as u32;
    archive.extend_from_slice(&directory);
    // End of central directory record.
    archive.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    archive.extend_from_slice(&[0; 2 + 2]);
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    archive.extend_from_slice(&directory_size.to_le_bytes());
    archive.extend_from_slice(&directory_offset.to_le_bytes());
    archive.extend_from_slice(&0u16.to_le_bytes());
    archive
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
