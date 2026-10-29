//! Closed eyes judged per person from synthetic face landmarks.
use photosort::core::{
    eyes::{self, Blink, Candidate, Frame, Person},
    faces::{self, EyeMeasures, EyeState, Face},
    model::MediaId,
};

/// A face in a `width` x `height` image with its eye centres `spacing`
/// pixels apart, each eye open to `ratios` (height over width) and the given
/// blink scores. Only the eye landmarks are placed; the rest sit mid-face.
fn face(width: u32, height: u32, spacing: f32, ratios: [f32; 2], blink: [f32; 2]) -> Face {
    let (w, h) = (width as f32, height as f32);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let eye = spacing * 0.6;
    let mut points = vec![[cx / w, cy / h]; 478];
    // Corner, upper lid, upper lid, corner, lower lid, lower lid.
    for (indices, centre, ratio) in [
        ([33, 160, 158, 133, 153, 144], cx - spacing / 2.0, ratios[0]),
        (
            [362, 385, 387, 263, 373, 380],
            cx + spacing / 2.0,
            ratios[1],
        ),
    ] {
        let left = centre - eye / 2.0;
        let lid = ratio * eye / 2.0;
        let at = |x: f32, y: f32| [x / w, y / h];
        points[indices[0]] = at(left, cy);
        points[indices[1]] = at(left + eye / 3.0, cy - lid);
        points[indices[2]] = at(left + 2.0 * eye / 3.0, cy - lid);
        points[indices[3]] = at(left + eye, cy);
        points[indices[4]] = at(left + 2.0 * eye / 3.0, cy + lid);
        points[indices[5]] = at(left + eye / 3.0, cy + lid);
    }
    let half = spacing;
    Face {
        bbox: [
            (cx - half) / w,
            (cy - half) / h,
            (cx + half) / w,
            (cy + half) / h,
        ],
        eyes: [points[33], points[263]],
        landmarks: points,
        blink: Some(blink),
    }
}

#[test]
fn eyes_are_measured_in_pixels_whatever_the_photo_shape() {
    for (width, height) in [(1024, 1024), (577, 1024), (1024, 576)] {
        let open = face(width, height, 40.0, [0.3, 0.32], [0.05, 0.1]);
        let m = faces::eye_measures(&open, width, height).unwrap();
        assert!((m.spacing - 40.0).abs() < 0.01, "{width}x{height}: {m:?}");
        assert!((m.ratios[0] - 0.3).abs() < 0.001 && (m.ratios[1] - 0.32).abs() < 0.001);
        assert_eq!(faces::eye_state(&open, width, height), EyeState::Open);
        let shut = face(width, height, 40.0, [0.03, 0.04], [0.8, 0.75]);
        assert_eq!(faces::eye_state(&shut, width, height), EyeState::Closed);
        let rect = faces::eye_rect(&shut, width, height).unwrap();
        assert!(rect[0] < rect[2] && rect[1] < rect[3]);
    }
}

#[test]
fn unknown_is_never_closed() {
    let judge = |spacing, ratios, blink| {
        faces::judge(&EyeMeasures {
            spacing,
            ratios,
            blink,
        })
    };
    // Shut, but the face is too small to tell (under 20 px between the eyes).
    assert_eq!(judge(15.0, [0.03, 0.03], [0.9, 0.9]), EyeState::Unknown);
    // A squint or a laugh: narrowed eyes and high blink scores.
    assert_eq!(judge(40.0, [0.13, 0.12], [0.7, 0.68]), EyeState::Unknown);
    assert_eq!(judge(40.0, [0.16, 0.14], [0.6, 0.62]), EyeState::Unknown);
    // A wink, or one eye hidden: one open, one shut.
    assert_eq!(judge(40.0, [0.21, 0.07], [0.68, 0.29]), EyeState::Unknown);
    // Shut by shape, but the blink scores are not sure (sunglasses can
    // flatten the landmarks without a blink).
    assert_eq!(judge(40.0, [0.05, 0.06], [0.45, 0.5]), EyeState::Unknown);
    // Neither clearly open nor shut.
    assert_eq!(judge(40.0, [0.17, 0.2], [0.3, 0.3]), EyeState::Unknown);
    assert_eq!(judge(40.0, [0.25, 0.25], [0.5, 0.2]), EyeState::Unknown);
    // Clear cases.
    assert_eq!(judge(40.0, [0.03, 0.03], [0.8, 0.75]), EyeState::Closed);
    assert_eq!(judge(40.0, [0.25, 0.3], [0.1, 0.2]), EyeState::Open);
    // A legacy face without landmarks or blink scores.
    let mut legacy = face(1024, 1024, 40.0, [0.03, 0.03], [0.9, 0.9]);
    legacy.landmarks.clear();
    assert_eq!(faces::eye_state(&legacy, 1024, 1024), EyeState::Unknown);
    legacy = face(1024, 1024, 40.0, [0.03, 0.03], [0.9, 0.9]);
    legacy.blink = None;
    assert_eq!(faces::eye_state(&legacy, 1024, 1024), EyeState::Unknown);
}

fn person(x: f32, state: EyeState) -> Person {
    Person {
        bbox: [x, 0.4, x + 0.1, 0.5],
        eyes: Some([x + 0.02, 0.43, x + 0.08, 0.45]),
        state,
    }
}
use EyeState::{Closed, Open, Unknown};

