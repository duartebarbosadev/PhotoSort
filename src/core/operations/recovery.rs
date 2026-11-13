//! Durable, per-library recovery on the shared filesystem worker. Reopening
//! restores intent and verified identities only; file operations require review.
use super::{Plan, Report, execute_logged, reject_symlink_components};
mod amendment;
pub(super) mod moves;
use crate::core::{metadata, model::*, session::Session, views};
use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

// Large accepted plans stream through Serde rather than a second whole-record
// input buffer. Enforce the same header budget before any file action begins.
const MAX_HEADER: u64 = 512 * 1024 * 1024;
const MAX_RECORD: u64 = MAX_HEADER * 4;
/// Marks a journal or index that was set aside as unreadable, in its name:
/// `operations-abc.damaged-<time>.jsonl`. Such a file is never read again
/// and never removed by PhotoSort.
const DAMAGED: &str = "damaged-";
/// A journal no unfinished plan needs is kept this long, for support.
const KEEP_FINISHED: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
/// Such journals are kept, newest first, until they hold this much. A large
/// library writes hundreds of megabytes per Apply; the newest always stays.
const KEEP_FINISHED_BYTES: u64 = 256 * 1024 * 1024;
/// A journal written this recently is never removed: its operation may not
/// have published the index that names it yet.
const SETTLED_AFTER: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// Attached to an error that proves a journal or its index can never be
/// read: the journal is gone, or what the file holds does not parse or does
/// not add up. Only such a record is set aside when a library is opened. A
/// failure without it (an I/O or permission error, a record written by a
/// newer PhotoSort, a cancelled read) may pass, so the record stays.
#[derive(Debug)]
struct Damaged;
impl std::fmt::Display for Damaged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The record of the operation is damaged")
    }
}
impl std::error::Error for Damaged {}
/// `ensure!` for a check on what a journal or index holds: failing it is
/// proof of damage.
macro_rules! valid {
    ($condition:expr, $($message:tt)+) => {
        if !$condition {
            return Err(anyhow::anyhow!($($message)+).context(Damaged));
        }
    };
}
/// A journal that is not there cannot become readable again. Any other error
/// opening it might.
fn missing_is_damage(error: std::io::Error) -> anyhow::Error {
    match error.kind() {
        std::io::ErrorKind::NotFound => anyhow::Error::new(error).context(Damaged),
        _ => error.into(),
    }
}
struct Cancellable<'a> {
    file: File,
    cancelled: &'a dyn Fn() -> bool,
}
impl Read for Cancellable<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(std::io::Error::other("Recovery read cancelled"));
        }
        self.file.read(out)
    }
}
impl Write for Cancellable<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if (self.cancelled)() {
            return Err(std::io::Error::other("Recovery write cancelled"));
        }
        self.file.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
