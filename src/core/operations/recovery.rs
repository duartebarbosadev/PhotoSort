//! Durable, per-library recovery on the shared filesystem worker. Reopening
//! restores intent and verified identities only; file operations require review.
use super::{Plan, Report, execute_logged, reject_symlink_components};
mod amendment;
mod moves;
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
                ensure!(
                    self.bytes.saturating_sub(self.offset) <= limit,
                    "Recovery record exceeds the supported size"
                );
                return Ok(None);
            }
            Some(Err(error)) if error.is_eof() => {
                ensure!(
                    self.bytes.saturating_sub(self.offset) <= limit,
                    "Recovery record exceeds the supported size"
                );
                return Ok(None); // A torn final record cannot prove completion.
            }
            Some(value) => value.context("Invalid recovery journal record")?,
        };
        ensure!(used <= limit, "Recovery record exceeds the supported size");
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
        ensure!(
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
}
struct Loaded {
    pending: Vec<usize>,
    valid_bytes: u64,
    header: Header,
    report: Report,
    ticket: Ticket,
    uncertain: usize,
}
/// The injected directory owns both unique audit journals and small active-plan
/// indexes. Old unindexed journals remain audit records; they are never guessed.
pub struct Store {
    directory: PathBuf,
    max_header: u64,
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
        let path = self.pointer(root);
        reject_symlink_components(&path)?;
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(meta) => ensure!(
                meta.is_file() && meta.len() <= 4096,
                "Invalid recovery index"
            ),
        }
        let pointer: Pointer = serde_json::from_slice(&fs::read(&path)?)?;
        let name = Path::new(&pointer.journal);
        ensure!(
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
        let fingerprint = Fingerprint::read(&path)?;
        let mut records = Records {
            reader: BufReader::new(Cancellable {
                file: File::open(&path)?,
                cancelled,
            }),
            offset: 0,
            bytes: fingerprint.bytes,
        };
        let header: Header = records
            .next(self.max_header)?
            .context("No complete recovery plan was saved; retain the journal for inspection")?;
        ensure!(
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
        let mut seen = HashSet::new();
        while let Some(entry) = records.next::<Entry>(MAX_RECORD)? {
            let plan = &header.plan;
            ensure!(
                entry.index < plan.actions.len(),
                "Recovery action index is outside the plan"
            );
            match entry.status.as_str() {
                "pending" => {
                    ensure!(seen.insert(entry.index), "Duplicate recovery action start");
                    pending.insert(entry.index);
                }
                "completed" | "recovered" => {
                    ensure!(
                        entry.status != "recovered"
                            || matches!(
                                plan.actions[entry.index],
                                super::Action::Move { .. } | super::Action::FinishMove { .. }
                            ),
                        "Only verified moves can have recovered receipts"
                    );
                    ensure!(
                        pending.remove(&entry.index) && completed.insert(entry.index),
                        "Completion without a pending action"
                    );
                    report.complete(&plan.actions[entry.index], &entry.relocated);
                }
                "recovered-move" => {
                    ensure!(
                        pending.remove(&entry.index) && !partial.contains_key(&entry.index),
                        "Move recovery without a pending action"
                    );
                    let remainder = moves::restore(
                        &plan.actions[entry.index],
                        &entry.move_states,
                        &mut report,
                    )?;
                    partial.insert(entry.index, remainder);
                }
                "failed" | "cancelled" => {
                    ensure!(
                        pending.remove(&entry.index),
                        "Failure without a pending action"
                    );
                    if !entry.error.is_empty() {
                        report
                            .errors
                            .push(plan.actions[entry.index].failure_message(&entry.error));
                    }
                }
                "skipped" => {
                    ensure!(seen.insert(entry.index), "Duplicate skipped action");
                }
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
        for &index in &loaded.pending {
            if let Some(identities) = moves::verify(&loaded.header.plan.actions[index], cancelled)?
            {
                verified.push((index, identities));
            }
        }
        if verified.is_empty() {
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
            moves::write(checkpoint.as_file_mut(), *index, action, identities)?;
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
    pub fn restore(&self, root: &Path, cancelled: &impl Fn() -> bool) -> Result<Option<Restored>> {
        // Opening a library without recovery state needs no lock files or directory writes.
        match fs::symlink_metadata(self.pointer(root)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
        }
        let _lock = self.lock(root)?;
        let Some(mut pointer) = self.read_pointer(root)? else {
            return Ok(None);
        };
        let loaded = self.read(root, &pointer, cancelled)?;
        let loaded = self.recover_moves(root, &mut pointer, loaded, cancelled)?;
        self.settle(&loaded, &mut pointer)?;
        if loaded.report.remaining.actions.is_empty() {
            fs::remove_file(self.pointer(root))?;
        }
        Ok(Some(Restored {
            ticket: loaded.ticket,
            plan: loaded.report.remaining,
            rotation_only: loaded.header.context.rotation_only,
            uncertain: loaded.uncertain,
            errors: loaded.report.errors,
        }))
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
