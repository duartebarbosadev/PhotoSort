//! Shared byte-image measurements. No decoding, model ownership or filesystem IO.
use image::{GrayImage, RgbImage, RgbaImage};

/// OpenCV's integer RGB luminance, independent of alpha and workflow.
pub fn luminance(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 9798 + u32::from(g) * 19235 + u32::from(b) * 3735 + 16384) >> 15) as u8
}
pub fn gray(rgb: &RgbImage, cancelled: &impl Fn() -> bool) -> Option<GrayImage> {
    let mut bytes = Vec::with_capacity(rgb.width() as usize * rgb.height() as usize);
    for row in rgb.rows() {
        if cancelled() {
            return None;
        }
        bytes.extend(row.map(|p| luminance(p[0], p[1], p[2])));
    }
    GrayImage::from_raw(rgb.width(), rgb.height(), bytes)
}
pub fn rgb(rgba: &RgbaImage, cancelled: &impl Fn() -> bool) -> Option<RgbImage> {
    let mut bytes = Vec::with_capacity(rgba.width() as usize * rgba.height() as usize * 3);
    for row in rgba.rows() {
        if cancelled() {
            return None;
        }
        for pixel in row {
            bytes.extend_from_slice(&pixel.0[..3]);
        }
    }
    RgbImage::from_raw(rgba.width(), rgba.height(), bytes)
}
/// BORDER_REFLECT_101, including one-pixel dimensions.
pub fn reflect101(mut i: isize, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let end = n as isize;
    while i < 0 || i >= end {
        i = if i < 0 { -i } else { 2 * end - i - 2 };
    }
    i as usize
}
/// Rectangles are isolated images, as in numpy array_split followed by Laplacian.
/// Accumulate integer stencil responses directly, without a floating image allocation.
pub fn laplacian_variance(
    gray: &GrayImage,
    rect: [u32; 4],
    cancelled: &impl Fn() -> bool,
) -> Option<f64> {
    let [left, top, width, height] = rect;
    if width == 0 || height == 0 {
        return Some(0.);
    }
    assert!(left + width <= gray.width() && top + height <= gray.height());
    let at = |x: usize, y: usize| f64::from(gray.get_pixel(left + x as u32, top + y as u32)[0]);
    let (mut sum, mut squares) = (0., 0.);
    for y in 0..height as usize {
        if cancelled() {
            return None;
        }
        let up = reflect101(y as isize - 1, height as usize);
        let down = reflect101(y as isize + 1, height as usize);
        for x in 0..width as usize {
            let a = reflect101(x as isize - 1, width as usize);
            let b = reflect101(x as isize + 1, width as usize);
            let value = at(a, y) + at(b, y) + at(x, up) + at(x, down) - 4. * at(x, y);
            sum += value;
            squares += value * value;
        }
    }
    let count = f64::from(width) * f64::from(height);
    Some((squares / count - (sum / count).powi(2)).max(0.))
}
fn area_weights(source: u32, target: u32) -> Vec<Vec<(u32, f32)>> {
    let scale = 1. / (f64::from(target) / f64::from(source));
    (0..target)
        .map(|d| {
            let start = f64::from(d) * scale;
            let end = start + scale;
            let cell = scale.min(f64::from(source) - start);
            let right = (end.floor() as u32).min(source - 1);
            let left = (start.ceil() as u32).min(right);
            let mut weights = Vec::new();
            if f64::from(left) - start > 1e-3 {
                weights.push((left - 1, ((f64::from(left) - start) / cell) as f32));
            }
            for x in left..right {
                weights.push((x, (1. / cell) as f32));
            }
            if end - f64::from(right) > 1e-3 {
                weights.push((
                    right,
                    ((end - f64::from(right)).min(1.).min(cell) / cell) as f32,
                ));
            }
            weights
        })
        .collect()
}
/// Downsampling only: area coverage is accumulated as f32 and quantized to RGB8
/// before grayscale conversion. Two scratch rows bound working memory by width.
pub fn resize_area(
    rgb: &RgbImage,
    width: u32,
    height: u32,
    cancelled: &impl Fn() -> bool,
) -> Option<RgbImage> {
    if cancelled() || width == 0 || height == 0 || width > rgb.width() || height > rgb.height() {
        return None;
    }
    if (width, height) == rgb.dimensions() {
        return Some(rgb.clone());
    }
    let (sw, sh) = rgb.dimensions();
    let mut output = RgbImage::new(width, height);
    if sw % width == 0 && sh % height == 0 {
        let (sx, sy) = (sw / width, sh / height);
        let reciprocal = 1. / (sx * sy) as f32;
        for y in 0..height {
            if cancelled() {
                return None;
            }
            for x in 0..width {
                let mut sum = [0u32; 3];
                for yy in y * sy..(y + 1) * sy {
                    for xx in x * sx..(x + 1) * sx {
                        let p = rgb.get_pixel(xx, yy);
                        for c in 0..3 {
                            sum[c] += u32::from(p[c]);
                        }
                    }
                }
                output.put_pixel(
                    x,
                    y,
                    image::Rgb(sum.map(|v| {
                        if sx == 2 && sy == 2 {
                            ((v + 2) >> 2) as u8
                        } else {
                            (v as f32 * reciprocal).round_ties_even().clamp(0., 255.) as u8
                        }
                    })),
                );
            }
        }
        return Some(output);
    }
    let xs = area_weights(sw, width);
    let ys = area_weights(sh, height);
    let mut row = vec![[0f32; 3]; width as usize];
    let mut sum = row.clone();
    for (y, yweights) in ys.iter().enumerate() {
        if cancelled() {
            return None;
        }
        sum.fill([0.; 3]);
        for &(source_y, wy) in yweights {
            row.fill([0.; 3]);
            for (x, xweights) in xs.iter().enumerate() {
                for &(source_x, wx) in xweights {
                    let p = rgb.get_pixel(source_x, source_y);
                    for c in 0..3 {
                        // Native OpenCV contracts this horizontal multiply-add.
                        // Explicit fusion keeps boundary rounding deterministic.
                        row[x][c] = f32::from(p[c]).mul_add(wx, row[x][c]);
                    }
                }
            }
            for (out, row) in sum.iter_mut().zip(&row) {
                for c in 0..3 {
                    out[c] += row[c] * wy;
                }
            }
        }
        for (x, pixel) in sum.iter().enumerate() {
            output.put_pixel(
                x as u32,
                y as u32,
                image::Rgb(pixel.map(|v| v.round_ties_even().clamp(0., 255.) as u8)),
            );
        }
    }
    Some(output)
}
