//! Shared by unit tests (`lib.rs`, `main.rs`) and integration tests (`#[path]`).
//! Each test crate uses only part of it.
#![allow(dead_code)]

use std::path::Path;
use std::time::{Duration, Instant};

/// How long a test waits for a worker before calling it hung. Waits end as soon
/// as the worker reports, so this only bounds a failure. On Windows each file
/// replacement may retry for up to 5 s (`core::replace`), and shared CI runners
/// can pause every thread for seconds, so CI gets a much longer bound.
pub fn worker_patience() -> Duration {
    Duration::from_secs(if std::env::var_os("CI").is_some() {
        120
    } else {
        15
    })
}

/// Puts another file holding `contents` in `path`'s place and gives it the
/// old modified time, as a tool that restores dates does. The new file is
/// born at a later moment, as a real replacement is: Linux stamps new files
/// with a clock that moves only every few milliseconds, so one written
/// straight after the original shares its birth time, and nothing but the
/// inode, which a volume may number anew on every mount, tells the two
/// apart. A volume that keeps no birth time has nothing to wait for.
pub fn replace_keeping_modified_time(path: &Path, contents: &[u8]) {
    let old = std::fs::metadata(path).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut attempt = 0;
    let replacement = loop {
        // A new name each time: writing to the same file again keeps its
        // birth time, and Windows hands a file made under a name that was
        // just freed the creation time of the one that had it.
        let candidate = path.with_extension(format!("replacement-{attempt}"));
        std::fs::write(&candidate, contents).unwrap();
        let born = std::fs::metadata(&candidate).unwrap().created().ok();
        if born.is_none() || born != old.created().ok() {
            break candidate;
        }
        std::fs::remove_file(&candidate).unwrap();
        assert!(
            Instant::now() < deadline,
            "A new file never got a later birth time"
        );
        std::thread::sleep(Duration::from_millis(1));
        attempt += 1;
    };
    std::fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old.modified().unwrap()))
        .unwrap();
    std::fs::rename(&replacement, path).unwrap();
}
