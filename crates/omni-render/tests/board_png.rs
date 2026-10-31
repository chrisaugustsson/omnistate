//! US-121: `view::board_png`, the picture of a board that `omni export` writes.

use omni_core::design::Node;
use omni_render::paint::Assets;
use omni_render::snapshot::page_of_tags;
use omni_render::view::board_png;
use skia_safe::{AlphaType, ColorType, Data, Image, ImageInfo, image::CachingHint};

/// The board in `tags`, as the API reads it from a head.
fn board(tags: &str) -> Node {
    let page: Node = serde_json::from_str(&page_of_tags(tags).expect("the tags are a board"))
        .expect("the page is a node");
    page.children()[0].clone()
}

/// The PNG's width, height and unpremultiplied RGBA pixels.
fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let image = Image::from_encoded(Data::new_copy(png)).expect("the bytes are a PNG");
    let info = ImageInfo::new(
        (image.width(), image.height()),
        ColorType::RGBA8888,
        AlphaType::Unpremul,
        None,
    );
    let mut rgba = vec![0; info.compute_min_byte_size()];
    assert!(image.read_pixels(
        &info,
        &mut rgba,
        info.min_row_bytes(),
        (0, 0),
        CachingHint::Disallow
    ));
    (image.width() as u32, image.height() as u32, rgba)
}

fn picture(tags: &str) -> (u32, u32, Vec<u8>) {
    decode(&board_png(board(tags), &Assets::new(), None, 2.0).expect("the board is drawn"))
}

fn pixel(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = (y as usize * width as usize + x as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn each_side_is_twice_the_board_rounded_up_and_never_less_than_one_pixel() {
    let (width, height, _) = picture(
        r##"<Board name="Strip" x={0} y={0} width={100.2} height={10.7} fill="#0000ff"/>"##,
    );
    assert_eq!((width, height), (201, 22));

    let (width, height, _) =
        picture(r##"<Board name="Line" x={0} y={0} width={0.2} height={0} fill="#0000ff"/>"##);
    assert_eq!((width, height), (1, 1));
}

#[test]
fn a_board_without_a_fill_is_transparent_at_its_corners() {
    let (width, height, rgba) = picture(
        r##"<Board name="Bare" x={40} y={30} width={50} height={40}>
  <Frame name="Middle" x={10} y={10} width={30} height={20} fill="#ff0000"/>
</Board>"##,
    );
    assert_eq!((width, height), (100, 80));
    for (x, y) in [(2, 2), (97, 2), (2, 77), (97, 77)] {
        assert_eq!(pixel(&rgba, width, x, y)[3], 0, "the corner at ({x}, {y})");
    }
    assert_eq!(
        pixel(&rgba, width, 50, 40),
        [255, 0, 0, 255],
        "the frame in the middle"
    );
}

#[test]
fn a_board_with_a_drop_shadow_is_cut_at_its_own_box() {
    let (width, height, rgba) = picture(
        r##"<Board name="Card" x={40} y={30} width={50} height={40} fill="#0000ff" shadows={[{type: "drop", color: "#000000", offsetX: 20, offsetY: 20, blur: 0, spread: 10}]}/>"##,
    );
    assert_eq!((width, height), (100, 80));
    for (x, y) in [(2, 2), (97, 2), (2, 77), (97, 77)] {
        assert_eq!(
            pixel(&rgba, width, x, y),
            [0, 0, 255, 255],
            "the corner at ({x}, {y})"
        );
    }
}
