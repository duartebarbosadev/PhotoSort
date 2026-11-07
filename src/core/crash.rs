//! Crash reports. A panic anywhere writes a plain-text report under the data
//! directory before the default hook runs. The window shows a readable notice:
//! for a background panic during the session, or for a report left by a crash
//! in the previous session. Reports hold no photo pixels, only the panic
//! message, its location, the thread name and a backtrace.
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const PREFIX: &str = "crash-";
const UNSEEN: &str = ".txt";
const SEEN: &str = ".seen.txt";
/// Older reports beyond this count are removed when a new one is written.
const KEEP: usize = 10;

static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
static BACKGROUND: Mutex<Option<PathBuf>> = Mutex::new(None);

thread_local! {
    /// Set while [`catch`] runs per-item work that recovers from a panic.
    static RECOVERABLE: Cell<bool> = const { Cell::new(false) };
}

/// Runs work for one item (a file, a photo). A panic becomes `Err` with a
/// readable message instead of ending the worker; its report is kept for
/// diagnosis but does not raise the restart notice.
pub fn catch<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    let outer = RECOVERABLE.with(|flag| flag.replace(true));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work));
    RECOVERABLE.with(|flag| flag.set(outer));
    result.map_err(|payload| {
        let detail = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        format!("PhotoSort hit an internal error with this file and skipped it ({detail}). A report was saved.")
    })
}

/// Wraps work for a thread that the current thread starts and joins, so a
/// panic there counts as recoverable exactly when it does here: inside
/// [`catch`], the joining thread turns it into the item's error.
pub fn inherit<T>(work: impl FnOnce() -> T + Send) -> impl FnOnce() -> T + Send {
    let recoverable = RECOVERABLE.with(Cell::get);
    move || {
        RECOVERABLE.with(|flag| flag.set(recoverable));
        work()
    }
}

/// Where reports are kept: `crashes` inside the data directory.
pub fn directory() -> PathBuf {
    super::session::data_dir().join("crashes")
}

/// Installs the panic hook once. Later calls are ignored.
pub fn install(directory: PathBuf) {
    if DIRECTORY.set(directory).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed").to_owned();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-text panic payload".into());
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        tracing::error!(thread = %name, %location, %message, "panic");
        let recoverable = RECOVERABLE.with(Cell::get);
        if let Some(directory) = DIRECTORY.get()
            && let Ok(path) = write(
                directory,
                &name,
                &message,
                &location,
                &backtrace,
                // Recovered item panics are kept for diagnosis but never
                // reported as a crash on the next launch.
                if recoverable { SEEN } else { UNSEEN },
            )
            && name != "main"
            && !recoverable
            && let Ok(mut slot) = BACKGROUND.lock()
        {
            slot.get_or_insert(path);
        }
        previous(info);
    }));
}

/// The report for the first background-thread panic in this session, if any.
pub fn background_panic() -> Option<PathBuf> {
    BACKGROUND.lock().ok().and_then(|slot| slot.clone())
}

/// Writes one report and prunes old ones. Never overwrites an existing file.
pub fn write_report(
    directory: &Path,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: &str,
) -> std::io::Result<PathBuf> {
    write(directory, thread, message, location, backtrace, UNSEEN)
}

fn write(
    directory: &Path,
    thread: &str,
    message: &str,
    location: &str,
    backtrace: &str,
    suffix: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(directory)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    // Several items can fail within one second; the sequence keeps each report.
    static SEQUENCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = directory.join(format!(
        "{PREFIX}{stamp}-{}-{sequence:06}{suffix}",
        std::process::id()
    ));
    let report = format!(
        "PhotoSort {}\nOS: {} {}\nThread: {thread}\nLocation: {location}\nMessage: {message}\n\nBacktrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    std::io::Write::write_all(&mut file, report.as_bytes())?;
    prune(directory);
    Ok(path)
}

fn reports(directory: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(UNSEEN))
        })
        .collect();
    // Timestamped names sort chronologically.
    found.sort();
    found
}

/// Seen and unseen reports are kept apart, so a burst of recovered item
/// panics can never prune a crash report the user has not been told about.
fn prune(directory: &Path) {
    let (seen, unseen): (Vec<_>, Vec<_>) = reports(directory)
        .into_iter()
        .partition(|path| path.to_string_lossy().ends_with(SEEN));
    for found in [seen, unseen] {
        for old in found.iter().take(found.len().saturating_sub(KEEP)) {
            let _ = std::fs::remove_file(old);
        }
    }
}

/// The newest report the user has not been told about yet.
pub fn unseen_report(directory: &Path) -> Option<PathBuf> {
    reports(directory)
        .into_iter()
        .rev()
        .find(|path| !path.to_string_lossy().ends_with(SEEN))
}

/// Marks every unseen report as seen so the notice appears once per crash.
/// Returns the new path of `report` so it can still be revealed.
pub fn mark_seen(directory: &Path, report: &Path) -> PathBuf {
    let mut renamed = report.to_path_buf();
    for path in reports(directory) {
        let name = path.to_string_lossy();
        if name.ends_with(SEEN) {
            continue;
        }
        let target = PathBuf::from(format!("{}{SEEN}", &name[..name.len() - UNSEEN.len()]));
        if std::fs::rename(&path, &target).is_ok() && path == report {
            renamed = target;
        }
    }
    renamed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn started_threads_inherit_recoverability() {
        let on_thread = |work: fn() -> bool| std::thread::scope(|s| s.spawn(inherit(work)).join());
        let recoverable = || RECOVERABLE.with(Cell::get);
        assert_eq!(on_thread(recoverable).ok(), Some(false));
        assert_eq!(catch(|| on_thread(recoverable).ok()), Ok(Some(true)));
    }

    #[test]
    fn reports_are_written_found_once_and_pruned() {
        let dir = tempfile::tempdir().unwrap();
        assert!(unseen_report(dir.path()).is_none());
        let first = write_report(dir.path(), "worker", "boom", "src/x.rs:1:1", "frames").unwrap();
        let text = std::fs::read_to_string(&first).unwrap();
        assert!(text.contains("Message: boom") && text.contains("Thread: worker"));
        assert_eq!(unseen_report(dir.path()).as_deref(), Some(first.as_path()));
        let seen = mark_seen(dir.path(), &first);
        assert!(seen.exists() && !first.exists());
        assert!(unseen_report(dir.path()).is_none());
        for i in 0..KEEP + 3 {
            std::fs::write(dir.path().join(format!("{PREFIX}2000{i:04}{UNSEEN}")), "x").unwrap();
        }
        prune(dir.path());
        assert_eq!(reports(dir.path()).len(), KEEP + 1);
        // Recovered item panics fill the seen pile, never the unseen one.
        for _ in 0..KEEP + 3 {
            write(dir.path(), "worker", "item", "", "", SEEN).unwrap();
        }
        let unseen = unseen_report(dir.path()).unwrap();
        assert!(unseen.ends_with(format!("{PREFIX}2000{:04}{UNSEEN}", KEEP + 2)));
        assert_eq!(reports(dir.path()).len(), 2 * KEEP);
    }
}
