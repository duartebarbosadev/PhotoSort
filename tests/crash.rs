//! The panic hook writes a report and flags background panics. It runs in its
//! own test process because the hook is global.
use photosort::core::crash;

#[test]
fn background_panic_writes_a_report_and_is_reported_once() {
    let dir = tempfile::tempdir().unwrap();
    crash::install(dir.path().to_path_buf());
    assert!(crash::background_panic().is_none());
    let worker = std::thread::Builder::new()
        .name("analysis-worker".into())
        .spawn(|| panic!("model output had the wrong shape"))
        .unwrap();
    assert!(worker.join().is_err());
    let report = crash::background_panic().expect("background panic recorded");
    let text = std::fs::read_to_string(&report).unwrap();
    assert!(text.contains("Thread: analysis-worker"), "{text}");
    assert!(
        text.contains("Message: model output had the wrong shape"),
        "{text}"
    );
    assert_eq!(crash::unseen_report(dir.path()), Some(report.clone()));
    let seen = crash::mark_seen(dir.path(), &report);
    assert!(seen.exists());
    assert!(crash::unseen_report(dir.path()).is_none());
}
