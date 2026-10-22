//! The executable owns logging; reusable core services only emit events.
use std::{
    fs::OpenOptions,
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
};
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::{filter::Targets, fmt::MakeWriter, prelude::*};

const FILE_LIMIT: usize = 16 * 1024 * 1024;
const RECORD_LIMIT: usize = 8 * 1024;
const QUEUE_LIMIT: usize = 256;
const FILE_END: &[u8] = b"\nPhotoSort: log size limit reached; further events omitted.\n";
const RECORD_END: &[u8] = b" [event truncated]\n";

pub struct Guard {
    worker: Option<WorkerGuard>,
    dropped: ErrorCounter,
    state: Arc<AtomicU8>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        // Drain while the file still exists; this guard outlives all app workers.
        drop(self.worker.take());
        let dropped = self.dropped.dropped_lines();
        if dropped != 0 {
            eprintln!(
                "PhotoSort: {dropped} diagnostic events omitted because the log queue was full."
            );
        }
        match self.state.load(Ordering::Relaxed) {
            1 => eprintln!("PhotoSort: diagnostic log reached its 16 MiB limit."),
            2 => eprintln!("PhotoSort: diagnostic log could not be written completely."),
            _ => {}
        }
    }
}

pub fn start(path: Option<&Path>, level: Option<tracing::Level>) -> anyhow::Result<Option<Guard>> {
    let Some(path) = path else {
        // Without a log file, warnings and errors still reach the terminal.
        let console = tracing_subscriber::registry()
            .with(
                Targets::new()
                    .with_target("photosort", tracing::Level::WARN)
                    .with_target("photosort_imaging_vips", tracing::Level::WARN),
            )
            .with(
                tracing_subscriber::fmt::layer()
                    .with_thread_names(true)
                    .with_writer(io::stderr),
            );
        tracing::dispatcher::set_global_default(tracing::Dispatch::new(console))?;
        return Ok(None);
    };
    let (dispatch, guard) = prepare(path, level.unwrap_or(tracing::Level::INFO))?;
    tracing::dispatcher::set_global_default(dispatch)?;
    eprintln!(
        "PhotoSort: session diagnostics: {} (16 MiB maximum)",
        path.display()
    );
    Ok(Some(guard))
}

fn prepare(path: &Path, level: tracing::Level) -> anyhow::Result<(tracing::Dispatch, Guard)> {
    // Explicit destination only: never append, replace, or follow an existing link.
    // A missing parent is an actionable startup error, not an implicit mkdir.
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    let state = Arc::new(AtomicU8::new(0));
    let capped = Capped {
        writer: file,
        remaining: FILE_LIMIT,
        state: state.clone(),
    };
    let (writer, worker) = background(capped);
    let dropped = writer.error_counter();
    let subscriber = tracing_subscriber::registry()
        .with(
            Targets::new()
                .with_target("photosort", level)
                .with_target("photosort_imaging_vips", level),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_thread_names(true)
                .with_writer(Records(writer)),
        );
    Ok((
        tracing::Dispatch::new(subscriber),
        Guard {
            worker: Some(worker),
            dropped,
            state,
        },
    ))
}

fn background(writer: impl Write + Send + 'static) -> (NonBlocking, WorkerGuard) {
    NonBlockingBuilder::default()
        .thread_name("photosort-log")
        .buffered_lines_limit(QUEUE_LIMIT)
        .lossy(true)
        .finish(writer)
}

