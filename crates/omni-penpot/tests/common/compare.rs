//! Helpers for the US-103 comparison tests: PNGs to build and read, the
//! `omni-penpot compare` command, and readers for the report it writes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};
use skia_safe::{
    AlphaType, ColorType, Data, EncodedImageFormat, Image, ImageInfo, gpu::DirectContext, images,
};

use super::convert::{S, run_cli, shape, temp_dir, write_export_with};

pub const WHITE: [u8; 4] = [255, 255, 255, 255];
pub const RED: [u8; 4] = [255, 0, 0, 255];
pub const BLUE: [u8; 4] = [0, 0, 255, 255];

/// The pixels of an image, unpremultiplied RGBA, row by row from the top left.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    pub fn solid(width: u32, height: u32, color: [u8; 4]) -> Pixels {
        Pixels {
            width,
            height,
            rgba: color.repeat(width as usize * height as usize),
        }
    }

    pub fn at(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(
            x < self.width && y < self.height,
            "pixel ({x}, {y}) is outside the {} x {} image",
            self.width,
            self.height
        );
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// Paints the pixels `[x0, x1) x [y0, y1)` with `color`.
    pub fn fill(&mut self, [x0, y0, x1, y1]: [u32; 4], color: [u8; 4]) {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y as usize * self.width as usize + x as usize) * 4;
                self.rgba[i..i + 4].copy_from_slice(&color);
            }
        }
    }

    /// Whether `(x, y)` is in `[x0, x1) x [y0, y1)`.
    pub fn within(x: u32, y: u32, [x0, y0, x1, y1]: [u32; 4]) -> bool {
        x >= x0 && x < x1 && y >= y0 && y < y1
    }

    pub fn to_png(&self) -> Vec<u8> {
        let info = ImageInfo::new(
            (self.width as i32, self.height as i32),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let image =
            images::raster_from_data(&info, Data::new_copy(&self.rgba), self.width as usize * 4)
                .expect("Skia makes an image of the pixels");
        image
            .encode(None::<&mut DirectContext>, EncodedImageFormat::PNG, None)
            .expect("the image encodes as PNG")
            .as_bytes()
            .to_vec()
    }

    pub fn from_png(bytes: &[u8]) -> Pixels {
        let image = Image::from_encoded(Data::new_copy(bytes)).expect("the bytes are a PNG");
        let (width, height) = (image.width(), image.height());
        let info = ImageInfo::new(
            (width, height),
            ColorType::RGBA8888,
            AlphaType::Unpremul,
            None,
        );
        let mut rgba = vec![0; info.compute_min_byte_size()];
        assert!(
            image.read_pixels(
                &info,
                &mut rgba,
                info.min_row_bytes(),
                (0, 0),
                skia_safe::image::CachingHint::Disallow
            ),
            "Skia reads the PNG back as RGBA"
        );
        Pixels {
            width: width as u32,
            height: height as u32,
            rgba,
        }
    }
}

/// The width and height in the header of the PNG file at `path`.
pub fn png_size(path: &Path) -> (u32, u32) {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
    assert!(
        bytes.len() > 24 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        "{path:?} is not a PNG"
    );
    let number =
        |at: usize| u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    (number(16), number(20))
}

/// A solid fill of the color `hex`, written `#rrggbb`.
pub fn fill(hex: &str) -> Value {
    json!({ "fills": [{ "fillColor": hex, "fillOpacity": 1 }] })
}

/// A white board numbered `n` called `name` in `bounds` (page coordinates)
/// holding `kids`.
pub fn white_board(n: u32, name: &str, bounds: [f64; 4], kids: Vec<S>) -> S {
    shape(n, "frame", name, bounds)
        .with(fill("#ffffff"))
        .kids(kids)
}

/// A rectangle numbered `n` filled with `hex`.
pub fn rect(n: u32, name: &str, bounds: [f64; 4], hex: &str) -> S {
    shape(n, "rect", name, bounds).with(fill(hex))
}

