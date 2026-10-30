//! US-006: drawing a page in a viewport draws only the boards that are in view,
//! and draws them in less detail when they are too small to see it.
//!
//! `omni_render::view::Scene` holds a page laid out once, and `render` (or
//! `render_rgba`, on a CPU surface) draws what a `Viewport` shows and reports
//! what it did in `FrameStats`. What the tests expect:
//!
//! - Culling: the boards that intersect the visible area are drawn, and no
//!   other. A board whose drop shadow reaches into the view is drawn as well,
//!   so that culling never changes the picture: a frame is pixel-for-pixel
//!   what `paint::draw` of the whole page gives under the same zoom and
//!   pan.
//! - `FrameStats::nodes_painted` counts the design nodes the frame painted
//!   (a board and everything inside it), not cached pictures that it only
//!   composited. Whichever technique is chosen for the level of detail (cached
//!   tiles, board snapshots, simplified drawing), a frame of the zoomed-out
//!   stress file paints at most `NODES_PER_FRAME_BUDGET` nodes once the first
//!   frame at that zoom is done, a fraction of the 36,000 in the file.
//! - Detail is decided by the device scale, the zoom times the pixel ratio:
//!   it is `Full` at 100% and `Reduced` far out, and zooming further out
//!   never brings `Full` back. A reduced frame still looks like the full
//!   drawing: the average color of each board is close to the true one and
//!   the gaps between boards stay empty.
//!
//! A page too big to read whole opens on its first board, and a link to a board
//! out of that view opens on that board (US-118).
//!
//! The 60 frames per second themselves are measured on a GPU in a browser
//! (US-008), and the smoothness is the human check.

mod common;
mod design;

use design::{at, board, fixed, id, node, none, rect, with};
use omni_core::design::{AttachmentRef, Node};
use omni_render::layout::{self, LayoutBox};
use omni_render::paint::{self, Assets};
use omni_render::select::NodeAddress;
use omni_render::view::{Detail, MIN_ZOOM, Scene, Viewport};
use serde_json::{Value, json};
use skia_safe::{AlphaType, ColorType, EncodedImageFormat, ImageInfo, surfaces};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

const WIDTH: f32 = 1440.0;
const HEIGHT: f32 = 900.0;

/// The most design nodes a frame of the zoomed-out stress file may paint
/// after its first frame at that zoom: about a seventh of the 36,600 nodes,
/// which a frame painting all of them (2 s natively in debug builds) is far
/// from.
const NODES_PER_FRAME_BUDGET: usize = 5_000;

const COLUMNS: u32 = 10;
const ROWS: u32 = 10;
/// Nodes in each board of `grid_page`: the board, a dark half and four dots.
const NODES_PER_BOARD: usize = 6;

// ---------------------------------------------------------------------------
// A page of 100 boards
// ---------------------------------------------------------------------------

fn board_id(n: u32) -> String {
    format!("b{n:07}")
}

fn board_number(id: &str) -> u32 {
    id[1..].parse().expect("fixture board IDs end in a number")
}

fn shade(n: u32) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        20 + (n * 7) % 100,
        30 + (n * 13) % 100,
        60 + (n * 29) % 150
    )
}

fn solid(color: &str) -> Value {
    json!({"type": "solid", "color": color})
}

fn filled(node: &Value, color: &str) -> Value {
    with(node, "fills", json!([solid(color)]))
}

/// A 300 x 200 white board at `x`, `y` with a dark half of its own shade and
/// four red dots.
fn board_at(n: u32, x: f64, y: f64) -> Value {
    let mut children = vec![filled(
        &at(&rect(&format!("c{n:04}000"), 150.0, 200.0), 0.0, 0.0),
        &shade(n),
    )];
    for k in 1..=4u32 {
        children.push(filled(
            &at(
                &rect(&format!("c{n:04}{k:03}"), 20.0, 20.0),
                160.0 + 30.0 * f64::from(k),
                20.0,
            ),
            "#dc2626",
        ));
    }
    let board = board(&board_id(n), fixed(300.0), fixed(200.0), none(), children);
    filled(&at(&board, x, y), "#ffffff")
}

