//! Closed eyes across similar photos, judged per person.
//!
//! A blink matters only when it could have been avoided: when the same
//! person has their eyes open in another photo of the same burst. Faces are
//! matched by where they are in the frame, which holds for the near-identical
//! frames Pick Best groups. A face whose eyes cannot be judged (too small,
//! turned away, no blink scores) is never counted as closed, and someone who
//! squints or laughs in every frame is never held against any of them.
use super::faces::{EyeState, FaceEyes};
use super::model::MediaId;
use std::collections::HashMap;

/// The open counterpart must be clearly open, not merely past the closed
/// rule: at least this eye aspect ratio, and `OPEN_FACTOR` times the closed
/// face's. A laugh that narrows the eyes in one frame and not the next stays
/// close to the other frame's ratio and is not treated as a blink.
const MIN_OPEN_RATIO: f32 = 0.2;
const OPEN_FACTOR: f32 = 1.5;
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
            let (EyeState::Closed, Some(ratio)) = (face.state, face.ratio) else {
                continue;
            };
            let open_elsewhere = person.iter().any(|(other, f)| {
                other != slot
                    && f.state == EyeState::Open
                    && f.ratio
                        .is_some_and(|r| r >= MIN_OPEN_RATIO && r >= ratio * OPEN_FACTOR)
            });
            if open_elsewhere {
                eyes[*slot].closed += 1;
            }
        }
    }
    photos.iter().map(|(id, _)| *id).zip(eyes).collect()
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_laugh_or_squint_is_not_a_blink_against_a_barely_open_frame() {
        // Flagged closed by the blink rule at 0.13, but the other frame's
        // eyes are only a little more open: not clearly a blink.
        let a = [face(0.3, EyeState::Closed, Some(0.13))];
        let b = [face(0.3, EyeState::Open, Some(0.17))];
        let eyes = compare(&[(MediaId(1), &a), (MediaId(2), &b)]);
        assert_eq!(eyes[&MediaId(1)].closed, 0);
        // Against wide-open eyes it is.
        let c = [face(0.3, EyeState::Open, Some(0.3))];
        let eyes = compare(&[(MediaId(1), &a), (MediaId(3), &c)]);
        assert_eq!(eyes[&MediaId(1)].closed, 1);
    }
}
