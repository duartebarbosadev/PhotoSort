use photosort::core::{
    model::*,
    models::{Models, orientation_model},
    pipeline::Pipeline,
};
use std::{path::PathBuf, time::Instant};
fn main() -> anyhow::Result<()> {
    let path = PathBuf::from(std::env::args_os().nth(1).expect("Pass a test photo"));
    let media = Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path)?,
        kind: MediaKind::from_path(&path).unwrap(),
        path,
    };
    let temp = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(64 * 1024 * 1024, temp.path().to_owned());
    let asset = pipeline.load(&media)?;
    let mut models = Models::default();
    let start = Instant::now();
    let orientation =
        orientation_model().ok_or_else(|| anyhow::anyhow!("Missing orientation model"))?;
    let rotation = models.rotation(&orientation, &asset)?;
    let embedding = models.embedding(&asset)?;
    let aesthetic = models.aesthetic(&asset)?;
    let faces = models.faces(&asset)?;
    println!("Full-frame faces: {}", faces.count);
    // Exercise the landmark session even when a wide scene's face is too small.
    let (w, h) = asset.pixels.dimensions();
    let pixels = std::sync::Arc::new(
        image::imageops::crop_imm(&*asset.pixels, w / 4, h * 9 / 20, w / 2, h * 2 / 5).to_image(),
    );
    let portrait = photosort::core::pipeline::Asset {
        decoder_warning: asset.decoder_warning.clone(),
        pixels: pixels.clone(),
        thumbnail: pixels,
        metadata: asset.metadata.clone(),
    };
    let portrait_faces = models.faces(&portrait)?;
    println!("Portrait faces: {}", portrait_faces.count);
    anyhow::ensure!(
        portrait_faces.count > 0,
        "No face detected in the portrait fixture"
    );
    assert_eq!(portrait_faces.geometry.len(), portrait_faces.count);
    let inferences_before_geometry = models.inferences;
    let identity = photosort::core::near_duplicates::Alignment::validate(
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        1.0,
        portrait.pixels.width() as usize,
        portrait.pixels.height() as usize,
    )
    .unwrap();
    let motion = photosort::core::near_duplicates::face_motion(
        &portrait_faces.geometry,
        &portrait_faces.geometry,
        &identity,
        portrait.pixels.width() as usize,
        portrait.pixels.height() as usize,
        || false,
    )
    .unwrap();
    assert!(!motion.changed);
    assert_eq!(models.inferences, inferences_before_geometry);
    for face in &portrait_faces.geometry {
        assert!(face.landmarks.len() >= 468);
        assert!(face.landmarks.iter().flatten().all(|v| v.is_finite()));
        assert!(face.bbox[2] > face.bbox[0] && face.bbox[3] > face.bbox[1]);
        let crop = photosort::core::faces::aligned_crop(&portrait, face)?;
        assert_eq!(models.embedding(&crop)?.len(), 384);
    }
    assert_eq!(embedding.len(), 384);
    assert!((embedding.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 1e-4);
    assert!((0.0..=1.0).contains(&aesthetic));
    println!(
        "Cold model execution: {:?}; rotation {:?}; aesthetic {:.4}; DINO dimensions {}",
        start.elapsed(),
        rotation,
        aesthetic,
        embedding.len()
    );
    let warm = Instant::now();
    models.rotation(&orientation, &asset)?;
    let second = models.embedding(&asset)?;
    let second_score = models.aesthetic(&asset)?;
    models.faces(&asset)?;
    assert_eq!(models.faces(&portrait)?.count, portrait_faces.count);
    assert_eq!(models.initializations, 5);
    assert!((aesthetic - second_score).abs() < 1e-5);
    assert!(
        embedding
            .iter()
            .zip(second)
            .all(|(a, b)| (*a - b).abs() < 1e-5)
    );
    println!(
        "Warm execution: {:?}; model initializations {} (one per model)",
        warm.elapsed(),
        models.initializations
    );
    Ok(())
}
