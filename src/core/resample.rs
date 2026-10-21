//! The display proxy's downscale, without `image`'s full-size float buffer.
//!
//! `imageops::resize` filters every source column into a source-width by
//! target-height `Rgba32F` image before it filters rows: about 74 MB for the
//! 2048 px proxy of a 9 MP photo. [`fit_triangle`] does the same f32
//! arithmetic in the same order one output row at a time, so its pixels are
//! bit-identical while only one row of column sums is live. Proxies cached
//! by earlier builds therefore stay valid; any change to the arithmetic here
//! must bump the render version in `Pipeline::key`.
use image::{DynamicImage, GenericImageView, ImageBuffer, Pixel, imageops::FilterType};

/// `image.resize(edge, edge, FilterType::Triangle)`, streamed.
pub fn fit_triangle(image: &DynamicImage, edge: u32) -> DynamicImage {
    if (edge, edge) == image.dimensions() {
        return image.clone();
    }
    if image.width() == 0 || image.height() == 0 {
        // Nothing to sample; `image` returns a blank buffer of the fitted size.
        return image.resize(edge, edge, FilterType::Triangle);
    }
    let (width, height) = fit(image.width(), image.height(), edge);
    match image {
        DynamicImage::ImageLuma8(p) => DynamicImage::ImageLuma8(resize(p, width, height)),
        DynamicImage::ImageLumaA8(p) => DynamicImage::ImageLumaA8(resize(p, width, height)),
        DynamicImage::ImageRgb8(p) => DynamicImage::ImageRgb8(resize(p, width, height)),
        DynamicImage::ImageRgba8(p) => DynamicImage::ImageRgba8(resize(p, width, height)),
        DynamicImage::ImageLuma16(p) => DynamicImage::ImageLuma16(resize(p, width, height)),
        DynamicImage::ImageLumaA16(p) => DynamicImage::ImageLumaA16(resize(p, width, height)),
        DynamicImage::ImageRgb16(p) => DynamicImage::ImageRgb16(resize(p, width, height)),
        DynamicImage::ImageRgba16(p) => DynamicImage::ImageRgba16(resize(p, width, height)),
        DynamicImage::ImageRgb32F(p) => DynamicImage::ImageRgb32F(resize(p, width, height)),
        DynamicImage::ImageRgba32F(p) => DynamicImage::ImageRgba32F(resize(p, width, height)),
        _ => image.resize(edge, edge, FilterType::Triangle),
    }
}

/// `image`'s private `resize_dimensions` for a square bound without fill.
fn fit(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let ratio = f64::min(
        f64::from(edge) / f64::from(width),
        f64::from(edge) / f64::from(height),
    );
    let scaled = |side: u32| ((f64::from(side) * ratio).round() as u32).max(1);
    (scaled(width), scaled(height))
}

/// Channel types with `image`'s clamping and rounding back from f32.
trait Sample: Copy {
    fn to_f32(self) -> f32;
    fn from_f32(value: f32) -> Self;
}
impl Sample for u8 {
    fn to_f32(self) -> f32 {
        f32::from(self)
    }
    fn from_f32(value: f32) -> Self {
        value.clamp(0.0, 255.0).round() as u8
    }
}
impl Sample for u16 {
    fn to_f32(self) -> f32 {
        f32::from(self)
    }
    fn from_f32(value: f32) -> Self {
        value.clamp(0.0, 65535.0).round() as u16
    }
}
impl Sample for f32 {
    fn to_f32(self) -> f32 {
        self
    }
    fn from_f32(value: f32) -> Self {
        value.clamp(0.0, 1.0)
    }
}

fn triangle(x: f32) -> f32 {
    if x.abs() < 1.0 { 1.0 - x.abs() } else { 0.0 }
}

/// First source sample and normalized weights of each output sample along
/// one axis, computed as `image::imageops::sample` does for `Triangle`
/// (support 1.0), zero-weight edge taps included.
fn taps(source: u32, target: u32) -> Vec<(usize, Vec<f32>)> {
    let ratio = source as f32 / target as f32;
    let scale = if ratio < 1.0 { 1.0 } else { ratio };
    (0..target)
        .map(|out| {
            let center = (out as f32 + 0.5) * ratio;
            let left = ((center - scale).floor() as i64).clamp(0, i64::from(source) - 1);
            let right = ((center + scale).ceil() as i64).clamp(left + 1, i64::from(source));
            let center = center - 0.5;
            let mut weights: Vec<f32> = (left..right)
                .map(|i| triangle((i as f32 - center) / scale))
                .collect();
            let mut sum = 0.0;
            for weight in &weights {
                sum += weight;
            }
            for weight in &mut weights {
                *weight /= sum;
            }
            (left as usize, weights)
        })
        .collect()
}