// Cap records before the library queue: at most 2 MiB of queued record bytes.
#[derive(Clone)]
struct Records(NonBlocking);
impl<'a> MakeWriter<'a> for Records {
    type Writer = Record;
    fn make_writer(&'a self) -> Record {
        Record {
            writer: self.0.clone(),
            bytes: Vec::new(),
            truncated: false,
        }
    }
}
struct Record {
    writer: NonBlocking,
    bytes: Vec<u8>,
    truncated: bool,
}
impl Write for Record {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let available = (RECORD_LIMIT - RECORD_END.len()).saturating_sub(self.bytes.len());
        let count = available.min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        self.truncated |= count < bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Drop for Record {
    fn drop(&mut self) {
        if self.truncated {
            if let Err(error) = std::str::from_utf8(&self.bytes) {
                self.bytes.truncate(error.valid_up_to());
            }
            self.bytes.extend_from_slice(RECORD_END);
        }
        let _ = self.writer.write_all(&self.bytes);
    }
}

struct Capped<W> {
    writer: W,
    remaining: usize,
    // 0 writing, 1 capped, 2 I/O error. Read only after worker shutdown.
    state: Arc<AtomicU8>,
}
impl<W: Write> Write for Capped<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.state.load(Ordering::Relaxed) != 0 {
            return Ok(bytes.len());
        }
        let output = if bytes.len() > self.remaining.saturating_sub(FILE_END.len()) {
            self.state.store(1, Ordering::Relaxed);
            FILE_END
        } else {
            bytes
        };
        match self.writer.write_all(output) {
            Ok(()) => self.remaining -= output.len(),
            Err(_) => self.state.store(2, Ordering::Relaxed),
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.writer.flush().is_err() {
            self.state.store(2, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::worker_patience;

    #[test]
    fn opt_in_preserves_existing_files_and_flushes_filtered_utf8_records() {
        let temp = tempfile::tempdir().unwrap();
        assert!(start(None, None).unwrap().is_none());
        assert_eq!(temp.path().read_dir().unwrap().count(), 0);
        let path = temp.path().join("session.log");
        let (dispatch, guard) = prepare(&path, tracing::Level::DEBUG).unwrap();
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!("session fixture");
            tracing::debug!("debug fixture");
            tracing::trace!("hidden trace fixture");
            tracing::info!(target: "unrelated", "hidden foreign fixture");
            tracing::warn!(detail = "é".repeat(RECORD_LIMIT), "large fixture");
        });
        drop(guard);
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(before.contains("session fixture") && before.contains("debug fixture"));
        assert!(!before.contains("hidden"));
        assert!(before.contains("[event truncated]"));
        assert!(before.lines().all(|line| line.len() < RECORD_LIMIT));
        assert!(prepare(&path, tracing::Level::INFO).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert!(
            prepare(
                &temp.path().join("missing/session.log"),
                tracing::Level::INFO
            )
            .is_err()
        );
        assert!(!temp.path().join("missing").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn disk_cap_preserves_whole_records_and_reports_write_failure() {
        let state = Arc::new(AtomicU8::new(0));
        let mut writer = Capped {
            writer: Vec::new(),
            remaining: 128,
            state: state.clone(),
        };
        writer.write_all(b"first record\n").unwrap();
        writer.write_all(&[b'x'; 128]).unwrap();
        writer.write_all(b"after cap\n").unwrap();
        assert_eq!(state.load(Ordering::Relaxed), 1);
        assert_eq!(
            writer.writer,
            [b"first record\n".as_slice(), FILE_END].concat()
        );
        assert!(writer.writer.len() <= 128);
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk failed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("disk failed"))
            }
        }
        state.store(0, Ordering::Relaxed);
        let mut broken = Capped {
            writer: Broken,
            remaining: 128,
            state: state.clone(),
        };
        broken.write_all(b"test").unwrap();
        broken.flush().unwrap();
        assert_eq!(state.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn blocked_disk_drops_at_queue_limit_without_blocking_producers() {
        use std::sync::mpsc;
        let (entered, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        struct Blocked {
            entered: mpsc::Sender<()>,
            wait: mpsc::Receiver<()>,
            first: bool,
        }
        impl Write for Blocked {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.first {
                    self.first = false;
                    self.entered.send(()).unwrap();
                    self.wait.recv().unwrap();
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (mut writer, guard) = background(Blocked {
            entered,
            wait,
            first: true,
        });
        let errors = writer.error_counter();
        writer.write_all(b"first").unwrap();
        ready.recv_timeout(worker_patience()).unwrap();
        // A separate producer gives the test a deadline if backpressure regresses.
        let producer = std::thread::spawn(move || {
            for _ in 0..QUEUE_LIMIT + 17 {
                writer.write_all(b"event").unwrap();
            }
        });
        let deadline = std::time::Instant::now() + worker_patience();
        while !producer.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let finished_without_disk = producer.is_finished();
        let dropped = errors.dropped_lines();
        release.send(()).unwrap();
        producer.join().unwrap();
        drop(guard);
        assert!(finished_without_disk);
        assert_eq!(dropped, 17);
    }
}