fn page(boards: Vec<Value>) -> Value {
    json!({"id": "p0000001", "type": "page", "name": "Page", "children": boards})
}

/// Boards numbered row by row, 500 px apart across and 400 px down, so that
/// board `n` is at column `n % COLUMNS` and row `n / COLUMNS`.
fn grid_page() -> Value {
    page(
        (0..COLUMNS * ROWS)
            .map(|n| {
                board_at(
                    n,
                    f64::from(n % COLUMNS) * 500.0,
                    f64::from(n / COLUMNS) * 400.0,
                )
            })
            .collect(),
    )
}

fn scene_of(json: &Value) -> Scene {
    Scene::new(node(json), Assets::new())
}

fn ids(boards: &[omni_core::design::NodeId]) -> BTreeSet<String> {
    boards.iter().map(|id| id.as_str().to_string()).collect()
}

/// The boards of `grid_page` in `columns` x `rows` from the top-left.
fn corner(columns: u32, rows: u32) -> BTreeSet<String> {
    (0..rows)
        .flat_map(|row| (0..columns).map(move |column| board_id(row * COLUMNS + column)))
        .collect()
}

// ---------------------------------------------------------------------------
// Pixels
// ---------------------------------------------------------------------------

struct Pixels {
    width: u32,
    rgba: Vec<u8>,
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }

    /// The average of each channel over `[x0, x1) x [y0, y1)`.
    fn mean(&self, [x0, y0, x1, y1]: [u32; 4]) -> [f32; 4] {
        let mut sum = [0.0f32; 4];
        for y in y0..y1 {
            for x in x0..x1 {
                for (total, channel) in sum.iter_mut().zip(self.at(x, y)) {
                    *total += f32::from(channel);
                }
            }
        }
        let count = ((x1 - x0) * (y1 - y0)) as f32;
        sum.map(|total| total / count)
    }

    /// How many pixels differ from `other` by more than `tolerance` in a channel.
    fn differing(&self, other: &Pixels, tolerance: u8) -> usize {
        assert_eq!(self.rgba.len(), other.rgba.len());
        self.rgba
            .chunks_exact(4)
            .zip(other.rgba.chunks_exact(4))
            .filter(|(a, b)| {
                a.iter()
                    .zip(b.iter())
                    .any(|(&a, &b)| a.abs_diff(b) > tolerance)
            })
            .count()
    }
}

/// A frame of `scene` in `viewport`, and what the frame reported.
fn frame(
    scene: &mut Scene,
    viewport: &Viewport,
    pixel_ratio: f32,
) -> (omni_render::view::FrameStats, Pixels) {
    let (stats, rgba) = scene.render_rgba(viewport, pixel_ratio);
    let (width, height) = viewport.size();
    let width = (width * pixel_ratio).round() as u32;
    let height = (height * pixel_ratio).round() as u32;
    assert_eq!(
        rgba.len(),
        width as usize * height as usize * 4,
        "render_rgba must return one RGBA pixel per device pixel of the viewport"
    );
    (stats, Pixels { width, rgba })
}

/// What drawing the whole page with `paint::draw`, under the transform of
/// `viewport`, gives: the picture that culling and level of detail must not
/// change.
fn reference(page: &Node, viewport: &Viewport, pixel_ratio: f32) -> Pixels {
    let assets = Assets::new();
    let boxes = layout::compute(page, assets.fonts());
    let (width, height) = viewport.size();
    let size = (
        (width * pixel_ratio).round() as i32,
        (height * pixel_ratio).round() as i32,
    );
    let mut surface = surfaces::raster_n32_premul(size).expect("Skia creates a surface");
    let canvas = surface.canvas();
    canvas.scale((pixel_ratio, pixel_ratio));
    canvas.scale((viewport.zoom(), viewport.zoom()));
    let (x, y) = viewport.origin();
    canvas.translate((-x, -y));
    paint::draw(canvas, page, &boxes, &assets);

    let info = ImageInfo::new(size, ColorType::RGBA8888, AlphaType::Unpremul, None);
    let mut rgba = vec![0; info.compute_min_byte_size()];
    assert!(surface.read_pixels(&info, &mut rgba, info.min_row_bytes(), (0, 0)));
    Pixels {
        width: size.0 as u32,
        rgba,
    }
}

