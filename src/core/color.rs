//! Embedded-profile normalization owned by the shared image pipeline.
//! Image decoders return encoded samples; egui expects sRGB encoded samples.
use anyhow::{Context, Result, ensure};
use image::{ColorType, DynamicImage, ImageDecoder};
use moxcms::{ColorProfile, Layout, ParsingOptions, RenderingIntent, TransformOptions};
use std::{
    collections::VecDeque,
    io::{BufRead, Seek, SeekFrom},
    sync::Arc,
};

const MAX_PROFILE: usize = 4 * 1024 * 1024;
const MAX_IMAGE: usize = 512 * 1024 * 1024;
const MAX_TRANSFORMS: usize = 4;
const CHUNK_PIXELS: usize = 8192;
#[derive(Clone)]
enum Transform {
    U8(Arc<moxcms::Transform8BitExecutor>),
    U16(Arc<moxcms::Transform16BitExecutor>),
    F32(Arc<moxcms::TransformF32Executor>),
}
#[derive(Default)]
pub(super) struct Manager {
    cache: VecDeque<(blake3::Hash, Transform)>,
    pub initializations: u64,
    pub conversions: u64,
}
impl Manager {
    pub fn clear(&mut self) {
        self.cache.clear();
    }
    pub fn decode<R: BufRead + Seek>(
        &mut self,
        mut reader: image::ImageReader<R>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DynamicImage> {
        ensure!(!cancelled(), "Image preparation cancelled");
        reader.limits(decode_limits());
        if reader.format() == Some(image::ImageFormat::Jpeg) {
            let mut input = reader.into_inner();
            let start = input.stream_position()?;
            let mut decoder = image::codecs::jpeg::JpegDecoder::new(&mut input)?;
            decoder.set_limits(decode_limits())?;
            let profile = decoder
                .icc_profile()
                .context("Cannot read embedded color profile")?;
            if let Some(profile) = profile.as_ref().filter(|p| p.get(16..20) == Some(b"CMYK")) {
                drop(decoder);
                input.seek(SeekFrom::Start(start))?;
                return self.decode_cmyk(input, profile, cancelled);
            }
            return self.decode_rgb(decoder, profile, cancelled);
        }
        let mut decoder = reader.into_decoder()?;
        let profile = decoder
            .icc_profile()
            .context("Cannot read embedded color profile")?;
        self.decode_rgb(decoder, profile, cancelled)
    }
    fn decode_rgb(
        &mut self,
        decoder: impl ImageDecoder,
        profile: Option<Vec<u8>>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<DynamicImage> {
        // into_decoder preserves codec limits but, unlike ImageReader::decode,
        // does not reserve the final DynamicImage allocation itself.
        ensure!(
            decoder.total_bytes() <= MAX_IMAGE as u64,
            "Image exceeds the 512 MiB decode budget"
        );
        if let Some(profile) = &profile {
            ensure!(
                profile.len() <= MAX_PROFILE,
                "Embedded color profile exceeds 4 MiB"
            );
        }
        ensure!(!cancelled(), "Image preparation cancelled");
        let image = DynamicImage::from_decoder(decoder)?;
        match profile {
            Some(profile) => self.normalize(image, &profile, cancelled),
            None => Ok(image), // Preserve the established sRGB assumption for untagged files.
        }
    }
    fn decode_cmyk(
        &mut self,
        input: impl std::io::Read,
        profile: &[u8],
        cancelled: &impl Fn() -> bool,
    ) -> Result<DynamicImage> {
        ensure!(!cancelled(), "Image preparation cancelled");
        ensure!(
            profile.len() <= MAX_PROFILE,
            "Embedded color profile exceeds 4 MiB"
        );
        let mut decoder = jpeg_decoder::Decoder::new(input);
        decoder.set_max_decoding_buffer_size(MAX_IMAGE);
        decoder.read_info().context("Cannot read CMYK JPEG")?;
        let info = decoder.info().context("Missing CMYK JPEG dimensions")?;
        decode_limits().check_dimensions(info.width.into(), info.height.into())?;
        ensure!(
            info.pixel_format == jpeg_decoder::PixelFormat::CMYK32,
            "CMYK profile does not match JPEG color channels"
        );
        let pixels = decoder.decode().context("Cannot decode CMYK JPEG")?;
        ensure!(!cancelled(), "Image preparation cancelled");
        // jpeg-decoder normalizes Adobe CMYK and YCCK polarity to ordinary
        // ink coverage (0 = no ink). No approximate RGB intermediate is used.
        let Transform::U8(transform) = self.transform(profile, ColorType::Rgba8, true)? else {
            anyhow::bail!("CMYK color conversion layout mismatch");
        };
        let rgb = expand(
            &pixels,
            4,
            3,
            |src, dst| transform.transform(src, dst),
            cancelled,
        )?;
        ensure!(!cancelled(), "Color conversion cancelled");
        let image = image::RgbImage::from_raw(info.width.into(), info.height.into(), rgb)
            .context("Invalid CMYK conversion dimensions")?;
        self.conversions += 1;
        Ok(DynamicImage::ImageRgb8(image))
    }
    fn transform(&mut self, bytes: &[u8], color: ColorType, cmyk: bool) -> Result<Transform> {
        ensure!(
            bytes.len() <= MAX_PROFILE,
            "Embedded color profile exceeds 4 MiB"
        );
        let mut hash = blake3::Hasher::new();
        hash.update(bytes);
        hash.update(&[color as u8, u8::from(cmyk)]);
        let key = hash.finalize();
        if let Some(index) = self.cache.iter().position(|(cached, _)| *cached == key) {
            let entry = self.cache.remove(index).unwrap();
            let transform = entry.1.clone();
            self.cache.push_back(entry);
            return Ok(transform);
        }
        let profile = ColorProfile::new_from_slice_with_options(
            bytes,
            ParsingOptions {
                max_profile_size: MAX_PROFILE,
                max_allowed_clut_size: MAX_PROFILE,
                max_allowed_trc_size: 40_000,
            },
        )
        .context("Invalid embedded color profile")?;
        let (source, target) = if cmyk {
            (Layout::Rgba, Layout::Rgb) // moxcms represents CMYK with four channels.
        } else {
            match color {
                ColorType::L8 | ColorType::L16 => (Layout::Gray, Layout::Rgb),
                ColorType::La8 | ColorType::La16 => (Layout::GrayAlpha, Layout::Rgba),
                ColorType::Rgb8 | ColorType::Rgb16 | ColorType::Rgb32F => {
                    (Layout::Rgb, Layout::Rgb)
                }
                ColorType::Rgba8 | ColorType::Rgba16 | ColorType::Rgba32F => {
                    (Layout::Rgba, Layout::Rgba)
                }
                _ => anyhow::bail!("Unsupported color sample format"),
            }
        };
        // Never interpret decoder-converted CMYK samples as the original ink channels.
        ensure!(
            if cmyk {
                profile.color_space == moxcms::DataColorSpace::Cmyk
            } else {
                matches!(
                    profile.color_space,
                    moxcms::DataColorSpace::Rgb | moxcms::DataColorSpace::Gray
                )
            },
            "Embedded color profile is not RGB or grayscale; accurate preview is unavailable"
        );
        let options = TransformOptions {
            rendering_intent: RenderingIntent::RelativeColorimetric,
            ..Default::default()
        };
        let srgb = ColorProfile::new_srgb();
        let transform = match color {
            ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8 => {
                Transform::U8(profile.create_transform_8bit(source, &srgb, target, options)?)
            }
            ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16 => {
                Transform::U16(profile.create_transform_16bit(source, &srgb, target, options)?)
            }
            ColorType::Rgb32F | ColorType::Rgba32F => {
                Transform::F32(profile.create_transform_f32(source, &srgb, target, options)?)
            }
            _ => anyhow::bail!("Unsupported color sample format"),
        };
        if self.cache.len() == MAX_TRANSFORMS {
            self.cache.pop_front();
        }
        self.cache.push_back((key, transform.clone()));
        self.initializations += 1;
        Ok(transform)
    }
    pub(super) fn normalize(
        &mut self,
        mut image: DynamicImage,
        profile: &[u8],
        cancelled: &impl Fn() -> bool,
    ) -> Result<DynamicImage> {
        ensure!(!cancelled(), "Color conversion cancelled");
        let transform = self
            .transform(profile, image.color(), false)
            .context("Cannot apply embedded color profile")?;
        ensure!(!cancelled(), "Color conversion cancelled");
        macro_rules! same {
            ($buffer:expr, $transform:expr, $channels:expr) => {
                same(
                    $buffer.as_mut(),
                    $channels,
                    |src, dst| $transform.transform(src, dst),
                    cancelled,
                )?
            };
        }
        macro_rules! expand {
            ($buffer:expr, $transform:expr, $source:expr, $target:expr, $variant:ident) => {{
                let (w, h) = $buffer.dimensions();
                let pixels = expand(
                    $buffer.as_raw(),
                    $source,
                    $target,
                    |src, dst| $transform.transform(src, dst),
                    cancelled,
                )?;
                image = DynamicImage::$variant(
                    image::ImageBuffer::from_raw(w, h, pixels)
                        .context("Invalid color conversion dimensions")?,
                );
            }};
        }
        match (&mut image, transform) {
            (DynamicImage::ImageRgb8(b), Transform::U8(t)) => same!(b, t, 3),
            (DynamicImage::ImageRgba8(b), Transform::U8(t)) => same!(b, t, 4),
            (DynamicImage::ImageRgb16(b), Transform::U16(t)) => same!(b, t, 3),
            (DynamicImage::ImageRgba16(b), Transform::U16(t)) => same!(b, t, 4),
            (DynamicImage::ImageRgb32F(b), Transform::F32(t)) => same!(b, t, 3),
            (DynamicImage::ImageRgba32F(b), Transform::F32(t)) => same!(b, t, 4),
            (DynamicImage::ImageLuma8(b), Transform::U8(t)) => expand!(b, t, 1, 3, ImageRgb8),
            (DynamicImage::ImageLumaA8(b), Transform::U8(t)) => expand!(b, t, 2, 4, ImageRgba8),
            (DynamicImage::ImageLuma16(b), Transform::U16(t)) => expand!(b, t, 1, 3, ImageRgb16),
            (DynamicImage::ImageLumaA16(b), Transform::U16(t)) => expand!(b, t, 2, 4, ImageRgba16),
            _ => anyhow::bail!("Color conversion layout mismatch"),
        }
        ensure!(!cancelled(), "Color conversion cancelled");
        image.set_color_space(image::metadata::Cicp::SRGB)?;
        self.conversions += 1;
        Ok(image)
    }
}
// One limit policy for standard images, embedded RAW JPEGs and CMYK fallback.
fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_IMAGE as u64);
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits
}
fn same<T: Copy + Default>(
    pixels: &mut [T],
    channels: usize,
    transform: impl Fn(&[T], &mut [T]) -> Result<(), moxcms::CmsError>,
    cancelled: &impl Fn() -> bool,
) -> Result<()> {
    let mut scratch = Vec::with_capacity(CHUNK_PIXELS * channels);
    for chunk in pixels.chunks_mut(CHUNK_PIXELS * channels) {
        ensure!(!cancelled(), "Color conversion cancelled");
        scratch.clear();
        scratch.extend_from_slice(chunk);
        transform(&scratch, chunk)?;
    }
    Ok(())
}
fn expand<T: Copy + Default>(
    source: &[T],
    from: usize,
    to: usize,
    transform: impl Fn(&[T], &mut [T]) -> Result<(), moxcms::CmsError>,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<T>> {
    let samples = (source.len() / from)
        .checked_mul(to)
        .context("Color conversion size overflow")?;
    ensure!(
        samples
            .checked_mul(std::mem::size_of::<T>())
            .is_some_and(|bytes| bytes <= MAX_IMAGE),
        "Color conversion exceeds the 512 MiB decode budget"
    );
    let mut output = Vec::new();
    output.try_reserve_exact(samples)?;
    output.resize(samples, T::default());
    for (source, output) in source
        .chunks(CHUNK_PIXELS * from)
        .zip(output.chunks_mut(CHUNK_PIXELS * to))
    {
        ensure!(!cancelled(), "Color conversion cancelled");
        transform(source, output)?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    fn linear() -> Vec<u8> {
        let mut profile = ColorProfile::new_srgb();
        profile.cicp = None; // The fixture changes the transfer curve to linear.
        let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
        profile.red_trc = Some(curve.clone());
        profile.green_trc = Some(curve.clone());
        profile.blue_trc = Some(curve);
        profile.encode().unwrap()
    }
    #[test]
    fn linear_rgb_and_gray_use_srgb_encoding_and_preserve_alpha_at_native_precision() {
        let mut manager = Manager::default();
        let p = linear();
        let result = manager
            .normalize(
                DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                    2,
                    2,
                    image::Rgba([128, 128, 128, 73]),
                )),
                &p,
                &|| false,
            )
            .unwrap();
        assert_eq!(result.to_rgba8().get_pixel(0, 0).0, [188, 188, 188, 73]);
        let result = manager
            .normalize(
                DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                    2,
                    2,
                    image::Rgba([32768u16, 32768, 32768, 12345]),
                )),
                &p,
                &|| false,
            )
            .unwrap();
        assert_eq!(result.color(), ColorType::Rgba16);
        let pixels = result.to_rgba16();
        let pixel = pixels.get_pixel(0, 0).0;
        assert!((i32::from(pixel[0]) - 48192).abs() <= 4, "{pixel:?}");
        assert_eq!(pixel[3], 12345);
        let gray = ColorProfile::new_gray_with_gamma(1.).encode().unwrap();
        let result = manager
            .normalize(
                DynamicImage::ImageLumaA8(image::ImageBuffer::from_pixel(
                    2,
                    2,
                    image::LumaA([128, 71]),
                )),
                &gray,
                &|| false,
            )
            .unwrap();
        for value in &result.to_rgba8().get_pixel(0, 0).0[..3] {
            assert!((i16::from(*value) - 188).abs() <= 1);
        }
        assert_eq!(result.to_rgba8().get_pixel(0, 0).0[3], 71);
    }
    #[test]
    fn display_p3_uses_its_primaries() {
        let mut manager = Manager::default();
        let input = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1,
            1,
            image::Rgb([180, 100, 50]),
        ));
        let result = manager
            .normalize(
                input,
                &ColorProfile::new_display_p3().encode().unwrap(),
                &|| false,
            )
            .unwrap();
        // Independent sRGB transfer + P3-to-sRGB matrix reference, rounded to 8 bit.
        let eotf = |x: f64| {
            let x = x / 255.;
            if x <= 0.04045 {
                x / 12.92
            } else {
                ((x + 0.055) / 1.055).powf(2.4)
            }
        };
        let oetf = |x: f64| {
            let x = x.clamp(0., 1.);
            ((if x <= 0.0031308 {
                12.92 * x
            } else {
                1.055 * x.powf(1. / 2.4) - 0.055
            }) * 255.)
                .round() as i16
        };
        let input = [eotf(180.), eotf(100.), eotf(50.)];
        let matrix = [
            [1.224745, -0.224904, 0.],
            [-0.042058, 1.042081, 0.],
            [-0.019642, -0.078655, 1.098537],
        ];
        let expected = matrix.map(|row| oetf(row.iter().zip(input).map(|(a, b)| a * b).sum()));
        for (got, expected) in result.to_rgb8().get_pixel(0, 0).0.into_iter().zip(expected) {
            assert!((i16::from(got) - expected).abs() <= 2);
        }
    }
    #[test]
    fn transforms_are_reused_bounded_and_cleared_without_retaining_images() {
        let mut manager = Manager::default();
        let profile = linear();
        for _ in 0..3 {
            manager
                .normalize(DynamicImage::new_rgb8(2, 2), &profile, &|| false)
                .unwrap();
        }
        assert_eq!(manager.initializations, 1);
        assert_eq!(manager.conversions, 3);
        for gamma in 1..=8 {
            let profile = ColorProfile::new_gray_with_gamma(gamma as f32)
                .encode()
                .unwrap();
            manager
                .normalize(DynamicImage::new_luma8(1, 1), &profile, &|| false)
                .unwrap();
        }
        assert_eq!(manager.cache.len(), MAX_TRANSFORMS);
        manager.clear();
        assert!(manager.cache.is_empty());
    }
    #[test]
    fn jpeg_paths_keep_the_shared_dimension_limit_before_pixel_allocation() {
        for color in [
            jpeg_encoder::ColorType::Rgb,
            jpeg_encoder::ColorType::Cmyk,
            jpeg_encoder::ColorType::CmykAsYcck,
        ] {
            let mut bytes = Vec::new();
            let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, 90);
            let mut profile = vec![0; 128];
            profile[16..20].copy_from_slice(b"CMYK");
            encoder.add_icc_profile(&profile).unwrap();
            let channels = if color == jpeg_encoder::ColorType::Rgb {
                3
            } else {
                4
            };
            encoder
                .encode(&vec![64; 8 * 8 * channels], 8, 8, color)
                .unwrap();
            let sof = bytes.windows(2).position(|m| m == [0xff, 0xc0]).unwrap();
            bytes[sof + 5..sof + 7].copy_from_slice(&32769u16.to_be_bytes());
            let reader = image::ImageReader::with_format(
                std::io::Cursor::new(bytes),
                image::ImageFormat::Jpeg,
            );
            let mut manager = Manager::default();
            let error = manager.decode(reader, &|| false).unwrap_err();
            assert!(
                error.to_string().contains("Image size exceeds limit"),
                "{error:#}"
            );
            assert_eq!(manager.conversions, 0);
            assert_eq!(manager.initializations, 0);
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn cmyk_cancellation_during_chunks_does_not_publish_and_reuses_transform_on_retry() {
        let profile =
            std::fs::read("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc").unwrap();
        let mut bytes = Vec::new();
        let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, 95);
        encoder.add_icc_profile(&profile).unwrap();
        encoder
            .encode(
                &vec![64; 128 * 128 * 4],
                128,
                128,
                jpeg_encoder::ColorType::CmykAsYcck,
            )
            .unwrap();
        let mut manager = Manager::default();
        let calls = Cell::new(0);
        let reader = || {
            image::ImageReader::with_format(std::io::Cursor::new(&bytes), image::ImageFormat::Jpeg)
        };
        let error = manager
            .decode(reader(), &|| {
                calls.set(calls.get() + 1);
                calls.get() > 4
            })
            .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        assert_eq!(manager.conversions, 0);
        assert_eq!(manager.initializations, 1);
        assert_eq!(manager.decode(reader(), &|| false).unwrap().width(), 128);
        assert_eq!(manager.conversions, 1);
        assert_eq!(manager.initializations, 1);
    }
    #[test]
    fn malformed_oversized_or_mismatched_profiles_never_publish_partial_pixels() {
        let mut manager = Manager::default();
        for profile in [
            vec![0; 128],
            vec![0; MAX_PROFILE + 1],
            ColorProfile::new_gray_with_gamma(1.).encode().unwrap(),
        ] {
            assert!(
                manager
                    .normalize(DynamicImage::new_rgb8(1, 1), &profile, &|| false)
                    .is_err()
            );
        }
        assert_eq!(manager.conversions, 0);
        let count = Cell::new(0);
        assert!(
            manager
                .normalize(DynamicImage::new_rgb8(512, 512), &linear(), &|| {
                    count.set(count.get() + 1);
                    count.get() > 4
                })
                .is_err()
        );
        assert_eq!(manager.conversions, 0);
    }
}
