//! Shared resampling for registration, normal-view evidence and face crops.
use super::Gray;

pub(super) struct Plane {
    pub pixels: Vec<f32>,
    pub width: usize,
    pub height: usize,
}
impl Plane {
    pub fn gray(&self) -> Gray<'_> {
        Gray {
            pixels: &self.pixels,
            width: self.width,
            height: self.height,
        }
    }
}
fn weights(source: usize, target: usize) -> Vec<Vec<(usize, f64)>> {
    let scale = source as f64 / target as f64;
    (0..target)
        .map(|i| {
            let start = i as f64 * scale;
            let end = (i + 1) as f64 * scale;
            (start.floor() as usize..(end.ceil() as usize).min(source))
                .map(|j| (j, ((j + 1) as f64).min(end) - (j as f64).max(start)))
                .map(|(j, v)| (j, v / scale))
                .collect()
        })
        .collect()
}
pub(super) fn resize(
    image: Gray<'_>,
    width: usize,
    height: usize,
    cancelled: &impl Fn() -> bool,
) -> Option<Plane> {
    if cancelled() || width == 0 || height == 0 {
        return None;
    }
    if width == image.width && height == image.height {
        return Some(Plane {
            pixels: image.pixels.to_vec(),
            width,
            height,
        });
    }
    let xweights = weights(image.width, width);
    let yweights = weights(image.height, height);
    let mut tmp = vec![0.0; width * image.height];
    let mut pixels = vec![0.0; width * height];
    for y in 0..image.height {
        if cancelled() {
            return None;
        }
        for (x, weights) in xweights.iter().enumerate() {
            tmp[y * width + x] = weights
                .iter()
                .map(|(sx, v)| f64::from(image.pixels[y * image.width + sx]) * v)
                .sum::<f64>() as f32;
        }
    }
    for (y, weights) in yweights.iter().enumerate() {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            pixels[y * width + x] = weights
                .iter()
                .map(|(sy, v)| f64::from(tmp[sy * width + x]) * v)
                .sum::<f64>() as f32;
        }
    }
    Some(Plane {
        pixels,
        width,
        height,
    })
}
pub(super) fn resize_mask(
    mask: &[bool],
    sw: usize,
    sh: usize,
    width: usize,
    height: usize,
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<bool>> {
    let mut out = Vec::with_capacity(width * height);
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            out.push(mask[(y * sh / height) * sw + x * sw / width]);
        }
    }
    Some(out)
}
pub(super) use crate::core::pixels::reflect101 as reflect;
pub(super) fn blur(
    pixels: &[f32],
    width: usize,
    height: usize,
    kernel: &[f64],
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<f32>> {
    let radius = kernel.len() as isize / 2;
    let mut tmp = vec![0.0; pixels.len()];
    let mut out = vec![0.0; pixels.len()];
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            tmp[y * width + x] = kernel
                .iter()
                .enumerate()
                .map(|(k, v)| {
                    f64::from(pixels[y * width + reflect(x as isize + k as isize - radius, width)])
                        * v
                })
                .sum::<f64>() as f32;
        }
    }
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            out[y * width + x] = kernel
                .iter()
                .enumerate()
                .map(|(k, v)| {
                    f64::from(tmp[reflect(y as isize + k as isize - radius, height) * width + x])
                        * v
                })
                .sum::<f64>() as f32;
        }
    }
    Some(out)
}
pub(super) fn gaussian(
    pixels: &[f32],
    width: usize,
    height: usize,
    sigma: f64,
    size: usize,
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<f32>> {
    let radius = size as i32 / 2;
    let mut kernel: Vec<_> = (-radius..=radius)
        .map(|i| (-f64::from(i * i) / (2.0 * sigma * sigma)).exp())
        .collect();
    let sum: f64 = kernel.iter().sum();
    for v in &mut kernel {
        *v /= sum;
    }
    blur(pixels, width, height, &kernel, cancelled)
}

/// Pixel-center inverse mapping with full-precision linear interpolation.
/// Constant zero border and a separate nearest-neighbor support mask mirror the
/// reference distinction between interpolation support and geometric overlap.
pub(super) fn warp(
    pixels: &[f32],
    width: usize,
    height: usize,
    transform: [[f64; 3]; 2],
    cancelled: &impl Fn() -> bool,
) -> Option<(Vec<f32>, Vec<bool>)> {
    let mut out = vec![0.0; pixels.len()];
    let mut valid = vec![false; pixels.len()];
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            let point = |row: usize| {
                (transform[row][0] as f32).mul_add(
                    x as f32,
                    (transform[row][1] * y as f64 + transform[row][2]) as f32,
                )
            };
            let (px, py) = (point(0), point(1));
            if !px.is_finite()
                || !py.is_finite()
                || px.abs() > i32::MAX as f32
                || py.abs() > i32::MAX as f32
            {
                return None;
            }
            let (nx, ny) = (px.round_ties_even() as isize, py.round_ties_even() as isize);
            valid[y * width + x] =
                nx >= 0 && ny >= 0 && nx < width as isize && ny < height as isize;
            let (left, top) = (px.floor() as isize, py.floor() as isize);
            let (fx, fy) = (px - px.floor(), py - py.floor());
            let pixel = |xx: isize, yy: isize| {
                if xx >= 0 && yy >= 0 && xx < width as isize && yy < height as isize {
                    pixels[yy as usize * width + xx as usize]
                } else {
                    0.0
                }
            };
            let a = pixel(left, top);
            let b = pixel(left + 1, top);
            let c = pixel(left, top + 1);
            let d = pixel(left + 1, top + 1);
            let top = (b - a).mul_add(fx, a);
            let bottom = (d - c).mul_add(fx, c);
            out[y * width + x] = (bottom - top).mul_add(fy, top);
        }
    }
    Some((out, valid))
}
pub(super) fn preview(image: Gray<'_>, cancelled: &impl Fn() -> bool) -> Option<Plane> {
    let mut p = resize(image, 128, 96, cancelled)?;
    p.pixels = gaussian(&p.pixels, 128, 96, 2.0, 17, cancelled)?;
    Some(p)
}
pub(super) fn normal(image: Gray<'_>, cancelled: &impl Fn() -> bool) -> Option<Plane> {
    let scale = (384.0 / image.width.max(image.height) as f64).min(1.0);
    let mut p = resize(
        image,
        (image.width as f64 * scale).round_ties_even().max(1.0) as usize,
        (image.height as f64 * scale).round_ties_even().max(1.0) as usize,
        cancelled,
    )?;
    p.pixels = gaussian(&p.pixels, p.width, p.height, 0.6, 7, cancelled)?;
    Some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::path::PathBuf;
    #[derive(Deserialize)]
    struct Probe {
        index: usize,
        value: f32,
        valid: bool,
    }
    #[derive(Deserialize)]
    struct Stages {
        first: String,
        second: String,
        transform: [[f64; 3]; 2],
        width: usize,
        height: usize,
        normal_width: usize,
        normal_height: usize,
        valid_count: usize,
        probes: Vec<Probe>,
    }
    #[test]
    fn reference_resampling_is_checked_separately_from_registration() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/near_duplicate");
        let stages: Stages = serde_json::from_str(include_str!(
            "../../../tests/fixtures/near_duplicate/stages.json"
        ))
        .unwrap();
        let load = |name: &str| {
            image::open(root.join(name))
                .unwrap()
                .to_luma8()
                .into_raw()
                .into_iter()
                .map(|v| f32::from(v) / 255.0)
                .collect::<Vec<_>>()
        };
        let a = load(&stages.first);
        let b = load(&stages.second);
        let normal = normal(Gray::new(&a, stages.width, stages.height).unwrap(), &|| {
            false
        })
        .unwrap();
        assert_eq!(
            (normal.width, normal.height),
            (stages.normal_width, stages.normal_height)
        );
        let preview = preview(normal.gray(), &|| false).unwrap();
        for (name, actual) in [
            ("normal.f32", normal.pixels),
            ("preview.f32", preview.pixels),
        ] {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let expected: Vec<_> = bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            assert_eq!(actual.len(), expected.len());
            let maximum = actual
                .iter()
                .zip(expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(maximum < 2e-6, "{name}: max pixel error {maximum}");
        }
        let (warped, valid) =
            warp(&b, stages.width, stages.height, stages.transform, &|| false).unwrap();
        assert_eq!(valid.iter().filter(|v| **v).count(), stages.valid_count);
        let mask_reference = load("valid_mask.png")
            .into_iter()
            .map(|v| v > 0.5)
            .collect::<Vec<_>>();
        assert!(valid == mask_reference, "full support mask");
        let corrected = crate::core::near_duplicates::exposure_correct(
            Gray::new(&a, stages.width, stages.height).unwrap(),
            Gray::new(&warped, stages.width, stages.height).unwrap(),
            Some(&valid),
            || false,
        )
        .unwrap();
        for (name, actual) in [("warped.f32", &warped), ("corrected.f32", &corrected)] {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let expected: Vec<_> = bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let (index, maximum) = actual
                .iter()
                .zip(&expected)
                .enumerate()
                .map(|(i, (a, b))| (i, (a - b).abs()))
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            assert!(
                maximum < 2e-6,
                "{name}: pixel {index}, max error {maximum}: {} / {}",
                actual[index],
                expected[index]
            );
        }
        let n = super::normal(
            Gray::new(&corrected, stages.width, stages.height).unwrap(),
            &|| false,
        )
        .unwrap();
        let p = super::preview(n.gray(), &|| false).unwrap();
        for (name, actual) in [
            ("normal_second.f32", n.pixels),
            ("preview_second.f32", p.pixels),
        ] {
            let bytes = std::fs::read(root.join(name)).unwrap();
            let expected: Vec<_> = bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let maximum = actual
                .iter()
                .zip(expected)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0_f32, f32::max);
            assert!(maximum < 2e-6, "{name}: max pixel error {maximum}");
        }
        let normal_mask = resize_mask(
            &valid,
            stages.width,
            stages.height,
            stages.normal_width,
            stages.normal_height,
            &|| false,
        )
        .unwrap();
        let preview_mask = resize_mask(
            &normal_mask,
            stages.normal_width,
            stages.normal_height,
            128,
            96,
            &|| false,
        )
        .unwrap();
        for (name, actual) in [
            ("normal_mask.png", normal_mask),
            ("preview_mask.png", preview_mask),
        ] {
            let expected = load(name).into_iter().map(|v| v > 0.5).collect::<Vec<_>>();
            let mismatches = actual
                .iter()
                .zip(expected)
                .filter(|(a, b)| **a != *b)
                .count();
            assert_eq!(mismatches, 0, "{name}");
        }
        for p in stages.probes {
            assert_eq!(valid[p.index], p.valid);
            assert!(
                (warped[p.index] - p.value).abs() < 2e-6,
                "warp {}: {} / {}",
                p.index,
                warped[p.index],
                p.value
            );
        }
    }
}