// ---------------------------------------------------------------------------
// Only boards in view are drawn
// ---------------------------------------------------------------------------

#[test]
fn only_the_boards_that_intersect_the_visible_area_are_drawn() {
    let mut scene = scene_of(&grid_page());
    // Columns start at x = 0, 500, 1000, 1500, ... and are 300 wide, rows
    // start at y = 0, 400, 800, 1200, ... and are 200 tall.
    for (name, viewport, expected) in [
        (
            "from the origin at 100%: columns 0-2 and rows 0-2",
            Viewport::new(WIDTH, HEIGHT),
            corner(3, 3),
        ),
        (
            "scrolled so that boards are cut by every edge: columns 0-3 and rows 0-2",
            Viewport::at(250.0, 150.0, 1.0, WIDTH, HEIGHT),
            (0..3u32)
                .flat_map(|row| (0..4u32).map(move |column| board_id(row * COLUMNS + column)))
                .collect(),
        ),
        (
            "zoomed out to 50%: columns 0-5 and rows 0-4",
            Viewport::at(0.0, 0.0, 0.5, WIDTH, HEIGHT),
            corner(6, 5),
        ),
        (
            "far out, on a small screen: columns 0-3 and rows 0-4",
            Viewport::at(0.0, 0.0, 0.05, 100.0, 100.0),
            corner(4, 5),
        ),
    ] {
        assert_eq!(
            ids(&scene.visible_boards(&viewport)),
            expected,
            "visible_boards, {name}"
        );
        let (stats, _) = frame(&mut scene, &viewport, 1.0);
        assert_eq!(
            ids(&stats.boards_drawn),
            expected,
            "the boards the frame drew, {name}"
        );
    }
}

#[test]
fn a_frame_is_the_same_picture_whether_or_not_boards_are_culled() {
    for (name, viewport, ratio) in [
        (
            "scrolled at 100%",
            Viewport::at(250.0, 150.0, 1.0, WIDTH, HEIGHT),
            1.0,
        ),
        (
            "zoomed in to 300%",
            Viewport::at(280.0, 170.0, 3.0, 480.0, 300.0),
            1.0,
        ),
        (
            "at 100% on a screen with a pixel ratio of 2",
            Viewport::at(-20.0, -20.0, 1.0, 720.0, 450.0),
            2.0,
        ),
    ] {
        let json = grid_page();
        let mut scene = scene_of(&json);
        let (_, drawn) = frame(&mut scene, &viewport, ratio);
        let expected = reference(&node(&json), &viewport, ratio);
        assert_eq!(
            drawn.differing(&expected, 2),
            0,
            "{name}: pixels differ from drawing the whole page"
        );
        assert!(
            drawn.mean([0, 0, drawn.width.min(100), 100])[3] > 0.0,
            "{name}: the frame must show boards"
        );
    }
}

#[test]
fn a_board_outside_the_view_still_casts_its_shadow_into_it() {
    // The board starts at x = 1500, beyond the 1440 px screen, but its shadow
    // is shifted 80 px left and blurred, so it reaches in.
    let shadowed = with(
        &board_at(1, 1500.0, 100.0),
        "shadows",
        json!([{
            "type": "drop",
            "color": "#000000",
            "offsetX": -80.0,
            "offsetY": 0.0,
            "blur": 40.0,
            "spread": 0.0
        }]),
    );
    let json = page(vec![shadowed]);
    let viewport = Viewport::new(WIDTH, HEIGHT);
    let expected = reference(&node(&json), &viewport, 1.0);
    assert!(
        expected.at(1430, 200)[3] > 100,
        "the fixture's shadow must reach into the view"
    );

    let mut scene = scene_of(&json);
    let (_, drawn) = frame(&mut scene, &viewport, 1.0);
    assert_eq!(
        drawn.differing(&expected, 2),
        0,
        "culling the board must not cut off the shadow it casts into the view"
    );
}

