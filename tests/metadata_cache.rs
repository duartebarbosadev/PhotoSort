//! Metadata persistence belongs to the shared image owner, including worker restart.
use photosort::core::{
    cache::{self, Limits},
    catalog::Catalog,
    jobs::{Event, Runtime},
    metadata,
    model::Media,
    pipeline::Pipeline,
};
use std::{cell::Cell, fs, path::Path, sync::Arc, time::Duration};

fn fixture(root: &Path) -> Media {
    let path = root.join("photo.png");
    image::RgbImage::new(32, 16).save(&path).unwrap();
    metadata::write_sidecar(&path, Some(4), Some(6)).unwrap();
    Catalog::scan(root, || false, |_| {})
        .unwrap()
        .media
        .remove(0)
}
fn pipeline(cache: &Path) -> Pipeline {
    Pipeline::new(1024 * 1024, cache.into())
}
fn records(cache: &Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(cache)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "bin"))
        .collect()
}
#[test]
fn metadata_restart_reuses_source_snapshot_across_browsing_display_and_analysis() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = fixture(root.path());
    let mut first = pipeline(cache.path());
    let expected = first.metadata(&media).unwrap();
    assert_eq!((first.stats.metadata_reads, first.stats.decodes), (1, 0));
    assert_eq!((expected.rating, expected.orientation), (4, 6));
    drop(first);
    let mut reopened = pipeline(cache.path());
    let actual = reopened.metadata(&media).unwrap();
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(reopened.stats.metadata_disk_hits, 1);
    reopened.load(&media).unwrap();
    reopened.analysis(&media, || false).unwrap();
    reopened.metadata(&media).unwrap();
    assert_eq!(
        (reopened.stats.metadata_reads, reopened.stats.decodes),
        (0, 1)
    );
    assert_eq!(reopened.stats.metadata_disk_hits, 1);
    // Display dimensions must not replace the raw metadata-only snapshot.
    assert_eq!(
        serde_json::to_value(pipeline(cache.path()).metadata(&media).unwrap()).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}