impl Seek for Cancellable<'_> {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}
struct Records<'a> {
    reader: BufReader<Cancellable<'a>>,
    offset: u64,
    bytes: u64,
}
impl Records<'_> {
    fn next<T: DeserializeOwned>(&mut self, limit: u64) -> Result<Option<T>> {
        ensure!(
            !(self.reader.get_ref().cancelled)(),
            "Recovery read cancelled"
        );
        self.reader.seek(SeekFrom::Start(self.offset))?;
        let mut stream =
            serde_json::Deserializer::from_reader(self.reader.by_ref().take(limit + 1))
                .into_iter::<T>();
        let item = stream.next();
        let used = stream.byte_offset() as u64;
        drop(stream);
        let value = match item {
            None => {
                valid!(
                    self.bytes.saturating_sub(self.offset) <= limit,
                    "Recovery record exceeds the supported size"
                );
                return Ok(None);
            }
            Some(Err(error)) if error.is_eof() => {
                valid!(
                    self.bytes.saturating_sub(self.offset) <= limit,
                    "Recovery record exceeds the supported size"
                );
                return Ok(None); // A torn final record cannot prove completion.
            }
            // The disk failing under the parser says nothing about the record.
            Some(Err(error)) if error.is_io() => {
                return Err(error).context("Could not read the recovery journal");
            }
            Some(Err(error)) => {
                return Err(anyhow::Error::new(error)
                    .context("Invalid recovery journal record")
                    .context(Damaged));
            }
            Some(Ok(value)) => value,
        };
        valid!(used <= limit, "Recovery record exceeds the supported size");
        let end = self
            .offset
            .checked_add(used)
            .context("Recovery offset overflow")?;
        if end == self.bytes {
            return Ok(None);
        }
        self.reader.seek(SeekFrom::Start(end))?;
        let mut newline = [0];
        self.reader.read_exact(&mut newline)?;
        valid!(
            newline == *b"\n",
            "Recovery record has no newline delimiter"
        );
        self.offset = end + 1;
        Ok(Some(value))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Context {
    pub root: PathBuf,
    pub rotation_only: bool,
    pub media: Vec<Media>,
    pub session: Session,
    pub views: views::Saved,
}
impl Context {
    /// Capture only durable user intent, using the application's existing index.
    /// Unchanged photos without decisions or manual membership need no session
    /// fingerprint in every operation log; bookmarks own their own identities.
    pub fn capture(
        root: PathBuf,
        rotation_only: bool,
        media: &[Media],
        index: &HashMap<MediaId, usize>,
        mut session: Session,
        views: views::Saved,
    ) -> Self {
        let item = |id: MediaId| {
            index
                .get(&id)
                .and_then(|i| media.get(*i))
                .filter(|m| m.id == id)
        };
        session.fingerprints = session
            .decisions
            .keys()
            .copied()
            .chain(session.manual_group_ids())
            .filter_map(|id| item(id).map(|m| (id, m.fingerprint)))
            .collect();
        let media = session
            .decisions
            .keys()
            .filter_map(|id| item(*id).cloned())
            .collect();
        Self {
            root,
            rotation_only,
            media,
            session,
            views,
        }
    }
    /// Use the same discovered identities for supporting-file review intent.
    /// Keep journal size proportional to decisions, without rescanning files.
    pub fn capture_catalog(
        rotation_only: bool,
        catalog: &crate::core::catalog::Catalog,
        index: &HashMap<MediaId, usize>,
        session: Session,
        views: views::Saved,
    ) -> Self {
        let mut context = Self::capture(
            catalog.root.clone(),
            rotation_only,
            &catalog.media,
            index,
            session,
            views,
        );
        context.session.fingerprints.extend(
            context
                .session
                .decisions
                .keys()
                .filter_map(|id| catalog.supporting_fingerprints.get(id).map(|f| (*id, *f))),
        );
        context
    }
    fn persist(&self, report: &Report) -> Result<()> {
        let mut session = self.session.clone();
        if self.rotation_only {
            session.apply_verified_identities(&report.relocated);
        } else {
            session.apply_report(report, &self.media);
        }
        session.save(&self.root)?;
        let mut views = self.views.clone();
        views.relocate(&report.relocated);
        views.save(&self.root)?;
        if let Some(output) = report
            .remaining
            .output_root
            .as_ref()
            .filter(|p| **p != self.root)
        {
            let mut existing = Session::load(output)?;
            existing.merge(session);
            existing.save(output)?;
            let arrivals = report.relocations_into(output);
            if !arrivals.is_empty() {
                let mut existing_views = views::Saved::load(output)?;
                if existing_views.import_relocated(&self.views, &arrivals) {
                    existing_views.save(output)?;
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub journal: String,
    pub fingerprint: Fingerprint,
}
#[derive(Clone, Debug)]
pub struct Restored {
    pub ticket: Ticket,
    pub plan: Plan,
    pub rotation_only: bool,
    pub uncertain: usize,
    pub errors: Vec<String>,
}
/// An unfinished plan that could not be restored when its library was opened.
#[derive(Clone, Debug)]
pub struct Unresumable {
    /// Where the unreadable journal, or its index, is: renamed aside when the
    /// plan was dropped, untouched when it was left. `None` when the journal
    /// was already gone.
    pub kept: Option<PathBuf>,
    pub reason: String,
    /// The record is not known to be damaged, only unreadable this time, so
    /// nothing was renamed or removed: the plan is still pending, the next
    /// open reads it again, and until then nothing can be applied in the
    /// library. `false` when the plan was dropped.
    pub left_in_place: bool,
}
#[derive(Serialize, Deserialize)]
struct Pointer {
    journal: String,
    settled: bool,
}
#[derive(Serialize, Deserialize)]
struct Header {
    version: u32,
    context: Context,
    plan: Plan,
}
#[derive(Deserialize)]
struct Entry {
    status: String,
    index: usize,
    #[serde(default)]
    relocated: Vec<(MediaId, MediaId, Fingerprint)>,
    #[serde(default)]
    error: String,
    #[serde(default)]
    move_states: Vec<moves::State>,
    /// Files a failed Trash group had already moved to the OS Trash.
    #[serde(default)]
    trashed: usize,
    /// A start that shared its journal sync with other moves, so it may
    /// never have run.
    #[serde(default)]
    batched: bool,
}
struct Loaded {
    pending: Vec<usize>,
    /// Pending moves recorded as started in a batch.
    batched: HashSet<usize>,
    valid_bytes: u64,
    header: Header,
    report: Report,
    ticket: Ticket,
    uncertain: usize,
}
/// The injected directory owns both unique audit journals and small active-plan
/// indexes. Old unindexed journals remain audit records; they are never guessed,
/// and [`Store::prune`] removes them once they are old.
pub struct Store {
    directory: PathBuf,
    max_header: u64,
    keep_finished_bytes: u64,
}
/// What [`Store::prune`] removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pruned {
    pub journals: usize,
    pub bytes: u64,
}
/// Scope ownership must release the lock explicitly: closing one descriptor
/// need not release it while a duplicated/inherited handle still exists.
struct LibraryLock {
    file: File,
}
impl Drop for LibraryLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl Store {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            max_header: MAX_HEADER,
            keep_finished_bytes: KEEP_FINISHED_BYTES,
        }
    }
    pub fn standard() -> Self {
        Self::new(super::journal_dir())
    }
    pub fn has_pending(&self, root: &Path) -> bool {
        fs::symlink_metadata(self.pointer(root)).is_ok()
    }
    fn key(root: &Path) -> String {
        let root: PathBuf = root.components().collect();
        blake3::hash(root.as_os_str().as_encoded_bytes())
            .to_hex()
            .to_string()
    }
    fn pointer(&self, root: &Path) -> PathBuf {
        self.directory
            .join("active")
            .join(format!("{}.json", Self::key(root)))
    }
    fn lock(&self, root: &Path) -> Result<LibraryLock> {
        ensure!(
            root.is_absolute(),
            "Recovery needs an absolute library path"
        );
        reject_symlink_components(&self.directory)?;
        fs::create_dir_all(self.directory.join("active"))?;
        fs::create_dir_all(self.directory.join("locks"))?;
        let path = self
            .directory
            .join("locks")
            .join(format!("{}.lock", Self::key(root)));
        reject_symlink_components(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.try_lock()
            .context("Another PhotoSort window is changing this library's file plan")?;
        Ok(LibraryLock { file })
    }
    fn read_pointer(&self, root: &Path) -> Result<Option<Pointer>> {
        Self::read_pointer_at(&self.pointer(root))
    }
    fn read_pointer_at(path: &Path) -> Result<Option<Pointer>> {
        reject_symlink_components(path)?;
        match fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(meta) => valid!(
                meta.is_file() && meta.len() <= 4096,
                "Invalid recovery index"
            ),
        }
        let bytes = fs::read(path)?;
        let pointer: Pointer = serde_json::from_slice(&bytes)
            .map_err(|error| anyhow::Error::new(error).context(Damaged))?;
        let name = Path::new(&pointer.journal);
        valid!(
            name.components().count() == 1
                && pointer.journal.starts_with("operations-")
                && pointer.journal.ends_with(".jsonl"),
            "Invalid recovery journal name"
        );
        Ok(Some(pointer))
    }
    fn write_pointer(&self, root: &Path, pointer: &Pointer) -> Result<()> {
        let path = self.pointer(root);
        reject_symlink_components(&path)?;
        metadata::atomic_write(&path, &serde_json::to_vec(pointer)?)
    }
    fn read(
        &self,
        root: &Path,
        pointer: &Pointer,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Loaded> {
        let path = self.directory.join(&pointer.journal);
        reject_symlink_components(&path)?;
        let fingerprint = Fingerprint::read(&path).map_err(missing_is_damage)?;
        let mut records = Records {
            reader: BufReader::new(Cancellable {
                file: File::open(&path).map_err(missing_is_damage)?,
                cancelled,
            }),
            offset: 0,
            bytes: fingerprint.bytes,
        };
        let Some(header) = records.next::<Header>(self.max_header)? else {
            return Err(anyhow::anyhow!(
                "No complete recovery plan was saved; retain the journal for inspection"
            )
            .context(Damaged));
        };
        // A later format is not damage: the build that wrote it can read it.
        ensure!(
            header.version <= 1,
            "This operation was saved by a newer version of PhotoSort"
        );
        valid!(
            header.version == 1 && header.context.root == root,
            "Recovery plan version or library does not match"
        );
        let mut report = Report {
            remaining: Plan {
                actions: vec![],
                description: header.plan.description.clone(),
                output_root: header.plan.output_root.clone(),
                completed_dependencies: header.plan.completed_dependencies.clone(),
            },
            ..Default::default()
        };
        let mut completed = HashSet::new();
        let mut partial = HashMap::new();
        let mut pending = HashSet::new();
        let mut batched = HashSet::new();
        let mut seen = HashSet::new();
        while let Some(entry) = records.next::<Entry>(MAX_RECORD)? {
            let plan = &header.plan;
            valid!(
                entry.index < plan.actions.len(),
                "Recovery action index is outside the plan"
            );
            match entry.status.as_str() {
                "pending" => {
                    valid!(seen.insert(entry.index), "Duplicate recovery action start");
                    pending.insert(entry.index);
                    if entry.batched {
                        batched.insert(entry.index);
                    }
                }
                "completed" | "recovered" => {
                    valid!(
                        entry.status != "recovered"
                            || matches!(
                                plan.actions[entry.index],
                                super::Action::Move { .. } | super::Action::FinishMove { .. }
                            ),
                        "Only verified moves can have recovered receipts"
                    );
                    valid!(
                        pending.remove(&entry.index) && completed.insert(entry.index),
                        "Completion without a pending action"
                    );
                    report.complete(&plan.actions[entry.index], &entry.relocated);
                    if !entry.error.is_empty() {
                        report
                            .errors
                            .push(plan.actions[entry.index].failure_message(&entry.error));
                    }
                }
                "recovered-move" => {
                    valid!(
                        pending.remove(&entry.index) && !partial.contains_key(&entry.index),
                        "Move recovery without a pending action"
                    );
                    let remainder =
                        moves::restore(&plan.actions[entry.index], &entry.move_states, &mut report)
                            .map_err(|error| error.context(Damaged))?;
                    partial.insert(entry.index, remainder);
                    if !entry.error.is_empty() {
                        report
                            .errors
                            .push(plan.actions[entry.index].failure_message(&entry.error));
                    }
                }
                "failed" | "cancelled" => {
                    valid!(
                        pending.remove(&entry.index),
                        "Failure without a pending action"
                    );
                    if entry.trashed > 0 {
                        let (done, rest) =
                            super::split_trash(&plan.actions[entry.index], entry.trashed)
                                .context("Invalid partial Trash record")
                                .map_err(|error| error.context(Damaged))?;
                        report.complete(&done, &[]);
                        partial.insert(entry.index, vec![rest]);
                    }
                    if !entry.error.is_empty() {
                        report
                            .errors
                            .push(plan.actions[entry.index].failure_message(&entry.error));
                    }
                }
                "skipped" => {
                    valid!(seen.insert(entry.index), "Duplicate skipped action");
                }
                // Well formed but unknown: more likely a newer PhotoSort's
                // record than damage, so this is not proof of it.
                _ => anyhow::bail!("Unknown recovery journal record"),
            }
        }
        report.remaining.actions.reserve(header.plan.actions.len());
        for (index, action) in header.plan.actions.iter().enumerate() {
            if completed.contains(&index) {
                continue;
            }
            if let Some(actions) = partial.remove(&index) {
                report.remaining.actions.extend(actions);
            } else {
                report.remaining.actions.push(action.clone());
            }
        }
        ensure!(
            !cancelled() && Fingerprint::read(&path)? == fingerprint,
            "Recovery journal changed or was cancelled while reading"
        );
        let mut pending: Vec<_> = pending.into_iter().collect();
        pending.sort_unstable();
        Ok(Loaded {
            valid_bytes: records.offset,
            uncertain: pending.len(),
            batched: pending
                .iter()
                .copied()
                .filter(|index| batched.contains(index))
                .collect(),
            pending,
            header,
            report,
            ticket: Ticket {
                journal: pointer.journal.clone(),
                fingerprint,
            },
        })
    }
    /// On the first recovery only, classify verified same-filesystem move
    /// members, recording completed files and explicit staged-source cleanup. Later opens must not replay old intent over new user edits.
    /// Original audit bytes (including a torn tail) are always retained.
    fn recover_moves(
        &self,
        root: &Path,
        pointer: &mut Pointer,
        loaded: Loaded,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Loaded> {
        if pointer.settled || loaded.pending.is_empty() {
            return Ok(loaded);
        }
        let mut verified = Vec::new();
        // Moves whose start was recorded with a batch and that never ran.
        let mut unstarted = Vec::new();
        for &index in &loaded.pending {
            let action = &loaded.header.plan.actions[index];
            // A crash may have left a partial copy, or an empty file holding a
            // final name, in a destination folder. Clear PhotoSort's own
            // temporary files there before reading what the move achieved.
            moves::sweep(action);
            if let Some(identities) = moves::verify(action, cancelled)? {
                verified.push((index, identities));
            } else if loaded.batched.contains(&index) && moves::unstarted(action, cancelled)? {
                unstarted.push(index);
            }
        }
        if verified.is_empty() && unstarted.is_empty() {
            return Ok(loaded);
        }
        let old_path = self.directory.join(&pointer.journal);
        reject_symlink_components(&old_path)?;
        let mut checkpoint = tempfile::Builder::new()
            .prefix("operations-")
            .suffix(".jsonl")
            .tempfile_in(&self.directory)?;
        let mut input = Cancellable {
            file: File::open(&old_path)?,
            cancelled,
        }
        .take(loaded.valid_bytes);
        ensure!(
            std::io::copy(&mut input, &mut checkpoint)? == loaded.valid_bytes,
            "Recovery journal was truncated"
        );
        ensure!(
            !cancelled() && Fingerprint::read(&old_path)? == loaded.ticket.fingerprint,
            "Recovery journal changed during verification"
        );
        // Recheck after streaming the journal, before publishing the checkpoint.
        for (index, identities) in &verified {
            let action = &loaded.header.plan.actions[*index];
            ensure!(
                moves::verify(action, cancelled)?.as_ref() == Some(identities),
                "Moved files changed during recovery; reopen the library"
            );
            moves::write(checkpoint.as_file_mut(), *index, action, identities, None)?;
        }
        for &index in &unstarted {
            let action = &loaded.header.plan.actions[index];
            ensure!(
                moves::unstarted(action, cancelled)?,
                "Moved files changed during recovery; reopen the library"
            );
            super::write_record(checkpoint.as_file_mut(), "cancelled", index, &[], None)?;
        }
        checkpoint.as_file().sync_all()?;
        ensure!(!cancelled(), "Recovery cancelled before publication");
        let (_, path) = checkpoint.keep().map_err(|error| error.error)?;
        let replacement = Pointer {
            journal: path
                .file_name()
                .unwrap()
                .to_str()
                .context("Journal name is not text")?
                .into(),
            settled: false,
        };
        if let Err(error) = self.write_pointer(root, &replacement) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        *pointer = replacement;
        self.read(root, pointer, cancelled)
    }
    fn settle(&self, loaded: &Loaded, pointer: &mut Pointer) -> Result<()> {
        if !pointer.settled {
            loaded.header.context.persist(&loaded.report)?;
            pointer.settled = true;
            self.write_pointer(&loaded.header.context.root, pointer)?;
        }
        Ok(())
    }
    /// Journals are PhotoSort's own bookkeeping and were never removed: about
    /// 30 MB per Organize of 10 000 photos, for ever. A journal is removed
    /// here, at startup, only when all of this holds:
    /// - no library's index names it, so no unfinished plan needs it (if any
    ///   index cannot be read, nothing is removed at all);
    /// - it was last written more than an hour ago;
    /// - it is older than seven days, or the newer journals kept under this
    ///   rule already hold 256 MB.
    ///
    /// An index only ever names a journal created a moment earlier, so one
    /// that is old and unnamed can never be needed again. Journals set aside
    /// as unreadable are kept for the user.
    pub fn prune(&self) -> Result<Pruned> {
        self.prune_at(std::time::SystemTime::now())
    }
    fn prune_at(&self, now: std::time::SystemTime) -> Result<Pruned> {
        let absent = |e: &std::io::Error| e.kind() == std::io::ErrorKind::NotFound;
        let journals = match fs::read_dir(&self.directory) {
            Ok(journals) => journals,
            // Nothing was ever applied.
            Err(e) if absent(&e) => return Ok(Pruned::default()),
            Err(e) => return Err(e.into()),
        };
        let mut needed = HashSet::new();
        match fs::read_dir(self.directory.join("active")) {
            Ok(indexes) => {
                for index in indexes {
                    let path = index?.path();
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    // Not an index: one set aside, or one still being written,
                    // which names a journal too new to be removed.
                    if !name.ends_with(".json") || name.contains(DAMAGED) {
                        continue;
                    }
                    if let Some(pointer) = Self::read_pointer_at(&path).context(
                        "An unfinished plan's index cannot be read; no journal is removed",
                    )? {
                        needed.insert(pointer.journal);
                    }
                }
            }
            Err(e) if absent(&e) => {}
            Err(e) => return Err(e.into()),
        }
        let mut finished = Vec::new();
        for journal in journals {
            let journal = journal?;
            let name = journal.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("operations-")
                || !name.ends_with(".jsonl")
                || name.contains(DAMAGED)
                || needed.contains(name)
            {
                continue;
            }
            // `DirEntry::metadata` does not follow links: only plain files count.
            let Ok(meta) = journal.metadata() else {
                continue;
            };
            let Ok(written) = meta.modified() else {
                continue;
            };
            if meta.is_file() {
                finished.push((written, meta.len(), journal.path()));
            }
        }
        finished.sort_by_key(|journal| std::cmp::Reverse(journal.0));
        let mut pruned = Pruned::default();
        let mut kept_bytes = 0u64;
        for (written, bytes, path) in finished {
            // A time in the future is a wrong clock, not an old journal.
            let age = now.duration_since(written).unwrap_or_default();
            if age > SETTLED_AFTER
                && (age > KEEP_FINISHED || kept_bytes >= self.keep_finished_bytes)
            {
                match fs::remove_file(&path) {
                    Ok(()) => {
                        pruned.journals += 1;
                        pruned.bytes += bytes;
                    }
                    Err(e) if absent(&e) => {}
                    Err(error) => {
                        tracing::warn!(path = %path.display(), %error, "finished operation journal could not be removed");
                    }
                }
            } else {
                kept_bytes = kept_bytes.saturating_add(bytes);
            }
        }
        Ok(pruned)
    }
    pub fn restore(&self, root: &Path, cancelled: &impl Fn() -> bool) -> Result<Option<Restored>> {
        Ok(self.restore_with(root, cancelled, false)?.0)
    }
    /// A missing or damaged journal must never lock the user out of their
    /// library, and Discard cannot help: it reads the journal too. The
    /// journal is renamed aside, kept for inspection, the unfinished plan is
    /// dropped, and the caller is told so it can say so. Nothing is deleted
    /// but the small index that named the journal.
    ///
    /// That is only for a record known to be unusable ([`Damaged`]). When
    /// reading fails for any other reason the library opens as well, but the
    /// journal and its index stay exactly as they are and the caller is told
    /// the plan is still pending ([`Unresumable::left_in_place`]).
    pub fn restore_or_set_aside(
        &self,
        root: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<(Option<Restored>, Option<Unresumable>)> {
        self.restore_with(root, cancelled, true)
    }
    fn restore_with(
        &self,
        root: &Path,
        cancelled: &impl Fn() -> bool,
        set_aside: bool,
    ) -> Result<(Option<Restored>, Option<Unresumable>)> {
        // Opening a library without recovery state needs no lock files or directory writes.
        match fs::symlink_metadata(self.pointer(root)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((None, None)),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let _lock = self.lock(root)?;
        // Only what cannot be read is set aside or left. A cancelled read, moved
        // files that changed meanwhile or a session that cannot be saved stay
        // errors: the journal is intact and the next open can resume it.
        let mut pointer = match self.read_pointer(root) {
            Ok(None) => return Ok((None, None)),
            Ok(Some(pointer)) => pointer,
            Err(error) if set_aside && !cancelled() => {
                return Ok((None, Some(self.unresumable(root, None, &error)?)));
            }
            Err(error) => return Err(error),
        };
        let loaded = match self.read(root, &pointer, cancelled) {
            Ok(loaded) => loaded,
            Err(error) if set_aside && !cancelled() => {
                return Ok((None, Some(self.unresumable(root, Some(&pointer), &error)?)));
            }
            Err(error) => return Err(error),
        };
        let loaded = self.recover_moves(root, &mut pointer, loaded, cancelled)?;
        self.settle(&loaded, &mut pointer)?;
        if loaded.report.remaining.actions.is_empty() {
            fs::remove_file(self.pointer(root))?;
        }
        Ok((
            Some(Restored {
                ticket: loaded.ticket,
                plan: loaded.report.remaining,
                rotation_only: loaded.header.context.rotation_only,
                uncertain: loaded.uncertain,
                errors: loaded.report.errors,
            }),
            None,
        ))
    }
    /// What becomes of a plan whose index (`pointer` is `None`) or journal
    /// could not be read. Proof of damage drops it ([`Self::set_aside`]).
    /// Without that proof nothing is touched: the failure may be gone by the
    /// next open, and a dropped plan can never be resumed.
    fn unresumable(
        &self,
        root: &Path,
        pointer: Option<&Pointer>,
        error: &anyhow::Error,
    ) -> Result<Unresumable> {
        if error.downcast_ref::<Damaged>().is_some() {
            return self.set_aside(root, pointer, error);
        }
        let kept = match pointer {
            Some(pointer) => self.directory.join(&pointer.journal),
            None => self.pointer(root),
        };
        let reason = format!("{error:#}");
        tracing::warn!(
            root = %root.display(),
            kept = %kept.display(),
            %reason,
            "an unfinished file plan could not be read this time and was left as it is"
        );
        Ok(Unresumable {
            kept: Some(kept),
            reason,
            left_in_place: true,
        })
    }
    /// With a readable index the journal it names is renamed and the index
    /// removed; an unreadable index is itself renamed. Either way the library
    /// has no unfinished plan afterwards.
    fn set_aside(
        &self,
        root: &Path,
        pointer: Option<&Pointer>,
        error: &anyhow::Error,
    ) -> Result<Unresumable> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let aside = |path: PathBuf, extension: &str| -> Result<Option<PathBuf>> {
            let kept = path.with_extension(format!("{DAMAGED}{stamp}.{extension}"));
            match fs::rename(&path, &kept) {
                Ok(()) => Ok(Some(kept)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        };
        let kept = match pointer {
            Some(pointer) => {
                let kept = aside(self.directory.join(&pointer.journal), "jsonl")?;
                fs::remove_file(self.pointer(root))?;
                kept
            }
            None => aside(self.pointer(root), "json")?,
        };
        let reason = format!("{error:#}");
        tracing::warn!(
            root = %root.display(),
            kept = ?kept,
            %reason,
            "dropped an unfinished file plan whose journal could not be read"
        );
        Ok(Unresumable {
            kept,
            reason,
            left_in_place: false,
        })
    }
    fn prepare(
        &self,
        context: &Context,
        plan: &Plan,
        cancelled: &impl Fn() -> bool,
    ) -> Result<tempfile::NamedTempFile> {
        let prepared = tempfile::Builder::new()
            .prefix("operations-")
            .suffix(".jsonl")
            .tempfile_in(&self.directory)?;
        #[derive(Serialize)]
        struct Accepted<'a> {
            version: u32,
            context: &'a Context,
            plan: &'a Plan,
        }
        let header = Accepted {
            version: 1,
            context,
            plan,
        };
        let mut writer = Cancellable {
            file: prepared.as_file().try_clone()?,
            cancelled,
        };
        serde_json::to_writer(&mut writer, &header)?;
        ensure!(
            prepared.as_file().metadata()?.len() <= self.max_header,
            "File plan is too large for recovery; split it into smaller reviewed batches"
        );
        writer.write_all(b"\n")?;
        prepared.as_file().sync_all()?;
        ensure!(!cancelled(), "File plan cancelled before publication");
        Ok(prepared)
    }
    pub fn execute(
        &self,
        context: &Context,
        plan: &Plan,
        ticket: Option<&Ticket>,
        cancelled: impl Fn() -> bool,
        progress: impl FnMut(usize, usize),
    ) -> Result<Report> {
        self.execute_stoppable(context, plan, ticket, cancelled, || false, progress)
    }
    /// A user Stop preserves the accepted plan first, then stops at an operation's
    /// safe cancellation boundary. Runtime shutdown still cancels preparation.
    pub fn execute_stoppable(
        &self,
        context: &Context,
        plan: &Plan,
        ticket: Option<&Ticket>,
        cancelled: impl Fn() -> bool,
        stop_requested: impl Fn() -> bool,
        progress: impl FnMut(usize, usize),
    ) -> Result<Report> {
        let _lock = self.lock(&context.root)?;
        ensure!(!cancelled(), "File plan cancelled before acceptance");
        if let Some(pointer) = self.read_pointer(&context.root)? {
            let previous = self.read(&context.root, &pointer, &cancelled)?;
            ensure!(
                ticket == Some(&previous.ticket),
                "An unfinished file plan changed; reopen the library before applying"
            );
            ensure!(
                plan.completed_dependencies == previous.report.remaining.completed_dependencies
                    && plan.output_root == previous.report.remaining.output_root
                    && serde_json::to_value(&plan.actions)?
                        == serde_json::to_value(&previous.report.remaining.actions)?,
                "Retry must use the exact unfinished destinations and source checks"
            );
        } else {
            ensure!(
                ticket.is_none(),
                "This unfinished plan was already resolved in another window"
            );
        }
        let prepared = self.prepare(context, plan, &cancelled)?;
        let (mut journal, path) = prepared.keep().map_err(|e| e.error)?;
        let mut pointer = Pointer {
            journal: path
                .file_name()
                .unwrap()
                .to_str()
                .context("Journal name is not text")?
                .into(),
            settled: false,
        };
        if let Err(error) = self.write_pointer(&context.root, &pointer) {
            drop(journal);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let mut report = execute_logged(
            plan,
            &mut journal,
            || cancelled() || stop_requested(),
            progress,
        )?;
        context.persist(&report)?;
        pointer.settled = true;
        self.write_pointer(&context.root, &pointer)?;
        if report.remaining.actions.is_empty() {
            fs::remove_file(self.pointer(&context.root))?;
        } else {
            report.recovery = Some(Ticket {
                journal: pointer.journal,
                fingerprint: Fingerprint::read(&path)?,
            });
        }
        Ok(report)
    }
    pub fn discard(&self, root: &Path, ticket: &Ticket) -> Result<()> {
        let _lock = self.lock(root)?;
        let mut pointer = self
            .read_pointer(root)?
            .context("Unfinished plan was already resolved")?;
        let loaded = self.read(root, &pointer, &|| false)?;
        ensure!(
            &loaded.ticket == ticket,
            "Unfinished plan changed; reopen the library before discarding"
        );
        self.settle(&loaded, &mut pointer)?;
        // Audit logs and all media remain untouched. Removing the index abandons intent only.
        fs::remove_file(self.pointer(root))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