#[test]
fn full_detail_paints_every_node_of_the_boards_in_view() {
    let mut scene = scene_of(&grid_page());
    // Columns 0-1 and rows 0-1 are wholly in view, so nothing in them can be
    // skipped.
    let viewport = Viewport::at(-10.0, -10.0, 1.0, 1000.0, 700.0);
    let (stats, _) = frame(&mut scene, &viewport, 1.0);
    assert_eq!(stats.detail, Detail::Full);
    assert_eq!(ids(&stats.boards_drawn), corner(2, 2));
    assert_eq!(
        stats.nodes_painted,
        4 * NODES_PER_BOARD,
        "four boards of {NODES_PER_BOARD} nodes each"
    );
}

// ---------------------------------------------------------------------------
// Level of detail
// ---------------------------------------------------------------------------

fn detail_at(zoom: f32, pixel_ratio: f32) -> Detail {
    let mut scene = scene_of(&grid_page());
    let viewport = Viewport::at(0.0, 0.0, zoom, 720.0, 450.0);
    frame(&mut scene, &viewport, pixel_ratio).0.detail
}

#[test]
fn detail_is_full_at_100_percent_and_reduced_far_out_by_device_scale() {
    for (zoom, ratio) in [(1.0, 1.0), (2.0, 1.0), (4.0, 1.0), (0.5, 2.0), (0.25, 4.0)] {
        assert_eq!(
            detail_at(zoom, ratio),
            Detail::Full,
            "a device scale of {} must draw every node",
            zoom * ratio
        );
    }
    for (zoom, ratio) in [(0.05, 1.0), (0.05, 2.0), (MIN_ZOOM, 1.0)] {
        assert_eq!(
            detail_at(zoom, ratio),
            Detail::Reduced,
            "a device scale of {} must use the cheaper level of detail",
            zoom * ratio
        );
    }
}

#[test]
fn a_reduced_frame_still_looks_like_the_full_drawing() {
    let json = grid_page();
    // 100 boards of 15 x 10 pixels: board n is at (25 * column, 20 * row).
    let viewport = Viewport::at(0.0, 0.0, 0.05, WIDTH, HEIGHT);
    let mut scene = scene_of(&json);
    let (stats, drawn) = frame(&mut scene, &viewport, 1.0);
    assert_eq!(stats.detail, Detail::Reduced);
    assert_eq!(
        stats.boards_drawn.len(),
        (COLUMNS * ROWS) as usize,
        "every board is in view"
    );
    let expected = reference(&node(&json), &viewport, 1.0);

    for id in &stats.boards_drawn {
        let n = board_number(id.as_str());
        let (column, row) = (n % COLUMNS, n / COLUMNS);
        let (x0, y0) = (column * 25 + 2, row * 20 + 2);
        let region = [x0, y0, x0 + 11, y0 + 6];
        let (got, want) = (drawn.mean(region), expected.mean(region));
        assert!(
            got[3] >= 200.0,
            "board {n} must be drawn solid, but its alpha is {}",
            got[3]
        );
        for channel in 0..3 {
            assert!(
                (got[channel] - want[channel]).abs() <= 40.0,
                "board {n}: the average color of channel {channel} must be near the full drawing's {} (give or take 40), but it is {}",
                want[channel],
                got[channel]
            );
        }
    }

    // The gaps between boards stay empty: 10 px between columns and rows.
    for (x, y) in [(19, 5), (20, 5), (7, 14), (7, 15), (44, 35)] {
        assert!(
            drawn.at(x, y)[3] <= 40,
            "pixel ({x}, {y}) is between boards and must stay empty, but it is {:?}",
            drawn.at(x, y)
        );
    }
}

// ---------------------------------------------------------------------------
// Where a page opens
// ---------------------------------------------------------------------------

const COVER: &str = "11qe2xvq";
const INSIDE_COVER: &str = "nb68dacy";
const FAR_BOARD: &str = "6j8ex84q";
const NODE_ON_FAR_BOARD: &str = "n0000001";

