//! US-121: the picture of a board that `omni export` writes
//! (`omni_render::view::board_png`) shows the board as the viewer draws it, on
//! three boards of the imported tirebud-idag.penpot. Skips when the export is
//! not in fixtures/penpot/.

mod common;

use common::compare::Pixels;
use common::fixture;
use common::import::import;
use omni_render::paint::Assets;
use omni_render::system::DesignSystem;
use omni_render::view::{Scene, Viewport, board_png};

#[test]
fn the_picture_of_a_board_is_the_viewers_frame_of_it_at_zoom_2() {
    let Some(path) = fixture("tirebud-idag.penpot") else {
        return;
    };
    let imported = import(&path);
    let system = DesignSystem::from_json(&imported.design_system).expect("the system is valid");
    for board in &imported.pages[0].children()[..3] {
        let png = board_png(board.clone(), &Assets::new(), Some(&system), 2.0)
            .expect("the board is drawn");
        let picture = Pixels::from_png(&png);

        let mut scene = Scene::with_system(board.clone(), Assets::new(), system.clone());
        let b = scene.bounds();
        let view = Viewport::at(
            b.x,
            b.y,
            2.0,
            (b.width * 2.0).ceil(),
            (b.height * 2.0).ceil(),
        );
        let (_, frame) = scene.render_rgba(&view, 1.0);

        assert_eq!(picture.rgba.len(), frame.len(), "board {}", board.id());
        let differ = picture
            .rgba
            .iter()
            .zip(&frame)
            .filter(|(a, b)| a.abs_diff(**b) > 2)
            .count();
        assert_eq!(
            differ,
            0,
            "channels of board {} that differ by more than 2",
            board.id()
        );
    }
}
