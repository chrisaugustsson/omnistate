//! The omni-render WebAssembly module that `make render-wasm` builds.
//! Emscripten links programs, not libraries, into a JavaScript loader plus a
//! `.wasm` file, so this program exports the functions the browser calls:
//! rustc hands its `#[unsafe(no_mangle)]` functions to Emscripten's
//! `EXPORTED_FUNCTIONS`.
//!
//! The functions of the design pages take text and bytes as a pointer and a
//! length into the module's memory. The page gets room for them from
//! `omni_render_alloc`, copies them in (through the exported `HEAPU8`), and
//! gives the room back with `omni_render_free`. A function that fails returns
//! `false` or a null pointer, and `omni_render_error` then has the reason.

#[cfg(target_os = "emscripten")]
mod exports {
    use std::cell::RefCell;
    use std::ffi::{CString, c_char};

    thread_local! {
        /// The reason the last function that failed failed.
        static ERROR: RefCell<CString> = RefCell::new(CString::default());
        /// The result of the last `omni_render_layout_design`.
        static LAYOUT: RefCell<CString> = RefCell::new(CString::default());
        /// The result of the last `omni_render_scene_stats`.
        static STATS: RefCell<CString> = RefCell::new(CString::default());
    }

    /// Takes the text or bytes at `ptr` and `len` from the page.
    ///
    /// # Safety
    /// `ptr` and `len` must be a block that `omni_render_alloc` returned.
    unsafe fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
        // SAFETY: the caller promises the block is there.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }

    /// # Safety
    /// As [`bytes`].
    unsafe fn text<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
        // SAFETY: the caller promises the block is there.
        std::str::from_utf8(unsafe { bytes(ptr, len) }).map_err(|error| error.to_string())
    }

    /// Returns whether `result` is `Ok`, and keeps the reason if it is not.
    fn succeeded(result: Result<(), String>) -> bool {
        let Err(reason) = result else {
            return true;
        };
        fail(&reason);
        false
    }

    fn fail(reason: &str) {
        let reason = CString::new(reason.replace('\0', " ")).unwrap_or_default();
        ERROR.set(reason);
    }

    /// Draws the sandbox scene on the default framebuffer of the page's current
    /// WebGL 2 context: `width` x `height` device pixels, with `pixel_ratio`
    /// device pixels per canvas pixel. Returns whether Skia drew it.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_draw_sandbox(width: i32, height: i32, pixel_ratio: f32) -> bool {
        omni_render::web::draw_sandbox(width, height, pixel_ratio)
    }

    /// Room for `len` bytes of the page's own to write in.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_alloc(len: usize) -> *mut u8 {
        Box::into_raw(vec![0u8; len].into_boxed_slice()).cast()
    }

    /// Gives back the room of `len` bytes that `omni_render_alloc` gave.
    ///
    /// # Safety
    /// `ptr` and `len` must be what `omni_render_alloc` returned and was given.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn omni_render_free(ptr: *mut u8, len: usize) {
        // SAFETY: the caller promises the block came from `omni_render_alloc`.
        drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
    }

    /// The reason the last function that returned `false` or null failed, as
    /// a NUL-ended string that stays until the next failure.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_error() -> *const c_char {
        ERROR.with_borrow(|reason| reason.as_ptr())
    }

    /// Supplies the image that the attachment (`attachment:<id>`) names, from
    /// PNG or JPEG bytes, to draw nodes that show it. Returns whether it was
    /// an image.
    ///
    /// # Safety
    /// Every pointer and length must be a block from `omni_render_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn omni_render_add_image(
        attachment: *const u8,
        attachment_len: usize,
        data: *const u8,
        data_len: usize,
    ) -> bool {
        // SAFETY: the caller promises the blocks are there.
        let result = unsafe { text(attachment, attachment_len) }.and_then(|attachment| {
            omni_render::web::add_image(attachment, unsafe { bytes(data, data_len) })
        });
        succeeded(result)
    }

    /// Supplies a font file (TrueType or OpenType) for text in `family`.
    /// Returns whether it was a font.
    ///
    /// # Safety
    /// Every pointer and length must be a block from `omni_render_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn omni_render_add_font(
        family: *const u8,
        family_len: usize,
        data: *const u8,
        data_len: usize,
    ) -> bool {
        // SAFETY: the caller promises the blocks are there.
        let result = unsafe { text(family, family_len) }.and_then(|family| {
            omni_render::web::add_font(family, unsafe { bytes(data, data_len) })
        });
        succeeded(result)
    }

    /// Draws the design, JSON in the format of `omni_core::design`, on the
    /// default framebuffer of the page's current WebGL 2 context:
    /// `width` x `height` device pixels, with `pixel_ratio` device pixels per
    /// CSS pixel. Returns whether Skia drew it.
    ///
    /// # Safety
    /// The pointer and length must be a block from `omni_render_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn omni_render_draw_design(
        design: *const u8,
        design_len: usize,
        width: i32,
        height: i32,
        pixel_ratio: f32,
    ) -> bool {
        // SAFETY: the caller promises the block is there.
        let result = unsafe { text(design, design_len) }
            .and_then(|design| omni_render::web::draw_design(design, width, height, pixel_ratio));
        succeeded(result)
    }

    /// Loads the design, a page as JSON in the format of `omni_core::design`,
    /// into the scene of the stress page, with the images and fonts supplied
    /// so far (supply them first). The view is fitted to the boards when the
    /// first frame is drawn. Returns whether the design was valid.
    ///
    /// # Safety
    /// The pointer and length must be a block from `omni_render_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn omni_render_scene_load(design: *const u8, design_len: usize) -> bool {
        // SAFETY: the caller promises the block is there.
        let result = unsafe { text(design, design_len) }.and_then(omni_render::web::scene_load);
        succeeded(result)
    }

    /// Draws the scene in its current view on the default framebuffer of the
    /// page's current WebGL 2 context: `width` x `height` device pixels, with
    /// `pixel_ratio` device pixels per CSS pixel. The view takes the size of
    /// the canvas in CSS pixels first. Returns whether Skia drew it.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_scene_draw(width: i32, height: i32, pixel_ratio: f32) -> bool {
        succeeded(omni_render::web::scene_draw(width, height, pixel_ratio))
    }

    /// What the last frame did, as a NUL-ended JSON object that stays until
    /// the next call: `{"boardsDrawn": 3, "nodesPainted": 640, "detail":
    /// "full", "zoom": 1}`. Null before the first frame.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_scene_stats() -> *const c_char {
        let Some(stats) = omni_render::web::scene_stats() else {
            return std::ptr::null();
        };
        STATS.set(CString::new(stats).unwrap_or_default());
        STATS.with_borrow(|stats| stats.as_ptr())
    }

    /// A wheel event at the canvas point `x`, `y` (CSS pixels): Ctrl or Cmd
    /// zooms around the point, the wheel alone pans. Returns whether it was
    /// used, so that the page stops the browser from acting on it too.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_view_wheel(
        delta_x: f32,
        delta_y: f32,
        x: f32,
        y: f32,
        ctrl: bool,
        meta: bool,
    ) -> bool {
        let event = omni_render::view::Wheel {
            delta_x,
            delta_y,
            x,
            y,
            ctrl,
            meta,
        };
        omni_render::web::with_controls(|controls| controls.wheel(event))
    }

    /// A touch pinch: the change in distance `scale` between the fingers
    /// since the last call, around the point `x`, `y` between them.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_view_pinch(scale: f32, x: f32, y: f32) -> bool {
        omni_render::web::with_controls(|controls| controls.pinch(scale, x, y))
    }

    /// The pointer went down at the canvas point `x`, `y`. Returns whether it
    /// began a pan, which it does while space is held.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_view_pointer_down(x: f32, y: f32) -> bool {
        omni_render::web::with_controls(|controls| controls.pointer_down(x, y))
    }

    /// The pointer moved to the canvas point `x`, `y`. Returns whether it
    /// panned the view.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_view_pointer_move(x: f32, y: f32) -> bool {
        omni_render::web::with_controls(|controls| controls.pointer_move(x, y))
    }

    /// The pointer went up.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_view_pointer_up() {
        omni_render::web::with_controls(|controls| {
            controls.pointer_up();
            true
        });
    }

    /// Whether the space key is held, which lets the pointer pan.
    #[unsafe(no_mangle)]
    pub extern "C" fn omni_render_view_space(held: bool) {
        omni_render::web::with_controls(|controls| {
            controls.set_space(held);
            true
        });
    }

    /// Lays the design out and returns the box of every node as a NUL-ended
    /// JSON object by node ID, which stays until the next call: `{"<node id>":
    /// {"x": 0, "y": 0, "width": 60, "height": 30}}`. Null if the design is not
    /// valid.
    ///
    /// # Safety
    /// The pointer and length must be a block from `omni_render_alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn omni_render_layout_design(
        design: *const u8,
        design_len: usize,
    ) -> *const c_char {
        // SAFETY: the caller promises the block is there.
        let boxes = unsafe { text(design, design_len) }.and_then(omni_render::web::layout_design);
        match boxes {
            Ok(boxes) => {
                LAYOUT.set(CString::new(boxes).unwrap_or_default());
                LAYOUT.with_borrow(|boxes| boxes.as_ptr())
            }
            Err(reason) => {
                fail(&reason);
                std::ptr::null()
            }
        }
    }
}

/// Nothing runs at startup: the page calls the exported functions once it has
/// made its WebGL context current.
fn main() {}