fn sized_board(id: &str, x: f64, y: f64, width: f64, height: f64, children: Vec<Value>) -> Value {
    at(
        &board(id, fixed(width), fixed(height), none(), children),
        x,
        y,
    )
}

/// The shape of Ordrar's Omslag page, 3038 x 62521 px: a cover of 1600 x 715, a
/// board on top of it, and a board at the far end.
fn omslag() -> Value {
    page(vec![
        sized_board(COVER, 0.0, 0.0, 1600.0, 715.0, vec![]),
        sized_board(INSIDE_COVER, 0.0, 0.0, 1440.0, 500.0, vec![]),
        sized_board(
            FAR_BOARD,
            0.0,
            61133.0,
            3038.0,
            1388.0,
            vec![at(&rect(NODE_ON_FAR_BOARD, 100.0, 100.0), 20.0, 20.0)],
        ),
    ])
}

/// Two boards 4800 px across in all, so that all of the page is at zoom 0.3:
/// reduced at a pixel ratio of 1, and full at 2.
fn readable_page() -> Value {
    page(vec![
        sized_board(COVER, 0.0, 0.0, 1600.0, 1000.0, vec![]),
        sized_board(
            FAR_BOARD,
            3200.0,
            0.0,
            1600.0,
            1000.0,
            vec![at(&rect(NODE_ON_FAR_BOARD, 100.0, 100.0), 20.0, 20.0)],
        ),
    ])
}

/// The view a page opens in on a 1440 x 900 screen of pixel ratio 2, by a link
/// to the board `linked` if there is one.
fn opened(scene: &Scene, linked: Option<&str>) -> Viewport {
    let mut viewport = Viewport::new(WIDTH, HEIGHT);
    viewport.fit(scene.opening(&viewport, 2.0, linked.map(id)));
    viewport
}

#[track_caller]
fn assert_in_view(viewport: &Viewport, board: LayoutBox) {
    let visible = viewport.visible_rect();
    assert!(
        visible.x <= board.x + 1.0
            && visible.y <= board.y + 1.0
            && visible.x + visible.width >= board.x + board.width - 1.0
            && visible.y + visible.height >= board.y + board.height - 1.0,
        "{board:?} must be wholly in view, but the screen shows {visible:?}"
    );
}

const FAR_BOX: LayoutBox = LayoutBox {
    x: 0.0,
    y: 61133.0,
    width: 3038.0,
    height: 1388.0,
};

#[test]
fn a_page_too_big_to_read_whole_opens_on_its_first_board_in_full_detail() {
    let mut scene = scene_of(&omslag());
    let viewport = opened(&scene, None);
    // 1440 / 1600 is the tighter axis of the cover, where all of the page would
    // be 900 / 62521.
    assert!(
        (viewport.zoom() - 0.9).abs() < 1e-3,
        "the view fits the first board, but its zoom is {}",
        viewport.zoom()
    );
    let first = scene
        .first_board(&viewport)
        .expect("the page has a first board");
    assert_eq!(first.id, id(COVER));
    assert!(first.whole, "all of the first board is in view: {first:?}");
    let (stats, _) = frame(&mut scene, &viewport, 2.0);
    assert_eq!(stats.detail, Detail::Full);
}

#[test]
fn a_page_that_can_be_read_whole_opens_on_all_its_boards() {
    let scene = scene_of(&readable_page());
    assert_eq!(
        scene.opening(&Viewport::new(WIDTH, HEIGHT), 2.0, None),
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 4800.0,
            height: 1000.0
        }
    );
}

#[test]
fn a_link_to_a_board_out_of_the_opening_view_opens_on_that_board() {
    let scene = scene_of(&omslag());
    let viewport = opened(&scene, Some(FAR_BOARD));
    assert_in_view(&viewport, FAR_BOX);
    assert_eq!(
        ids(&scene.visible_boards(&viewport)),
        BTreeSet::from([FAR_BOARD.to_string()])
    );
}

