//! Compares OmniState's rendering of the converted boards with Penpot's own
//! export of them (docs/penpot-compare.md).
//!
//! Two comparisons are made for each board. The pixel comparison draws the board
//! at 2x and sets it beside Penpot's PNG; the layout comparison sets the box
//! that Penpot stores for each shape beside the box that OmniState's layout
//! computes. A board with no PNG gets the layout comparison only.
//!
//! The layout comparison is made twice (US-054, Q49). The structural mismatches
//! pin each text to the box Penpot stored for it, inside the comparison only, so
//! that Penpot's whole-pixel rounding of text boxes does not count; they show
//! real layout errors. The text drift is measured on the design as imported,
//! its texts following Chrome.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use omni_core::design::{AttachmentRef, Node, NodeId};
use omni_render::layout::{self, LayoutBox};
use omni_render::paint::{self, Assets};
use serde_json::{Value, json};
use skia_safe::{
    AlphaType, ColorType, Data, EncodedImageFormat, Image, ImageInfo, gpu::DirectContext, images,
    surfaces,
};

use crate::{ConvertedBoard, ReadError, Rect, ReportEntry, convert_file};

/// The largest difference in a channel (red, green, blue or alpha, each 0 to
/// 255) between two pixels that still counts as the same.
pub const TOLERANCE: u8 = 8;

/// Penpot's PNGs are 2x.
const SCALE: f32 = 2.0;

/// A shape is listed when it is more than this many thousandths of a pixel from
/// where Penpot stored it: 2 px.
const LAYOUT_LIMIT_MILLIS: i64 = 2_000;

/// What every differing pixel of the diff image is painted: magenta, a color
/// that no other pixel of the image can have, as they are all pale.
pub const HIGHLIGHT: [u8; 4] = [255, 0, 255, 255];

/// What a pixel of a masked shape's box is painted in the diff image.
const MASKED: [u8; 4] = [200, 214, 235, 255];

/// The comparison of a whole export.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    /// One per converted board, in the order of the conversion.
    pub boards: Vec<BoardComparison>,
}

/// The comparison of one board.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardComparison {
    pub board: String,
    pub board_id: String,
    pub page: String,
    /// The pixel comparison, or None for a board with no Penpot PNG.
    pub pixel: Option<PixelComparison>,
    /// Says why there is no pixel comparison, for a board that has none.
    pub note: Option<String>,
    /// The structural mismatches: measured with each text pinned to Penpot's
    /// box.
    pub layout: LayoutComparison,
    /// The text drift: measured on the design as imported, its texts following
    /// Chrome.
    pub text_drift: LayoutComparison,
    /// The shapes left out of the conversion whole, which the pixel comparison
    /// masks.
    pub unconverted: Vec<Unconverted>,
}

/// The pixel comparison of one board.
#[derive(Debug, Clone, PartialEq)]
pub struct PixelComparison {
    /// The size of Penpot's PNG, which the render has too.
    pub width: u32,
    pub height: u32,
    /// The pixels that were compared, and the pixels of masked boxes that were
    /// left out: together every pixel of the image.
    pub compared: u64,
    pub masked: u64,
    /// The compared pixels that differ by more than [`TOLERANCE`].
    pub differing: u64,
    /// Penpot's PNG, as it was read.
    pub penpot: Vec<u8>,
    /// OmniState's render, as the bytes of a PNG.
    pub render: Vec<u8>,
    /// The differing pixels in [`HIGHLIGHT`] over a pale copy of Penpot's PNG,
    /// as the bytes of a PNG.
    pub diff: Vec<u8>,
}

impl PixelComparison {
    /// The share of the compared pixels that differ, 0 when none were compared.
    pub fn share(&self) -> f64 {
        if self.compared == 0 {
            0.0
        } else {
            self.differing as f64 / self.compared as f64
        }
    }
}

/// The layout comparison of one board.
#[derive(Debug, Clone, PartialEq)]
pub struct LayoutComparison {
    /// How many shapes were compared: the converted ones that are not hidden, the
    /// board included.
    pub compared: usize,
    /// The shapes more than 2 px from where Penpot stored them, the furthest
    /// first.
    pub mismatches: Vec<Mismatch>,
}

/// A shape that OmniState's layout put somewhere else than Penpot did.
#[derive(Debug, Clone, PartialEq)]
pub struct Mismatch {
    pub shape_id: String,
    pub name: String,
    pub penpot: Rect,
    pub omnistate: Rect,
    /// The largest of the differences of x, y, width and height.
    pub off_by: f64,
}

/// A shape of the board that the converter left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Unconverted {
    pub shape_id: String,
    pub name: String,
    pub reason: String,
}

