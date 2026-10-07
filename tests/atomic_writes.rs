use photosort::core::{metadata, operations};
use std::{fs, sync::Barrier, thread};

#[test]
fn concurrent_atomic_writers_never_steal_or_remove_each_others_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let barrier = Barrier::new(24);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..24)
            .map(|n| {
                let barrier = &barrier;
                let root = root.path();
                scope.spawn(move || {
                    let data = vec![n as u8; 4096];
                    barrier.wait();
                    for _ in 0..8 {
                        metadata::atomic_write(&root.join(format!("{n}.json")), &data).unwrap();
                        metadata::atomic_write(&root.join("shared.json"), &data).unwrap();
                    }
                    assert_eq!(fs::read(root.join(format!("{n}.json"))).unwrap(), data);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let shared = fs::read(root.path().join("shared.json")).unwrap();
    assert_eq!(shared.len(), 4096);
    assert!(shared.iter().all(|byte| *byte == shared[0]));
    assert_eq!(
        fs::read_dir(root.path()).unwrap().count(),
        25,
        "Temporary files leaked"
    );
}

#[test]
fn simultaneous_operation_journals_are_unique_and_retained() {
    let root = tempfile::tempdir().unwrap();
    let barrier = Barrier::new(24);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..24)
            .map(|_| {
                let barrier = &barrier;
                let root = root.path();
                scope.spawn(move || {
                    barrier.wait();
                    let report = operations::execute(
                        &operations::Plan::default(),
                        root,
                        || false,
                        |_, _| {},
                    )
                    .unwrap();
                    assert!(report.errors.is_empty());
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    let journals: Vec<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(journals.len(), 24);
    assert!(journals.iter().all(|p| p.extension().unwrap() == "jsonl"));
}
