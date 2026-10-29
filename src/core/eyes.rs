//! Closed eyes judged per person, for Easy Delete's Eyes closed group and
//! Pick Best. The eye verdict itself is `faces::eye_state`; this module
//! matches the people of two shots of one moment and says whether one shot
//! is clearly better for their eyes, and (`compare`) ranks a whole burst of
//! similar photos by who blinks in which frame.
use super::faces::{self, EyeState, Face, FaceEyes};
use super::model::MediaId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Two shots count as one moment only this close in time.
pub const MOMENT_MS: i64 = 10_000;

/// One person in a photo.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Person {
    /// The face, normalized `[left, top, right, bottom]` in the shown photo.
    pub bbox: [f32; 4],
    /// The strip around both eyes, in the same coordinates.
    pub eyes: Option<[f32; 4]>,
    pub state: EyeState,
}

/// Part of the eye cache's identity; change it whenever judging changes.
pub const RECIPE: &str = "eyes-v1-display";

impl Person {
    /// Finite coordinates inside the photo.
    pub fn valid(&self) -> bool {
        self.bbox
            .iter()
            .chain(self.eyes.iter().flatten())
            .all(|v| v.is_finite())
    }
}

/// Every person in `faces`, found in a `width` x `height` analysis image.
pub fn people(faces: &[Face], width: u32, height: u32) -> Vec<Person> {
    judge_people(faces, width, height, |_| None)
}

/// Every person in `faces`, found in a `width` x `height` analysis image.
/// A face too small to judge there (`faces::rejudgeable`) is judged on what
/// `rejudge` returns instead: the face measured again on a larger image, and
/// that image's size. In a group photo people often stand far enough away
/// that their eyes are 10 to 20 pixels apart on the analysis image.
pub fn judge_people(
    faces: &[Face],
    width: u32,
    height: u32,
    mut rejudge: impl FnMut(&Face) -> Option<(Face, u32, u32)>,
) -> Vec<Person> {
    faces
        .iter()
        .map(|face| {
            let again = faces::rejudgeable(face, width, height)
                .then(|| rejudge(face))
                .flatten();
            let (face, w, h) = again
                .as_ref()
                .map_or((face, width, height), |(f, w, h)| (f, *w, *h));
            Person {
                bbox: face.bbox,
                eyes: faces::eye_rect(face, w, h),
                state: faces::eye_state(face, w, h),
            }
        })
        .collect()
}

/// Ranks a state for "at least as good": closed, then unknown, then open.
fn rank(state: EyeState) -> u8 {
    match state {
        EyeState::Closed => 0,
        EyeState::Unknown => 1,
        EyeState::Open => 2,
    }
}

/// Pairs each face of `a` with the face of `b` at the same place, mutual
/// nearest by centre distance relative to face size and of a similar size.
/// `None` unless both shots have the same number of faces and every face
/// pairs: a person who left, arrived or was not found makes the shots
/// different, never comparable.
pub fn match_people(a: &[Person], b: &[Person]) -> Option<Vec<(usize, usize)>> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let centre = |p: &Person| [(p.bbox[0] + p.bbox[2]) / 2.0, (p.bbox[1] + p.bbox[3]) / 2.0];
    let size = |p: &Person| (p.bbox[2] - p.bbox[0]).max(p.bbox[3] - p.bbox[1]);
    let distance = |x: &Person, y: &Person| {
        let ([ax, ay], [bx, by]) = (centre(x), centre(y));
        let scale = (size(x) + size(y)) / 2.0;
        let ratio = size(x) / size(y).max(f32::EPSILON);
        if scale > 0.0 && (0.7..=1.0 / 0.7).contains(&ratio) {
            (ax - bx).hypot(ay - by) / scale
        } else {
            f32::INFINITY
        }
    };
    let nearest = |x: &Person, others: &[Person]| {
        others
            .iter()
            .enumerate()
            .map(|(i, y)| (i, distance(x, y)))
            .filter(|(_, d)| d.is_finite())
            .min_by(|p, q| p.1.total_cmp(&q.1))
    };
    let mut pairs = Vec::with_capacity(a.len());
    for (i, x) in a.iter().enumerate() {
        let (j, d) = nearest(x, b)?;
        // Half a face apart at most, and each the other's nearest.
        if d > 0.5 || nearest(&b[j], a).map(|(k, _)| k) != Some(i) {
            return None;
        }
        pairs.push((i, j));
    }
    Some(pairs)
}