#[test]
fn a_link_to_a_board_inside_the_opening_view_leaves_the_view_on_the_first_board() {
    let scene = scene_of(&omslag());
    let viewport = opened(&scene, Some(INSIDE_COVER));
    assert!(
        (viewport.zoom() - 0.9).abs() < 1e-3,
        "the view stays fitted to the first board, but its zoom is {}",
        viewport.zoom()
    );
    assert_eq!(
        ids(&scene.visible_boards(&viewport)),
        BTreeSet::from([COVER.to_string(), INSIDE_COVER.to_string()])
    );
}

#[test]
fn a_link_to_a_board_the_page_does_not_have_opens_on_the_first_board() {
    let scene = scene_of(&omslag());
    let viewport = opened(&scene, Some("zzzzzzzz"));
    let first = scene
        .first_board(&viewport)
        .expect("the page has a first board");
    assert!(first.whole, "all of the first board is in view: {first:?}");
    assert!(
        (viewport.zoom() - 0.9).abs() < 1e-3,
        "the view fits the first board, but its zoom is {}",
        viewport.zoom()
    );
}

#[test]
fn a_link_to_a_node_out_of_view_moves_the_view_to_its_board() {
    let mut scene = scene_of(&omslag());
    let mut viewport = opened(&scene, None);
    scene.select_address(Some(&NodeAddress::new(None, id(NODE_ON_FAR_BOARD))));
    scene.reveal_selected(&mut viewport);
    assert_in_view(&viewport, FAR_BOX);
}

#[test]
fn a_link_to_a_node_in_view_leaves_the_view_where_it_is() {
    let mut scene = scene_of(&readable_page());
    let mut viewport = Viewport::new(WIDTH, HEIGHT);
    viewport.fit(scene.bounds());
    let fitted = viewport;
    scene.select_address(Some(&NodeAddress::new(None, id(NODE_ON_FAR_BOARD))));
    scene.reveal_selected(&mut viewport);
    assert_eq!(viewport.zoom(), fitted.zoom());
    assert_eq!(viewport.origin(), fitted.origin());
}

/// Whether all of a 300 x 200 first board at the page's origin is in a view of
/// `width` x `height` with its top-left corner at `x`, `y`.
fn first_board_whole(x: f32, y: f32, width: f32, height: f32) -> bool {
    let scene = scene_of(&page(vec![board_at(1, 0.0, 0.0)]));
    let viewport = Viewport::at(x, y, 1.0, width, height);
    scene
        .first_board(&viewport)
        .expect("the page has a first board")
        .whole
}

#[test]
fn the_first_board_is_whole_only_while_all_of_it_is_in_view() {
    assert!(first_board_whole(-10.0, -10.0, 400.0, 300.0));
    assert!(
        !first_board_whole(5.0, -10.0, 400.0, 300.0),
        "cut on the left"
    );
    assert!(
        !first_board_whole(-10.0, 5.0, 400.0, 300.0),
        "cut at the top"
    );
    assert!(
        !first_board_whole(-10.0, -10.0, 300.0, 300.0),
        "cut on the right"
    );
    assert!(
        !first_board_whole(-10.0, -10.0, 400.0, 200.0),
        "cut at the bottom"
    );
}

#[test]
fn the_box_of_the_first_board_is_where_the_screen_shows_it() {
    let scene = scene_of(&page(vec![board_at(1, 40.0, 30.0)]));
    let viewport = Viewport::at(-100.0, -50.0, 0.5, WIDTH, HEIGHT);
    let first = scene
        .first_board(&viewport)
        .expect("the page has a first board");
    assert_eq!(
        first.screen,
        LayoutBox {
            x: 70.0,
            y: 40.0,
            width: 150.0,
            height: 100.0
        }
    );
}

const PHOTO: &str = "attachment:0192f1c4-5b6a-7d8e-9f01-23456789abcd";

/// A board with an image and a rectangle filled with an image, both `PHOTO`.
fn board_of_photos() -> Value {
    let image = json!({
        "id": "m0000001",
        "type": "image",
        "name": "Image",
        "width": fixed(100.0),
        "height": fixed(100.0),
        "image": PHOTO,
        "fit": "cover"
    });
    let fill = json!([{"type": "image", "image": PHOTO, "fit": "cover"}]);
    let filled = with(&rect("m0000002", 100.0, 100.0), "fills", fill);
    sized_board(
        COVER,
        0.0,
        0.0,
        300.0,
        200.0,
        vec![at(&image, 0.0, 0.0), at(&filled, 120.0, 0.0)],
    )
}

