use super::Gray;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Change {
    pub threshold: f64,
    pub largest_component_fraction: f64,
    pub coherent_fraction: f64,
    pub largest_component_p90: f64,
    pub meaningful_change: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct ChangeOptions {
    pub component_min_fraction: f64,
    pub component_min_pixels: usize,
    pub combined_min_fraction: f64,
    pub component_min_thickness: usize,
    pub component_max_elongation: f64,
    pub exposure_correct: bool,
}
impl Default for ChangeOptions {
    fn default() -> Self {
        Self {
            component_min_fraction: 0.0005,
            component_min_pixels: 48,
            combined_min_fraction: 0.0015,
            component_min_thickness: 1,
            component_max_elongation: f64::INFINITY,
            exposure_correct: true,
        }
    }
}
impl ChangeOptions {
    pub fn normal_view() -> Self {
        Self {
            component_min_fraction: 0.0008,
            component_min_pixels: 24,
            combined_min_fraction: 0.0025,
            component_min_thickness: 6,
            component_max_elongation: 4.0,
            exposure_correct: false,
        }
    }
    fn valid(self) -> bool {
        self.component_min_fraction.is_finite()
            && self.component_min_fraction >= 0.0
            && self.combined_min_fraction.is_finite()
            && self.combined_min_fraction >= 0.0
            && self.component_max_elongation >= 1.0
    }
}

/// Linear interpolation of order statistics, matching NumPy's default quantile.
fn quantile(values: &mut [f32], q: f64) -> f64 {
    let rank = (values.len() - 1) as f64 * q;
    let lo = rank.floor() as usize;
    let (_, v, above) = values.select_nth_unstable_by(lo, f32::total_cmp);
    let a = f64::from(*v);
    if rank == lo as f64 {
        return a;
    }
    let b = above.iter().copied().min_by(f32::total_cmp).unwrap();
    a + (f64::from(b) - a) * (rank - lo as f64)
}

/// Fit gain/offset using the interior 2–98% intensity range. Flat or unstable
/// evidence remains unknown, rather than being promoted to a safe duplicate.
pub fn exposure_correct(
    target: Gray<'_>,
    source: Gray<'_>,
    mask: Option<&[bool]>,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<f32>> {
    if !target.compatible(source, mask) || cancelled() {
        return None;
    }
    let mut values: Vec<f32> = source
        .pixels
        .iter()
        .enumerate()
        .filter(|(i, _)| mask.is_none_or(|m| m[*i]))
        .map(|(_, v)| *v)
        .collect();
    if values.len() < 64 {
        return None;
    }
    let low = quantile(&mut values, 0.02);
    let high = quantile(&mut values, 0.98);
    if cancelled() {
        return None;
    }
    let mut count = 0;
    let (mut source_sum, mut target_sum) = (0.0_f64, 0.0_f64);
    let included = |i: usize| {
        mask.is_none_or(|m| m[i])
            && f64::from(source.pixels[i]) > low
            && f64::from(source.pixels[i]) < high
    };
    for (i, (&s, &t)) in source.pixels.iter().zip(target.pixels).enumerate() {
        if i % 4096 == 0 && cancelled() {
            return None;
        }
        if included(i) {
            count += 1;
            source_sum += f64::from(s);
            target_sum += f64::from(t);
        }
    }
    if count < 64 {
        return None;
    }
    let sm = (source_sum / count as f64) as f32;
    let tm = (target_sum / count as f64) as f32;
    let (mut variance, mut covariance) = (0.0_f64, 0.0_f64);
    for (i, (&s, &t)) in source.pixels.iter().zip(target.pixels).enumerate() {
        if i % 4096 == 0 && cancelled() {
            return None;
        }
        if included(i) {
            let x = s - sm;
            variance += f64::from(x * x);
            covariance += f64::from(x * (t - tm));
        }
    }
    let (gain, offset) = if variance <= f64::from(f32::EPSILON) {
        (1.0, tm - sm)
    } else {
        let gain = (covariance / variance) as f32;
        if !gain.is_finite() || !(0.5..=2.0).contains(&gain) {
            return None;
        }
        let offset = f64::from(tm) - f64::from(gain) * f64::from(sm);
        if !offset.is_finite() || offset.abs() > 0.25 {
            return None;
        }
        (gain, offset as f32)
    };
    let mut result = Vec::with_capacity(source.pixels.len());
    for chunk in source.pixels.chunks(4096) {
        if cancelled() {
            return None;
        }
        result.extend(chunk.iter().map(|v| (*v * gain + offset).clamp(0.0, 1.0)));
    }
    Some(result)
}

// Separable rectangular morphology. Outside pixels use OpenCV's neutral border
// (true for erosion, false for dilation); an alignment mask is real image data.
fn morphology(
    mask: &[bool],
    width: usize,
    height: usize,
    radius: usize,
    dilate: bool,
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<bool>> {
    let mut horizontal = vec![false; mask.len()];
    let mut out = vec![false; mask.len()];
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            let span = &mask
                [y * width + x.saturating_sub(radius)..y * width + (x + radius + 1).min(width)];
            horizontal[y * width + x] = if dilate {
                span.iter().any(|v| *v)
            } else {
                span.iter().all(|v| *v)
            };
        }
    }
    for y in 0..height {
        if cancelled() {
            return None;
        }
        for x in 0..width {
            let mut span = (y.saturating_sub(radius)..(y + radius + 1).min(height))
                .map(|yy| horizontal[yy * width + x]);
            out[y * width + x] = if dilate {
                span.any(|v| v)
            } else {
                span.all(|v| v)
            };
        }
    }
    Some(out)
}

/// Robust difference threshold, 3x3 opening, 5x5 closing and eight-connected
/// components. A flood visits each pixel once, avoiding one full image scan per
/// component as in the reference implementation. No decoding/model ownership.
pub fn coherent_change(
    first: Gray<'_>,
    second: Gray<'_>,
    mask: Option<&[bool]>,
    options: ChangeOptions,
    cancelled: impl Fn() -> bool,
) -> Option<Change> {
    if !first.compatible(second, mask) || !options.valid() || cancelled() {
        return None;
    }
    let corrected;
    let second = if options.exposure_correct {
        corrected = exposure_correct(first, second, mask, &cancelled)?;
        corrected.as_slice()
    } else {
        second.pixels
    };
    let mut difference = Vec::with_capacity(first.pixels.len());
    let mut values = Vec::with_capacity(first.pixels.len());
    for (i, (&a, &b)) in first.pixels.iter().zip(second).enumerate() {
        if i % 4096 == 0 && cancelled() {
            return None;
        }
        let d = (a - b).abs();
        difference.push(d);
        if mask.is_none_or(|m| m[i]) {
            values.push(d);
        }
    }
    let area = values.len();
    let median = quantile(&mut values, 0.5) as f32;
    for v in &mut values {
        *v = (*v - median).abs();
    }
    let mad = quantile(&mut values, 0.5) * 1.4826;
    let threshold = (10.0_f64 / 255.0).max(f64::from(median) + 6.0 * mad);
    let mut changed: Vec<_> = difference
        .iter()
        .enumerate()
        .map(|(i, d)| *d >= threshold as f32 && mask.is_none_or(|m| m[i]))
        .collect();
    for (radius, dilate) in [(1, false), (1, true), (2, true), (2, false)] {
        changed = morphology(
            &changed,
            first.width,
            first.height,
            radius,
            dilate,
            &cancelled,
        )?;
    }
    let minimum_area = options
        .component_min_pixels
        .max((area as f64 * options.component_min_fraction).ceil() as usize);
    let (mut total, mut largest, mut largest_p90, mut significant) = (0, 0, 0.0, false);
    // Row-major label order keeps equal-area ties consistent with the reference.
    let mut queue = Vec::new();
    let mut component = Vec::new();
    for start in 0..changed.len() {
        if start % 4096 == 0 && cancelled() {
            return None;
        }
        if !changed[start] {
            continue;
        }
        changed[start] = false;
        queue.clear();
        component.clear();
        queue.push(start);
        let (mut left, mut right, mut top, mut bottom) = (
            start % first.width,
            start % first.width,
            start / first.width,
            start / first.width,
        );
        while let Some(i) = queue.pop() {
            if component.len() % 1024 == 0 && cancelled() {
                return None;
            }
            component.push(difference[i]);
            let (x, y) = (i % first.width, i / first.width);
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            for yy in y.saturating_sub(1)..=(y + 1).min(first.height - 1) {
                for xx in x.saturating_sub(1)..=(x + 1).min(first.width - 1) {
                    let j = yy * first.width + xx;
                    if changed[j] {
                        changed[j] = false;
                        queue.push(j);
                    }
                }
            }
        }
        let (w, h) = (right - left + 1, bottom - top + 1);
        if w.min(h) < options.component_min_thickness
            || w.max(h) as f64 / w.min(h) as f64 > options.component_max_elongation
            || component.len() < minimum_area
        {
            continue;
        }
        let p90 = quantile(&mut component, 0.9);
        total += component.len();
        if component.len() > largest {
            largest = component.len();
            largest_p90 = p90;
        }
        significant |= p90 >= 16.0 / 255.0;
    }
    let coherent_fraction = total as f64 / area as f64;
    Some(Change {
        threshold,
        largest_component_fraction: largest as f64 / area as f64,
        coherent_fraction,
        largest_component_p90: largest_p90,
        meaningful_change: significant || coherent_fraction >= options.combined_min_fraction,
    })
}

fn gaussian(
    pixels: &[f32],
    width: usize,
    height: usize,
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<f32>> {
    super::raster::gaussian(pixels, width, height, 1.5, 11, cancelled)
}
fn partition(index: usize, length: usize) -> usize {
    let (base, extra) = (length / 4, length % 4);
    let larger = (base + 1) * extra;
    if index < larger {
        index / (base + 1)
    } else {
        extra + (index - larger) / base
    }
}

/// Global SSIM and lower-quartile 4x4 tile SSIM, with an 11x11 support mask.
/// Inputs are already aligned unit-range grayscale. Alignment and face evidence
/// are separate requirements; this score alone never declares a safe duplicate.
pub fn structural_similarity(
    first: Gray<'_>,
    second: Gray<'_>,
    mask: Option<&[bool]>,
    cancelled: impl Fn() -> bool,
) -> Option<f64> {
    if !first.compatible(second, mask) || cancelled() {
        return None;
    }
    let (w, h) = (first.width, first.height);
    let valid = match mask {
        Some(m) => morphology(m, w, h, 5, false, &cancelled)?,
        None => vec![true; first.pixels.len()],
    };
    if !valid.iter().any(|v| *v) {
        return None;
    }
    let a: Vec<_> = first.pixels.iter().map(|v| v * 255.0).collect();
    let b: Vec<_> = second.pixels.iter().map(|v| v * 255.0).collect();
    let ma = gaussian(&a, w, h, &cancelled)?;
    let mb = gaussian(&b, w, h, &cancelled)?;
    let aa = gaussian(
        &a.iter().map(|v| v * v).collect::<Vec<_>>(),
        w,
        h,
        &cancelled,
    )?;
    let bb = gaussian(
        &b.iter().map(|v| v * v).collect::<Vec<_>>(),
        w,
        h,
        &cancelled,
    )?;
    let ab = gaussian(
        &a.iter().zip(&b).map(|(a, b)| a * b).collect::<Vec<_>>(),
        w,
        h,
        &cancelled,
    )?;
    let mut total = 0.0_f64;
    let mut count = 0;
    let mut sums = [0.0_f64; 16];
    let mut counts = [0_usize; 16];
    for i in 0..a.len() {
        if i % 4096 == 0 && cancelled() {
            return None;
        }
        if !valid[i] {
            continue;
        }
        let (ma2, mb2, mab) = (ma[i] * ma[i], mb[i] * mb[i], ma[i] * mb[i]);
        let numerator = (2.0 * mab + 6.5025) * (2.0 * (ab[i] - mab) + 58.5225);
        let denominator = (ma2 + mb2 + 6.5025) * (aa[i] - ma2 + bb[i] - mb2 + 58.5225);
        let score = numerator / denominator.max(f32::EPSILON);
        if !score.is_finite() {
            return None;
        }
        let tile = partition(i / w, h) * 4 + partition(i % w, w);
        sums[tile] += f64::from(score);
        counts[tile] += 1;
        total += f64::from(score);
        count += 1;
    }
    let mut tiles: Vec<_> = sums
        .iter()
        .zip(counts)
        .filter(|(_, n)| *n > 0)
        .map(|(s, n)| (s / n as f64) as f32)
        .collect();
    Some(
        (total / count as f64)
            .min(quantile(&mut tiles, 0.25))
            .clamp(0.0, 1.0),
    )
}
