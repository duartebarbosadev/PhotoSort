#[path = "../src/test_support.rs"]
mod test_support;
use image::{DynamicImage, Rgba, RgbaImage};
use photosort::core::{
    analysis::{self, Thresholds},
    catalog::Catalog,
    metadata,
    model::*,
    operations::{self, Action, Plan, Source},
    pipeline::{Pipeline, orient},
    session::Session,
};
use std::{collections::HashMap, fs, sync::Arc};
use tempfile::TempDir;

fn photo(dir: &std::path::Path, name: &str) -> Media {
    let path = dir.canonicalize().unwrap().join(name);
    image::RgbImage::from_fn(96, 64, |x, y| {
        image::Rgb([((x * 7 + y * 3) % 255) as u8, (y * 3) as u8, (x * 2) as u8])
    })
    .save(&path)
    .unwrap();
    Media {
        id: MediaId::for_path(&path),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
    }
}
fn catalog(root: &std::path::Path) -> Catalog {
    Catalog::scan(root, || false, |_| {}).unwrap()
}
#[test]
fn scanner_is_batched_and_companions_reuse_inventory() {
    let dir = TempDir::new().unwrap();
    photo(dir.path(), "pair.JPG");
    fs::write(dir.path().join("pair.ARW"), b"raw").unwrap();
    fs::write(dir.path().join("pair.xmp"), b"xmp").unwrap();
    fs::write(dir.path().join("notes.txt"), b"notes").unwrap();
    let mut batches = 0;
    let c = Catalog::scan(dir.path(), || false, |b| batches += b.len()).unwrap();
    assert_eq!(batches, 2);
    assert_eq!(c.inventory.len(), 4);
    assert_eq!(
        c.companion_index()[&dir.path().canonicalize().unwrap().join("pair")].len(),
        3
    );
}
#[test]
fn scan_cancellation_does_not_emit_completed_catalog() {
    let d = TempDir::new().unwrap();
    photo(d.path(), "a.png");
    assert!(Catalog::scan(d.path(), || true, |_| panic!("cancelled scan emitted")).is_err());
}
#[cfg(unix)]
#[test]
fn scanner_never_follows_directory_or_file_symlinks() {
    let d = TempDir::new().unwrap();
    let external = TempDir::new().unwrap();
    photo(external.path(), "a.png");
    std::os::unix::fs::symlink(external.path(), d.path().join("link")).unwrap();
    assert!(catalog(d.path()).media.is_empty());
}
#[test]
fn all_five_steps_share_one_decode_and_one_thumbnail() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let cache = TempDir::new().unwrap();
    let mut p = Pipeline::new(4 * 1024 * 1024, cache.path().to_owned());
    let first = p.load(&m).unwrap();
    for _step in Step::ALL {
        let next = p.load(&m).unwrap();
        assert!(Arc::ptr_eq(&first.pixels, &next.pixels));
        assert!(Arc::ptr_eq(&first.thumbnail, &next.thumbnail));
    }
    assert_eq!(p.stats.decodes, 1);
    assert_eq!(p.stats.hits, 5);
}
#[test]
fn proxy_survives_memory_eviction_without_redecoding_source() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let cache = TempDir::new().unwrap();
    let mut p = Pipeline::new(1, cache.path().to_owned());
    p.load(&m).unwrap();
    assert_eq!(p.stats.resident_bytes, 0);
    p.load(&m).unwrap();
    assert_eq!(p.stats.decodes, 1);
    assert_eq!(p.stats.disk_hits, 1);
}
#[test]
fn grid_thumbnails_reopen_from_their_own_artifact_without_decoding() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let cache = TempDir::new().unwrap();
    let loaded = Pipeline::new(1_000_000, cache.path().to_owned())
        .load(&m)
        .unwrap();
    // A new session: nothing in memory, the display proxy is not decoded.
    let mut reopened = Pipeline::new(1_000_000, cache.path().to_owned());
    let thumb = reopened.cached_thumbnail(&m, || false).unwrap();
    assert_eq!(*thumb.thumbnail, *loaded.thumbnail);
    assert_eq!(thumb.metadata.width, loaded.metadata.width);
    assert_eq!(reopened.stats.decodes, 0);
    // Editing the sidecar changes the photo's key, so the old tile is not reused.
    metadata::write_sidecar(&m.path, Some(4), Some(6)).unwrap();
    assert!(reopened.cached_thumbnail(&m, || false).is_none());
}
#[test]
fn sidecar_mutation_invalidates_canonical_preview() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let cache = TempDir::new().unwrap();
    let mut p = Pipeline::new(1_000_000, cache.path().to_owned());
    assert_eq!(p.load(&m).unwrap().pixels.dimensions(), (96, 64));
    metadata::write_sidecar(&m.path, Some(4), Some(6)).unwrap();
    let result = p.load(&m).unwrap();
    assert_eq!(result.pixels.dimensions(), (64, 96));
    assert_eq!(result.metadata.rating, 4);
    assert_eq!(p.stats.decodes, 2);
}
#[test]
fn all_exif_rotations_form_four_turn_cycles() {
    for orientation in 1..=8 {
        let mut current = orientation;
        for _ in 0..4 {
            current = metadata::rotate_orientation(current, 1);
        }
        assert_eq!(current, orientation);
    }
}
#[test]
fn exif_mirrored_orientations_are_not_lost() {
    let image = RgbaImage::from_fn(3, 2, |x, y| Rgba([(x + y * 3) as u8, 0, 0, 255]));
    let transposed = orient(DynamicImage::ImageRgba8(image.clone()), 5).to_rgba8();
    assert_eq!(transposed.dimensions(), (2, 3));
    for y in 0..2 {
        for x in 0..3 {
            assert_eq!(transposed.get_pixel(y, x), image.get_pixel(x, y));
        }
    }
}
#[test]
fn xmp_preserves_unknown_namespaces_and_replaces_alias_attributes() {
    let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><r:Description xmlns:score="http://ns.adobe.com/xap/1.0/" xmlns:custom="urn:mine" score:Rating="1" custom:Keep="yes"><custom:Edit>retain me</custom:Edit></r:Description></r:RDF></x:xmpmeta>"#;
    let updated = metadata::update_xmp(xml, Some(5), Some(8)).unwrap();
    assert!(updated.contains("custom:Keep=\"yes\""));
    assert!(updated.contains("retain me"));
    assert!(!updated.contains("score:Rating"));
    assert_eq!(metadata::read_xmp(&updated).unwrap(), (Some(5), Some(8)));
}
#[test]
fn xmp_replaces_element_values_without_losing_siblings() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/"><xmp:Rating>2</xmp:Rating><xmp:Label>Green</xmp:Label></rdf:Description></rdf:RDF>"#;
    let updated = metadata::update_xmp(xml, Some(4), None).unwrap();
    assert!(!updated.contains("<xmp:Rating>"));
    assert!(updated.contains("Green"));
    assert_eq!(metadata::read_xmp(&updated).unwrap().0, Some(4));
}
#[test]
fn invalid_xmp_is_never_silently_overwritten() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let path = m.path.with_extension("xmp");
    fs::write(&path, "not valid xmp").unwrap();
    assert!(metadata::write_sidecar(&m.path, Some(4), None).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "not valid xmp");
}
#[test]
fn invalid_ratings_are_rejected() {
    assert!(metadata::update_xmp("", Some(6), None).is_err());
}
#[test]
fn move_plan_keeps_companions_on_same_collision_suffix() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "pair.jpg");
    fs::write(d.path().join("pair.arw"), b"raw").unwrap();
    fs::write(d.path().join("pair.xmp"), b"xmp").unwrap();
    let c = catalog(d.path());
    let output = TempDir::new().unwrap();
    fs::create_dir(output.path().join("Trip")).unwrap();
    fs::write(output.path().join("Trip/pair.jpg"), b"existing").unwrap();
    let labels = c.media.iter().map(|m| (m.id, "Trip".to_owned())).collect();
    let plan = operations::organize_plan(&c, &labels, output.path(), true).unwrap();
    assert_eq!(plan.actions.len(), 1);
    let Action::Move { files } = &plan.actions[0] else {
        panic!()
    };
    assert_eq!(files.len(), 3);
    for (_, to) in files {
        assert!(to.file_stem().unwrap().to_string_lossy().ends_with("_1"));
    }
    assert!(m.path.exists());
    let journals = TempDir::new().unwrap();
    let result = operations::execute(&plan, journals.path(), || false, |_, _| {}).unwrap();
    assert!(result.errors.is_empty());
    assert!(!m.path.exists());
    assert_eq!(
        fs::read(output.path().join("Trip/pair.jpg")).unwrap(),
        b"existing"
    );
    assert!(output.path().join("Trip/pair_1.xmp").exists());
}
#[test]
fn stale_sources_are_rejected_before_any_move() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let output = TempDir::new().unwrap();
    let source = Source::capture(m.path.clone()).unwrap();
    fs::write(&m.path, b"changed").unwrap();
    let dest = output.path().join("a.png");
    let plan = Plan {
        actions: vec![Action::Move {
            files: vec![(source, dest.clone())],
        }],
        description: String::new(),
        ..Default::default()
    };
    let result = operations::execute(&plan, output.path(), || false, |_, _| {}).unwrap();
    assert_eq!(result.errors.len(), 1);
    assert!(!dest.exists());
    assert_eq!(fs::read(m.path).unwrap(), b"changed");
}
#[test]
fn late_destination_collision_is_never_overwritten() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let output = TempDir::new().unwrap();
    let dest = output.path().join("a.png");
    let action = Action::Move {
        files: vec![(Source::capture(m.path.clone()).unwrap(), dest.clone())],
    };
    fs::write(&dest, b"precious").unwrap();
    let result = operations::execute(
        &Plan {
            actions: vec![action],
            description: String::new(),
            ..Default::default()
        },
        output.path(),
        || false,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(result.errors.len(), 1);
    assert!(m.path.exists());
    assert_eq!(fs::read(dest).unwrap(), b"precious");
}
#[test]
fn cancellation_preserves_unstarted_operations() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let plan = Plan {
        actions: vec![Action::Trash {
            files: vec![Source::capture(m.path.clone()).unwrap()],
        }],
        description: String::new(),
        ..Default::default()
    };
    let result = operations::execute(&plan, d.path(), || true, |_, _| {}).unwrap();
    assert!(result.cancelled);
    assert!(m.path.exists());
}
#[test]
fn conflicting_raw_jpeg_sidecar_edits_require_resolution() {
    let d = TempDir::new().unwrap();
    photo(d.path(), "pair.jpg");
    photo(d.path(), "pair.png");
    let c = catalog(d.path());
    let mut decisions = HashMap::new();
    decisions.insert(
        c.media[0].id,
        Decision {
            rating: Some(1),
            ..Default::default()
        },
    );
    decisions.insert(
        c.media[1].id,
        Decision {
            rating: Some(5),
            ..Default::default()
        },
    );
    assert!(operations::decisions_plan(&c, &decisions, true).is_err());
}
#[test]
fn metadata_plan_detects_external_sidecar_edits() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let c = catalog(d.path());
    let decisions = HashMap::from([(
        m.id,
        Decision {
            rating: Some(5),
            ..Default::default()
        },
    )]);
    let plan = operations::decisions_plan(&c, &decisions, false).unwrap();
    metadata::write_sidecar(&m.path, Some(2), None).unwrap();
    let result = operations::execute(&plan, d.path(), || false, |_, _| {}).unwrap();
    assert_eq!(result.errors.len(), 1);
    assert_eq!(metadata::read(&m.path).rating, 2);
}
#[test]
fn session_drops_stale_decisions() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "a.png");
    let mut s = Session {
        suggested: Default::default(),
        pick_suggested: Default::default(),
        organize: None,
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            m.id,
            Decision {
                rejected: true,
                ..Default::default()
            },
        )]),
        fingerprints: HashMap::from([(m.id, m.fingerprint)]),
    };
    let mut changed = m.clone();
    changed.fingerprint.bytes += 1;
    s.reconcile(&[changed]);
    assert!(s.decisions.is_empty());
}
#[test]
fn blur_detection_prefers_detailed_over_uniform_images() {
    let d = TempDir::new().unwrap();
    let m = photo(d.path(), "detailed.png");
    let blank = d.path().canonicalize().unwrap().join("blank.png");
    RgbaImage::from_pixel(96, 64, Rgba([120, 120, 120, 255]))
        .save(&blank)
        .unwrap();
    let c = catalog(d.path());
    let mut pipeline = Pipeline::new(1_000_000, d.path().join("cache"));
    let sharp = analysis::analyze(&pipeline.load(&m).unwrap(), Thresholds::default());
    let flat = analysis::analyze(
        &pipeline
            .load(c.media.iter().find(|m| m.path == blank).unwrap())
            .unwrap(),
        Thresholds::default(),
    );
    assert!(sharp.sharpness > flat.sharpness);
    assert!(flat.issues.contains(&"Possibly soft".to_string()));
    assert!(sharp.score > flat.score);
}
#[test]
fn sanitization_prevents_folder_traversal() {
    assert_eq!(operations::safe_component(".."), "Unsorted");
    assert!(!operations::safe_component("../../outside").contains('/'));
}

