//! Portable HEIF integration. libheif owns codecs, crop and container transforms;
//! the shared color manager owns RGB-to-sRGB conversion and transform reuse.
use super::{color, pipeline::orient};
use anyhow::{Context, Result, ensure};
use image::{DynamicImage, RgbaImage};
use libheif_rs::{ColorSpace, DecodingOptions, HeifContext, LibHeif, RgbChroma, StreamReader};
use std::{ffi::CString, fs::File, path::Path, sync::OnceLock};

fn library() -> Result<&'static LibHeif> {
    static LIBRARY: OnceLock<Result<LibHeif, String>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| LibHeif::new_checked().map_err(|error| error.to_string()))
        .as_ref()
        .map_err(|error| anyhow::anyhow!("HEIF initialization: {error}"))
}
// Linear parts of EXIF's eight orthogonal transforms in image coordinates.
const MATRICES: [[i8; 4]; 8] = [
    [1, 0, 0, 1],
    [-1, 0, 0, 1],
    [-1, 0, 0, -1],
    [1, 0, 0, -1],
    [0, 1, 1, 0],
    [0, -1, 1, 0],
    [0, -1, -1, 0],
    [0, 1, -1, 0],
];
fn compose(left: u8, right: u8) -> u8 {
    let [a, b, c, d] = MATRICES[usize::from(left - 1)];
    let [e, f, g, h] = MATRICES[usize::from(right - 1)];
    let product = [a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h];
    (MATRICES
        .iter()
        .position(|m| *m == product)
        .expect("Orthogonal transform closure")
        + 1) as u8
}
fn relative_orientation(container: u8, desired: u8) -> u8 {
    let inverse = match container {
        6 => 8,
        8 => 6,
        other => other,
    };
    compose(desired, inverse)
}
pub(super) fn orientation(path: &Path) -> Option<u8> {
    library().ok()?;
    let path = CString::new(path.to_str()?).ok()?;
    let mut operations = [0i32; 32];
    unsafe extern "C" {
        fn photosort_heif_transforms(
            path: *const std::ffi::c_char,
            operations: *mut i32,
            capacity: i32,
        ) -> i32;
    }
    // SAFETY: the terminated path and bounded writable array live through this call.
    let count = unsafe { photosort_heif_transforms(path.as_ptr(), operations.as_mut_ptr(), 32) };
    if !(0..=32).contains(&count) {
        return None;
    }
    let mut result = 1;
    for operation in &operations[..count as usize] {
        let transform = match operation {
            0 => 1,
            90 => 8,
            180 => 3,
            270 => 6,
            -1 => 2,
            -2 => 4,
            _ => return None,
        };
        result = compose(transform, result);
    }
    Some(result)
}