/// What the board of [`standard_board`] looks like at 2x, as Penpot would
/// export it: 400 by 200 pixels, white, with the red square in the top left
/// corner, 200 by 100 pixels.
pub fn standard_png() -> Pixels {
    let mut pixels = Pixels::solid(400, 200, WHITE);
    pixels.fill([0, 0, 200, 100], RED);
    pixels
}

/// The board of most tests: "Home", 200 by 100 at (100, 50) on its page, white,
/// with a red 100 by 50 square in its top left corner, plus `more` in it.
pub fn standard_board(n: u32, more: Vec<S>) -> S {
    let mut kids = vec![rect(n + 1, "Square", [100.0, 50.0, 100.0, 50.0], "#ff0000")];
    kids.extend(more);
    white_board(n, "Home", [100.0, 50.0, 200.0, 100.0], kids)
}

/// What a run of `omni-penpot compare` wrote.
pub struct Outcome {
    pub out: PathBuf,
    /// The parsed `compare.json`.
    pub report: Value,
}

impl Outcome {
    /// The result for the board with the Penpot id `board_id`.
    pub fn board(&self, board_id: &str) -> &Value {
        board_result(&self.report, board_id)
    }

    /// The pixels of the PNG that the report names `relative`, a path below the
    /// output directory.
    pub fn png(&self, relative: &Value) -> Pixels {
        let relative = relative
            .as_str()
            .unwrap_or_else(|| panic!("expected a path, got {relative}"));
        let path = self.out.join(relative);
        let bytes = fs::read(&path).unwrap_or_else(|error| panic!("cannot read {path:?}: {error}"));
        Pixels::from_png(&bytes)
    }
}

pub fn board_result<'a>(report: &'a Value, board_id: &str) -> &'a Value {
    let boards = report["boards"]
        .as_array()
        .unwrap_or_else(|| panic!("compare.json has no list of boards: {report}"));
    boards
        .iter()
        .find(|board| board["boardId"] == board_id)
        .unwrap_or_else(|| panic!("compare.json has no board {board_id}"))
}

/// Runs `omni-penpot compare <file> --out <out>`.
pub fn run_compare(file: &Path, out: &Path) -> Output {
    run_cli(&[
        "compare",
        file.to_str().expect("a UTF-8 path"),
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ])
}

/// Runs the comparison, which must work, and reads what it wrote.
pub fn compare_ok(file: &Path, out: &Path) -> Outcome {
    let result = run_compare(file, out);
    assert!(
        result.status.success(),
        "omni-penpot compare failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = fs::read_to_string(out.join("compare.json")).expect("compare.json is written");
    let report = serde_json::from_str(&text).expect("compare.json is JSON");
    Outcome {
        out: out.to_path_buf(),
        report,
    }
}

/// Compares a synthetic export of one page that has `top_level` on it with
/// `pngs`, files `(name, pixels)` in the folder beside the export that has the
/// export's name. With no PNGs the folder does not exist.
pub fn compare_synthetic(test: &str, top_level: Vec<S>, pngs: &[(&str, &Pixels)]) -> Outcome {
    let file = write_export_with(&format!("compare-{test}"), top_level, Vec::new());
    let folder = file.with_extension("");
    let _ = fs::remove_dir_all(&folder);
    if !pngs.is_empty() {
        fs::create_dir_all(&folder).expect("the PNG folder can be created");
        for (name, pixels) in pngs {
            fs::write(folder.join(name), pixels.to_png()).expect("a PNG can be written");
        }
    }
    compare_ok(&file, &temp_dir(&format!("compare-{test}")))
}

/// The number at `key` of `value`, which must be there.
pub fn field(value: &Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} is not a number in {value}"))
}

/// The tolerance that the report says it compared with: the largest difference
/// in a channel of a pixel, from 0 to 255, that still counts as the same.
pub fn tolerance(report: &Value) -> u8 {
    let tolerance = field(report, "tolerance");
    assert!(
        (1.0..=64.0).contains(&tolerance) && tolerance.fract() == 0.0,
        "the tolerance is a whole number of 1 to 64 steps of a channel, not {tolerance}"
    );
    tolerance as u8
}
