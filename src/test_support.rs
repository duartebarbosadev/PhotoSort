//! Shared by unit tests (`lib.rs`, `main.rs`) and integration tests (`#[path]`).
//! An integration test crate may use it only under some features.
#![allow(dead_code)]

use std::time::Duration;

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
