//! Drawing designs: the nodes of a laid-out design, painted with Skia from the
//! boxes that `layout::compute` gives them, with the images and fonts that the
//! host supplies.

use crate::layout::LayoutBox;
use crate::text::Fonts;
use omni_core::design::{AttachmentRef, Node, NodeId};
use skia_safe::Canvas;
use std::collections::HashMap;

/// What the host supplies for drawing: the images that `attachment:<id>`
/// references name, and font files. The bundled Inter is always there, and
/// stands in for every font family that the host did not supply.
pub struct Assets {
    fonts: Fonts,
}

impl Assets {
    /// Assets with the bundled Inter only, and no images.
    pub fn new() -> Self {
        Self {
            fonts: Fonts::bundled(),
        }
    }

    /// Supplies the image that `attachment` names, from encoded PNG or JPEG
    /// bytes. Fails if the bytes are not an image.
    pub fn add_image(&mut self, attachment: &AttachmentRef, data: &[u8]) -> Result<(), String> {
        let _ = (attachment, data);
        todo!("US-005: decode and keep the image")
    }

    /// Supplies a font file (TrueType or OpenType) for text in `family`. Fails
    /// if the bytes are not a font.
    pub fn add_font(&mut self, family: &str, data: &[u8]) -> Result<(), String> {
        let _ = (family, data);
        todo!("US-005: register the font for layout and drawing")
    }

    /// The fonts that layout measures text with and drawing draws it with.
    pub fn fonts(&self) -> &Fonts {
        &self.fonts
    }
}

impl Default for Assets {
    fn default() -> Self {
        Self::new()
    }
}

/// Draws `root` and every node below it on `canvas`, in page coordinates (one
/// canvas pixel to a CSS pixel), each node in the box that `boxes` gives it.
pub fn draw(canvas: &Canvas, root: &Node, boxes: &HashMap<NodeId, LayoutBox>, assets: &Assets) {
    let _ = (canvas, root, boxes, assets);
    todo!("US-005: draw frames, text, images and vectors")
}

/// Lays `root` out with the fonts of `assets`, draws it with Skia on a
/// transparent CPU raster surface of `width` x `height` pixels, and returns
/// the pixels as unpremultiplied RGBA, row by row from the top-left corner.
pub fn render_rgba(root: &Node, assets: &Assets, width: u32, height: u32) -> Vec<u8> {
    let _ = (root, assets, width, height);
    todo!("US-005: lay out and draw on a raster surface")
}
