//! Replacing a file atomically when other threads may be replacing it too.
//!
//! On Windows, replacing or opening a file fails with "Access is denied" for a
//! moment while another thread is replacing the same file. Writers here race
//! by design (several workers publish the same cache record; edits land on
//! one sidecar), so on Windows they wait briefly and try again. Elsewhere the
//! first error is final.
use std::{
    io,
    path::Path,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;

/// How long a Windows writer keeps retrying before the error stands.
const PATIENCE: Duration = Duration::from_secs(2);

/// Publishes `temp` at `dest`, replacing any existing file.
pub(crate) fn persist(mut temp: NamedTempFile, dest: &Path) -> io::Result<()> {
    let started = Instant::now();
    let mut attempt = 0;
    loop {
        match temp.persist(dest) {
            Ok(_) => return Ok(()),
            // A directory in the way is a real refusal, not a race.
            Err(error) if !dest.is_dir() && busy(&error.error, started, &mut attempt) => {
                temp = error.file
            }
            Err(error) => return Err(error.error),
        }
    }
}

/// Opens `path` for reading, waiting out a concurrent replacement on Windows.
pub(crate) fn open(path: &Path) -> io::Result<std::fs::File> {
    let started = Instant::now();
    let mut attempt = 0;
    loop {
        match std::fs::File::open(path) {
            Err(error) if busy(&error, started, &mut attempt) => {}
            result => return result,
        }
    }
}

/// True (after a short back-off) when `error` is Windows reporting a
/// replacement in progress and there is still time to retry.
fn busy(error: &io::Error, started: Instant, attempt: &mut u64) -> bool {
    if !cfg!(windows)
        || error.kind() != io::ErrorKind::PermissionDenied
        || started.elapsed() >= PATIENCE
    {
        return false;
    }
    *attempt += 1;
    std::thread::sleep(Duration::from_millis((*attempt).min(10)));
    true
}
