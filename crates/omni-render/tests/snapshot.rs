//! US-113: the snapshot of a removed design node is drawn by the renderer
//! from its tag form, as a small picture of the node alone.

use omni_core::design::Node;
use omni_render::paint::Assets;
use omni_render::snapshot::page_of_tags;
use omni_render::view::{Scene, Viewport};

const WIDTH: u32 = 160;
const HEIGHT: u32 = 96;

/// The snapshot drawn on a picture of `WIDTH` x `HEIGHT`, fitted to it.
fn picture(tags: &str) -> Vec<u8> {
    let page: Node = serde_json::from_str(&page_of_tags(tags).expect("the tags are a node"))
        .expect("the snapshot is a page");
    let mut scene = Scene::new(page, Assets::new());
    let mut view = Viewport::new(WIDTH as f32, HEIGHT as f32);
    view.fit(scene.bounds());
    scene.render_rgba(&view, 1.0).1
}

fn rgb(rgba: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = (y as usize * WIDTH as usize + x as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn a_node_that_is_not_a_board_is_drawn_from_its_tags() {
    let rgba = picture(
        r##"<Frame id="f0000009" name="Old total" width={40} height={20} fill="#ff0000"/>"##,
    );
    assert_eq!(
        rgb(&rgba, WIDTH / 2, HEIGHT / 2),
        [255, 0, 0],
        "the frame's fill fills the middle of the picture"
    );
}

#[test]
fn a_board_is_drawn_from_its_tags() {
    let rgba = picture(
        r##"<Board id="b0000009" name="Old cart" x={500} y={300} width={60} height={30} fill="#0000ff"/>"##,
    );
    assert_eq!(
        rgb(&rgba, WIDTH / 2, HEIGHT / 2),
        [0, 0, 255],
        "the board's fill fills the middle of the picture"
    );
}
