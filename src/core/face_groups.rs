//! People grouping from verified landmarks and native SFace recognition.
//! Detection and decoding belong to the shared image owner; grouping consumes
//! immutable cached evidence only and never allocates a pairwise distance matrix.
use super::{
    faces::{Face, Faces},
    models::{self, Models},
    pipeline::Asset,
    regional,
    subjects::store::Store,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const MODEL: &str = "face_recognition_sface.onnx";
/// Part of the recognition-cache identity; change it whenever the choice of
/// source pixels or alignment below changes.
pub const RECIPE: &str = "sface-detail-v1";
/// Eye distance of the SFace template. Faces smaller than this in the analysis
/// image are recognized from the larger display image when one exists, so
/// distant faces are not upsampled from a few pixels.
const TEMPLATE_EYES: f64 = 35.;
/// Deliberately stricter than the upstream pair-verification threshold. This is
/// a conservative initial grouping policy, not a calibrated probability.
pub const MIN_SIMILARITY: f32 = 0.5;
const SIZE: usize = 112;
const TEMPLATE: [[f64; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recognition {
    /// One normalized descriptor per sufficiently large main face. Empty means
    /// no reliable complete set; never silently omit an unclear main person.
    pub vectors: Vec<Vec<f32>>,
}
impl Recognition {
    pub fn valid(&self) -> bool {
        self.vectors.len() <= 10
            && self.vectors.iter().all(|v| {
                v.len() == 128
                    && v.iter().all(|x| x.is_finite())
                    && (v.iter().map(|x| x * x).sum::<f32>() - 1.).abs() < 0.001
            })
    }
}
/// Retain the established main-subject area threshold, but recognize all main
/// people instead of discarding the whole group portrait. Small background faces
/// do not change the destination of a foreground portrait.
fn prominent(faces: &Faces) -> Vec<&Face> {
    let mut candidates: Vec<_> = faces
        .geometry
        .iter()
        .filter_map(|f| {
            let [l, t, r, b] = f.bbox;
            (f.bbox.iter().all(|x| x.is_finite()) && r > l && b > t)
                .then_some((f, (r - l) * (b - t)))
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.bbox[0].total_cmp(&b.0.bbox[0]))
    });
    let minimum = candidates
        .first()
        .map_or(f32::INFINITY, |(_, area)| area * 0.6);
    candidates
        .into_iter()
        .take_while(|(_, area)| *area >= minimum)
        .map(|(face, _)| face)
        .collect()
}
/// Least-squares orientation-preserving similarity transform, from five source
/// landmarks to the SFace template. Returns inverse coefficients for sampling.
fn transform(points: [[f64; 2]; 5]) -> Option<[f64; 4]> {
    if !points.iter().flatten().all(|x| x.is_finite()) {
        return None;
    }
    let mean = |p: &[[f64; 2]; 5]| {
        [
            p.iter().map(|v| v[0]).sum::<f64>() / 5.,
            p.iter().map(|v| v[1]).sum::<f64>() / 5.,
        ]
    };
    let s = mean(&points);
    let d = mean(&TEMPLATE);
    let mut den = 0.;
    let mut a = 0.;
    let mut b = 0.;
    for (p, q) in points.iter().zip(TEMPLATE) {
        let x = p[0] - s[0];
        let y = p[1] - s[1];
        let u = q[0] - d[0];
        let v = q[1] - d[1];
        den += x * x + y * y;
        a += x * u + y * v;
        b += x * v - y * u;
    }
    if den < 1e-8 {
        return None;
    }
    a /= den;
    b /= den;
    let tx = d[0] - a * s[0] + b * s[1];
    let ty = d[1] - b * s[0] - a * s[1];
    let error = points
        .iter()
        .zip(TEMPLATE)
        .map(|(p, q)| {
            (a * p[0] - b * p[1] + tx - q[0]).powi(2) + (b * p[0] + a * p[1] + ty - q[1]).powi(2)
        })
        .sum::<f64>()
        / 5.;
    let scale = a * a + b * b;
    if scale < 1e-10 || !scale.is_finite() || error > 64. {
        return None;
    }
    Some([a / scale, b / scale, tx, ty])
}
/// RGB float 0..255, NCHW. The model contains its own input normalization.
/// Bilinear sampling uses a black border, with no full-frame intermediate copy.
fn aligned_input(
    asset: &Asset,
    face: &Face,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<f32>>> {
    let lm = &face.landmarks;
    if lm.len() < 363 {
        return Ok(None);
    }
    let midpoint = |a: usize, b: usize| [(lm[a][0] + lm[b][0]) / 2., (lm[a][1] + lm[b][1]) / 2.];
    let points = [
        midpoint(33, 133),
        midpoint(362, 263),
        lm[1],
        lm[61],
        lm[291],
    ];
    if !points
        .iter()
        .flatten()
        .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
    {
        return Ok(None);
    }
    let image = &*asset.pixels;
    let points = points.map(|p| {
        [
            f64::from(p[0]) * f64::from(image.width()),
            f64::from(p[1]) * f64::from(image.height()),
        ]
    });
    // Insufficient eye separation means too few source pixels for recognition.
    if (points[0][0] - points[1][0]).hypot(points[0][1] - points[1][1]) < 12. {
        return Ok(None);
    }
    let Some([a, b, tx, ty]) = transform(points) else {
        return Ok(None);
    };
    let mut input = vec![0.; 3 * SIZE * SIZE];
    for y in 0..SIZE {
        anyhow::ensure!(!cancelled(), "Face recognition cancelled");
        for x in 0..SIZE {
            let u = x as f64 - tx;
            let v = y as f64 - ty;
            let sx = a * u + b * v;
            let sy = -b * u + a * v;
            let left = sx.floor() as i64;
            let top = sy.floor() as i64;
            let fx = (sx - sx.floor()) as f32;
            let fy = (sy - sy.floor()) as f32;
            for (px, py, w) in [
                (left, top, (1. - fx) * (1. - fy)),
                (left + 1, top, fx * (1. - fy)),
                (left, top + 1, (1. - fx) * fy),
                (left + 1, top + 1, fx * fy),
            ] {
                if px >= 0
                    && py >= 0
                    && px < i64::from(image.width())
                    && py < i64::from(image.height())
                {
                    let p = image.get_pixel(px as u32, py as u32);
                    for c in 0..3 {
                        input[c * SIZE * SIZE + y * SIZE + x] += f32::from(p[c]) * w;
                    }
                }
            }
        }
    }
    Ok(Some(input))
}
/// Distance between the eye centres in source pixels.
pub fn eye_distance(asset: &Asset, face: &Face) -> Option<f64> {
    let lm = &face.landmarks;
    if lm.len() < 363 {
        return None;
    }
    let (w, h) = (
        f64::from(asset.pixels.width()),
        f64::from(asset.pixels.height()),
    );
    let midpoint = |a: usize, b: usize| {
        [
            f64::from(lm[a][0] + lm[b][0]) / 2. * w,
            f64::from(lm[a][1] + lm[b][1]) / 2. * h,
        ]
    };
    let ([ax, ay], [bx, by]) = (midpoint(33, 133), midpoint(362, 263));
    Some((ax - bx).hypot(ay - by))
}
/// Like `describe`, but faces too small in `asset` for the SFace template are
/// recognized from `detail`, a larger rendering of the same photo, which is
/// only loaded when needed. Face geometry is normalized, so it applies to both.
pub fn describe_detailed(
    models: &mut Models,
    asset: &Asset,
    faces: &Faces,
    detail: impl FnOnce() -> Result<Asset>,
    cancelled: impl Fn() -> bool,
) -> Result<Recognition> {
    let small = prominent(faces)
        .into_iter()
        .any(|face| eye_distance(asset, face).is_none_or(|eyes| eyes < TEMPLATE_EYES));
    if small {
        let detail = detail()?;
        if detail.pixels.width() > asset.pixels.width() {
            let mut refined = faces.clone();
            for face in &mut refined.geometry {
                anyhow::ensure!(!cancelled(), "Face recognition cancelled");
                match super::faces::refine(models, &detail, face)? {
                    Some(better) => *face = better,
                    None => face.landmarks.clear(),
                }
            }
            return describe(models, &detail, &refined, cancelled);
        }
    }
    describe(models, asset, faces, cancelled)
}
pub fn describe(
    models: &mut Models,
    asset: &Asset,
    faces: &Faces,
    cancelled: impl Fn() -> bool,
) -> Result<Recognition> {
    anyhow::ensure!(!cancelled(), "Face recognition cancelled");
    let people = prominent(faces);
    if people.is_empty() || people.len() > 10 {
        return Ok(Recognition::default());
    }
    // Align the complete set before inference: one unusable main face makes the
    // photo unassigned and avoids spending model work on a partial destination.
    let mut inputs = Vec::with_capacity(people.len());
    for face in people {
        let Some(input) = aligned_input(asset, face, &cancelled)? else {
            return Ok(Recognition::default());
        };
        inputs.push(input);
    }
    let path = models::model_path(MODEL).context("SFace recognition model is not installed")?;
    let mut vectors = Vec::with_capacity(inputs.len());
    for input in inputs {
        anyhow::ensure!(!cancelled(), "Face recognition cancelled");
        let mut outputs = models.infer_all("sface", &path, SIZE, input)?;
        anyhow::ensure!(!cancelled(), "Face recognition cancelled");
        anyhow::ensure!(
            outputs.len() == 1 && outputs[0].len() == 128,
            "SFace must return one 128-dimensional descriptor"
        );
        let mut vector = outputs.remove(0);
        let norm = vector
            .iter()
            .map(|x| f64::from(*x).powi(2))
            .sum::<f64>()
            .sqrt();
        anyhow::ensure!(
            norm.is_finite() && norm > 1e-8,
            "SFace returned an invalid descriptor"
        );
        for x in &mut vector {
            *x = (f64::from(*x) / norm) as f32;
        }
        vectors.push(vector);
    }
    Ok(Recognition { vectors })
}

fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
/// One recognized person while grouping: descriptor indices, their sum, and the
/// sorted photos they appear in.
#[cfg_attr(test, derive(Clone, Debug, PartialEq))]
struct Person {
    members: Vec<usize>,
    sum: Vec<f32>,
    photos: Vec<usize>,
}
impl Person {
    fn absorb(&mut self, other: Person) {
        self.members.extend(other.members);
        for (total, x) in self.sum.iter_mut().zip(other.sum) {
            *total += x;
        }
        self.photos.extend(other.photos);
        self.photos.sort_unstable();
        self.photos.dedup();
    }
    fn seen_with(&self, other: &Person) -> bool {
        let (mut a, mut b) = (
            self.photos.iter().peekable(),
            other.photos.iter().peekable(),
        );
        while let (Some(x), Some(y)) = (a.peek(), b.peek()) {
            match x.cmp(y) {
                std::cmp::Ordering::Less => {
                    a.next();
                }
                std::cmp::Ordering::Greater => {
                    b.next();
                }
                std::cmp::Ordering::Equal => return true,
            }
        }
        false
    }
}
/// How well the faces of two people match on average.
fn average(first: &Person, second: &Person) -> f32 {
    similarity(&first.sum, &second.sum) / (first.members.len() * second.members.len()) as f32
}
/// The person after `a` that `a` would merge with: the best average match of
/// at least `MIN_SIMILARITY` never seen with `a`, the earliest on equal scores.
fn partner(groups: &[Person], a: usize) -> Option<(f32, usize)> {
    let first = &groups[a];
    let mut best = None;
    let mut best_score = MIN_SIMILARITY;
    for (b, second) in groups.iter().enumerate().skip(a + 1) {
        let score = average(first, second);
        if score >= best_score
            && best.is_none_or(|_| score > best_score)
            && !first.seen_with(second)
        {
            best = Some(b);
            best_score = score;
        }
    }
    best.map(|b| (best_score, b))
}
/// Merges the closest pair of people that match on average until none does;
/// `None` when cancelled. Every round merges the pair a scan of all pairs in
/// (a, b) order would pick, the first of the best scores, so the result does
/// not depend on how the pair is found. Each person remembers its best partner
/// among the people after it, and a merge only looks again at the scores it
/// changed: about one comparison per person instead of one per pair, and
/// still no pairwise matrix.
fn merge(mut groups: Vec<Person>, cancelled: impl Fn() -> bool) -> Option<Vec<Person>> {
    let mut partners = Vec::with_capacity(groups.len());
    for a in 0..groups.len() {
        if cancelled() {
            return None;
        }
        partners.push(partner(&groups, a));
    }
    loop {
        if cancelled() {
            return None;
        }
        let mut best: Option<(f32, usize, usize)> = None;
        for (a, found) in partners.iter().enumerate() {
            if let Some((score, b)) = *found
                && best.is_none_or(|(top, ..)| score > top)
            {
                best = Some((score, a, b));
            }
        }
        let Some((_, a, b)) = best else {
            return Some(groups);
        };
        let second = groups.remove(b);
        groups[a].absorb(second);
        partners.remove(b);
        for i in 0..groups.len() {
            // Remembered partners still count positions as before the removal.
            if i == a || partners[i].is_some_and(|(_, p)| p == a || p == b) {
                // Its own faces changed, or its partner changed or is gone.
                if cancelled() {
                    return None;
                }
                partners[i] = partner(&groups, i);
                continue;
            }
            if let Some((_, p)) = &mut partners[i]
                && *p > b
            {
                *p -= 1;
            }
            // Everyone before `a` has a new score with it; its pairs with
            // the people after it were looked at again above.
            if i < a {
                let (first, second) = (&groups[i], &groups[a]);
                let score = average(first, second);
                let better = match partners[i] {
                    Some((top, p)) => score > top || (score == top && a < p),
                    None => score >= MIN_SIMILARITY,
                };
                if better && !first.seen_with(second) {
                    partners[i] = Some((score, a));
                }
            }
        }
    }
}
/// Deterministic average-linkage grouping. A face joins the person whose faces
/// it matches best on average, if that average reaches `MIN_SIMILARITY`; then
/// people whose faces match each other that well on average are merged, unless
/// they appear together in a photo. Averaging keeps one poor face, such as a
/// tiny inset selfie, from splitting a person in two.
/// Unlike subject grouping, this has no geometric pair graph: a running sum of
/// 128-float descriptors per person gives the average without pairwise storage.
/// Exact descriptors collapse immediately. Memory is O(n); diverse-library
/// worst-case CPU remains O(n²).
pub fn group(
    records: &[regional::grouping::Record],
    store: &Store,
    cancelled: impl Fn() -> bool,
) -> Result<Option<regional::grouping::Output>> {
    // Gather exact descriptors once, retaining only compact indices per photo.
    // Photo IDs include paths, so neither identity admission nor numbering may
    // depend on those IDs: a verified rename must not reshuffle people folders.
    let mut vectors = Vec::<Vec<f32>>::new();
    let mut exact = HashMap::new();
    let mut faces_by_photo = Vec::with_capacity(records.len());
    for record in records {
        if cancelled() {
            return Ok(None);
        }
        let Some(key) = &record.key else {
            faces_by_photo.push((record.id, Vec::new()));
            continue;
        };
        let recognition = store
            .recognition(key)
            .context("Face recognition evidence is missing; retry analysis")?;
        let mut indices = Vec::with_capacity(recognition.vectors.len());
        for vector in recognition.vectors {
            if cancelled() {
                return Ok(None);
            }
            let mut hash = blake3::Hasher::new();
            for x in &vector {
                hash.update(&x.to_le_bytes());
            }
            let index = *exact.entry(hash.finalize()).or_insert_with(|| {
                let index = vectors.len();
                vectors.push(vector);
                index
            });
            indices.push(index);
        }
        faces_by_photo.push((record.id, indices));
    }
    drop(exact);
    let mut order: Vec<_> = (0..vectors.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        vectors[b]
            .iter()
            .zip(&vectors[a])
            .map(|(a, b)| a.total_cmp(b))
            .find(|c| !c.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    // Photos each descriptor appears in; people seen together are never merged.
    let mut photos = vec![Vec::new(); vectors.len()];
    for (photo, (_, indices)) in faces_by_photo.iter().enumerate() {
        for &index in indices {
            photos[index].push(photo);
        }
    }
    let mut groups = Vec::<Person>::new();
    for index in order {
        if cancelled() {
            return Ok(None);
        }
        let vector = &vectors[index];
        let face = Person {
            members: vec![index],
            sum: vector.clone(),
            photos: photos[index].clone(),
        };
        let mut best = None;
        let mut best_score = MIN_SIMILARITY;
        for (g, person) in groups.iter().enumerate() {
            let score = similarity(vector, &person.sum) / person.members.len() as f32;
            if score >= best_score
                && best.is_none_or(|_| score > best_score)
                && !person.seen_with(&face)
            {
                best = Some(g);
                best_score = score;
            }
        }
        match best {
            Some(g) => groups[g].absorb(face),
            None => groups.push(face),
        }
    }
    // Admission order can still leave one person in two groups.
    let Some(groups) = merge(groups, &cancelled) else {
        return Ok(None);
    };
    let mut identity = vec![0; vectors.len()];
    for (g, person) in groups.iter().enumerate() {
        for &index in &person.members {
            identity[index] = g + 1;
        }
    }
    let individual_groups = groups.len();
    drop(groups);
    drop(vectors);
    let mut people_by_photo = Vec::with_capacity(records.len());
    let mut combinations = BTreeMap::new();
    for (id, mut people) in faces_by_photo {
        if cancelled() {
            return Ok(None);
        }
        for index in &mut people {
            *index = identity[*index];
        }
        people.sort_unstable();
        // A repeated identity within one frame is ambiguous (lookalikes, collage,
        // or recognition error). Never silently reduce it to a single person.
        if people.windows(2).any(|pair| pair[0] == pair[1]) {
            people.clear();
        }
        if people.len() > 1 && !combinations.contains_key(&people) {
            combinations.insert(people.clone(), 0);
        }
        people_by_photo.push((id, people));
    }
    for (i, group) in combinations.values_mut().enumerate() {
        *group = individual_groups + i + 1;
    }

    let mut output = regional::grouping::Output {
        groups: HashMap::with_capacity(records.len()),
        folders: HashMap::with_capacity(records.len()),
    };
    for (id, people) in people_by_photo {
        if cancelled() {
            return Ok(None);
        }
        let (group, label) = match people.as_slice() {
            [] => (0, "Unassigned".into()),
            [person] => (*person, format!("Person {person:03}")),
            _ => {
                let label = format!(
                    "People {}",
                    people
                        .iter()
                        .map(|person| format!("{person:03}"))
                        .collect::<Vec<_>>()
                        .join(" + ")
                );
                let group = combinations[&people];
                (group, label)
            }
        };
        output.groups.insert(id, group);
        output.folders.insert(id, label);
    }
    Ok((!cancelled()).then_some(output))
}
/// What a people grouping found, for the message shown when it finishes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Photos with at least one recognized main person.
    pub photos: usize,
    /// Distinct people recognized.
    pub people: usize,
    /// People recognized in more than one photo.
    pub repeated: usize,
}
/// Counts people from the folder names `group` produces ("Person 003",
/// "People 001 + 003"); group 0 holds photos without a recognized person,
/// which Organize keeps in place.
pub fn summarize(output: &regional::grouping::Output) -> Summary {
    let mut photos = 0;
    let mut appearances = HashMap::<&str, usize>::new();
    for (id, group) in &output.groups {
        let Some(label) = output.folders.get(id).filter(|_| *group != 0) else {
            continue;
        };
        let Some(people) = label
            .strip_prefix("Person ")
            .or_else(|| label.strip_prefix("People "))
        else {
            continue;
        };
        photos += 1;
        for person in people.split(" + ") {
            *appearances.entry(person).or_default() += 1;
        }
    }
    Summary {
        photos,
        people: appearances.len(),
        repeated: appearances.values().filter(|n| **n > 1).count(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_recovers_rotation_scale_translation_and_rejects_bad_geometry() {
        let points = TEMPLATE.map(|[x, y]| [2. * y + 80., -2. * x + 300.]);
        let [a, b, tx, ty] = transform(points).unwrap();
        for (source, [u, v]) in points.into_iter().zip(TEMPLATE) {
            assert!((a * (u - tx) + b * (v - ty) - source[0]).abs() < 1e-6);
            assert!((-b * (u - tx) + a * (v - ty) - source[1]).abs() < 1e-6);
        }
        assert!(transform([[0.; 2]; 5]).is_none());
        let mut bad = TEMPLATE;
        bad[2][0] = f64::NAN;
        assert!(transform(bad).is_none());
        let mut bad = TEMPLATE;
        bad[2][0] += 60.;
        assert!(transform(bad).is_none());
    }
    #[test]
    fn prominent_faces_include_main_people_but_exclude_small_background_faces() {
        let face = |bbox| Face {
            bbox,
            eyes: [[0.; 2]; 2],
            landmarks: Vec::new(),
            blink: [0.; 2],
        };
        let faces = Faces {
            count: 4,
            geometry: vec![
                face([0., 0., 1., 1.]),
                face([1., 0., 1.8, 1.]),
                face([0., 0., 0.1, 0.1]),
                face([f32::NAN, 0., 1., 1.]),
            ],
            ..Default::default()
        };
        assert_eq!(prominent(&faces).len(), 2);
    }
    #[test]
    fn sampling_preserves_rgb_channels_and_model_scale() {
        let mut landmarks = vec![[0.; 2]; 468];
        for (indexes, p) in [
            (&[33, 133][..], TEMPLATE[0]),
            (&[362, 263][..], TEMPLATE[1]),
            (&[1][..], TEMPLATE[2]),
            (&[61][..], TEMPLATE[3]),
            (&[291][..], TEMPLATE[4]),
        ] {
            for &i in indexes {
                landmarks[i] = p.map(|x| x as f32 / 112.);
            }
        }
        let face = Face {
            bbox: [0., 0., 1., 1.],
            eyes: [[0.; 2]; 2],
            landmarks,
            blink: [0.; 2],
        };
        let pixels = std::sync::Arc::new(image::RgbaImage::from_pixel(
            112,
            112,
            image::Rgba([23, 117, 231, 255]),
        ));
        let asset = Asset {
            decoder_warning: None,
            pixels: pixels.clone(),
            thumbnail: pixels,
            metadata: Default::default(),
        };
        let input = aligned_input(&asset, &face, &|| false).unwrap().unwrap();
        for (c, expected) in [23., 117., 231.].into_iter().enumerate() {
            assert!((input[c * 112 * 112 + 56 * 112 + 56] - expected).abs() < 0.001);
        }
        assert!(aligned_input(&asset, &face, &|| true).is_err());
        let mut models = Models::default();
        let empty = describe(&mut models, &asset, &Faces::default(), || false).unwrap();
        assert!(empty.vectors.is_empty());
        let group = Faces {
            count: 2,
            geometry: vec![face.clone(), face],
            ..Default::default()
        };
        assert_eq!(prominent(&group).len(), 2);
        let mut unclear = group.clone();
        unclear.geometry[1].landmarks.clear();
        assert!(
            describe(&mut models, &asset, &unclear, || false)
                .unwrap()
                .vectors
                .is_empty()
        );
        assert_eq!(models.inferences, 0);
    }
    #[test]
    fn summary_tells_no_faces_from_people_seen_only_once_or_more_often() {
        use crate::core::model::MediaId;
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path().join("store"), false).unwrap();
        let axis = |i: usize| {
            let mut v = vec![0.; 128];
            v[i] = 1.;
            v
        };
        let record = |n: u64, people: &[usize]| {
            let key =
                (!people.is_empty()).then(|| Store::recognition_key(&format!("photo-{n}"), "test"));
            if let Some(key) = &key {
                let vectors = people.iter().map(|&i| axis(i)).collect();
                store
                    .put_recognition(key, &Recognition { vectors })
                    .unwrap();
            }
            regional::grouping::Record {
                id: MediaId(n),
                key,
                portrait: false,
                date: None,
            }
        };
        let summary = |records: &[regional::grouping::Record]| {
            summarize(&group(records, &store, || false).unwrap().unwrap())
        };
        // No recognized face at all: every photo is unassigned.
        assert_eq!(
            summary(&[record(1, &[]), record(2, &[])]),
            Summary::default()
        );
        // Two different people, each in one photo.
        assert_eq!(
            summary(&[record(3, &[0]), record(4, &[1]), record(5, &[])]),
            Summary {
                photos: 2,
                people: 2,
                repeated: 0
            }
        );
        // A person alone and again in a group portrait appears twice, even
        // though each destination folder holds a single photo.
        assert_eq!(
            summary(&[record(6, &[0]), record(7, &[0, 1])]),
            Summary {
                photos: 2,
                people: 2,
                repeated: 1
            }
        );
    }
    /// The merge as first written: every round compares every pair again.
    fn merge_by_full_scan(mut groups: Vec<Person>) -> Vec<Person> {
        loop {
            let mut best = None;
            let mut best_score = MIN_SIMILARITY;
            for (a, first) in groups.iter().enumerate() {
                for (b, second) in groups.iter().enumerate().skip(a + 1) {
                    let score = similarity(&first.sum, &second.sum)
                        / (first.members.len() * second.members.len()) as f32;
                    if score >= best_score
                        && best.is_none_or(|_| score > best_score)
                        && !first.seen_with(second)
                    {
                        best = Some((a, b));
                        best_score = score;
                    }
                }
            }
            let Some((a, b)) = best else {
                return groups;
            };
            let second = groups.remove(b);
            groups[a].absorb(second);
        }
    }
    /// Repeatable pseudo-random numbers in 0..1.
    fn random(seed: u64) -> impl FnMut() -> f32 {
        let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32
        }
    }
    #[test]
    fn merging_picks_the_same_pairs_as_comparing_every_pair_each_round() {
        let (mut merges, mut kept_apart) = (0, 0);
        for seed in 0..600 {
            let mut next = random(seed);
            let count = 2 + (next() * 40.) as usize;
            let mut members = 0..;
            let groups: Vec<Person> = (0..count)
                .map(|_| {
                    let size = 1 + (next() * 3.) as usize;
                    let sum = (0..6)
                        .map(|_| match seed % 3 {
                            // Quarter steps: many pairs score exactly the same,
                            // so the order of equal pairs decides.
                            0 => (next() * 5.).floor() / 4. * size as f32,
                            // Every pair within a few rounding steps of the
                            // threshold: rounding decides who matches.
                            1 => f32::from_bits(
                                (0.28867513 * size as f32).to_bits() - 1 + (next() * 3.) as u32,
                            ),
                            _ => (next() * 1.6 - 0.4) * size as f32,
                        })
                        .collect();
                    let mut photos: Vec<_> = (0..1 + (next() * 3.) as usize)
                        .map(|_| (next() * 2. * count as f32) as usize)
                        .collect();
                    photos.sort_unstable();
                    photos.dedup();
                    Person {
                        members: members.by_ref().take(size).collect(),
                        sum,
                        photos,
                    }
                })
                .collect();
            let expected = merge_by_full_scan(groups.clone());
            assert_eq!(merge(groups.clone(), || false).unwrap(), expected);
            merges += count - expected.len();
            kept_apart += expected
                .iter()
                .enumerate()
                .flat_map(|(a, first)| expected[a + 1..].iter().map(move |second| (first, second)))
                .filter(|(first, second)| {
                    average(first, second) >= MIN_SIMILARITY && first.seen_with(second)
                })
                .count();
            assert!(merge(groups, || true).is_none());
        }
        // The inputs must exercise both chains of merges and people who match
        // but share a photo.
        assert!(merges > 5000 && kept_apart > 1000, "{merges} {kept_apart}");
    }
    /// Not a check: prints how long both merges take on people that mostly do
    /// not match. `cargo test --release --lib merge_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn merge_timing() {
        for (count, twins) in [
            (1000, 100),
            (2000, 100),
            (2000, 200),
            (4000, 100),
            (15000, 500),
        ] {
            let mut next = random(count as u64);
            let mut unit = |near: Option<&Vec<f32>>| {
                let mut v: Vec<f32> = (0..128).map(|_| next() * 2. - 1.).collect();
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                for (i, x) in v.iter_mut().enumerate() {
                    *x = *x / norm * 0.5 + near.map_or(0., |near| near[i]);
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                v.iter().map(|x| x / norm).collect::<Vec<f32>>()
            };
            let mut groups = Vec::<Person>::new();
            for i in 0..count {
                // The last `twins` people are near copies of the first ones.
                let near = (i >= count - twins).then(|| groups[i + twins - count].sum.clone());
                groups.push(Person {
                    members: vec![i],
                    sum: unit(near.as_ref()),
                    photos: vec![i],
                });
            }
            let start = std::time::Instant::now();
            let merged = merge(groups.clone(), || false).unwrap();
            let fast = start.elapsed();
            assert_eq!(merged.len(), count - twins);
            let slow = (count <= 4000).then(|| {
                let start = std::time::Instant::now();
                assert_eq!(merge_by_full_scan(groups), merged);
                start.elapsed()
            });
            println!("{count} people, {twins} merges: {fast:.2?}, full scan {slow:.2?}");
        }
    }
}
