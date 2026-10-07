use photosort::core::{
    model_assets::{self, ALL},
    startup::Options,
};
use std::{cell::Cell, collections::HashSet, ffi::OsString};
fn options(args: &[&str]) -> Result<Options, String> {
    Options::parse(args.iter().map(OsString::from))
}
fn fixture() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("models")).unwrap();
    for asset in ALL {
        std::fs::write(temp.path().join("models").join(asset.name), b"managed").unwrap();
    }
    temp
}
#[test]
fn startup_diagnostics_are_exclusive_and_model_clearing_composes_with_normal_startup() {
    assert!(options(&["--diagnostics"]).unwrap().diagnostics);
    let o = options(&["--clear-models", "--clear-cache", "--last-folder"]).unwrap();
    assert!(o.clear_models && o.clear_cache && o.last_folder);
    for option in [
        "--clear-models",
        "--clear-cache",
        "--last-folder",
        "/photos",
    ] {
        assert!(options(&["--diagnostics", option]).is_err());
        assert!(options(&[option, "--diagnostics"]).is_err());
    }
    assert!(options(&["--clear-models", "--help"]).unwrap().help);
    assert!(options(&["--diagnostics", "--version"]).unwrap().version);
    assert_eq!(
        options(&["--", "--clear-models"]).unwrap().folder,
        Some("--clear-models".into())
    );
}
#[test]
fn clearing_uses_catalog_names_only_and_preserves_every_other_resource() {
    let managed = fixture();
    let bundled = fixture();
    let custom = managed.path().join("models/custom.onnx");
    std::fs::write(&custom, b"custom").unwrap();
    std::fs::create_dir(managed.path().join("models/private")).unwrap();
    std::fs::write(managed.path().join("models/private/a.onnx"), b"private").unwrap();
    std::fs::write(managed.path().join("runtime.dll"), b"runtime").unwrap();
    let report = model_assets::clear_managed(managed.path(), &|| false).unwrap();
    assert!(!report.cancelled && report.errors.is_empty());
    assert_eq!(report.bytes, 7 * ALL.len() as u64);
    assert_eq!(report.removed.len(), ALL.len());
    assert_eq!(
        report
            .removed
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<HashSet<_>>(),
        ALL.iter().map(|a| a.name.to_owned()).collect()
    );
    assert_eq!(std::fs::read(custom).unwrap(), b"custom");
    assert_eq!(
        std::fs::read(managed.path().join("models/private/a.onnx")).unwrap(),
        b"private"
    );
    assert_eq!(
        std::fs::read(managed.path().join("runtime.dll")).unwrap(),
        b"runtime"
    );
    for asset in ALL {
        assert_eq!(
            std::fs::read(bundled.path().join("models").join(asset.name)).unwrap(),
            b"managed"
        );
    }
    assert!(
        model_assets::clear_managed(managed.path(), &|| false)
            .unwrap()
            .removed
            .is_empty()
    );
    let missing = managed.path().join("absent");
    assert!(
        model_assets::clear_managed(&missing, &|| false)
            .unwrap()
            .removed
            .is_empty()
    );
    assert!(!missing.exists());
}
#[test]
fn cancelled_clear_reports_completed_work_and_retry_only_removes_remaining_files() {
    let temp = fixture();
    assert!(model_assets::clear_managed(temp.path(), &|| true).is_err());
    let checks = Cell::new(0);
    let report = model_assets::clear_managed(temp.path(), &|| {
        checks.set(checks.get() + 1);
        checks.get() >= 9
    })
    .unwrap();
    assert!(report.cancelled && report.errors.is_empty());
    assert_eq!(report.removed.len(), 1);
    let retry = model_assets::clear_managed(temp.path(), &|| false).unwrap();
    assert_eq!(retry.removed.len(), ALL.len() - 1);
}
#[test]
fn clearing_rechecks_reviewed_files_and_reports_partial_failure() {
    let temp = fixture();
    let changed = temp.path().join("models").join(ALL[0].name);
    let checks = Cell::new(0);
    let report = model_assets::clear_managed(temp.path(), &|| {
        checks.set(checks.get() + 1);
        if checks.get() == 8 {
            std::fs::write(&changed, b"replacement").unwrap();
        }
        false
    })
    .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.removed.len(), ALL.len() - 1);
    assert_eq!(std::fs::read(changed).unwrap(), b"replacement");
}
#[cfg(unix)]
#[test]
fn linked_models_or_directories_fail_preflight_before_any_removal() {
    let temp = fixture();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("untouched");
    std::fs::write(&target, b"outside").unwrap();
    let model = temp.path().join("models").join(ALL.last().unwrap().name);
    std::fs::remove_file(&model).unwrap();
    std::os::unix::fs::symlink(&target, &model).unwrap();
    assert!(model_assets::clear_managed(temp.path(), &|| false).is_err());
    assert!(temp.path().join("models").join(ALL[0].name).is_file());
    assert_eq!(std::fs::read(&target).unwrap(), b"outside");
    let linked = outside.path().join("linked-root");
    std::os::unix::fs::symlink(temp.path(), &linked).unwrap();
    assert!(model_assets::clear_managed(&linked, &|| false).is_err());
    let root = outside.path().join("root");
    std::fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(temp.path().join("models"), root.join("models")).unwrap();
    assert!(model_assets::clear_managed(&root, &|| false).is_err());
}

#[test]
fn runtime_checks_are_standalone_and_cannot_clear_files_or_open_folders() {
    assert!(options(&["--check-runtime"]).unwrap().check_runtime);
    for option in [
        "--clear-models",
        "--clear-cache",
        "--last-folder",
        "/photos",
        "--diagnostics",
    ] {
        assert!(options(&["--check-runtime", option]).is_err());
        assert!(options(&[option, "--check-runtime"]).is_err());
    }
    assert!(options(&["--check-runtime", "--help"]).unwrap().help);
    assert_eq!(
        options(&["--", "--check-runtime"]).unwrap().folder,
        Some("--check-runtime".into())
    );
}