/// Columns first, then rows, as `imageops::resize` does. Each sum starts at
/// zero and adds `sample * weight` in tap order, unfused, so every output
/// sample rounds exactly as it does there.
fn resize<P>(
    source: &ImageBuffer<P, Vec<P::Subpixel>>,
    width: u32,
    height: u32,
) -> ImageBuffer<P, Vec<P::Subpixel>>
where
    P: Pixel,
    P::Subpixel: Sample,
{
    let channels = usize::from(P::CHANNEL_COUNT);
    let source_row = source.width() as usize * channels;
    let columns = taps(source.width(), width);
    let mut out = source.buffer_with_dimensions(width, height);
    let mut sums = vec![0.0f32; source_row];
    let out_rows = out.chunks_exact_mut(width as usize * channels);
    for ((top, weights), out_row) in taps(source.height(), height).into_iter().zip(out_rows) {
        sums.fill(0.0);
        let rows = source.as_raw()[top * source_row..].chunks_exact(source_row);
        for (row, &weight) in rows.zip(&weights) {
            for (sum, &sample) in sums.iter_mut().zip(row) {
                *sum += sample.to_f32() * weight;
            }
        }
        for ((left, weights), out_pixel) in columns.iter().zip(out_row.chunks_exact_mut(channels)) {
            let mut pixel = [0.0f32; 4];
            for (sums, &weight) in sums[left * channels..].chunks_exact(channels).zip(weights) {
                for (total, &sum) in pixel.iter_mut().zip(sums) {
                    *total += sum * weight;
                }
            }
            for (out, &total) in out_pixel.iter_mut().zip(&pixel) {
                *out = Sample::from_f32(total);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise with smooth regions, so both rounding paths and
    /// exact halves are exercised.
    fn source(width: u32, height: u32, seed: u32) -> DynamicImage {
        let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        let pixels = (0..width * height * 4)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if (i / 4) % 7 < 3 {
                    (i / 4 % width) as u8
                } else {
                    (state >> 24) as u8
                }
            })
            .collect();
        DynamicImage::ImageRgba8(ImageBuffer::from_raw(width, height, pixels).unwrap())
    }

    fn variants(image: &DynamicImage) -> Vec<DynamicImage> {
        let mut rgb16 = image.to_rgb16();
        // Values that are not multiples of 257 reach the u16 rounding.
        for (i, sample) in rgb16.iter_mut().enumerate() {
            *sample = sample.wrapping_add(i as u16 % 251);
        }
        vec![
            DynamicImage::ImageLuma8(image.to_luma8()),
            DynamicImage::ImageLumaA8(image.to_luma_alpha8()),
            DynamicImage::ImageRgb8(image.to_rgb8()),
            image.clone(),
            DynamicImage::ImageLuma16(image.to_luma16()),
            DynamicImage::ImageLumaA16(image.to_luma_alpha16()),
            DynamicImage::ImageRgb16(rgb16),
            DynamicImage::ImageRgba16(image.to_rgba16()),
            DynamicImage::ImageRgb32F(image.to_rgb32f()),
            DynamicImage::ImageRgba32F(image.to_rgba32f()),
        ]
    }

    fn assert_identical(image: &DynamicImage, edge: u32) {
        let expected = image.resize(edge, edge, FilterType::Triangle);
        let actual = fit_triangle(image, edge);
        let label = format!("{:?} {:?} to {edge}", image.color(), image.dimensions());
        assert_eq!(actual.color(), expected.color(), "{label}");
        assert_eq!(actual.dimensions(), expected.dimensions(), "{label}");
        let differs = (actual.as_bytes().iter())
            .zip(expected.as_bytes())
            .position(|(a, b)| a != b);
        assert_eq!(differs, None, "{label}: first differing byte");
        assert_eq!(actual.color_space(), expected.color_space(), "{label}");
    }

    #[test]
    fn matches_image_triangle_resize_bit_for_bit() {
        let shapes = [
            (400, 225, 205), // a phone photo's aspect at about 1/10 scale
            (225, 400, 205),
            (301, 199, 128),
            (199, 301, 64),
            (97, 61, 60),   // barely shrinks
            (1000, 3, 100), // one output row
            (3, 1000, 100), // one output column
            (257, 129, 1),
            (40, 20, 60), // upscale
            (1000, 0, 100),
            (0, 1000, 100),
        ];
        for (seed, &(width, height, edge)) in shapes.iter().enumerate() {
            for image in variants(&source(width, height, seed as u32)) {
                assert_identical(&image, edge);
            }
        }
    }

    #[test]
    fn keeps_the_source_color_space() {
        let mut image = source(120, 80, 9);
        image
            .set_color_space(image::metadata::Cicp::DISPLAY_P3)
            .unwrap();
        assert_identical(&image, 50);
    }

    #[test]
    fn same_size_is_a_copy() {
        let image = source(64, 64, 3);
        assert_eq!(fit_triangle(&image, 64).as_bytes(), image.as_bytes());
    }
}