/// Whether `better` is clearly better than `closed` for their eyes: the same
/// people, everyone whose eyes are closed in `closed` has them open in
/// `better`, and nobody is worse off (open stays open, and nobody is closed
/// in `better`). Returns the matched people, `closed`'s index first.
///
/// Per person, so in a group where A blinks in one shot and B in the other,
/// neither shot is better. Unknown is never closed: a face too small, turned
/// away, or squinting counts neither for nor against a shot, but a person
/// open in `closed` and unknown in `better` makes `better` no better.
pub fn better_shot(closed: &[Person], better: &[Person]) -> Option<Vec<(usize, usize)>> {
    if !closed.iter().any(|p| p.state == EyeState::Closed)
        || better.iter().any(|p| p.state == EyeState::Closed)
    {
        return None;
    }
    let pairs = match_people(closed, better)?;
    pairs
        .iter()
        .all(|&(i, j)| {
            let (before, after) = (closed[i].state, better[j].state);
            if before == EyeState::Closed {
                after == EyeState::Open
            } else {
                rank(after) >= rank(before)
            }
        })
        .then_some(pairs)
}

/// A photo of a moment: when it was taken and the people in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub id: MediaId,
    /// Capture time with sub-seconds, in milliseconds.
    pub at_ms: i64,
    pub people: Vec<Person>,
}

/// A photo where someone's eyes are closed, and the other photos of its
/// moment (same scene, same camera, within `MOMENT_MS`) that could be the
/// better shot. Easy Delete picks among them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Blink {
    pub frame: Frame,
    pub others: Vec<Frame>,
}

/// What `moments` needs to know about one photo.
pub struct Candidate<'a> {
    pub id: MediaId,
    /// The scene group from similarity grouping.
    pub group: usize,
    pub camera: &'a str,
    pub at_ms: Option<i64>,
    /// Faces found, from the cached face summary. Only photos with some are
    /// looked at: their eyes may be closed, and too small to judge before
    /// a closer look.
    pub faces: usize,
}

/// The photos with closed eyes and the other shots of their moment.
/// `people` judges a photo's people; it is asked only for photos with faces
/// and, once, for each. A photo without a capture time or camera has no
/// moment. `None` when cancelled.
pub fn moments(
    candidates: &[Candidate],
    mut people: impl FnMut(usize) -> Option<Vec<Person>>,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<Blink>> {
    let mut frames: std::collections::HashMap<usize, Option<Frame>> = Default::default();
    let mut frame = |i: usize, at_ms: i64| -> Option<Frame> {
        frames
            .entry(i)
            .or_insert_with(|| {
                Some(Frame {
                    id: candidates[i].id,
                    at_ms,
                    people: people(i)?,
                })
            })
            .clone()
    };
    let mut blinks = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let Some(at_ms) = c.at_ms.filter(|_| c.faces > 0 && !c.camera.is_empty()) else {
            continue;
        };
        let Some(this) = frame(i, at_ms).filter(|f| {
            f.people
                .iter()
                .any(|person| person.state == EyeState::Closed)
        }) else {
            continue;
        };
        let mut others = Vec::new();
        for (j, other) in candidates.iter().enumerate() {
            if cancelled() {
                return None;
            }
            let Some(other_ms) = other.at_ms.filter(|ms| {
                j != i
                    && other.faces > 0
                    && other.group == c.group
                    && other.camera == c.camera
                    && (ms - at_ms).abs() <= MOMENT_MS
            }) else {
                continue;
            };
            if let Some(f) = frame(j, other_ms) {
                others.push(f);
            }
        }
        blinks.push(Blink {
            frame: this,
            others,
        });
    }
    Some(blinks)
}

/// A burst's photos, judged together: a blink matters only when it could
/// have been avoided, when the same person has their eyes open in another
/// photo of the group. Faces are matched by where they are in the frame,
/// which holds for the near-identical frames Pick Best groups.
/// Two faces in different photos are one person when their centres are
/// closer than this fraction of the larger face's width.
const SAME_PERSON: f32 = 0.6;

/// One photo's eyes, as they compare with the rest of its group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Eyes {
    /// People whose eyes are closed here and clearly open in another photo.
    pub closed: usize,
    /// Faces whose eyes are open here.
    pub open: usize,
}

