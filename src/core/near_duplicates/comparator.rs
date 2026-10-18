//! Complete evidence policy, with lazy face access through the shared owner.
use super::{
    Aligner, Alignment, Change, ChangeOptions, FaceMotion, Gray, coherent_change, exposure_correct,
    face_motion, raster, structural_similarity,
};
use crate::core::faces::Face;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    ExactDuplicate,
    SafeNearDuplicate,
    SubjectChanged,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    ExactDuplicate,
    NormalViewEquivalent,
    VisibleSubjectChange,
    FaceChanged,
    FaceAnalysisUnavailable,
    AlignmentUncertain,
    BorderlineSimilarity,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Assessment {
    pub decision: Decision,
    pub reason: Reason,
    pub structural_similarity: Option<f64>,
    pub full_change: Option<Change>,
    pub visible_change: Option<Change>,
    pub alignment: Option<Alignment>,
    pub overlap: Option<f64>,
    pub face_count: Option<[usize; 2]>,
    pub face_motion: Option<FaceMotion>,
    pub face_min_crop_similarity: Option<f64>,
}
impl Assessment {
    fn unknown(reason: Reason) -> Self {
        Self {
            decision: Decision::Uncertain,
            reason,
            structural_similarity: None,
            full_change: None,
            visible_change: None,
            alignment: None,
            overlap: None,
            face_count: None,
            face_motion: None,
            face_min_crop_similarity: None,
        }
    }
    pub fn accepted(&self) -> bool {
        matches!(
            self.decision,
            Decision::ExactDuplicate | Decision::SafeNearDuplicate
        )
    }
}
#[derive(Clone, Copy)]
pub enum Side {
    First,
    Second,
}
#[derive(Default)]
pub struct Comparator {
    aligner: Aligner,
}
impl Comparator {
    /// Byte equality is established by the file-operation owner's full-content
    /// hash. All other pairs require pixels; face loaders run only after the
    /// inexpensive rejection stages and never after cancellation.
    pub fn assess(
        &mut self,
        first: Option<Gray<'_>>,
        second: Option<Gray<'_>>,
        identical: bool,
        mut faces: impl FnMut(Side) -> Option<Vec<Face>>,
        cancelled: impl Fn() -> bool,
    ) -> Assessment {
        let mut result = Assessment::unknown(Reason::AlignmentUncertain);
        if identical {
            result.decision = Decision::ExactDuplicate;
            result.reason = Reason::ExactDuplicate;
            return result;
        }
        if cancelled() {
            result.reason = Reason::Cancelled;
            return result;
        }
        let Some((first, second)) = first.zip(second) else {
            return result;
        };
        let Some(pair) = self.aligner.prepare(first, second, &cancelled) else {
            if cancelled() {
                result.reason = Reason::Cancelled;
            }
            return result;
        };
        result.alignment = Some(pair.alignment);
        result.overlap = Some(pair.overlap);
        let measure = || -> Option<(f64, Option<Change>, Option<Change>)> {
            let corrected =
                exposure_correct(first, pair.second_gray(), Some(&pair.valid), &cancelled)?;
            let normal_first = raster::normal(first, &cancelled)?;
            let normal_second = raster::normal(
                Gray::new(&corrected, first.width, first.height)?,
                &cancelled,
            )?;
            let normal_mask = raster::resize_mask(
                &pair.valid,
                first.width,
                first.height,
                normal_first.width,
                normal_first.height,
                &cancelled,
            )?;
            let preview_first = raster::preview(normal_first.gray(), &cancelled)?;
            let preview_second = raster::preview(normal_second.gray(), &cancelled)?;
            let preview_mask = raster::resize_mask(
                &normal_mask,
                normal_first.width,
                normal_first.height,
                128,
                96,
                &cancelled,
            )?;
            let structural = structural_similarity(
                preview_first.gray(),
                preview_second.gray(),
                Some(&preview_mask),
                &cancelled,
            )?;
            let full = coherent_change(
                first,
                pair.second_gray(),
                Some(&pair.valid),
                ChangeOptions::default(),
                &cancelled,
            );
            let visible = coherent_change(
                normal_first.gray(),
                normal_second.gray(),
                Some(&normal_mask),
                ChangeOptions::normal_view(),
                &cancelled,
            );
            Some((structural, full, visible))
        };
        let Some((structural, full, visible)) = measure() else {
            if cancelled() {
                result.reason = Reason::Cancelled;
            }
            return result;
        };
        result.structural_similarity = Some(structural);
        result.full_change = full;
        result.visible_change = visible;
        if cancelled() {
            result.reason = Reason::Cancelled;
            return result;
        }
        if structural < 0.98 {
            result.decision = Decision::SubjectChanged;
            result.reason = Reason::VisibleSubjectChange;
            return result;
        }
        let Some(visible) = visible else {
            return result;
        };
        let a = faces(Side::First);
        if cancelled() {
            result.reason = Reason::Cancelled;
            return result;
        }
        let b = faces(Side::Second);
        if cancelled() {
            result.reason = Reason::Cancelled;
            return result;
        }
        let Some((a, b)) = a.zip(b) else {
            result.reason = Reason::FaceAnalysisUnavailable;
            return result;
        };
        result.face_count = Some([a.len(), b.len()]);
        let Some(motion) = face_motion(
            &a,
            &b,
            &pair.alignment,
            first.width,
            first.height,
            &cancelled,
        ) else {
            result.reason = if cancelled() {
                Reason::Cancelled
            } else {
                Reason::FaceAnalysisUnavailable
            };
            return result;
        };
        result.face_motion = Some(motion);
        // The face count/overlap veto runs before any crop work.
        if motion.changed && motion.max_median.is_none() {
            result.decision = Decision::SubjectChanged;
            result.reason = Reason::FaceChanged;
            return result;
        }
        let mut minimum = 1.0_f64;
        for face in &a {
            if cancelled() {
                result.reason = Reason::Cancelled;
                return result;
            }
            let Some(score) = crop_similarity(
                first,
                pair.second_gray(),
                &pair.valid,
                face.bbox,
                &cancelled,
            ) else {
                result.reason = if cancelled() {
                    Reason::Cancelled
                } else {
                    Reason::FaceAnalysisUnavailable
                };
                return result;
            };
            minimum = minimum.min(score);
        }
        if !a.is_empty() {
            result.face_min_crop_similarity = Some(minimum);
        }
        if motion.changed || minimum < 0.985 {
            result.decision = Decision::SubjectChanged;
            result.reason = Reason::FaceChanged;
        } else if visible.meaningful_change {
            result.decision = Decision::SubjectChanged;
            result.reason = Reason::VisibleSubjectChange;
        } else if pair.alignment.correlation >= 0.995 && structural >= 0.995 {
            result.decision = Decision::SafeNearDuplicate;
            result.reason = Reason::NormalViewEquivalent;
        } else {
            result.reason = Reason::BorderlineSimilarity;
        }
        result
    }
}
fn crop_similarity(
    first: Gray<'_>,
    second: Gray<'_>,
    mask: &[bool],
    bbox: [f32; 4],
    cancelled: &impl Fn() -> bool,
) -> Option<f64> {
    let [x0, y0, x1, y1] = bbox.map(f64::from);
    let (px, py) = ((x1 - x0) * 0.25, (y1 - y0) * 0.25);
    let left = ((x0 - px) * first.width as f64).floor().max(0.0) as usize;
    let top = ((y0 - py) * first.height as f64).floor().max(0.0) as usize;
    let right = ((x1 + px) * first.width as f64)
        .ceil()
        .min(first.width as f64)
        .max(0.0) as usize;
    let bottom = ((y1 + py) * first.height as f64)
        .ceil()
        .min(first.height as f64)
        .max(0.0) as usize;
    let width = right.checked_sub(left)?;
    let height = bottom.checked_sub(top)?;
    if width < 16 || height < 16 {
        return None;
    }
    let mut a = Vec::with_capacity(width * height);
    let mut b = Vec::with_capacity(width * height);
    let mut valid = Vec::with_capacity(width * height);
    for y in top..bottom {
        if cancelled() {
            return None;
        }
        let span = y * first.width + left..y * first.width + right;
        a.extend_from_slice(&first.pixels[span.clone()]);
        b.extend_from_slice(&second.pixels[span.clone()]);
        valid.extend_from_slice(&mask[span]);
    }
    let a = raster::preview(Gray::new(&a, width, height)?, cancelled)?;
    let b = raster::preview(Gray::new(&b, width, height)?, cancelled)?;
    let valid = raster::resize_mask(&valid, width, height, 128, 96, cancelled)?;
    structural_similarity(a.gray(), b.gray(), Some(&valid), cancelled)
}
