//! The panic hook writes a report and flags background panics. It runs in its
//! own test process because the hook is global.
use photosort::core::crash;

#[test]
fn background_panic_writes_a_report_and_is_reported_once() {
    let dir = tempfile::tempdir().unwrap();
    crash::install(dir.path().to_path_buf());
    assert!(crash::background_panic().is_none());
    // A panic inside per-item work is that item's error: kept as a report for
    // diagnosis, but no restart notice now and no crash notice next launch.
    let item = std::thread::Builder::new()
        .name("metadata-index".into())
        .spawn(|| crash::catch(|| -> u32 { panic!("bad header") }))
        .unwrap()
        .join()
        .unwrap();
    assert!(item.unwrap_err().contains("bad header"));
    assert!(crash::background_panic().is_none());
    assert!(crash::unseen_report(dir.path()).is_none());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    assert_eq!(crash::catch(|| 7).unwrap(), 7);
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