/// The images missing from the first board once a frame painted it with `assets`.
fn missing_images(assets: Assets) -> usize {
    let mut scene = Scene::new(node(&page(vec![board_of_photos()])), assets);
    let viewport = Viewport::new(WIDTH, HEIGHT);
    frame(&mut scene, &viewport, 1.0);
    let first = scene.first_board(&viewport).expect("a first board");
    first.painted.missing_images
}

#[test]
fn an_image_is_missing_from_the_first_board_until_the_host_supplies_it() {
    assert_eq!(missing_images(Assets::new()), 2);

    let mut surface = surfaces::raster_n32_premul((2, 2)).expect("a surface");
    let png = surface
        .image_snapshot()
        .encode(
            None::<&mut skia_safe::gpu::DirectContext>,
            EncodedImageFormat::PNG,
            None,
        )
        .expect("the image encodes as PNG");
    let attachment: AttachmentRef = PHOTO.parse().expect("a valid attachment");
    let mut assets = Assets::new();
    assets
        .add_image(&attachment, png.as_bytes())
        .expect("the host can supply a PNG");
    assert_eq!(missing_images(assets), 0);
}

// ---------------------------------------------------------------------------
// The stress file
// ---------------------------------------------------------------------------

/// The stress file `make stress-file` writes (its default seed).
fn stress_file() -> &'static str {
    static FILE: OnceLock<String> = OnceLock::new();
    FILE.get_or_init(|| {
        let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("view");
        fs::create_dir_all(&directory).expect("the scratch directory can be created");
        let path = directory.join("stress-file.json");
        let out = format!("OUT={}", path.display());
        let output = common::make(&["stress-file", &out]);
        assert!(
            output.status.success(),
            "`make stress-file` must write the stress file:\n{}",
            common::describe(&output)
        );
        fs::read_to_string(&path).expect("make stress-file wrote the file")
    })
}

fn stress_page() -> Node {
    serde_json::from_str(stress_file()).expect("the stress file is a design")
}

#[test]
fn a_zoomed_out_frame_of_the_stress_file_paints_a_small_fraction_of_its_nodes() {
    let mut scene = Scene::new(stress_page(), Assets::new());
    let mut viewport = Viewport::new(WIDTH, HEIGHT);
    viewport.fit(scene.bounds());
    assert!(
        viewport.zoom() < 0.1,
        "the whole file (about 9,600 x 15,400 px) is below 10% on a 1440 x 900 screen"
    );

    // The first frame at a zoom may build whatever the technique caches.
    frame(&mut scene, &viewport, 1.0);

    // Panning and the frames after it are the ones that must be cheap.
    for pan in 0..3 {
        viewport.pan_by(25.0, 15.0);
        let (stats, _) = frame(&mut scene, &viewport, 1.0);
        assert_eq!(stats.detail, Detail::Reduced, "pan {pan}");
        assert!(
            !stats.boards_drawn.is_empty(),
            "pan {pan}: the boards in view are still drawn"
        );
        assert!(
            stats.nodes_painted <= NODES_PER_FRAME_BUDGET,
            "pan {pan}: a zoomed-out frame must paint at most {NODES_PER_FRAME_BUDGET} of the file's 36,000 nodes, but it painted {}",
            stats.nodes_painted
        );
    }

    // With every board in view, they are all drawn.
    viewport.fit(scene.bounds());
    let (stats, _) = frame(&mut scene, &viewport, 1.0);
    assert_eq!(
        stats.boards_drawn.len(),
        100,
        "the fitted view shows every board"
    );
    let (stats, _) = frame(&mut scene, &viewport, 1.0);
    assert!(
        stats.nodes_painted <= NODES_PER_FRAME_BUDGET,
        "the second frame of the fitted view painted {} nodes",
        stats.nodes_painted
    );
}
