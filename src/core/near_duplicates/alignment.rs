//! Phase correlation followed by a two-resolution affine ECC fit.
//! Numerical policy matches the captured test oracles; the solver keeps only
//! image-sized buffers and a 6x6 normal system, not six full Jacobian planes.
use super::{Alignment, Gray, raster};
use nalgebra::{SMatrix, SVector};
use rustfft::{FftPlanner, num_complex::Complex};

pub struct Aligned<'a> {
    pub(crate) first: Gray<'a>,
    pub(crate) second: Vec<f32>,
    pub(crate) valid: Vec<bool>,
    pub(crate) alignment: Alignment,
    pub(crate) overlap: f64,
}
impl Aligned<'_> {
    pub fn alignment(&self) -> Alignment {
        self.alignment
    }
    pub fn overlap(&self) -> f64 {
        self.overlap
    }
    pub fn valid_mask(&self) -> &[bool] {
        &self.valid
    }
    pub fn second_gray(&self) -> Gray<'_> {
        Gray {
            pixels: &self.second,
            width: self.first.width,
            height: self.first.height,
        }
    }
}
/// Own one per comparison worker; FFT plans are reused across pairs.
pub struct Aligner {
    planner: FftPlanner<f32>,
    shape: Option<(usize, usize)>,
}
impl Default for Aligner {
    fn default() -> Self {
        Self {
            planner: FftPlanner::new(),
            shape: None,
        }
    }
}
fn optimal(mut size: usize) -> usize {
    loop {
        let mut n = size;
        for p in [2, 3, 5] {
            while n.is_multiple_of(p) {
                n /= p;
            }
        }
        if n == 1 {
            return size;
        }
        size += 1;
    }
}
impl Aligner {
    fn fft(
        &mut self,
        data: &mut [Complex<f32>],
        width: usize,
        height: usize,
        inverse: bool,
        cancelled: &impl Fn() -> bool,
    ) -> Option<()> {
        let row = if inverse {
            self.planner.plan_fft_inverse(width)
        } else {
            self.planner.plan_fft_forward(width)
        };
        let col = if inverse {
            self.planner.plan_fft_inverse(height)
        } else {
            self.planner.plan_fft_forward(height)
        };
        let mut scratch = vec![
            Complex::default();
            row.get_inplace_scratch_len()
                .max(col.get_inplace_scratch_len())
        ];
        for chunk in data.chunks_exact_mut(width) {
            if cancelled() {
                return None;
            }
            row.process_with_scratch(chunk, &mut scratch);
        }
        let mut column = vec![Complex::default(); height];
        for x in 0..width {
            if cancelled() {
                return None;
            }
            for y in 0..height {
                column[y] = data[y * width + x];
            }
            col.process_with_scratch(&mut column, &mut scratch);
            for y in 0..height {
                data[y * width + x] = column[y];
            }
        }
        Some(())
    }
    fn phase(
        &mut self,
        a: Gray<'_>,
        b: Gray<'_>,
        cancelled: &impl Fn() -> bool,
    ) -> Option<[f64; 2]> {
        let (w, h) = (optimal(a.width), optimal(a.height));
        // Bound retained plans to this source shape; successive burst pairs
        // reuse them without accumulating every aspect ratio in a large library.
        if self.shape != Some((w, h)) {
            self.planner = FftPlanner::new();
            self.shape = Some((w, h));
        }
        let mut first = vec![Complex::default(); w * h];
        let mut second = first.clone();
        for y in 0..a.height {
            if cancelled() {
                return None;
            }
            for x in 0..a.width {
                first[y * w + x].re = a.pixels[y * a.width + x];
                second[y * w + x].re = b.pixels[y * a.width + x];
            }
        }
        self.fft(&mut first, w, h, false, cancelled)?;
        self.fft(&mut second, w, h, false, cancelled)?;
        for (a, b) in first.iter_mut().zip(second) {
            let cross = *a * b.conj();
            *a = cross / (cross.norm() + f32::EPSILON);
        }
        self.fft(&mut first, w, h, true, cancelled)?;
        let value = |x: usize, y: usize| {
            f64::from(first[((y + h.div_ceil(2)) % h) * w + (x + w.div_ceil(2)) % w].re)
        };
        let (mut px, mut py, mut peak) = (0, 0, f64::NEG_INFINITY);
        for y in 0..h {
            if cancelled() {
                return None;
            }
            for x in 0..w {
                let v = value(x, y);
                if v > peak {
                    px = x;
                    py = y;
                    peak = v;
                }
            }
        }
        let (mut sum, mut sx, mut sy) = (0.0, 0.0, 0.0);
        for y in py.saturating_sub(2)..=(py + 2).min(h - 1) {
            for x in px.saturating_sub(2)..=(px + 2).min(w - 1) {
                let v = value(x, y);
                sum += v;
                sx += x as f64 * v;
                sy += y as f64 * v;
            }
        }
        if !sum.is_finite() || sum <= 0.0 {
            return None;
        }
        Some([
            w as f64 / 2.0 - sx / (sum + f64::EPSILON),
            h as f64 / 2.0 - sy / (sum + f64::EPSILON),
        ])
    }
    pub fn prepare<'a>(
        &mut self,
        first: Gray<'a>,
        second: Gray<'_>,
        cancelled: impl Fn() -> bool,
    ) -> Option<Aligned<'a>> {
        if !first.compatible(second, None) || cancelled() {
            return None;
        }
        let scale = (512.0 / first.width.max(first.height) as f64).min(1.0);
        let w = (first.width as f64 * scale).round_ties_even().max(32.0) as usize;
        let h = (first.height as f64 * scale).round_ties_even().max(32.0) as usize;
        let a = raster::resize(first, w, h, &cancelled)?;
        let b = raster::resize(second, w, h, &cancelled)?;
        let [x, y] = self.phase(a.gray(), b.gray(), &cancelled)?;
        let mut transform = [[1.0, 0.0, x], [0.0, 1.0, y]];
        let mut correlation = 0.0;
        for level in [0.5, 1.0] {
            if cancelled() {
                return None;
            }
            let lw = (w as f64 * level).round_ties_even().max(32.0) as usize;
            let lh = (h as f64 * level).round_ties_even().max(32.0) as usize;
            let la = raster::resize(a.gray(), lw, lh, &cancelled)?;
            let lb = raster::resize(b.gray(), lw, lh, &cancelled)?;
            transform[0][2] *= level;
            transform[1][2] *= level;
            correlation = ecc(la.gray(), lb.gray(), &mut transform, &cancelled)?;
            transform[0][2] /= level;
            transform[1][2] /= level;
        }
        transform[0][2] /= scale;
        transform[1][2] /= scale;
        let alignment = Alignment::validate(transform, correlation, first.width, first.height)?;
        let (second, valid) = raster::warp(
            second.pixels,
            first.width,
            first.height,
            transform,
            &cancelled,
        )?;
        let overlap = valid.iter().filter(|v| **v).count() as f64 / valid.len() as f64;
        if overlap < 0.85 || cancelled() {
            return None;
        }
        Some(Aligned {
            first,
            second,
            valid,
            alignment,
            overlap,
        })
    }
}
fn ecc(
    first: Gray<'_>,
    second: Gray<'_>,
    transform: &mut [[f64; 3]; 2],
    cancelled: &impl Fn() -> bool,
) -> Option<f64> {
    let (w, h) = (first.width, first.height);
    let kernel = [0.0625, 0.25, 0.375, 0.25, 0.0625];
    let target = raster::blur(first.pixels, w, h, &kernel, cancelled)?;
    let source = raster::blur(second.pixels, w, h, &kernel, cancelled)?;
    let mut gx = vec![0.0; w * h];
    let mut gy = gx.clone();
    for y in 0..h {
        if cancelled() {
            return None;
        }
        for x in 0..w {
            gx[y * w + x] = (source[y * w + raster::reflect(x as isize + 1, w)]
                - source[y * w + raster::reflect(x as isize - 1, w)])
                * 0.5;
            gy[y * w + x] = (source[raster::reflect(y as isize + 1, h) * w + x]
                - source[raster::reflect(y as isize - 1, h) * w + x])
                * 0.5;
        }
    }
    let mut previous = -1.0_f64;
    let mut correlation = 0.0;
    for _ in 0..100 {
        if cancelled() {
            return None;
        }
        let (warped, valid) = raster::warp(&source, w, h, *transform, cancelled)?;
        let (dx, _) = raster::warp(&gx, w, h, *transform, cancelled)?;
        let (dy, _) = raster::warp(&gy, w, h, *transform, cancelled)?;
        let (mut sa, mut sb, mut count) = (0.0, 0.0, 0);
        for i in 0..valid.len() {
            if valid[i] {
                sa += f64::from(target[i]);
                sb += f64::from(warped[i]);
                count += 1;
            }
        }
        if count < 64 {
            return None;
        }
        let (ma, mb) = (sa / count as f64, sb / count as f64);
        let mut hessian = SMatrix::<f64, 6, 6>::zeros();
        let mut ip = SVector::<f64, 6>::zeros();
        let mut tp = ip;
        let (mut aa, mut bb, mut ab) = (0.0, 0.0, 0.0);
        for y in 0..h {
            if cancelled() {
                return None;
            }
            for x in 0..w {
                let i = y * w + x;
                let a = if valid[i] {
                    f64::from((f64::from(target[i]) - ma) as f32)
                } else {
                    0.0
                };
                let b = if valid[i] {
                    f64::from((f64::from(warped[i]) - mb) as f32)
                } else {
                    f64::from(warped[i])
                };
                let (u, v) = (f64::from(dx[i]), f64::from(dy[i]));
                // Normalize coordinates for a well-conditioned normal system.
                let (nx, ny) = (x as f64 / w as f64, y as f64 / h as f64);
                let jac = [u * nx, v * nx, u * ny, v * ny, u, v];
                for j in 0..6 {
                    ip[j] += jac[j] * b;
                    tp[j] += jac[j] * a;
                    for k in 0..=j {
                        hessian[(j, k)] += jac[j] * jac[k];
                    }
                }
                aa += a * a;
                if valid[i] {
                    bb += b * b;
                }
                ab += a * b;
            }
        }
        correlation = ab / (aa * bb).sqrt();
        if !correlation.is_finite() {
            return None;
        }
        for j in 0..6 {
            for k in 0..j {
                hessian[(k, j)] = hessian[(j, k)];
            }
        }
        let factor = hessian.lu();
        let projected = factor.solve(&ip)?;
        let denominator = ab - tp.dot(&projected);
        if denominator <= 0.0 {
            return None;
        }
        let illumination = (bb - ip.dot(&projected)) / denominator;
        let delta = factor.solve(&(tp * illumination - ip))?;
        if delta.iter().any(|v| !v.is_finite()) {
            return None;
        }
        for (row, col, index, scale) in [
            (0, 0, 0, w as f64),
            (1, 0, 1, w as f64),
            (0, 1, 2, h as f64),
            (1, 1, 3, h as f64),
            (0, 2, 4, 1.0),
            (1, 2, 5, 1.0),
        ] {
            transform[row][col] = f64::from((transform[row][col] + delta[index] / scale) as f32);
        }
        if (correlation - previous).abs() < 1e-6 {
            break;
        }
        previous = correlation;
    }
    Some(correlation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn phase_plans_are_reused_for_bursts_and_released_for_other_dimensions() {
        let pixels: Vec<_> = (0..128 * 96)
            .map(|i| ((i * 73 + i / 128 * 37) % 181 + 32) as f32 / 255.0)
            .collect();
        let gray = Gray::new(&pixels, 128, 96).unwrap();
        let mut aligner = Aligner::default();
        assert!(aligner.phase(gray, gray, &|| false).is_some());
        let retained = aligner.planner.plan_fft_forward(128);
        assert!(aligner.phase(gray, gray, &|| false).is_some());
        assert!(Arc::ptr_eq(
            &retained,
            &aligner.planner.plan_fft_forward(128)
        ));
        let other = Gray::new(&pixels, 96, 128).unwrap();
        assert!(aligner.phase(other, other, &|| false).is_some());
        assert!(!Arc::ptr_eq(
            &retained,
            &aligner.planner.plan_fft_forward(128)
        ));
    }
}