/// Converts the export at `path` and compares each board with Penpot's PNG of
/// it, found in the folder beside the export that has its name without
/// `.penpot`.
pub fn compare_file(path: &Path) -> Result<Comparison, ReadError> {
    let conversion = convert_file(path)?;
    let folder = path.with_extension("");
    let folder = folder.is_dir().then_some(folder);

    let mut assets = Assets::new();
    load_fonts(&mut assets, path);
    for attachment in &conversion.attachments {
        // An image that cannot be loaded is not drawn, which shows in the diff.
        if let Ok(reference) = format!("attachment:{}", attachment.id).parse::<AttachmentRef>() {
            let _ = assets.add_image(&reference, &attachment.bytes);
        }
    }

    let mut left_out: HashMap<&str, Vec<&ReportEntry>> = HashMap::new();
    for entry in &conversion.report.entries {
        if entry.property.is_none() && !entry.board_id.is_empty() {
            left_out.entry(&entry.board_id).or_default().push(entry);
        }
    }

    let files = match &folder {
        Some(_) => png_names(&conversion.boards),
        None => vec![String::new(); conversion.boards.len()],
    };
    let mut boards = Vec::with_capacity(conversion.boards.len());
    for (board, file) in conversion.boards.iter().zip(&files) {
        let boxes = layout::compute(&board.design, assets.fonts());
        let pinned = pin_texts(board);
        let layout = match &pinned {
            Some(design) => compare_layout(board, &layout::compute(design, assets.fonts())),
            None => compare_layout(board, &boxes),
        };
        let text_drift = compare_layout(board, &boxes);
        let unconverted: Vec<&ReportEntry> =
            left_out.get(board.id.as_str()).cloned().unwrap_or_default();
        let (pixel, note) = match &folder {
            None => (None, Some(layout_only("there is no folder of Penpot PNGs"))),
            Some(folder) => match fs::read(folder.join(file)) {
                Err(_) => (
                    None,
                    Some(layout_only(&format!("Penpot has no PNG {file} for it"))),
                ),
                Ok(bytes) => match decode(&bytes) {
                    None => (
                        None,
                        Some(layout_only(&format!("the PNG {file} cannot be read"))),
                    ),
                    Some(penpot) => (
                        Some(compare_pixels(
                            board,
                            &boxes,
                            &assets,
                            &penpot,
                            bytes,
                            &unconverted,
                        )),
                        None,
                    ),
                },
            },
        };
        boards.push(BoardComparison {
            board: board.name.clone(),
            board_id: board.id.clone(),
            page: board.page.clone(),
            pixel,
            note,
            layout,
            text_drift,
            unconverted: unconverted
                .iter()
                .map(|entry| Unconverted {
                    shape_id: entry.shape_id.clone(),
                    name: entry.name.clone(),
                    reason: entry.reason.clone(),
                })
                .collect(),
        });
    }
    Ok(Comparison { boards })
}

/// Supplies the fonts in the `fonts` folder beside the export: each subfolder
/// is a font family, named as the designs name it, holding its TrueType or
/// OpenType files. A file that is not a font is skipped; text in a family
/// without files falls back to the bundled default.
fn load_fonts(assets: &mut Assets, export: &Path) {
    let Some(fonts) = export.parent().map(|dir| dir.join("fonts")) else {
        return;
    };
    let Ok(families) = fs::read_dir(&fonts) else {
        return;
    };
    let mut families: Vec<_> = families.flatten().map(|entry| entry.path()).collect();
    families.sort();
    for family_dir in families.into_iter().filter(|dir| dir.is_dir()) {
        let Some(family) = family_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(files) = fs::read_dir(&family_dir) else {
            continue;
        };
        let mut files: Vec<_> = files.flatten().map(|entry| entry.path()).collect();
        files.sort();
        for file in files {
            let is_font = file
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("ttf") || ext.eq_ignore_ascii_case("otf")
                });
            if let (true, Ok(bytes)) = (is_font, fs::read(&file)) {
                let _ = assets.add_font(family, &bytes);
            }
        }
    }
}

fn layout_only(why: &str) -> String {
    format!("Layout comparison only: {why}, so the pixels are not compared.")
}

/// The file name of Penpot's PNG of each board: its name with each ` / ` (or
/// `/`) written ` _ `, and `-2`, `-3` and so on for boards of the same name, the
/// last one in the stacking order keeping the bare name.
fn png_names(boards: &[ConvertedBoard]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut names: Vec<String> = boards
        .iter()
        .rev()
        .map(|board| {
            let base = board.name.replace(" / ", " _ ").replace('/', " _ ");
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            match *count {
                1 => format!("{base}.png"),
                n => format!("{base}-{n}.png"),
            }
        })
        .collect();
    names.reverse();
    names
}