#[test]
fn switching_folders_discards_old_generation_results() {
    use photosort::core::jobs::{Event, IoWork, Runtime};
    use std::time::{Duration, Instant};
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    for i in 0..500 {
        fs::write(first.path().join(format!("{i}.jpg")), b"scan-only").unwrap();
    }
    photo(second.path(), "current.png");
    let mut runtime = Runtime::with_cache(|| {}, cache.path().to_owned());
    runtime.next_epoch();
    runtime.io(IoWork::Scan(first.path().to_owned())).unwrap();
    let current = runtime.next_epoch();
    runtime.io(IoWork::Scan(second.path().to_owned())).unwrap();
    let deadline = Instant::now() + test_support::worker_patience();
    let mut opened = false;
    while Instant::now() < deadline {
        if let Ok(event) = runtime.events.recv_timeout(Duration::from_millis(100)) {
            if event.epoch != current {
                continue;
            }
            if let Event::Opened(catalog, _, _) = event.event {
                assert_eq!(catalog.media.len(), 1);
                assert_eq!(catalog.media[0].name(), "current.png");
                opened = true;
                break;
            }
        }
    }
    runtime.shutdown();
    assert!(opened, "Latest folder did not finish");
}

#[test]
fn shutdown_does_not_deadlock_when_result_queue_is_full() {
    use photosort::core::jobs::{ImageWork, Runtime};
    use std::time::{Duration, Instant};
    let folder = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let media = photo(folder.path(), "a.png");
    let mut runtime = Runtime::with_cache(|| {}, cache.path().to_owned());
    for _ in 0..32 {
        let _ = runtime.image(ImageWork::Asset(media.clone(), false));
    }
    std::thread::sleep(Duration::from_millis(100));
    let start = Instant::now();
    runtime.shutdown();
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn nearest_city_handles_the_antimeridian() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("places.tsv");
    fs::write(&path, "0\t-179.9\tFiji\tEast\tNear\n0\t170\tFiji\tWest\tFar\n-31.95\t115.86\tAustralia\tWA\tPerth\n").unwrap();
    let geo = photosort::core::geography::Geography::load(&path).unwrap();
    assert_eq!(geo.nearest(0.0, 179.9).unwrap().city, "Near");
    assert_eq!(geo.nearest(-31.9, 115.9).unwrap().city, "Perth");
    assert!(geo.nearest(f64::NAN, 0.0).is_none());
}
#[test]
fn sharpness_gate_protects_against_aesthetic_but_blurred_choice() {
    let entries = vec![
        (
            MediaId(1),
            1,
            analysis::Analysis {
                sharpness: 200.0,
                score: 75.0,
                ..Default::default()
            },
        ),
        (
            MediaId(2),
            1,
            analysis::Analysis {
                sharpness: 80.0,
                score: 99.0,
                ..Default::default()
            },
        ),
        (
            MediaId(3),
            2,
            analysis::Analysis {
                sharpness: 10.0,
                score: 30.0,
                ..Default::default()
            },
        ),
    ];
    assert_eq!(
        analysis::best_per_group(entries),
        vec![MediaId(1), MediaId(3)]
    );
}
#[test]
fn eye_ratio_is_scale_invariant_and_distinguishes_closed_eyes() {
    use photosort::core::faces::eye_aspect_ratio;
    let open = [
        0.0, 0.0, 0.0, 2.0, 1.0, 0.0, 4.0, 1.0, 0.0, 6.0, 0.0, 0.0, 4.0, -1.0, 0.0, 2.0, -1.0, 0.0,
    ];
    let indices = [0, 1, 2, 3, 4, 5];
    assert!(eye_aspect_ratio(&open, indices) > 0.21);
    let scaled: Vec<_> = open.iter().map(|x| x * 3.0).collect();
    assert!((eye_aspect_ratio(&open, indices) - eye_aspect_ratio(&scaled, indices)).abs() < 1e-6);
    let mut closed = open;
    for i in 0..6 {
        closed[i * 3 + 1] *= 0.1;
    }
    assert!(eye_aspect_ratio(&closed, indices) < 0.21);
}
#[test]
fn organize_preserves_nested_paths_and_renames_companions() {
    let source = TempDir::new().unwrap();
    let destination = TempDir::new().unwrap();
    let m = photo(source.path(), "a.png");
    fs::write(source.path().join("a.xmp"), b"metadata").unwrap();
    let c = catalog(source.path());
    let plan = operations::organize_plan_with_names(
        &c,
        &HashMap::from([(m.id, "2026/September".into())]),
        &HashMap::from([(m.id, "Holiday".into())]),
        destination.path(),
        false,
    )
    .unwrap();
    let Action::Move { files } = &plan.actions[0] else {
        panic!()
    };
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .any(|(_, p)| p.ends_with("2026/September/Holiday.png"))
    );
    assert!(
        files
            .iter()
            .any(|(_, p)| p.ends_with("2026/September/Holiday.xmp"))
    );
}
