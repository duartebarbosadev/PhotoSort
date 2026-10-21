use photosort::core::{
    model_assets::{self, ALL},
    startup::Options,
};
use std::{cell::Cell, collections::HashSet, ffi::OsString};
fn options(args: &[&str]) -> Result<Options, String> {
    Options::parse(args.iter().map(OsString::from))
}
#[test]
fn session_logging_is_explicit_validated_and_composes_with_diagnostics() {
    let parsed = options(&[
        "--log-file",
        "/tmp/session.log",
        "--log-level",
        "debug",
        "--diagnostics",
    ])
    .unwrap();
    assert!(parsed.diagnostics);
    assert_eq!(parsed.log_file, Some("/tmp/session.log".into()));
    assert_eq!(parsed.log_level, Some(tracing::Level::DEBUG));
    assert!(options(&["--log-file", "session.log", "--check-runtime"]).is_ok());
    assert!(options(&["--log-file", "session.log", "--folder", "/photos"]).is_ok());
    for args in [
        vec!["--log-file"],
        vec!["--log-file", ""],
        vec!["--log-file", "--diagnostics"],
        vec!["--log-file", "a", "--log-file", "b"],
        vec!["--log-level", "debug"],
        vec!["--log-file", "a", "--log-level"],
        vec!["--log-file", "a", "--log-level", "verbose"],
        vec![
            "--log-file",
            "a",
            "--log-level",
            "info",
            "--log-level",
            "debug",
        ],
    ] {
        assert!(options(&args).is_err(), "{args:?}");
    }
    assert!(options(&[]).unwrap().log_file.is_none());
    assert_eq!(
        options(&["--", "--log-file"]).unwrap().folder,
        Some("--log-file".into())
    );
}
#[test]
fn binary_logging_preserves_json_stdout_and_never_replaces_files() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("session.log");
    let binary = env!("CARGO_BIN_EXE_photosort");
    let help = Command::new(binary)
        .args(["--log-file"])
        .arg(&path)
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(!path.exists());
    let diagnostic = Command::new(binary)
        .arg("--diagnostics")
        .arg("--log-file")
        .arg(&path)
        .output()
        .unwrap();
    // Readiness may be false in a source checkout; stdout must still be pure JSON.
    let report: serde_json::Value = serde_json::from_slice(&diagnostic.stdout).unwrap();
    assert!(report.is_object());
    let log = std::fs::read_to_string(&path).unwrap();
    assert!(log.contains("application started"));
    assert!(log.contains("application stopped") || log.contains("application failed"));
    let repeated = Command::new(binary)
        .arg("--diagnostics")
        .arg("--log-file")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), log);
    #[cfg(unix)]
    {
        let link = temp.path().join("linked.log");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        let result = Command::new(binary)
            .arg("--diagnostics")
            .arg("--log-file")
            .arg(link)
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert_eq!(std::fs::read_to_string(path).unwrap(), log);
    }
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
    // One check up front and one per catalogued model before any removal.
    let preflight = 1 + ALL.len();
    let report = model_assets::clear_managed(temp.path(), &|| {
        checks.set(checks.get() + 1);
        checks.get() >= preflight + 2
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
        if checks.get() == 2 + ALL.len() {
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

#[test]
fn runtime_backend_selection_is_explicit_validated_and_never_changes_startup_preferences() {
    use photosort::core::processing::Backend;
    assert_eq!(options(&["--check-runtime"]).unwrap().runtime_backend, None);
    for (name, backend) in [
        ("cpu", Backend::Cpu),
        ("auto", Backend::Auto),
        ("coreml", Backend::CoreMl),
        ("cuda", Backend::Cuda),
        ("directml", Backend::DirectMl),
    ] {
        for args in [
            vec!["--check-runtime", "--runtime-backend", name],
            vec!["--runtime-backend", name, "--check-runtime"],
        ] {
            let parsed = options(&args).unwrap();
            assert!(parsed.check_runtime);
            assert_eq!(parsed.runtime_backend, Some(backend));
            assert!(!parsed.clear_cache && !parsed.clear_models && parsed.folder.is_none());
        }
        for other in [
            "--diagnostics",
            "--clear-models",
            "--clear-cache",
            "--smoke-test",
            "--last-folder",
            "/photos",
        ] {
            assert!(options(&["--runtime-backend", name, other]).is_err());
            assert!(options(&["--check-runtime", "--runtime-backend", name, other]).is_err());
        }
        assert!(options(&["--runtime-backend", name]).is_err());
    }
    for args in [
        vec!["--check-runtime", "--runtime-backend"],
        vec!["--check-runtime", "--runtime-backend", "metal"],
        vec!["--check-runtime", "--runtime-backend", ""],
        vec![
            "--check-runtime",
            "--runtime-backend",
            "cpu",
            "--runtime-backend",
            "cuda",
        ],
    ] {
        assert!(options(&args).is_err(), "{args:?}");
    }
    assert_eq!(
        options(&["--", "--runtime-backend"]).unwrap().folder,
        Some("--runtime-backend".into())
    );
}
