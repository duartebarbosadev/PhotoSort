//! Checked native image preparation, for the application's single image owner.
//!
//! No cache, filesystem mutations, worker pool or workflow state lives here.
//! Coordinates are in the unrotated source. The caller owns EXIF/sidecar rotation.
use std::{
    ffi::{CString, c_char, c_int, c_void},
    path::Path,
    sync::{
        Mutex, OnceLock, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
};

/// Include both versions in any persistent rendered-pixel cache recipe.
pub const VIPS_VERSION: &str = env!("PHOTOSORT_VIPS_VERSION");
pub const BUILD_ID: &str = env!("PHOTOSORT_VIPS_BUILD_ID");
pub const LCMS_VERSION: &str = env!("PHOTOSORT_LCMS_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
#[derive(Debug, Clone, Copy)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Copy, Default)]
pub struct Request {
    /// None retains source resolution/precision. Previews accept 1..=4096.
    pub edge: Option<u32>,
    pub region: Option<Region>,
}
#[derive(Debug)]
pub enum Pixels {
    U8(Vec<u8>),
    U16(Vec<u16>),
}
#[derive(Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub profiled: bool,
    pub source_sample_bytes: u32,
    /// First warning during loader/graph setup, including a profile discarded by
    /// a codec before libvips can validate it. The caller must surface this.
    pub decoder_warning: Option<String>,
    /// Straight-alpha RGBA, normalized to sRGB, in native-endian samples.
    pub pixels: Pixels,
}
#[repr(C)]
#[derive(Default)]
struct NativeOutput {
    pixels: *mut c_void,
    length: usize,
    width: u32,
    height: u32,
    source_width: u32,
    source_height: u32,
    sample_bytes: u32,
    profiled: u32,
    source_sample_bytes: u32,
}
impl Drop for NativeOutput {
    fn drop(&mut self) {
        // SAFETY: the C adapter transfers only g_malloc-owned memory, or null.
        unsafe { photosort_vips_free(self.pixels) };
    }
}
unsafe extern "C" {
    fn photosort_vips_init(error: *mut c_char, capacity: usize) -> c_int;
    fn photosort_vips_free(pixels: *mut c_void);
    fn photosort_vips_render(
        path: *const c_char,
        edge: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        orientation: u32,
        tile_edge: u32,
        cancelled: unsafe extern "C" fn(*mut c_void) -> c_int,
        context: *mut c_void,
        output: *mut NativeOutput,
        error: *mut c_char,
        capacity: usize,
        warning: *mut c_char,
        warning_capacity: usize,
    ) -> c_int;
}
fn error_text(error: &[u8]) -> Error {
    let end = error.iter().position(|c| *c == 0).unwrap_or(error.len());
    Error(String::from_utf8_lossy(&error[..end]).trim().to_owned())
}
fn initialize() -> Result<(), Error> {
    static INIT: OnceLock<Result<(), Error>> = OnceLock::new();
    INIT.get_or_init(|| {
        let mut error = [0u8; 2048];
        // SAFETY: the writable error buffer remains alive for the native call.
        let result = unsafe { photosort_vips_init(error.as_mut_ptr().cast(), error.len()) };
        if result == 0 {
            Ok(())
        } else {
            Err(error_text(&error))
        }
    })
    .clone()
    // No vips_shutdown: process-wide initialization outlives all image owners.
}
struct Cancellation<'a, F> {
    check: &'a F,
    panicked: AtomicBool,
}
unsafe extern "C" fn check_cancel<F: Fn() -> bool + Sync>(context: *mut c_void) -> c_int {
    // SAFETY: render retains the borrowed context until synchronous evaluation
    // and all its callbacks finish; F is Sync for native worker callbacks.
    let state = unsafe { &*context.cast::<Cancellation<'_, F>>() };
    if state.panicked.load(Ordering::Relaxed) {
        return 1;
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(state.check)) {
        Ok(value) => i32::from(value),
        Err(_) => {
            state.panicked.store(true, Ordering::Relaxed);
            1
        }
    }
}
/// Render without allowing cancellation failures, native errors or null buffers
/// to cross the FFI ownership boundary. A cancelled result is never returned.
pub fn render<F: Fn() -> bool + Sync>(
    path: &Path,
    request: Request,
    cancelled: F,
) -> Result<Image, Error> {
    render_inner(path, request, 1, 0, cancelled)
}
/// Render a region expressed in EXIF/sidecar-oriented source coordinates.
/// Pixels are cropped before rotation and returned in display orientation.
/// A nonzero tile grid expands the bounds to grid lines, clipped to the image;
/// its origin is `(region.x / grid * grid, region.y / grid * grid)`. The caller
/// owns the grid policy, request cache and memory budget. Source dimensions
/// remain unrotated. `edge: None` preserves native sample precision.
pub fn render_region<F: Fn() -> bool + Sync>(
    path: &Path,
    request: Request,
    orientation: u8,
    tile_edge: u32,
    cancelled: F,
) -> Result<Image, Error> {
    if request.region.is_none() || !(1..=8).contains(&orientation) || tile_edge > 4096 {
        return Err(Error("Invalid oriented inspection request".into()));
    }
    render_inner(path, request, orientation.into(), tile_edge, cancelled)
}
fn render_inner<F: Fn() -> bool + Sync>(
    path: &Path,
    request: Request,
    orientation: u32,
    tile_edge: u32,
    cancelled: F,
) -> Result<Image, Error> {
    if request.edge.is_some_and(|edge| !(1..=4096).contains(&edge)) {
        return Err(Error("Preview edge must be between 1 and 4096".into()));
    }
    if request
        .region
        .is_some_and(|r| r.width == 0 || r.height == 0)
    {
        return Err(Error("Inspection region must not be empty".into()));
    }
    #[cfg(unix)]
    let path = CString::new(path.as_os_str().as_encoded_bytes());
    #[cfg(not(unix))]
    let path = CString::new(
        path.to_str()
            .ok_or_else(|| Error("Image path is not valid Unicode".into()))?,
    );
    let path = path.map_err(|_| Error("Image path contains a null byte".into()))?;
    let mut context = Cancellation {
        check: &cancelled,
        panicked: AtomicBool::new(false),
    };
    let context_ptr = (&mut context as *mut Cancellation<'_, _>).cast();
    // Check before initializing the native engine or touching a file.
    if unsafe { check_cancel::<F>(context_ptr) } != 0 {
        return Err(Error("Image preparation cancelled".into()));
    }
    initialize()?;
    // libvips has one process-global error buffer. The application's image actor
    // is already serial; this guard also prevents accidental concurrent callers
    // from clearing or borrowing each other's error text.
    static RENDER: Mutex<()> = Mutex::new(());
    let _permit = loop {
        match RENDER.try_lock() {
            Ok(permit) => break permit,
            Err(TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                if unsafe { check_cancel::<F>(context_ptr) } != 0 {
                    return Err(Error(
                        if context.panicked.load(Ordering::Relaxed) {
                            "Cancellation callback panicked; image preparation stopped"
                        } else {
                            "Image preparation cancelled"
                        }
                        .into(),
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    };
    let mut output = NativeOutput::default();
    let mut error = [0u8; 2048];
    let mut warning = [0u8; 1024];
    let region = request.region.unwrap_or(Region {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    });
    // SAFETY: pointers remain valid until the synchronous call has joined its
    // evaluation workers. NativeOutput owns any returned buffer on every path.
    let result = unsafe {
        photosort_vips_render(
            path.as_ptr(),
            request.edge.unwrap_or(0),
            region.x,
            region.y,
            region.width,
            region.height,
            orientation,
            tile_edge,
            check_cancel::<F>,
            context_ptr,
            &mut output,
            error.as_mut_ptr().cast(),
            error.len(),
            warning.as_mut_ptr().cast(),
            warning.len(),
        )
    };
    if context.panicked.load(Ordering::Relaxed) {
        return Err(Error(
            "Cancellation callback panicked; image preparation stopped".into(),
        ));
    }
    if result != 0 {
        return Err(error_text(&error));
    }
    if output.width > 32768 || output.height > 32768 || !matches!(output.sample_bytes, 1 | 2) {
        return Err(Error(
            "Native image dimensions or sample size exceed their limits".into(),
        ));
    }
    let expected =
        u64::from(output.width) * u64::from(output.height) * 4 * u64::from(output.sample_bytes);
    if output.pixels.is_null()
        || output.width == 0
        || output.height == 0
        || !matches!(output.sample_bytes, 1 | 2)
        || expected > 512 * 1024 * 1024
        || expected != output.length as u64
    {
        return Err(Error("Invalid native image buffer".into()));
    }
    // SAFETY: the adapter returns initialized, suitably aligned g_malloc memory
    // with the independently checked sample count, held alive by output's Drop.
    let pixels = unsafe {
        match output.sample_bytes {
            1 => Pixels::U8(
                std::slice::from_raw_parts(output.pixels.cast::<u8>(), output.length).to_vec(),
            ),
            2 => Pixels::U16(
                std::slice::from_raw_parts(output.pixels.cast::<u16>(), output.length / 2).to_vec(),
            ),
            _ => unreachable!(),
        }
    };
    if unsafe { check_cancel::<F>(context_ptr) } != 0 {
        return Err(Error("Image preparation cancelled".into()));
    }
    Ok(Image {
        width: output.width,
        height: output.height,
        source_width: output.source_width,
        source_height: output.source_height,
        profiled: output.profiled != 0,
        source_sample_bytes: output.source_sample_bytes,
        decoder_warning: (warning[0] != 0).then(|| error_text(&warning).0),
        pixels,
    })
}

#[cfg(test)]
mod tests;