#[test]
fn persistent_metadata_invalidates_sidecar_addition_edit_removal_and_source_replacement() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = fixture(root.path());
    pipeline(cache.path()).metadata(&media).unwrap();
    for rating in [2, 5] {
        metadata::write_sidecar(&media.path, Some(rating), Some(3)).unwrap();
        let mut owner = pipeline(cache.path());
        assert_eq!(owner.metadata(&media).unwrap().rating, rating);
        assert_eq!(owner.stats.metadata_reads, 1);
    }
    fs::remove_file(metadata::sidecar(&media.path)).unwrap();
    let mut owner = pipeline(cache.path());
    assert_eq!(owner.metadata(&media).unwrap().rating, 0);
    assert_eq!(owner.stats.metadata_reads, 1);
    metadata::write_sidecar(&media.path, Some(1), None).unwrap();
    let mut owner = pipeline(cache.path());
    assert_eq!(owner.metadata(&media).unwrap().rating, 1);
    assert_eq!(owner.stats.metadata_reads, 1);
    image::RgbImage::new(7, 9).save(&media.path).unwrap();
    let mut owner = pipeline(cache.path());
    owner.metadata(&media).unwrap();
    assert_eq!(owner.stats.metadata_reads, 1);
    assert_eq!(owner.stats.decodes, 0);
}
#[test]
fn corrupt_cache_is_repaired_and_clear_disable_and_unwritable_cache_remain_usable() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = fixture(root.path());
    pipeline(cache.path()).metadata(&media).unwrap();
    let files = records(cache.path());
    assert_eq!(files.len(), 1);
    fs::write(&files[0], b"damaged record").unwrap();
    let mut owner = pipeline(cache.path());
    assert_eq!(owner.metadata(&media).unwrap().rating, 4);
    assert_eq!(owner.stats.metadata_reads, 1);
    assert!(fs::metadata(&files[0]).unwrap().len() > 14);
    let original = fs::read(&files[0]).unwrap();
    let mut disabled = pipeline(cache.path());
    disabled.configure_cache(Limits {
        disk_mib: 0,
        ..Default::default()
    });
    disabled.metadata(&media).unwrap();
    assert_eq!(
        (
            disabled.stats.metadata_reads,
            disabled.stats.metadata_disk_hits
        ),
        (1, 0)
    );
    assert_eq!(fs::read(&files[0]).unwrap(), original);
    owner.clear_memory();
    owner.maintain_cache(true, || false).unwrap();
    disabled.clear_memory();
    disabled.metadata(&media).unwrap();
    assert!(records(cache.path()).is_empty());
    owner.metadata(&media).unwrap();
    assert_eq!(owner.stats.metadata_reads, 2);
    let blocked = cache.path().join("not-a-directory");
    fs::write(&blocked, b"foreign").unwrap();
    assert_eq!(pipeline(&blocked).metadata(&media).unwrap().rating, 4);
    assert_eq!(fs::read(blocked).unwrap(), b"foreign");
}
#[test]
fn cancellation_and_source_changes_cannot_publish_metadata_from_an_old_identity() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = fixture(root.path());
    let mut owner = pipeline(cache.path());
    assert!(owner.metadata_cancellable(&media, &|| true).is_err());
    assert_eq!(owner.stats.metadata_reads, 0);
    let checks = Cell::new(0);
    assert!(
        owner
            .metadata_cancellable(&media, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 3
            })
            .is_err()
    );
    assert!(records(cache.path()).is_empty());
    let changed = Cell::new(false);
    assert!(
        owner
            .metadata_cancellable(&media, &|| {
                if !changed.replace(true) {
                    return false;
                }
                metadata::write_sidecar(&media.path, Some(2), None).unwrap();
                false
            })
            .is_err()
    );
    assert!(records(cache.path()).is_empty());
    assert_eq!(owner.metadata(&media).unwrap().rating, 2);
    let mut reopened = pipeline(cache.path());
    let checks = Cell::new(0);
    assert!(
        reopened
            .metadata_cancellable(&media, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 2
            })
            .is_err()
    );
    assert_eq!(reopened.stats.metadata_reads, 0);
    assert_eq!(reopened.metadata(&media).unwrap().rating, 2);
    assert_eq!(reopened.stats.metadata_disk_hits, 1);
}
#[test]
fn persistent_metadata_shares_the_cache_budget_and_foreign_file_guards() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = fixture(root.path());
    let mut owner = pipeline(cache.path());
    owner.configure_cache(Limits {
        disk_mib: 1,
        ..Default::default()
    });
    let owned = cache.path().join(format!("{}.bin", "a".repeat(64)));
    fs::File::create(owned)
        .unwrap()
        .set_len(2 * cache::MIB)
        .unwrap();
    fs::write(cache.path().join("keep.txt"), b"foreign").unwrap();
    owner.metadata(&media).unwrap();
    let usage = owner.maintain_cache(false, || false).unwrap();
    assert!(usage.disk_bytes <= cache::MIB);
    owner.maintain_cache(true, || false).unwrap();
    assert_eq!(fs::read(cache.path().join("keep.txt")).unwrap(), b"foreign");
}
#[test]
fn metadata_worker_restart_uses_disk_without_decoding() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = Arc::new(vec![fixture(root.path())]);
    for expected_reads in [1, 0] {
        let mut rt = Runtime::with_cache(|| {}, cache.path().into());
        let run = rt.index_metadata(media.clone()).unwrap();
        loop {
            if let Event::MetadataBatch {
                run: actual,
                done,
                total,
                items,
                stats,
            } = rt
                .events
                .recv_timeout(Duration::from_secs(10))
                .unwrap()
                .event
            {
                assert_eq!(actual, run);
                assert!(items.iter().all(|(_, v)| v.as_ref().unwrap().rating == 4));
                if done == total {
                    assert_eq!((stats.metadata_reads, stats.decodes), (expected_reads, 0));
                    assert_eq!(stats.metadata_disk_hits, 1 - expected_reads);
                    break;
                }
            }
        }
        rt.shutdown();
    }
}
