//! Text: the fonts that text is shaped and measured with.

/// The fonts that text is shaped and measured with.
pub struct Fonts {}

impl Fonts {
    /// The fonts bundled with the renderer: Inter Regular
    /// (`fonts/Inter-Regular.ttf`), which also stands in for every font family
    /// that is missing, so text measures the same on every machine.
    pub fn bundled() -> Self {
        todo!("US-003: load the bundled Inter for Skia's paragraph layout")
    }
}