/// A box in thousandths of a pixel, so that the distance of two boxes is exact.
type Millis = [i64; 4];

fn millis(x: f64, y: f64, width: f64, height: f64) -> Millis {
    [x, y, width, height].map(|value| (value * 1000.0).round() as i64)
}

fn rect(millis: Millis) -> Rect {
    let [x, y, width, height] = millis.map(|value| value as f64 / 1000.0);
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// A copy of the design of `board` with every text that is drawn fixed to the
/// size of the box Penpot stored for it, or None when no text changes. The pin
/// is for the comparison only: the converted design keeps its texts hugging.
fn pin_texts(board: &ConvertedBoard) -> Option<Node> {
    let stored: HashMap<&str, &Rect> = board
        .shapes
        .iter()
        .map(|shape| (shape.node_id.as_str(), &shape.bounds))
        .collect();
    let mut json = serde_json::to_value(&board.design).ok()?;
    let mut pinned = false;
    pin_in(&mut json, &stored, &mut pinned);
    if !pinned {
        return None;
    }
    serde_json::from_value(json).ok()
}

fn pin_in(json: &mut Value, stored: &HashMap<&str, &Rect>, pinned: &mut bool) {
    let Some(node) = json.as_object_mut() else {
        return;
    };
    if node.get("visible") == Some(&Value::Bool(false)) {
        return;
    }
    if node.get("type").and_then(Value::as_str) == Some("text")
        && let Some(bounds) = node
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| stored.get(id))
    {
        for (axis, min, max, length) in [
            ("width", "minWidth", "maxWidth", bounds.width),
            ("height", "minHeight", "maxHeight", bounds.height),
        ] {
            node.insert(axis.into(), json!({ "fixed": length.max(0.0) }));
            node.remove(min);
            node.remove(max);
        }
        *pinned = true;
    }
    if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
        for child in children {
            pin_in(child, stored, pinned);
        }
    }
}

/// Sets the box Penpot stores for each converted shape of `board` beside the
/// box that layout computed.
fn compare_layout(board: &ConvertedBoard, boxes: &HashMap<NodeId, LayoutBox>) -> LayoutComparison {
    let mut nodes = HashMap::new();
    collect_ids(&board.design, &mut nodes);
    let mut compared = 0;
    let mut mismatches = Vec::new();
    for shape in &board.shapes {
        let Some(computed) = nodes
            .get(shape.node_id.as_str())
            .and_then(|id| boxes.get(id))
        else {
            continue;
        };
        compared += 1;
        let stored = &shape.bounds;
        let penpot = millis(stored.x, stored.y, stored.width, stored.height);
        let omnistate = millis(
            f64::from(computed.x),
            f64::from(computed.y),
            f64::from(computed.width),
            f64::from(computed.height),
        );
        let off_by = (0..4)
            .map(|side| (penpot[side] - omnistate[side]).abs())
            .max()
            .unwrap_or(0);
        if off_by > LAYOUT_LIMIT_MILLIS {
            mismatches.push(Mismatch {
                shape_id: shape.id.clone(),
                name: shape.name.clone(),
                penpot: rect(penpot),
                omnistate: rect(omnistate),
                off_by: off_by as f64 / 1000.0,
            });
        }
    }
    // The furthest first; the sort is stable, so ties stay in the order of the
    // conversion.
    mismatches.sort_by(|a, b| b.off_by.total_cmp(&a.off_by));
    LayoutComparison {
        compared,
        mismatches,
    }
}

/// Maps the written id of every node of `node` that is drawn to its [`NodeId`].
/// A hidden node and everything in it is left out: layout gives it no size, and
/// Penpot's stored box for it is not a place anything is drawn.
fn collect_ids<'a>(node: &'a Node, ids: &mut HashMap<&'a str, NodeId>) {
    if !node.visible() {
        return;
    }
    ids.insert(node.id().as_str(), *node.id());
    for child in node.children() {
        collect_ids(child, ids);
    }
}

/// The pixels of an image, unpremultiplied RGBA, row by row from the top left.
struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

fn rgba_info(width: i32, height: i32) -> ImageInfo {
    ImageInfo::new(
        (width, height),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    )
}

fn decode(bytes: &[u8]) -> Option<Pixels> {
    let image = Image::from_encoded(Data::new_copy(bytes))?;
    let (width, height) = (image.width(), image.height());
    let info = rgba_info(width, height);
    let mut rgba = vec![0; info.compute_min_byte_size()];
    let read = image.read_pixels(
        &info,
        &mut rgba,
        info.min_row_bytes(),
        (0, 0),
        skia_safe::image::CachingHint::Disallow,
    );
    read.then_some(Pixels {
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        rgba,
    })
}

