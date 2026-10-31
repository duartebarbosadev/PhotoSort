use super::*;
fn root() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("cache");
    (temp, root)
}
fn ready(root: &Path, n: usize, bytes: &[u8]) -> Lease {
    let mut entry =
        Lease::acquire(Format::CoreMl, root, &format!("{n:064x}"), "m", &|| false).unwrap();
    fs::create_dir_all(entry.path.join("native/weights")).unwrap();
    fs::write(entry.path.join("native/weights/model.bin"), bytes).unwrap();
    entry.complete(&|| false).unwrap();
    entry
}
#[test]
fn stable_native_cache_reuses_verified_bytes_and_holds_a_process_lease() {
    let (_temp, root) = root();
    let first = ready(&root, 1, b"weights");
    let path = first.path.clone();
    assert!(!first.reused);
    assert!(first.retained());
    assert!(
        Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            false
        })
        .is_err()
    );
    let active = crate::core::cache::maintain(&root, Some(0), || false).unwrap();
    assert!(active.disk_bytes > 0 && active.pinned_disk_bytes == active.disk_bytes);
    assert!(path.join("native/weights/model.bin").exists());
    drop(first);
    let second = Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
        false
    })
    .unwrap();
    assert!(second.reused);
    assert_eq!(second.path, path);
    drop(second);
    assert_eq!(
        crate::core::cache::maintain(&root, Some(0), || false)
            .unwrap()
            .disk_bytes,
        0
    );
    assert!(!path.exists());
}
#[test]
fn retired_prepared_cpu_entries_are_removed_by_maintenance() {
    let (_temp, root) = root();
    drop(ready(&root, 1, b"coreml"));
    let legacy = root.join(format!("{:064x}.onnx-cache", 2));
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join(".owner"), OWNER).unwrap();
    fs::write(legacy.join("optimized.onnx"), vec![0; 4096]).unwrap();
    // Measuring alone never deletes; maintenance with any budget drops the
    // retired format but keeps the reusable Core ML program.
    crate::core::cache::maintain(&root, None, || false).unwrap();
    assert!(legacy.exists());
    let usage = crate::core::cache::maintain(&root, Some(1_000_000), || false).unwrap();
    assert!(!legacy.exists());
    assert!(root.join(format!("{:064x}.coreml", 1)).exists());
    assert!(usage.disk_bytes > 0 && usage.disk_bytes < 4096 + 1_000);
}
#[test]
fn changed_weights_runtime_or_recipe_cannot_reuse_an_old_directory() {
    let (temp, _) = root();
    let model = temp.path().join("model");
    let runtime = temp.path().join("runtime");
    fs::write(&model, b"weights one").unwrap();
    fs::write(&runtime, b"runtime one").unwrap();
    let original = key(&model, &runtime, "recipe1", &|| false).unwrap();
    assert_eq!(
        original,
        key(&model, &runtime, "recipe1", &|| false).unwrap()
    );
    fs::write(&model, b"weights two").unwrap();
    assert_ne!(
        original,
        key(&model, &runtime, "recipe1", &|| false).unwrap()
    );
    fs::write(&model, b"weights one").unwrap();
    fs::write(&runtime, b"runtime two").unwrap();
    assert_ne!(
        original,
        key(&model, &runtime, "recipe1", &|| false).unwrap()
    );
    fs::write(&runtime, b"runtime one").unwrap();
    assert_ne!(
        original,
        key(&model, &runtime, "recipe2", &|| false).unwrap()
    );
}
#[test]
fn corrupt_incomplete_or_extra_native_files_are_regenerated_before_loading() {
    for damage in [0, 1, 2] {
        let (_temp, root) = root();
        let old = ready(&root, 1, b"weights");
        let path = old.path.clone();
        drop(old);
        match damage {
            0 => fs::write(path.join("native/weights/model.bin"), b"changed").unwrap(),
            1 => fs::write(path.join(READY), b"torn").unwrap(),
            _ => fs::write(path.join("unexpected.bin"), b"extra").unwrap(),
        }
        let fresh = Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            false
        })
        .unwrap();
        assert!(!fresh.reused);
        assert!(!path.join("native").exists());
        drop(fresh);
        assert!(!path.exists());
    }
}
#[test]
fn cancelled_cache_validation_preserves_previously_complete_artifacts() {
    let (_temp, root) = root();
    let old = ready(&root, 1, &vec![7; 200_000]);
    let path = old.path.clone();
    drop(old);
    let calls = std::cell::Cell::new(0);
    assert!(
        Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            calls.set(calls.get() + 1);
            calls.get() > 5
        })
        .is_err()
    );
    assert!(path.join(READY).exists());
    assert!(
        Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            false
        })
        .unwrap()
        .reused
    );
}
#[test]
fn foreign_directories_and_unsafe_keys_are_refused() {
    let (temp, root) = root();
    for key in [format!("../{:064x}", 1), "bad".into()] {
        assert!(Lease::acquire(Format::CoreMl, &root, &key, "m", &|| false).is_err());
    }
    assert!(!root.exists());
    fs::create_dir_all(root.join(format!("{:064x}.coreml", 1))).unwrap();
    let foreign = root.join(format!("{:064x}.coreml/precious", 1));
    fs::write(&foreign, b"keep").unwrap();
    assert!(
        Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            false
        })
        .is_err()
    );
    crate::core::cache::maintain(&root, Some(0), || false).unwrap();
    assert_eq!(fs::read(foreign).unwrap(), b"keep");
    assert!(!temp.path().join(format!("{:064x}.coreml", 1)).exists());
}
#[cfg(unix)]
#[test]
fn native_cache_never_follows_or_deletes_link_targets() {
    let (temp, root) = root();
    let entry = ready(&root, 1, b"weights");
    let path = entry.path.clone();
    drop(entry);
    let photo = temp.path().join("photo");
    fs::write(&photo, b"original").unwrap();
    std::os::unix::fs::symlink(&photo, path.join("foreign-link")).unwrap();
    assert!(
        Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            false
        })
        .is_err()
    );
    assert!(crate::core::cache::maintain(&root, Some(0), || false).is_err());
    assert_eq!(fs::read(photo).unwrap(), b"original");
    assert!(path.join("foreign-link").is_symlink());
}
#[test]
fn failed_native_compilation_leaves_no_persistent_artifact() {
    let (_temp, root) = root();
    let failed = Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 2), "m", &|| {
        false
    })
    .unwrap();
    let path = failed.path.clone();
    drop(failed);
    assert!(!path.exists());
}
#[test]
fn failed_session_invalidates_previously_complete_native_files() {
    let (_temp, root) = root();
    let mut entry = ready(&root, 1, b"weights");
    let path = entry.path.clone();
    assert!(entry.retained());
    entry.invalidate();
    assert!(!entry.retained());
    drop(entry);
    assert!(!path.exists());
}
#[test]
fn prepared_models_stay_outside_the_disk_limit_until_a_newer_copy_replaces_them() {
    let (_temp, root) = root();
    let acquire = |n: usize, model: &str| {
        Lease::acquire(Format::CoreMl, &root, &format!("{n:064x}"), model, &|| {
            false
        })
        .unwrap()
    };
    let save = |n: usize, model: &str| {
        let mut entry = acquire(n, model);
        fs::write(entry.path.join("weights"), [1; 600]).unwrap();
        entry.complete(&|| false).unwrap();
        entry.path.clone()
    };
    // However large, and however long unused, a prepared model survives
    // previews filling the limit, even a zero one.
    let old = save(1, "aesthetic");
    let other = save(2, "dinov2");
    let preview = root.join(format!("{:064x}.bin", 3));
    fs::write(&preview, [7; 200]).unwrap();
    crate::core::cache::maintain_keeping_models(&root, Some(0), || false).unwrap();
    assert!(!preview.exists(), "previews still honour the disk limit");
    assert!(acquire(1, "aesthetic").reused);
    // An update gives the model a new copy: the old one goes, other models stay.
    let legacy = root.join(format!("{:064x}.coreml", 4));
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join(".owner"), OWNER).unwrap();
    let held = acquire(5, "aesthetic");
    let new = save(6, "aesthetic");
    assert!(!old.exists() && !legacy.exists());
    assert!(new.exists() && other.exists());
    assert!(held.path.exists(), "a copy another window has open stays");
}
// Invoked in isolated test processes by the lifetime regression below.
#[test]
fn cache_lease_process_probe() {
    let Some(root) = std::env::var_os("PHOTOSORT_TEST_COMPILED_CACHE") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("PHOTOSORT_TEST_COMPILED_MODE").unwrap();
    if mode == "held" {
        let result = crate::core::cache::maintain(&root, Some(0), || false).unwrap();
        assert!(result.disk_bytes > 0 && result.pinned_disk_bytes == result.disk_bytes);
        assert!(
            Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
                false
            })
            .is_err()
        );
    } else {
        let entry = Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), "m", &|| {
            false
        })
        .unwrap();
        assert!(entry.reused);
        if mode == "crash" {
            // Bypass Rust drops to verify that OS-owned locks do not strand entries.
            std::process::exit(0);
        }
    }
}
#[test]
fn another_process_cannot_prune_a_live_model_and_exit_releases_its_lease() {
    let (_temp, root) = root();
    let entry = ready(&root, 1, b"native weights");
    let probe = |mode: &str| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "core::cache::compiled::writer::tests::cache_lease_process_probe",
                "--nocapture",
            ])
            .env("PHOTOSORT_TEST_COMPILED_CACHE", &root)
            .env("PHOTOSORT_TEST_COMPILED_MODE", mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    probe("held");
    assert!(entry.path.exists());
    drop(entry);
    probe("crash");
    probe("released");
    assert_eq!(
        crate::core::cache::maintain(&root, Some(0), || false)
            .unwrap()
            .disk_bytes,
        0
    );
}