/// Each photo's avoidable blinks. `photos` lists each photo with its faces;
/// a photo without face evidence has an empty list.
pub fn compare(photos: &[(MediaId, &[FaceEyes])]) -> HashMap<MediaId, Eyes> {
    // People: for each, the face it has in each photo (photo slot, face).
    let mut people: Vec<Vec<(usize, FaceEyes)>> = Vec::new();
    for (slot, (_, faces)) in photos.iter().enumerate() {
        // Closest pairs first, so two neighbours never swap identities.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for (f, face) in faces.iter().enumerate() {
            for (p, person) in people.iter().enumerate() {
                let (_, last) = person.last().expect("people have a face");
                let distance =
                    (face.centre[0] - last.centre[0]).hypot(face.centre[1] - last.centre[1]);
                let width = face.size[0].max(last.size[0]);
                if width > 0.0 && distance < SAME_PERSON * width {
                    pairs.push((distance, f, p));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut matched = vec![None; faces.len()];
        let mut taken = vec![false; people.len()];
        for (_, f, p) in pairs {
            if matched[f].is_none() && !taken[p] {
                matched[f] = Some(p);
                taken[p] = true;
            }
        }
        for (f, face) in faces.iter().enumerate() {
            match matched[f] {
                Some(p) => people[p].push((slot, *face)),
                None => people.push(vec![(slot, *face)]),
            }
        }
    }
    let mut eyes: Vec<Eyes> = photos
        .iter()
        .map(|(_, faces)| Eyes {
            closed: 0,
            open: faces.iter().filter(|f| f.state == EyeState::Open).count(),
        })
        .collect();
    for person in &people {
        for (slot, face) in person {
            if face.state != EyeState::Closed {
                continue;
            }
            let open_elsewhere = person
                .iter()
                .any(|(other, f)| other != slot && f.state == EyeState::Open);
            if open_elsewhere {
                eyes[*slot].closed += 1;
            }
        }
    }
    photos.iter().map(|(id, _)| *id).zip(eyes).collect()
}

#[cfg(test)]
mod compare_tests {
    use super::*;
    fn face(x: f32, state: EyeState, ratio: Option<f32>) -> FaceEyes {
        FaceEyes {
            centre: [x, 0.4],
            size: [0.1, 0.12],
            state,
            ratio,
        }
    }
    fn open(x: f32) -> FaceEyes {
        face(x, EyeState::Open, Some(0.3))
    }
    fn closed(x: f32) -> FaceEyes {
        face(x, EyeState::Closed, Some(0.05))
    }
    fn unknown(x: f32) -> FaceEyes {
        face(x, EyeState::Unknown, None)
    }

    #[test]
    fn a_blink_counts_only_against_the_frame_it_happens_in() {
        let a = [open(0.3)];
        let b = [closed(0.31)];
        let eyes = compare(&[(MediaId(1), &a), (MediaId(2), &b)]);
        assert_eq!(eyes[&MediaId(1)], Eyes { closed: 0, open: 1 });
        assert_eq!(eyes[&MediaId(2)], Eyes { closed: 1, open: 0 });
    }

    #[test]
    fn unknown_eyes_are_never_closed_and_never_make_another_frame_closed() {
        // Too small to read, sunglasses read as unknown, turned away (no face).
        let a = [unknown(0.3)];
        let b = [closed(0.3)];
        let c: [FaceEyes; 0] = [];
        let eyes = compare(&[(MediaId(1), &a), (MediaId(2), &b), (MediaId(3), &c)]);
        assert!(eyes.values().all(|e| e.closed == 0), "{eyes:?}");
    }

    #[test]
    fn someone_with_eyes_closed_in_every_frame_is_held_against_none() {
        let a = [closed(0.3), open(0.6)];
        let b = [closed(0.3), open(0.6)];
        let eyes = compare(&[(MediaId(1), &a), (MediaId(2), &b)]);
        assert!(eyes.values().all(|e| e.closed == 0 && e.open == 1));
    }

    #[test]
    fn group_photos_are_judged_per_person() {
        // Left person blinks in the first frame, right person in the second.
        let a = [closed(0.2), open(0.7)];
        let b = [open(0.2), closed(0.7)];
        let c = [open(0.21), open(0.69)];
        let eyes = compare(&[(MediaId(1), &a), (MediaId(2), &b), (MediaId(3), &c)]);
        assert_eq!(eyes[&MediaId(1)].closed, 1);
        assert_eq!(eyes[&MediaId(2)].closed, 1);
        assert_eq!(eyes[&MediaId(3)], Eyes { closed: 0, open: 2 });
        // Another person's open eyes never excuse or condemn this one.
        let d = [closed(0.2)];
        let e = [open(0.7)];
        let eyes = compare(&[(MediaId(4), &d), (MediaId(5), &e)]);
        assert_eq!(eyes[&MediaId(4)].closed, 0);
    }
}
