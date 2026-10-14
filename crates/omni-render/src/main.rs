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