fn encode(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let info = rgba_info(width as i32, height as i32);
    let image = images::raster_from_data(&info, Data::new_copy(rgba), width as usize * 4)
        .expect("Skia makes an image of the pixels");
    image
        .encode(None::<&mut DirectContext>, EncodedImageFormat::PNG, None)
        .expect("the image encodes as PNG")
        .as_bytes()
        .to_vec()
}

/// Draws `board` at 2x on a canvas of the size of `penpot`, its top left corner
/// at the image's, and sets the result beside `penpot`.
fn compare_pixels(
    board: &ConvertedBoard,
    boxes: &HashMap<NodeId, LayoutBox>,
    assets: &Assets,
    penpot: &Pixels,
    penpot_png: Vec<u8>,
    unconverted: &[&ReportEntry],
) -> PixelComparison {
    let (width, height) = (penpot.width, penpot.height);
    let origin = board
        .bounds
        .map_or((0.0, 0.0), |bounds| (bounds.x, bounds.y));
    let size = (width as i32, height as i32);
    let mut surface =
        surfaces::raster_n32_premul(size).expect("Skia creates a raster surface of the size");
    let canvas = surface.canvas();
    canvas.scale((SCALE, SCALE));
    canvas.translate((-origin.0 as f32, -origin.1 as f32));
    paint::draw(canvas, &board.design, boxes, assets);
    let info = rgba_info(size.0, size.1);
    let mut render = vec![0; info.compute_min_byte_size()];
    let read = surface.read_pixels(&info, &mut render, info.min_row_bytes(), (0, 0));
    assert!(read, "Skia reads the drawn pixels back as RGBA");

    let masked = mask(board, origin, unconverted, width, height);
    let mut diff = vec![0; render.len()];
    let (mut compared, mut masked_count, mut differing) = (0, 0, 0);
    for (i, (ours, theirs)) in render.chunks(4).zip(penpot.rgba.chunks(4)).enumerate() {
        let out = &mut diff[i * 4..i * 4 + 4];
        if masked[i] {
            masked_count += 1;
            out.copy_from_slice(&MASKED);
        } else if differs(ours, theirs) {
            compared += 1;
            differing += 1;
            out.copy_from_slice(&HIGHLIGHT);
        } else {
            compared += 1;
            out.copy_from_slice(&pale(theirs));
        }
    }
    PixelComparison {
        width,
        height,
        compared,
        masked: masked_count,
        differing,
        penpot: penpot_png,
        render: encode(width, height, &render),
        diff: encode(width, height, &diff),
    }
}

/// Whether two pixels are more than [`TOLERANCE`] apart in a channel.
fn differs(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).any(|(a, b)| a.abs_diff(*b) > TOLERANCE)
}

/// Penpot's pixel over white, faded to a quarter, so that the highlight stands
/// out: every channel is 192 or more, and the alpha is 255.
fn pale(pixel: &[u8]) -> [u8; 4] {
    let alpha = u32::from(pixel[3]);
    let channel = |value: u8| {
        let over_white = (u32::from(value) * alpha + 255 * (255 - alpha)) / 255;
        (255 - (255 - over_white) / 4) as u8
    };
    [channel(pixel[0]), channel(pixel[1]), channel(pixel[2]), 255]
}

/// The pixels of the boxes of the unconverted shapes, at 2x from the board's
/// corner and clipped to the image.
fn mask(
    board: &ConvertedBoard,
    origin: (f64, f64),
    unconverted: &[&ReportEntry],
    width: u32,
    height: u32,
) -> Vec<bool> {
    let mut masked = vec![false; width as usize * height as usize];
    let scale = f64::from(SCALE);
    // Penpot stores 1142.0000377 for 1142, which must not take a pixel more.
    let span = |from: f64, length: f64, limit: u32| {
        let start = ((from * scale + 0.001).floor().max(0.0) as u32).min(limit);
        let end = (((from + length) * scale - 0.001).ceil().max(0.0) as u32).min(limit);
        start..end
    };
    for entry in unconverted {
        let Some(bounds) = board.left_out.get(&entry.shape_id) else {
            continue;
        };
        let columns = span(bounds.x - origin.0, bounds.width, width);
        for y in span(bounds.y - origin.1, bounds.height, height) {
            for x in columns.clone() {
                masked[y as usize * width as usize + x as usize] = true;
            }
        }
    }
    masked
}
