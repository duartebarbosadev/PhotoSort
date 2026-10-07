use photosort::core::{
    faces::{Face, aligned_crop},
    pipeline::Asset,
};
use serde::Deserialize;
use std::sync::Arc;
#[derive(Deserialize)]
struct Case {
    name: String,
    face: Face,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}
#[test]
fn aligned_face_crops_match_reference_pixels_without_mutating_source() {
    let pixels = Arc::new(image::RgbaImage::from_fn(32, 24, |x, y| {
        image::Rgba([
            (x * 7 % 256) as u8,
            (y * 11 % 256) as u8,
            ((x * 13 + y * 17) % 256) as u8,
            255,
        ])
    }));
    let before = pixels.clone();
    let asset = Asset {
        decoder_warning: None,
        pixels: pixels.clone(),
        thumbnail: pixels,
        metadata: Default::default(),
    };
    let cases: Vec<Case> = serde_json::from_str(include_str!("face_crop_reference.json")).unwrap();
    for c in cases {
        let crop = aligned_crop(&asset, &c.face).unwrap();
        assert_eq!(crop.pixels.dimensions(), (c.width, c.height), "{}", c.name);
        let rgb: Vec<_> = crop
            .pixels
            .pixels()
            .flat_map(|p| p.0[..3].to_vec())
            .collect();
        let differences: Vec<_> = rgb
            .iter()
            .zip(&c.pixels)
            .map(|(a, b)| a.abs_diff(*b))
            .collect();
        assert!(
            differences.iter().all(|d| *d <= 1),
            "{}: max delta {:?}",
            c.name,
            differences.iter().max()
        );
    }
    assert!(Arc::ptr_eq(&before, &asset.pixels));
    assert!(
        aligned_crop(
            &asset,
            &Face {
                landmarks: Vec::new(),
                bbox: [2., 2., 3., 3.],
                eyes: [[0., 0.], [1., 0.]]
            }
        )
        .is_err()
    );
    assert!(
        aligned_crop(
            &asset,
            &Face {
                landmarks: Vec::new(),
                bbox: [f32::NAN, 0., 1., 1.],
                eyes: [[0., 0.], [1., 0.]]
            }
        )
        .is_err()
    );
}
