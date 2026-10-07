use photosort::core::{
    analysis::{self, Thresholds},
    pipeline::Asset,
    pixels,
};
use serde::Deserialize;
use std::sync::Arc;
#[derive(Deserialize)]
struct Reference {
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    width: u32,
    height: u32,
    kind: String,
    seed: u64,
    detection_width: u32,
    detection_height: u32,
    offset: usize,
    length: usize,
    sharpness: f64,
    quality_sharpness: f64,
    brightness: f64,
    black_fraction: f64,
    issue: Option<String>,
}
fn source(c: &Case) -> image::RgbImage {
    image::RgbImage::from_fn(c.width, c.height, |x, y| {
        let i = u64::from(y) * u64::from(c.width) + u64::from(x);
        let value = match c.kind.as_str() {
            "noise" => {
                return image::Rgb(std::array::from_fn(|channel| {
                    ((i * 73 + u64::from(y) * 37 + c.seed * 19 + channel as u64 * 47) % 256) as u8
                }));
            }
            "gray" => ((i * 73 + u64::from(y) * 37 + c.seed * 19) % 256) as u8,
            "chess" => (127 + (x + y) % 2 * 2) as u8,
            "night" => (i % 17) as u8,
            "flat" => c.seed as u8,
            "tiles" => {
                if x >= c.width / 2 {
                    150
                } else {
                    50
                }
            }
            _ => panic!("Unknown pattern"),
        };
        image::Rgb([value; 3])
    })
}
fn reference() -> Reference {
    serde_json::from_str(include_str!("technical_reference.json")).unwrap()
}
#[test]
fn detection_pixels_match_python_rgb_area_and_gray_conversion() {
    let expected = include_bytes!("technical_reference.bin");
    let mut total = 0;
    for c in reference().cases {
        let src = source(&c);
        let size = analysis::detection_size(c.width, c.height);
        assert_eq!(size, (c.detection_width, c.detection_height), "{}", c.name);
        let rgb = pixels::resize_area(&src, size.0, size.1, &|| false).unwrap();
        let gray = pixels::gray(&rgb, &|| false).unwrap();
        let truth = &expected[c.offset..c.offset + c.length];
        let mismatches = gray
            .as_raw()
            .iter()
            .zip(truth)
            .filter(|(a, b)| a != b)
            .count();
        if mismatches > 0 {
            eprintln!(
                "{}: {mismatches} mismatches; first {:?}",
                c.name,
                gray.as_raw()
                    .iter()
                    .zip(truth)
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(5)
                    .collect::<Vec<_>>()
            );
        }
        total += mismatches;
    }
    assert_eq!(total, 0, "Grayscale byte mismatches");
}
#[test]
fn measurements_and_classifications_match_the_python_worker() {
    for c in reference().cases {
        let rgba = Arc::new(image::DynamicImage::ImageRgb8(source(&c)).to_rgba8());
        let asset = Asset {
            decoder_warning: None,
            pixels: rgba.clone(),
            thumbnail: rgba,
            metadata: Default::default(),
        };
        let actual = analysis::analyze(&asset, Thresholds::default());
        for (name, a, b) in [
            ("local", actual.sharpness, c.sharpness),
            (
                "whole",
                actual.quality_sharpness.unwrap(),
                c.quality_sharpness,
            ),
            ("brightness", actual.brightness, c.brightness),
            ("black", actual.black_fraction, c.black_fraction),
        ] {
            assert!((a - b).abs() < 1e-7, "{} {name}: {a} != {b}", c.name);
        }
        let expected = match c.issue.as_deref() {
            Some("blur") => vec!["Possibly soft"],
            Some("dark") => vec!["Very dark"],
            Some("white") => vec!["Overexposed"],
            None => vec![],
            _ => panic!(),
        };
        assert_eq!(actual.issues, expected, "{}", c.name);
    }
}
#[test]
fn cancelled_measurements_never_return_partial_descriptors() {
    use std::cell::Cell;
    let rgba = Arc::new(image::RgbaImage::from_pixel(
        1024,
        768,
        image::Rgba([20, 80, 160, 255]),
    ));
    let asset = Asset {
        decoder_warning: None,
        pixels: rgba.clone(),
        thumbnail: rgba,
        metadata: Default::default(),
    };
    for limit in [0, 500, 1600, 3000, 4500] {
        let checks = Cell::new(0);
        let result = analysis::analyze_cancellable(&asset, Thresholds::default(), || {
            checks.set(checks.get() + 1);
            checks.get() > limit
        });
        assert!(result.is_none(), "Cancelled pass {limit} published results");
    }
}