pub(super) fn decode(
    path: &Path,
    container: u8,
    desired: u8,
    colors: &mut color::Manager,
    cancelled: &impl Fn() -> bool,
) -> Result<DynamicImage> {
    ensure!(
        (1..=8).contains(&container) && (1..=8).contains(&desired),
        "Invalid HEIF orientation"
    );
    ensure!(!cancelled(), "HEIF preparation cancelled");
    let library = library()?;
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut context = HeifContext::new()?;
    let mut limits = context.security_limits();
    limits.set_max_image_size_pixels(100_000_000);
    limits.set_max_color_profile_size(4 * 1024 * 1024);
    limits.set_max_memory_block_size(512 * 1024 * 1024);
    limits.set_max_total_memory(512 * 1024 * 1024);
    context.set_security_limits(&limits)?;
    context.set_max_decoding_threads(4);
    context.read_reader(Box::new(StreamReader::new(file, size)))?;
    let handle = context.primary_image_handle()?;
    let icc = handle.color_profile_raw();
    let mut options = DecodingOptions::new().context("Create HEIF decoding options")?;
    options.set_strict_decoding(true);
    // Decode geometry once in libheif. Retain original RGB colorimetry so ICC or
    // NCLX normalization happens exactly once in the shared color manager.
    options.set_output_image_nclx_profile_passthrough(true);
    ensure!(!cancelled(), "HEIF preparation cancelled");
    let decoded = library.decode(&handle, ColorSpace::Rgb(RgbChroma::Rgba), Some(options))?;
    ensure!(!cancelled(), "HEIF preparation cancelled");
    let planes = decoded.planes();
    let plane = planes.interleaved.context("HEIF has no RGBA plane")?;
    let row_bytes = (plane.width as usize)
        .checked_mul(4)
        .context("HEIF row overflow")?;
    let bytes = row_bytes
        .checked_mul(plane.height as usize)
        .context("HEIF size overflow")?;
    ensure!(
        plane.width > 0 && plane.height > 0 && bytes <= 400_000_000 && plane.stride >= row_bytes,
        "Invalid HEIF dimensions or stride"
    );
    let mut pixels = Vec::with_capacity(bytes);
    let premultiplied = decoded.is_premultiplied_alpha();
    for y in 0..plane.height as usize {
        ensure!(!cancelled(), "HEIF preparation cancelled");
        let start = y.checked_mul(plane.stride).context("HEIF row overflow")?;
        let row = plane
            .data
            .get(start..start + row_bytes)
            .context("Incomplete HEIF RGBA plane")?;
        let output_start = pixels.len();
        pixels.extend_from_slice(row);
        // Normalize each row while it is hot in the cache, without traversing
        // the complete image again or allocating another pixel buffer.
        for pixel in pixels[output_start..].as_chunks_mut::<4>().0 {
            let alpha = u16::from(pixel[3]);
            if alpha == 0 {
                pixel[..3].fill(0);
            } else if premultiplied {
                for color in &mut pixel[..3] {
                    *color = ((u16::from(*color).min(alpha) * 255 + alpha / 2) / alpha) as u8;
                }
            }
        }
    }
    let mut image = DynamicImage::ImageRgba8(
        RgbaImage::from_raw(plane.width, plane.height, pixels).context("Invalid HEIF pixels")?,
    );
    if let Some(profile) = icc {
        image = colors.normalize(image, &profile.data, cancelled)?;
    } else if let Some(profile) = handle
        .color_profile_nclx()
        .or_else(|| decoded.color_profile_nclx())
    {
        let primaries = profile.color_primaries() as u16;
        let transfer = profile.transfer_characteristics() as u16;
        let profile = nclx_profile(primaries, transfer)?;
        image = colors.normalize(image, &profile, cancelled)?;
    }
    ensure!(!cancelled(), "HEIF preparation cancelled");
    let result = orient(image, relative_orientation(container, desired));
    ensure!(!cancelled(), "HEIF preparation cancelled");
    Ok(result)
}
fn nclx_profile(primaries: u16, transfer: u16) -> Result<Vec<u8>> {
    use moxcms::{
        CicpColorPrimaries, CicpProfile, ColorPrimaries, ColorProfile, MatrixCoefficients,
        ToneReprCurve, TransferCharacteristics,
    };
    let primaries =
        CicpColorPrimaries::try_from(u8::try_from(if primaries == 2 { 1 } else { primaries })?)?;
    let transfer = TransferCharacteristics::try_from(u8::try_from(if transfer == 2 {
        13
    } else {
        transfer
    })?)?;
    // Validate before constructing: unsupported/reserved identifiers must not
    // silently become a partially initialized color profile.
    let _: ColorPrimaries = primaries.try_into()?;
    let _: ToneReprCurve = transfer.try_into()?;
    let profile = ColorProfile::new_from_cicp(CicpProfile {
        color_primaries: primaries,
        transfer_characteristics: transfer,
        matrix_coefficients: MatrixCoefficients::Identity,
        full_range: true,
    });
    Ok(profile.encode()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn relative_transforms_match_all_container_and_sidecar_combinations() {
        let source = DynamicImage::ImageRgba8(RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba([(x + y * 3) as u8, 0, 0, 255])
        }));
        for container in 1..=8 {
            for desired in 1..=8 {
                let decoded = orient(source.clone(), container);
                assert_eq!(
                    orient(decoded, relative_orientation(container, desired)),
                    orient(source.clone(), desired),
                    "container {container}, desired {desired}"
                );
            }
        }
    }
    #[test]
    fn linear_nclx_uses_shared_profile_conversion_and_reuses_the_transform() {
        let profile = nclx_profile(1, 8).unwrap();
        let mut colors = color::Manager::default();
        for _ in 0..2 {
            let input = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
                1,
                1,
                image::Rgba([128, 128, 128, 64]),
            ));
            let output = colors
                .normalize(input, &profile, &|| false)
                .unwrap()
                .to_rgba8();
            assert!((187..=189).contains(&output.get_pixel(0, 0).0[0]));
            assert_eq!(output.get_pixel(0, 0).0[3], 64);
        }
        assert_eq!((colors.initializations, colors.conversions), (1, 2));
        assert!(nclx_profile(0, 13).is_err());
        assert!(nclx_profile(1, 0).is_err());
        assert!(nclx_profile(256, 13).is_err());
    }
}
