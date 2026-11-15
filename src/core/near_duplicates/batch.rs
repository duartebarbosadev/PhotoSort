//! Candidate review on a policy worker. Images and faces come from the shared image owner.
use super::{Assessment, Comparator, Gray, Reason, Side, formats};
use crate::core::{
    faces::Face,
    model::{Media, MediaId},
    pipeline::Asset,
    subjects::store::Store,
};
use anyhow::Result;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    rc::Rc,
};

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
    /// Capture facts for pairing the JPEG and HEIC of one shot.
    pub shot: Option<formats::Shot>,
    /// For finding a better shot of the same moment: the camera, the
    /// capture time with sub-seconds, and how many faces were found.
    pub camera: String,
    pub captured_ms: Option<i64>,
    pub faces: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Byte-for-byte identical files.
    Exact,
    /// Two frames with no visible subject change.
    Near,
    /// One capture saved as a JPEG (`discard`) and a HEIC (`keep`). Neither is
    /// worse: which to keep is the user's choice.
    Formats,
}
/// Why `keep` was suggested over `discard`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keeper {
    Sharper,
    MoreMetadata,
    LargerFile,
    /// Nothing tells them apart, or (for formats) nothing is suggested.
    First,
}
#[derive(Clone, Debug)]
pub struct Pair {
    pub discard: MediaId,
    pub keep: MediaId,
    pub kind: Kind,
    pub keeper: Keeper,
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
    let (keeper, reason) = if k.0 > d.0 {
        (Keeper::Sharper, "the sharper photo")
    } else if k.1 > d.1 {
        (Keeper::MoreMetadata, "the copy with more metadata fields")
    } else if k.2 > d.2 {
        (Keeper::LargerFile, "the larger file")
    } else {
        (Keeper::First, "the first photo in library order")
    };
    Pair {
        discard: discard.media.id,
        keep: keep.media.id,
        keeper,
        kind: if assessment.decision == super::Decision::ExactDuplicate {
            Kind::Exact
        } else {
            Kind::Near
        },
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
/// near candidates use the shared general similarity groups and a .97 cosine
/// gate. Never use Cull's same-subject groups as deletion evidence.
pub fn review(
    inputs: &[Input],
    store: &Store,
    evidence: &mut impl Evidence,
    cancelled: impl Fn() -> bool,
    progress: impl FnMut(usize, usize),
) -> Result<Option<Vec<Pair>>> {
    let held = crate::core::memory::grouping_allowance() / 4;
    review_holding(
        inputs,
        store,
        evidence,
        cancelled,
        progress,
        usize::try_from(held).unwrap_or(usize::MAX),
    )
}

/// `review` holding at most `held` bytes of the photos under comparison.
fn review_holding(
    inputs: &[Input],
    store: &Store,
    evidence: &mut impl Evidence,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
    held: usize,
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
        for pair_indexes in cohort.as_chunks::<2>().0 {
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
    let Some(same_shots) = format_pairs(inputs, &assigned, evidence, &cancelled)? else {
        return Ok(None);
    };
    for pair in same_shots {
        assigned.extend([pair.0, pair.1]);
        result.push(pair.2);
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
    // A photo meets every other photo of its group. Its pixels are asked for
    // once per group while they fit, and its faces once per review, found or
    // not: a photo whose faces cannot be had now is not asked again per pair.
    let mut planes = Planes::new(held);
    let mut faces = HashMap::<usize, Option<Vec<Face>>>::new();
    for group in groups.values() {
        planes.clear();
        let mut candidates = Vec::new();
        for (at, &a) in group.iter().enumerate() {
            let first = &inputs[a];
            // The first photo's assessments are one cache record, read when
            // a pair first needs it and written once the photo is done.
            let mut known = None;
            let row = (|| -> Result<Option<()>> {
                for &b in &group[at + 1..] {
                    if cancelled() {
                        return Ok(None);
                    }
                    done += 1;
                    if done % 16 == 0 || done == total {
                        progress(done, total);
                    }
                    let second = &inputs[b];
                    let Some(cosine) =
                        cosine(&first.embedding, &second.embedding).filter(|v| *v >= 0.97)
                    else {
                        continue;
                    };
                    let known = known.get_or_insert_with(|| store.duplicates(&first.key));
                    let id = Store::duplicate_id(&first.key, &second.key);
                    let assessment = if let Some(previous) = known.get(&id) {
                        previous.clone()
                    } else {
                        // Like a twin preview in `format_pairs`, a photo that cannot be
                        // read now only skips its pairs instead of failing the review.
                        let (left, right) = match (
                            planes.get(a, first, evidence, &cancelled),
                            planes.get(b, second, evidence, &cancelled),
                        ) {
                            (Ok(left), Ok(right)) => (left, right),
                            (Err(error), _) | (_, Err(error)) => {
                                if cancelled() {
                                    return Ok(None);
                                }
                                tracing::warn!(
                                    first = %first.media.path.display(),
                                    second = %second.media.path.display(),
                                    %error,
                                    "near-duplicate pair skipped"
                                );
                                continue;
                            }
                        };
                        let (Some(left), Some(right)) = (left, right) else {
                            return Ok(None);
                        };
                        let (left, right) = (left.plane(), right.plane());
                        let assessment = comparator.assess(
                            Some(left.view()),
                            Some(right.view()),
                            false,
                            |side| {
                                let index = match side {
                                    Side::First => a,
                                    Side::Second => b,
                                };
                                faces
                                    .entry(index)
                                    .or_insert_with(|| evidence.faces(&inputs[index]))
                                    .clone()
                            },
                            &cancelled,
                        );
                        if cancelled() || assessment.reason == Reason::Cancelled {
                            return Ok(None);
                        }
                        // Missing model/evidence may be transient. Never persist that failure.
                        if assessment.reason != Reason::FaceAnalysisUnavailable {
                            known.insert(id, assessment.clone());
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
                Ok(Some(()))
            })();
            // Also when the review stops here: what was assessed stays assessed.
            if let Some(known) = known {
                let written = store.put_duplicates(known);
                if row.is_ok() {
                    written?;
                }
            }
            if row?.is_none() {
                return Ok(None);
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
/// The JPEG and HEIC of one capture, before any similarity grouping, so the
/// pair is never reviewed as two near-duplicate frames. A preview that cannot
/// be decoded only skips its pair.
fn format_pairs(
    inputs: &[Input],
    assigned: &HashSet<usize>,
    evidence: &mut impl Evidence,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<(usize, usize, Pair)>>> {
    let shots: Vec<_> = inputs
        .iter()
        .enumerate()
        .map(|(i, input)| input.shot.as_ref().filter(|_| !assigned.contains(&i)))
        .collect();
    let mut previews = std::collections::HashMap::new();
    let mut stopped = false;
    let matched = formats::match_shots(&shots, |a, b, any_turn| {
        if stopped || cancelled() {
            stopped = true;
            return None;
        }
        for i in [a, b] {
            // Keep only a small copy: a library can hold thousands of pairs.
            previews.entry(i).or_insert_with(|| {
                evidence.image(&inputs[i]).ok().map(|asset| {
                    let (w, h) = asset.pixels.dimensions();
                    let scale = 96.0 / f64::from(w.max(h).max(96));
                    let size = |v: u32| ((f64::from(v) * scale).round() as u32).max(1);
                    image::imageops::thumbnail(&*asset.pixels, size(w), size(h))
                })
            });
        }
        formats::compare(previews[&a].as_ref()?, previews[&b].as_ref()?, any_turn)
    });
    if stopped || cancelled() {
        return Ok(None);
    }
    Ok(Some(
        matched
            .into_iter()
            .map(|(jpeg, heic, difference)| {
                let pair = Pair {
                    discard: inputs[jpeg].media.id,
                    keep: inputs[heic].media.id,
                    kind: Kind::Formats,
                    keeper: Keeper::First,
                    assessment: Assessment::same_shot(),
                    reason: format!(
                        "The same shot saved as JPEG and HEIC · same camera, capture time and size · pixels differ by {:.1}/255{}",
                        difference.mean,
                        if difference.turns == 0 {
                            ""
                        } else {
                            " · one file was rotated after capture, so they show turned"
                        }
                    ),
                };
                (jpeg, heic, pair)
            })
            .collect(),
    ))
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
/// A photo's luminance as bytes: a quarter of what the comparison works on,
/// so a whole burst can wait in memory for its next pair.
struct Luma {
    pixels: Vec<u8>,
    width: usize,
    height: usize,
}
impl Luma {
    /// Unit-range luminance for the comparison.
    fn plane(&self) -> Plane {
        Plane {
            pixels: self.pixels.iter().map(|v| f32::from(*v) / 255.).collect(),
            width: self.width,
            height: self.height,
        }
    }
}
/// OpenCV's integer RGB-to-gray coefficients. Work at the shared canonical
/// preview size; resolution differences remain documented.
fn luma(asset: &Asset, cancelled: &impl Fn() -> bool) -> Option<Luma> {
    let (width, height) = asset.pixels.dimensions();
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for row in asset.pixels.rows() {
        if cancelled() {
            return None;
        }
        pixels.extend(row.map(|p| crate::core::pixels::luminance(p[0], p[1], p[2])));
    }
    Some(Luma {
        pixels,
        width: width as usize,
        height: height as usize,
    })
}
/// The photos of the group under review, each asked of the image owner once
/// while they fit in `budget` bytes; beyond that the ones unused the longest
/// are let go and asked for again.
struct Planes {
    held: HashMap<usize, (Rc<Luma>, u64)>,
    bytes: usize,
    budget: usize,
    tick: u64,
}
impl Planes {
    fn new(budget: usize) -> Self {
        Self {
            held: HashMap::new(),
            bytes: 0,
            budget,
            tick: 0,
        }
    }
    fn clear(&mut self) {
        self.held.clear();
        self.bytes = 0;
    }
    /// `Ok(None)` when cancelled. A photo that cannot be read is asked for
    /// again by its next pair, as it may be readable then.
    fn get(
        &mut self,
        index: usize,
        input: &Input,
        evidence: &mut impl Evidence,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Option<Rc<Luma>>> {
        self.tick += 1;
        if let Some((luma, used)) = self.held.get_mut(&index) {
            *used = self.tick;
            return Ok(Some(luma.clone()));
        }
        let Some(luma) = luma(&evidence.image(input)?, cancelled) else {
            return Ok(None);
        };
        let luma = Rc::new(luma);
        self.bytes += luma.pixels.len();
        self.held.insert(index, (luma.clone(), self.tick));
        while self.bytes > self.budget {
            let Some(oldest) = self
                .held
                .iter()
                .filter(|(held, _)| **held != index)
                .min_by_key(|(_, (_, used))| *used)
                .map(|(held, _)| *held)
            else {
                break;
            };
            if let Some((dropped, _)) = self.held.remove(&oldest) {
                self.bytes -= dropped.pixels.len();
            }
        }
        Ok(Some(luma))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Fingerprint, MediaKind};
    use std::sync::Arc;

    /// A library on disk: some photos cannot be read, some have no face
    /// evidence, and every request is counted.
    struct Library {
        sizes: Vec<Asset>,
        images: HashMap<MediaId, usize>,
        faces: HashMap<MediaId, usize>,
    }
    impl Library {
        /// One picture at several sizes. Photos of one size are the same
        /// frame; photos of different sizes cannot be aligned, which is
        /// decided at once and keeps the test quick.
        fn new() -> Self {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/near_duplicate/00.png");
            let picture = image::open(path).unwrap().to_rgba8();
            let sizes = (0..6)
                .map(|n| {
                    let pixels = image::imageops::thumbnail(&picture, 64 - 4 * n, 48 - 3 * n);
                    let pixels = Arc::new(pixels);
                    Asset {
                        decoder_warning: None,
                        pixels: pixels.clone(),
                        thumbnail: pixels,
                        metadata: Default::default(),
                    }
                })
                .collect();
            Self {
                sizes,
                images: HashMap::new(),
                faces: HashMap::new(),
            }
        }
    }
    impl Evidence for Library {
        fn image(&mut self, input: &Input) -> Result<Asset> {
            anyhow::ensure!(input.media.id.0 % 11 != 5, "Cannot decode");
            *self.images.entry(input.media.id).or_default() += 1;
            // The test keeps each photo's size in its metadata count.
            Ok(self.sizes[input.metadata_fields].clone())
        }
        fn faces(&mut self, input: &Input) -> Option<Vec<Face>> {
            *self.faces.entry(input.media.id).or_default() += 1;
            (input.media.id.0 % 6 != 4).then(Vec::new)
        }
    }

    /// The near pass as it was before photos and faces were held: every pair
    /// asks for both photos and both faces again, and nothing is cached.
    fn pair_by_pair(inputs: &[Input], evidence: &mut impl Evidence) -> Vec<Pair> {
        let mut comparator = Comparator::default();
        let mut groups = BTreeMap::<usize, Vec<usize>>::new();
        for (i, input) in inputs.iter().enumerate() {
            groups.entry(input.group).or_default().push(i);
        }
        let mut assigned = HashSet::new();
        let mut result = Vec::new();
        for group in groups.values() {
            let mut candidates = Vec::new();
            for (at, &a) in group.iter().enumerate() {
                for &b in &group[at + 1..] {
                    let (first, second) = (&inputs[a], &inputs[b]);
                    let Some(cosine) =
                        cosine(&first.embedding, &second.embedding).filter(|v| *v >= 0.97)
                    else {
                        continue;
                    };
                    let (Ok(left), Ok(right)) = (evidence.image(first), evidence.image(second))
                    else {
                        continue;
                    };
                    let left = luma(&left, &|| false).unwrap().plane();
                    let right = luma(&right, &|| false).unwrap().plane();
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
                        || false,
                    );
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
                if !assigned.contains(&a) && !assigned.contains(&b) {
                    assigned.extend([a, b]);
                    result.push(pair(&inputs[a], &inputs[b], assessment));
                }
            }
        }
        result
    }

    #[test]
    fn holding_photos_and_faces_for_the_review_keeps_every_pair_and_asks_once() {
        let summary = |pairs: &[Pair]| -> Vec<_> {
            pairs
                .iter()
                .map(|p| {
                    (
                        p.keep,
                        p.discard,
                        p.keeper,
                        p.assessment.reason,
                        p.assessment.structural_similarity.map(f64::to_bits),
                    )
                })
                .collect()
        };
        let mut seed = 77_u64;
        let mut random = |below: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % below
        };
        // Room for every photo of a group, then for less than one.
        for held in [usize::MAX, 1] {
            let inputs: Vec<_> = (0..11)
                .map(|n| Input {
                    media: Media {
                        id: MediaId(n),
                        path: format!("/{n}.png").into(),
                        kind: MediaKind::Photo,
                        fingerprint: Fingerprint {
                            bytes: 100 + random(3),
                            ..Default::default()
                        },
                    },
                    source: n.to_string(),
                    key: Store::key(&n.to_string(), "model"),
                    hash: format!("hash{n}"),
                    // Three frames of one burst compete for two places.
                    group: if n < 3 { 0 } else { random(2) as usize },
                    // Most pairs pass the similarity gate, some do not.
                    embedding: vec![1., random(28) as f32 / 100.],
                    sharpness: Some(random(4) as f64),
                    metadata_fields: if n < 3 { 0 } else { random(6) as usize },
                    shot: None,
                    camera: String::new(),
                    captured_ms: None,
                    faces: 0,
                })
                .collect();
            let expected = summary(&pair_by_pair(&inputs, &mut Library::new()));
            assert!(!expected.is_empty());
            let root = tempfile::tempdir().unwrap();
            let store = Store::new(root.path().into(), true).unwrap();
            let mut library = Library::new();
            let review = |library: &mut Library| {
                review_holding(&inputs, &store, library, || false, |_, _| {}, held)
                    .unwrap()
                    .unwrap()
            };
            assert_eq!(summary(&review(&mut library)), expected, "holding {held}");
            assert!(library.faces.values().all(|asked| *asked == 1));
            if held == usize::MAX {
                assert!(library.images.values().all(|asked| *asked == 1));
            }
            // One record per first photo. A second review needs pixels only
            // for the pairs whose faces could not be had.
            assert!(std::fs::read_dir(root.path()).unwrap().count() < inputs.len());
            let before: usize = library.images.values().sum();
            assert_eq!(summary(&review(&mut library)), expected);
            let again = library.images.values().sum::<usize>() - before;
            if held == usize::MAX {
                assert!(again <= inputs.len());
            }
        }
    }
}