#[test]
fn a_better_shot_has_everyone_who_blinked_with_open_eyes() {
    let blink = [person(0.2, Closed)];
    assert_eq!(
        eyes::better_shot(&blink, &[person(0.21, Open)]),
        Some(vec![(0, 0)])
    );
    // Unknown in the other shot (turned away, too small) is not open.
    assert_eq!(eyes::better_shot(&blink, &[person(0.21, Unknown)]), None);
    // Someone else, elsewhere in the frame, is not the same person.
    assert_eq!(eyes::better_shot(&blink, &[person(0.6, Open)]), None);
    // A shot with nobody's eyes closed has nothing to improve.
    assert_eq!(
        eyes::better_shot(&[person(0.2, Open)], &[person(0.2, Open)]),
        None
    );
}

#[test]
fn in_a_group_each_person_is_judged_alone() {
    // A blinks in the first shot, B in the second: neither is better.
    let first = [person(0.1, Closed), person(0.5, Open)];
    let second = [person(0.1, Open), person(0.5, Closed)];
    assert_eq!(eyes::better_shot(&first, &second), None);
    assert_eq!(eyes::better_shot(&second, &first), None);
    // B turned away in the second shot: not better for B.
    let turned = [person(0.1, Open), person(0.5, Unknown)];
    assert_eq!(eyes::better_shot(&first, &turned), None);
    // Everyone open in the third, with the faces listed in another order.
    let third = [person(0.51, Open), person(0.11, Open)];
    assert_eq!(
        eyes::better_shot(&first, &third),
        Some(vec![(0, 1), (1, 0)])
    );
    // A tiny face unknown in both shots neither helps nor hurts.
    let with_child = [person(0.1, Closed), person(0.8, Unknown)];
    let later = [person(0.1, Open), person(0.8, Unknown)];
    assert!(eyes::better_shot(&with_child, &later).is_some());
    // Someone who left the frame makes the shots different.
    assert_eq!(eyes::better_shot(&first, &[person(0.1, Open)]), None);
}

fn shut_face() -> Face {
    face(1000, 1000, 40.0, [0.03, 0.03], [0.8, 0.8])
}
fn open_face() -> Face {
    face(1000, 1000, 40.0, [0.3, 0.3], [0.05, 0.05])
}

#[test]
fn moments_look_only_at_shots_of_the_same_scene_camera_and_seconds() {
    let candidate = |n: u64, group, camera, at_ms, faces| Candidate {
        id: MediaId(n),
        group,
        camera,
        at_ms,
        faces,
    };
    let candidates = [
        candidate(1, 1, "Phone", Some(10_000), 1),
        candidate(2, 1, "Phone", Some(13_000), 1),
        // Another scene, another camera, too late, no time, no faces.
        candidate(3, 2, "Phone", Some(11_000), 1),
        candidate(4, 1, "Camera", Some(11_000), 1),
        candidate(5, 1, "Phone", Some(25_000), 1),
        candidate(6, 1, "Phone", None, 1),
        candidate(7, 1, "Phone", Some(9_000), 1),
        candidate(8, 1, "Phone", Some(12_000), 0),
        // A squint: never a blink.
        candidate(9, 3, "Phone", Some(50_000), 1),
    ];
    let mut asked = Vec::new();
    let blinks = eyes::moments(
        &candidates,
        |i| {
            asked.push(i + 1);
            let face = match i + 1 {
                1 | 7 => shut_face(),
                9 => face(1000, 1000, 40.0, [0.13, 0.12], [0.7, 0.7]),
                _ => open_face(),
            };
            Some(eyes::people(&[face], 1000, 1000))
        },
        || false,
    )
    .unwrap();
    let found: Vec<(u64, Vec<u64>)> = blinks
        .iter()
        .map(|b: &Blink| {
            (
                b.frame.id.0,
                b.others.iter().map(|f: &Frame| f.id.0).collect(),
            )
        })
        .collect();
    // Each blink lists its moment's other shots, blinks included: which is
    // better is decided per person later.
    assert_eq!(found, vec![(1, vec![2, 7]), (7, vec![1, 2])]);
    assert_eq!(blinks[0].others[0].at_ms, 13_000);
    // Every photo with faces, a time and a camera is judged once; a closed
    // eye may hide on a face too small for the cached count.
    asked.sort();
    assert_eq!(asked, vec![1, 2, 3, 4, 5, 7, 9]);
}

#[test]
fn faces_too_small_here_are_judged_on_the_larger_image() {
    // Eyes 15 px apart: unknown on the analysis image.
    let small = face(1000, 1000, 15.0, [0.03, 0.03], [0.8, 0.8]);
    assert_eq!(
        eyes::people(std::slice::from_ref(&small), 1000, 1000)[0].state,
        Unknown
    );
    assert!(faces::rejudgeable(&small, 1000, 1000));
    // Measured again on an image twice as large, the eyes are shut.
    let mut asked = 0;
    let judged = eyes::judge_people(std::slice::from_ref(&small), 1000, 1000, |_| {
        asked += 1;
        Some((face(2000, 2000, 30.0, [0.03, 0.03], [0.8, 0.8]), 2000, 2000))
    });
    assert_eq!((asked, judged[0].state), (1, Closed));
    // Large faces are judged as they are; tiny ones are left unknown.
    let tiny = face(1000, 1000, 8.0, [0.03, 0.03], [0.8, 0.8]);
    let judged = eyes::judge_people(&[open_face(), tiny], 1000, 1000, |_| {
        panic!("only faces 10 to 20 px apart are looked at again")
    });
    assert_eq!(
        judged.iter().map(|p| p.state).collect::<Vec<_>>(),
        vec![Open, Unknown]
    );
    // When the closer look finds no face, the face stays unknown.
    let judged = eyes::judge_people(&[small], 1000, 1000, |_| None);
    assert_eq!(judged[0].state, Unknown);
}
