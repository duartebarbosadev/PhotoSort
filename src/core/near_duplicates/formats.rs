//! One shot saved in two formats, as phones do when they write a JPEG and a
//! HEIC of every photo. Two files are the same shot only when the camera, the
//! capture time (to the millisecond when both record it), the size and the
//! pixels all agree. A shared file name is a hint for the order of matching,
//! never evidence.
use crate::core::model::Metadata;
use image::RgbaImage;
use std::{collections::HashMap, path::Path};

/// The largest mean luminance difference, out of 255, between two previews of
/// one shot. Re-encoding the same capture differs by about 1-3; two frames of
/// a burst or an edited copy differ by more.
pub const MAX_DIFFERENCE: f64 = 5.0;
/// The largest difference allowed in any one region. Re-encoding stays near
/// 10; a hand or face that moved between two frames reaches 30 and more even
/// when the mean barely changes.
pub const MAX_LOCAL_DIFFERENCE: f64 = 24.0;
/// Previews are compared on this many cells along their long side.
const CELLS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Jpeg,
    Heif,
}
impl Family {
    pub fn of(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "heic" | "heif" | "hif" => Some(Self::Heif),
            _ => None,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Heif => "HEIC",
        }
    }
}

/// What the camera recorded about a capture, enough to tell two shots apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub family: Family,
    camera: String,
    second: i64,
    subsec_ms: Option<u16>,
    /// The stored size, long side first: orientation may swap it per file.
    size: (u32, u32),
    stem: String,
}
impl Shot {
    pub fn new(path: &Path, metadata: &Metadata) -> Option<Self> {
        let camera = metadata.camera.trim();
        let captured = metadata.captured_ms?;
        if camera.is_empty() || metadata.width == 0 || metadata.height == 0 {
            return None;
        }
        Some(Self {
            family: Family::of(path)?,
            camera: camera.to_owned(),
            second: captured.div_euclid(1000),
            subsec_ms: metadata.captured_subsec_ms,
            size: (
                metadata.width.max(metadata.height),
                metadata.width.min(metadata.height),
            ),
            stem: path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        })
    }
    /// Different formats of one capture, as far as the metadata can tell.
    fn same_capture(&self, other: &Self) -> bool {
        self.family != other.family
            && self.camera == other.camera
            && self.second == other.second
            && self.size == other.size
            && match (self.subsec_ms, other.subsec_ms) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

/// Pairs each JPEG with the HEIC of the same shot. `shots[i]` describes input
/// `i`; `difference(a, b, any_turn)` measures two inputs' pixels, normally
/// with `compare`, and returns `None` when they cannot be compared. A file
/// rotated after capture may be compared at any quarter turn, but only when
/// both files agree on the capture time to the millisecond. Returns
/// (JPEG, HEIC, difference) per pair, each input in at most one pair.
pub fn match_shots(
    shots: &[Option<&Shot>],
    mut difference: impl FnMut(usize, usize, bool) -> Option<Difference>,
) -> Vec<(usize, usize, Difference)> {
    let mut buckets = HashMap::<(&str, i64), Vec<usize>>::new();
    for (i, shot) in shots.iter().enumerate() {
        if let Some(shot) = shot {
            buckets
                .entry((shot.camera.as_str(), shot.second))
                .or_default()
                .push(i);
        }
    }
    let mut keys: Vec<_> = buckets.keys().copied().collect();
    keys.sort();
    let mut candidates = Vec::new();
    for key in keys {
        let members = &buckets[&key];
        for &a in members {
            for &b in members {
                let (first, second) = (shots[a].unwrap(), shots[b].unwrap());
                if first.family != Family::Jpeg || !first.same_capture(second) {
                    continue;
                }
                let any_turn = first.subsec_ms.is_some() && second.subsec_ms.is_some();
                if let Some(d) = difference(a, b, any_turn).filter(Difference::same_shot) {
                    candidates.push((first.stem != second.stem, d, a, b));
                }
            }
        }
    }
    // Matching names first, then the closest pixels: a burst of several
    // shots within one second still pairs each file with its own twin.
    candidates.sort_by(|x, y| {
        x.0.cmp(&y.0)
            .then(x.1.turns.min(1).cmp(&y.1.turns.min(1)))
            .then(x.1.mean.total_cmp(&y.1.mean))
            .then((x.2, x.3).cmp(&(y.2, y.3)))
    });
    let mut used = vec![false; shots.len()];
    let mut pairs = Vec::new();
    for (_, d, a, b) in candidates {
        if !used[a] && !used[b] {
            used[a] = true;
            used[b] = true;
            pairs.push((a, b, d));
        }
    }
    pairs.sort_by_key(|p| (p.0, p.1));
    pairs
}

/// How much two upright previews differ, out of 255, on one small grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Difference {
    pub mean: f64,
    /// The largest difference of any one cell: a local change, such as a
    /// hand that moved, that the mean alone would average away.
    pub largest: f64,
    /// Quarter turns clockwise that line the second up with the first: one
    /// of the files was rotated after capture.
    pub turns: u8,
}
impl Difference {
    pub fn same_shot(&self) -> bool {
        self.mean <= MAX_DIFFERENCE && self.largest <= MAX_LOCAL_DIFFERENCE
    }
}

/// `None` when the previews' shapes differ, which also catches a file whose
/// orientation disagrees with its twin's.
pub fn difference(first: &RgbaImage, second: &RgbaImage) -> Option<Difference> {
    let aspect = |i: &RgbaImage| f64::from(i.width()) / f64::from(i.height().max(1));
    if first.width() == 0 || second.width() == 0 || first.height() == 0 || second.height() == 0 {
        return None;
    }
    if (aspect(first) / aspect(second) - 1.0).abs() > 0.02 {
        return None;
    }
    let (columns, rows) = if first.width() >= first.height() {
        (
            CELLS,
            (CELLS as f64 / aspect(first)).round().max(1.0) as usize,
        )
    } else {
        (
            (CELLS as f64 * aspect(first)).round().max(1.0) as usize,
            CELLS,
        )
    };
    let a = cells(first, columns, rows);
    let b = cells(second, columns, rows);
    let differences: Vec<f64> = a.iter().zip(&b).map(|(a, b)| (a - b).abs()).collect();
    Some(Difference {
        mean: differences.iter().sum::<f64>() / differences.len() as f64,
        largest: differences.iter().copied().fold(0.0, f64::max),
        turns: 0,
    })
}

/// The closest of `difference` upright and, with `any_turn`, with `second`
/// turned by each quarter turn. Prefers upright when it is the same shot.
pub fn compare(first: &RgbaImage, second: &RgbaImage, any_turn: bool) -> Option<Difference> {
    let upright = difference(first, second);
    if !any_turn || upright.is_some_and(|d| d.same_shot()) {
        return upright;
    }
    [
        image::imageops::rotate90(second),
        image::imageops::rotate180(second),
        image::imageops::rotate270(second),
    ]
    .iter()
    .zip(1..)
    .filter_map(|(turned, turns)| difference(first, turned).map(|d| Difference { turns, ..d }))
    .chain(upright)
    .min_by(|a, b| a.mean.total_cmp(&b.mean))
}

/// Average luminance per cell of a `columns` by `rows` grid.
fn cells(image: &RgbaImage, columns: usize, rows: usize) -> Vec<f64> {
    let (width, height) = (image.width() as usize, image.height() as usize);
    let mut sums = vec![0.0; columns * rows];
    let mut counts = vec![0u32; columns * rows];
    for (x, y, p) in image.enumerate_pixels() {
        let cell = (y as usize * rows / height) * columns + x as usize * columns / width;
        sums[cell] += f64::from(crate::core::pixels::luminance(p[0], p[1], p[2]));
        counts[cell] += 1;
    }
    sums.iter()
        .zip(&counts)
        .map(|(sum, count)| sum / f64::from((*count).max(1)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn metadata(camera: &str, ms: i64, subsec: Option<u16>, w: u32, h: u32) -> Metadata {
        Metadata {
            camera: camera.into(),
            captured_ms: Some(ms),
            captured_subsec_ms: subsec,
            width: w,
            height: h,
            ..Default::default()
        }
    }
    fn shot(name: &str, m: &Metadata) -> Shot {
        Shot::new(Path::new(name), m).unwrap()
    }
    /// A soft diagonal gradient with a bright square at `x`, like a frame.
    fn frame(width: u32, height: u32, x: u32, noise: u8) -> RgbaImage {
        RgbaImage::from_fn(width, height, |px, py| {
            let base = ((px + py) * 255 / (width + height)) as u8;
            let v = if (x..x + width / 5).contains(&px) && py < height / 3 {
                250
            } else {
                base.saturating_add(((px * 7 + py * 3) % 2) as u8 * noise)
            };
            Rgba([v, v, v, 255])
        })
    }
    fn pairs(shots: &[Shot], images: &[RgbaImage]) -> Vec<(usize, usize)> {
        let refs: Vec<_> = shots.iter().map(Some).collect();
        match_shots(&refs, |a, b, any_turn| {
            compare(&images[a], &images[b], any_turn)
        })
        .into_iter()
        .map(|(a, b, _)| (a, b))
        .collect()
    }

    #[test]
    fn a_jpeg_and_heic_of_one_shot_pair_even_with_different_names() {
        let m = metadata("Galaxy S24 Ultra", 1_761_360_217_000, Some(631), 4000, 2252);
        let shots = [
            shot("a/20251025_154337.jpeg", &m),
            shot("b/IMG_0001.HEIC", &m),
        ];
        let images = [frame(160, 90, 40, 0), frame(160, 90, 40, 3)];
        assert_eq!(pairs(&shots, &images), vec![(0, 1)]);
        // Re-encoding noise stays well under the limits.
        let d = difference(&images[0], &images[1]).unwrap();
        assert!(d.mean < 2.0 && d.largest < 4.0, "{d:?}");
    }

    #[test]
    fn orientation_swapped_sizes_still_pair_when_the_upright_pixels_match() {
        let jpeg = metadata("Galaxy S24 Ultra", 1_761_533_909_000, None, 2252, 4000);
        let heic = metadata("Galaxy S24 Ultra", 1_761_533_909_000, None, 4000, 2252);
        let shots = [shot("x.jpg", &jpeg), shot("x.heic", &heic)];
        let images = [frame(90, 160, 20, 0), frame(90, 160, 20, 2)];
        assert_eq!(pairs(&shots, &images), vec![(0, 1)]);
        // An upright preview against a sideways one never pairs.
        assert_eq!(
            difference(&frame(90, 160, 20, 0), &frame(160, 90, 20, 0)),
            None
        );
    }

    #[test]
    fn a_shared_name_never_pairs_different_captures() {
        let image = [frame(160, 90, 40, 0), frame(160, 90, 40, 0)];
        let base = metadata("Galaxy S24 Ultra", 1_761_360_217_000, Some(631), 4000, 2252);
        for other in [
            // A second later.
            metadata("Galaxy S24 Ultra", 1_761_360_218_000, Some(631), 4000, 2252),
            // The same second, another sub-second: two shots of a burst.
            metadata("Galaxy S24 Ultra", 1_761_360_217_000, Some(15), 4000, 2252),
            // Another camera.
            metadata("Pixel 9", 1_761_360_217_000, Some(631), 4000, 2252),
            // Another size, such as a resized export.
            metadata("Galaxy S24 Ultra", 1_761_360_217_000, Some(631), 1920, 1080),
        ] {
            let shots = [shot("same.jpeg", &base), shot("same.heic", &other)];
            assert!(pairs(&shots, &image).is_empty(), "{other:?}");
        }
    }

    #[test]
    fn a_shared_name_and_capture_with_different_pixels_does_not_pair() {
        let m = metadata("Galaxy S24 Ultra", 1_761_360_217_000, None, 4000, 2252);
        let shots = [shot("same.jpeg", &m), shot("same.heic", &m)];
        // The subject moved: an edited copy or a second frame.
        let images = [frame(160, 90, 10, 0), frame(160, 90, 110, 0)];
        assert!(!difference(&images[0], &images[1]).unwrap().same_shot());
        assert!(pairs(&shots, &images).is_empty());
        // A small local change: the mean stays low, one region does not.
        let mut moved = images[0].clone();
        for x in 0..12 {
            for y in 60..72 {
                moved.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let d = difference(&images[0], &moved).unwrap();
        assert!(
            d.mean < MAX_DIFFERENCE && d.largest > MAX_LOCAL_DIFFERENCE,
            "{d:?}"
        );
        assert!(pairs(&shots, &[images[0].clone(), moved]).is_empty());
    }

    #[test]
    fn same_format_files_and_photos_without_capture_facts_never_pair() {
        let m = metadata("Galaxy S24 Ultra", 1_761_360_217_000, None, 4000, 2252);
        let images = [frame(160, 90, 40, 0), frame(160, 90, 40, 0)];
        let shots = [shot("a.jpg", &m), shot("b.jpeg", &m)];
        assert!(pairs(&shots, &images).is_empty());
        assert!(Shot::new(Path::new("a.png"), &m).is_none());
        let mut unknown = m.clone();
        unknown.camera.clear();
        assert!(Shot::new(Path::new("a.jpg"), &unknown).is_none());
        unknown = m.clone();
        unknown.captured_ms = None;
        assert!(Shot::new(Path::new("a.heic"), &unknown).is_none());
    }

    #[test]
    fn a_file_rotated_after_capture_pairs_only_when_both_record_the_millisecond() {
        let m = metadata("Galaxy S24 Ultra", 1_761_533_884_000, Some(859), 4000, 2252);
        let shots = [shot("x.jpeg", &m), shot("x.heic", &m)];
        let upright = frame(160, 90, 40, 0);
        let turned = image::imageops::rotate90(&frame(160, 90, 40, 2));
        let images = [upright.clone(), image::imageops::rotate270(&turned)];
        assert_eq!(pairs(&shots, &images), vec![(0, 1)]);
        let images = [upright.clone(), turned.clone()];
        let refs: Vec<_> = shots.iter().map(Some).collect();
        let matched = match_shots(&refs, |a, b, any| compare(&images[a], &images[b], any));
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].2.turns, 3);
        // Without sub-seconds, a turned twin is not trusted.
        let coarse = metadata("Galaxy S24 Ultra", 1_761_533_884_000, None, 4000, 2252);
        let shots = [shot("x.jpeg", &coarse), shot("x.heic", &coarse)];
        assert!(pairs(&shots, &images).is_empty());
    }

    #[test]
    fn a_burst_within_one_second_pairs_each_file_with_its_twin() {
        let m = metadata("Galaxy S24 Ultra", 1_761_360_217_000, None, 4000, 2252);
        let shots = [
            shot("burst_1.jpeg", &m),
            shot("burst_2.jpeg", &m),
            shot("burst_2.heic", &m),
            shot("burst_1.heic", &m),
        ];
        let images = [
            frame(160, 90, 10, 0),
            frame(160, 90, 30, 0),
            frame(160, 90, 30, 2),
            frame(160, 90, 10, 2),
        ];
        assert_eq!(pairs(&shots, &images), vec![(0, 3), (1, 2)]);
        // Without the name hint, the closest pixels still decide.
        let renamed = [
            shot("p.jpeg", &m),
            shot("q.jpeg", &m),
            shot("r.heic", &m),
            shot("s.heic", &m),
        ];
        assert_eq!(pairs(&renamed, &images), vec![(0, 3), (1, 2)]);
    }

    #[test]
    fn pixels_that_cannot_be_compared_never_pair() {
        let m = metadata("Galaxy S24 Ultra", 1_761_360_217_000, None, 4000, 2252);
        let shots = [shot("a.jpg", &m), shot("a.heic", &m)];
        let refs: Vec<_> = shots.iter().map(Some).collect();
        assert!(match_shots(&refs, |_, _, _| None).is_empty());
    }

    /// Pairs the photos in PHOTOSORT_SAMPLE_DIR with the app's own decoder
    /// and prints each pair with its pixel differences, as a reference.
    #[test]
    #[ignore = "read-only diagnostic requiring PHOTOSORT_SAMPLE_DIR"]
    fn sample_format_pairs() {
        use crate::core::model::{Fingerprint, Media, MediaId, MediaKind};
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
        let mut paths: Vec<_> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .filter(|p| Family::of(p).is_some())
            .collect();
        paths.sort();
        let mut pipeline = crate::core::pipeline::Pipeline::standard();
        let mut shots = Vec::new();
        let mut previews = Vec::new();
        for path in &paths {
            let media = Media {
                id: MediaId::for_path(path),
                path: path.clone(),
                kind: MediaKind::from_path(path).unwrap(),
                fingerprint: Fingerprint::read(path).unwrap(),
            };
            let asset = pipeline.analysis(&media, || false).unwrap();
            let (w, h) = asset.pixels.dimensions();
            let scale = 96.0 / f64::from(w.max(h));
            previews.push(image::imageops::thumbnail(
                &*asset.pixels,
                (f64::from(w) * scale).round() as u32,
                (f64::from(h) * scale).round() as u32,
            ));
            shots.push(Shot::new(path, &asset.metadata));
        }
        let refs: Vec<_> = shots.iter().map(Option::as_ref).collect();
        let pairs = match_shots(&refs, |a, b, any_turn| {
            compare(&previews[a], &previews[b], any_turn)
        });
        for (a, b, d) in &pairs {
            println!(
                "{} + {} · {:.2} · max {:.1} · turns {}",
                paths[*a].file_name().unwrap().to_string_lossy(),
                paths[*b].file_name().unwrap().to_string_lossy(),
                d.mean,
                d.largest,
                d.turns
            );
        }
        println!("{} pairs", pairs.len());
        // Same-stem candidates that did not pair, and why.
        for (i, a) in paths.iter().enumerate() {
            for (j, b) in paths.iter().enumerate() {
                if Family::of(a) == Some(Family::Jpeg)
                    && Family::of(b) == Some(Family::Heif)
                    && a.file_stem() == b.file_stem()
                    && !pairs.iter().any(|p| p.0 == i)
                {
                    println!(
                        "unpaired {} · {:?} vs {:?} · {:?} {:?} · {:?}",
                        a.display(),
                        shots[i],
                        shots[j],
                        previews[i].dimensions(),
                        previews[j].dimensions(),
                        difference(&previews[i], &previews[j])
                    );
                }
            }
        }
    }
}
