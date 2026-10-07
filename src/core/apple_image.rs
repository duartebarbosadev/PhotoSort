//! One owned boundary for the system ImageIO/ColorSync decoder and header reader.
use anyhow::{Context, Result, ensure};
use image::RgbaImage;
use std::{ffi::c_long, path::Path};
unsafe extern "C" {
    fn photosort_apple_decode(
        path: *const u8,
        len: c_long,
        edge: u32,
        w: *mut u32,
        h: *mut u32,
        source_w: *mut u32,
        source_h: *mut u32,
    ) -> *mut u8;
    fn photosort_apple_free(p: *mut u8);
    fn photosort_apple_orientation(path: *const u8, len: c_long) -> u32;
}
struct Owned(*mut u8);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { photosort_apple_free(self.0) };
    }
}
pub struct Rendered {
    pub pixels: RgbaImage,
    /// Dimensions before the application's orientation override.
    pub source: (u32, u32),
}
/// No cached objects escape ImageIO; its malloc-owned buffer is copied into Rust ownership.
/// `None` requests original pixels; a bounded view never substitutes embedded thumbnails.
pub fn decode(path: &Path, edge: Option<u32>) -> Result<Rendered> {
    ensure!(
        edge.is_none_or(|e| e > 0 && e <= 4096),
        "Invalid ImageIO preview edge"
    );
    let bytes = path.as_os_str().as_encoded_bytes();
    let (mut w, mut h, mut sw, mut sh) = (0, 0, 0, 0);
    // SAFETY: synchronous borrowed path bytes and valid output pointers. The C
    // adapter checks geometry and returns an owned, straight-RGBA allocation.
    let owned = Owned(unsafe {
        photosort_apple_decode(
            bytes.as_ptr(),
            bytes.len() as c_long,
            edge.unwrap_or(0),
            &mut w,
            &mut h,
            &mut sw,
            &mut sh,
        )
    });
    ensure!(!owned.0.is_null(), "ImageIO could not decode image");
    ensure!(
        w > 0
            && h > 0
            && sw > 0
            && sh > 0
            && u64::from(w) * u64::from(h) <= 100_000_000
            && u64::from(sw) * u64::from(sh) <= 100_000_000,
        "Invalid ImageIO dimensions"
    );
    let pixels =
        unsafe { std::slice::from_raw_parts(owned.0, w as usize * h as usize * 4) }.to_vec();
    Ok(Rendered {
        pixels: RgbaImage::from_raw(w, h, pixels).context("Invalid ImageIO output")?,
        source: (sw, sh),
    })
}
pub fn orientation(path: &Path) -> u8 {
    let bytes = path.as_os_str().as_encoded_bytes();
    // SAFETY: the path remains alive for this synchronous header-only call.
    unsafe { photosort_apple_orientation(bytes.as_ptr(), bytes.len() as c_long) }.clamp(1, 8) as u8
}
