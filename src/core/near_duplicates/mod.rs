//! Decode-independent evidence for subject-safe near-duplicate review.
//!
//! This is the comparator's measurement layer. Integration must supply
//! aligned previews and shared face geometry; similarity alone is not acceptance.
pub mod batch;
mod comparator;
pub use comparator::{Assessment, Comparator, Decision, Reason, Side};
mod alignment;
pub use alignment::{Aligned, Aligner};
mod faces;
pub mod formats;
mod raster;
pub use faces::{FaceMotion, face_motion};
mod signals;
pub use signals::{
    Change, ChangeOptions, coherent_change, exposure_correct, structural_similarity,
};

/// A borrowed unit-range, row-major grayscale image. Construction rejects malformed
/// evidence before any measurement, including NaNs hidden by an alignment mask.
#[derive(Clone, Copy)]
pub struct Gray<'a> {
    pub(crate) pixels: &'a [f32],
    pub(crate) width: usize,
    pub(crate) height: usize,
}
impl<'a> Gray<'a> {
    pub fn new(pixels: &'a [f32], width: usize, height: usize) -> Option<Self> {
        if width == 0
            || height == 0
            || width.checked_mul(height)? != pixels.len()
            || pixels
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return None;
        }
        Some(Self {
            pixels,
            width,
            height,
        })
    }
    fn compatible(self, other: Self, mask: Option<&[bool]>) -> bool {
        self.width == other.width
            && self.height == other.height
            && mask.is_none_or(|m| m.len() == self.pixels.len() && m.iter().any(|v| *v))
    }
}

/// Admissible affine motion between two frames, in source-pixel coordinates.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Alignment {
    pub transform: [[f64; 3]; 2],
    pub correlation: f64,
    pub rotation_degrees: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub shear: f64,
}
impl Alignment {
    /// This validates a computed transform, not its pixel support/overlap. The
    /// alignment stage must additionally require at least 85% valid overlap.
    pub fn validate(
        transform: [[f64; 3]; 2],
        correlation: f64,
        width: usize,
        height: usize,
    ) -> Option<Self> {
        if width == 0
            || height == 0
            || !correlation.is_finite()
            || correlation < 0.90
            || transform.iter().flatten().any(|v| !v.is_finite())
            || transform[0][2].abs() > width as f64 * 0.04
            || transform[1][2].abs() > height as f64 * 0.04
        {
            return None;
        }
        let [[a, b, _], [c, d, _]] = transform;
        let scale_x = a.hypot(c);
        let scale_y = b.hypot(d);
        if scale_x.min(scale_y) <= f64::EPSILON {
            return None;
        }
        let rotation_degrees = c.atan2(a).to_degrees();
        let shear = ((a * b + c * d) / (scale_x * scale_y)).abs();
        if rotation_degrees.abs() > 1.5
            || (scale_x - 1.0).abs() > 0.02
            || (scale_y - 1.0).abs() > 0.02
            || shear > 0.01
        {
            return None;
        }
        Some(Self {
            transform,
            correlation,
            rotation_degrees,
            scale_x,
            scale_y,
            shear,
        })
    }
}
