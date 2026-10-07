//! Candidate review on a policy worker. Images and faces come from the shared image owner.
use super::{Assessment, Comparator, Gray, Reason, Side};
use crate::core::{
    faces::Face,
    model::{Media, MediaId},
    pipeline::Asset,
    subjects::store::Store,
};
use anyhow::Result;
use std::collections::{BTreeMap, HashSet};

#[derive(Clone)]
pub struct Input {
    pub media: Media,
    pub source: String,
    pub key: String,
    pub hash: String,
    pub group: usize,
    pub embedding: Vec<f32>,
    pub sharpness: Option<f64>,
    pub metadata_fields: usize,
}
#[derive(Clone, Debug)]
pub struct Pair {
    pub discard: MediaId,
    pub keep: MediaId,
    pub assessment: Assessment,
    pub reason: String,
}
/// Deliberately cannot initialize a decoder, model, or cache. The application
/// supplies its single image owner's cancellable request/reply adapter.
pub trait Evidence {
    fn image(&mut self, input: &Input) -> Result<Asset>;
    fn faces(&mut self, input: &Input) -> Option<Vec<Face>>;
}
fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let (mut dot, mut aa, mut bb) = (0., 0., 0.);
    for (a, b) in a.iter().zip(b) {
        let (a, b) = (f64::from(*a), f64::from(*b));
        dot += a * b;
        aa += a * a;
        bb += b * b;
    }
    let value = dot / (aa * bb).sqrt();
    value.is_finite().then_some(value.clamp(-1., 1.))
}
fn pair(a: &Input, b: &Input, assessment: Assessment) -> Pair {
    let use_sharpness = a
        .sharpness
        .zip(b.sharpness)
        .is_some_and(|(a, b)| a.is_finite() && b.is_finite());
    let score = |i: &Input| {
        (
            if use_sharpness {
                i.sharpness.unwrap().round_ties_even()
            } else {
                0.
            },
            i.metadata_fields.min(999),
            i.media.fingerprint.bytes.min(999_999_999),
        )
    };
    let (keep, discard) = if score(a) >= score(b) { (a, b) } else { (b, a) };
    let k = score(keep);
    let d = score(discard);
    let reason = if k.0 > d.0 {
        "the sharper photo"
    } else if k.1 > d.1 {
        "the copy with more metadata fields"
    } else if k.2 > d.2 {
        "the larger file"
    } else {
        "the first photo in library order"
    };
    Pair {
        discard: discard.media.id,
        keep: keep.media.id,
        reason: format!(
            "{} · suggest keeping {reason}",
            if assessment.decision == super::Decision::ExactDuplicate {
                "Byte-for-byte identical"
            } else {
                "No visible subject change detected"
            }
        ),
        assessment,
    }
}
/// Exact cohorts are paired in linear space before any image requests. Remaining
/// near candidates use the shared general similarity groups and the reference
/// .97 cosine gate. Never use Cull's same-subject groups as deletion evidence.
pub fn review(
    inputs: &[Input],
    store: &Store,
    evidence: &mut impl Evidence,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<Vec<Pair>>> {
    let mut comparator = Comparator::default();
    let mut exact = BTreeMap::<&str, Vec<usize>>::new();
    for (i, input) in inputs.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        if input.media.fingerprint.bytes > 0 && !input.hash.is_empty() {
            exact.entry(&input.hash).or_default().push(i);
        }
    }
    let mut assigned = HashSet::new();
    let mut result = Vec::new();
    for cohort in exact.values() {
        for pair_indexes in cohort.chunks_exact(2) {
            if cancelled() {
                return Ok(None);
            }
            let [a, b] = [pair_indexes[0], pair_indexes[1]];
            assigned.extend([a, b]);
            result.push(pair(
                &inputs[a],
                &inputs[b],
                comparator.assess(None, None, true, |_| None, &cancelled),
            ));
        }
    }
    let mut groups = BTreeMap::<usize, Vec<usize>>::new();
    for (i, input) in inputs.iter().enumerate() {
        if !assigned.contains(&i) {
            groups.entry(input.group).or_default().push(i);
        }
    }
    let total = groups
        .values()
        .map(|g| g.len().saturating_mul(g.len().saturating_sub(1)) / 2)
        .sum();
    let mut done = 0;
    for group in groups.values() {
        let mut candidates = Vec::new();
        for (at, &a) in group.iter().enumerate() {
            for &b in &group[at + 1..] {
                if cancelled() {
                    return Ok(None);
                }
                done += 1;
                if done % 16 == 0 || done == total {
                    progress(done, total);
                }
                let (first, second) = (&inputs[a], &inputs[b]);
                let Some(cosine) =
                    cosine(&first.embedding, &second.embedding).filter(|v| *v >= 0.97)
                else {
                    continue;
                };
                let key = Store::duplicate_key(&first.key, &second.key);
                let assessment = if let Some(previous) = store.duplicate(&key) {
                    previous
                } else {
                    let left = evidence.image(first)?;
                    let right = evidence.image(second)?;
                    let Some(left) = gray(&left, &cancelled) else {
                        return Ok(None);
                    };
                    let Some(right) = gray(&right, &cancelled) else {
                        return Ok(None);
                    };
                    let assessment = comparator.assess(
                        Some(left.view()),
                        Some(right.view()),
                        false,
                        |side| {
                            evidence.faces(match side {
                                Side::First => first,
                                Side::Second => second,
                            })
                        },
                        &cancelled,
                    );
                    if cancelled() || assessment.reason == Reason::Cancelled {
                        return Ok(None);
                    }
                    // Missing model/evidence may be transient. Never persist that failure.
                    if assessment.reason != Reason::FaceAnalysisUnavailable {
                        store.put_duplicate(&key, &assessment)?;
                    }
                    assessment
                };
                if assessment.accepted() {
                    let distance = (1. - cosine).min(
                        assessment
                            .structural_similarity
                            .map_or(1. - cosine, |s| 1. - s),
                    );
                    candidates.push((distance, a, b, assessment));
                }
            }
        }
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        for (_, a, b, assessment) in candidates {
            if cancelled() {
                return Ok(None);
            }
            if assigned.contains(&a) || assigned.contains(&b) {
                continue;
            }
            assigned.extend([a, b]);
            result.push(pair(&inputs[a], &inputs[b], assessment));
        }
    }
    if cancelled() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}
struct Plane {
    pixels: Vec<f32>,
    width: usize,
    height: usize,
}
impl Plane {
    fn view(&self) -> Gray<'_> {
        Gray::new(&self.pixels, self.width, self.height).unwrap()
    }
}
/// OpenCV's integer RGB-to-gray coefficients, then unit normalization. Work at
/// the shared canonical preview size; resolution differences remain documented.
fn gray(asset: &Asset, cancelled: &impl Fn() -> bool) -> Option<Plane> {
    let (width, height) = asset.pixels.dimensions();
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for row in asset.pixels.rows() {
        if cancelled() {
            return None;
        }
        pixels.extend(
            row.map(|p| f32::from(crate::core::pixels::luminance(p[0], p[1], p[2])) / 255.),
        );
    }
    Some(Plane {
        pixels,
        width: width as usize,
        height: height as usize,
    })
}
