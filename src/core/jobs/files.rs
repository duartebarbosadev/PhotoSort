//! Filesystem actor: scans, saves, plans and applies file operations.
use super::{Event, IoWork, Message, Publisher};
use crate::core::{
    catalog::Catalog,
    manual_groups,
    model::*,
    operations::{self, Plan},
    session::Session,
    views,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    thread,
    time::Duration,
};

pub(super) fn spawn(publisher: Publisher, rx: Receiver<Message<IoWork>>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("photosort-files".into())
        .spawn(move || run(&publisher, &rx))
        .expect("Start filesystem worker")
}

fn run(publisher: &Publisher, rx: &Receiver<Message<IoWork>>) {
    // Once per start, on the thread that applies file plans, so it never runs
    // beside one of this window's own operations.
    match operations::recovery::Store::standard().prune() {
        Ok(pruned) if pruned.journals > 0 => tracing::info!(
            journals = pruned.journals,
            bytes = pruned.bytes,
            "removed finished operation journals"
        ),
        Ok(_) => {}
        Err(error) => {
            tracing::warn!(error = %format!("{error:#}"), "operation journals were not pruned")
        }
    }
    loop {
        // Explicit-root saves own already accepted review decisions.
        // Drain those on shutdown even if a scan was interrupted.
        let stopping = publisher.stop.load(Ordering::Acquire);
        let message = if stopping {
            let Ok(message) = rx.try_recv() else {
                break;
            };
            message
        } else {
            let Ok(message) = rx.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            message
        };
        let epoch = message.epoch;
        // A queued save belongs to its explicit root and is safe after navigation.
        let saving = matches!(
            message.work,
            IoWork::Save { .. }
                | IoWork::MergeSession { .. }
                | IoWork::SaveViews { .. }
                | IoWork::ForgetViews(_)
                | IoWork::SavePreferences { .. }
        );
        if publisher.cancelled(epoch)
            && !saving
            && !matches!(message.work, IoWork::Checkpoint { .. })
        {
            continue;
        }
        let _ = publisher.guard(super::Worker::Files, || {
            let files = Files {
                publisher,
                epoch,
                saving,
            };
            if let Err(e) = files.handle(message.work) {
                files.send(Event::Error(format!("{e:#}")));
            }
        });
    }
}

struct Files<'a> {
    publisher: &'a Publisher,
    epoch: u64,
    /// A save often runs just after the folder changed (an apply saves, then
    /// rescans). Its outcome still matters, so it reaches the current folder.
    saving: bool,
}

impl Files<'_> {
    fn cancelled(&self) -> bool {
        self.publisher.cancelled(self.epoch)
    }
    /// Cancelled by a newer folder generation or by the caller's own flag.
    fn stopped(&self, flag: &AtomicBool) -> bool {
        self.cancelled() || flag.load(Ordering::Acquire)
    }
    fn send(&self, event: Event) {
        if self.saving {
            self.publisher.send_current(event);
        } else {
            self.publisher.send(self.epoch, event);
        }
    }
    fn progress(&self, done: usize, total: usize) {
        self.send(Event::Progress {
            run: 0,
            done,
            total,
        });
    }

    fn handle(&self, work: IoWork) -> anyhow::Result<()> {
        match work {
            IoWork::EditGroups {
                scene,
                cancelled,
                run,
                catalog,
                automatic,
                manual,
                edit,
            } => self.edit_groups(scene, &cancelled, run, &catalog, (automatic, manual), edit),
            IoWork::Scan(root) => self.scan(root),
            IoWork::PlanTrash {
                catalog,
                request,
                companions,
                cancelled,
            } => self.plan_trash(&catalog, &request, companions, &cancelled),
            IoWork::PlanOrganize {
                cancelled,
                directories,
                catalog,
                labels,
                names,
                output,
                companions,
            } => self.plan_organize(
                &cancelled,
                &directories,
                &catalog,
                (&labels, &names),
                &output,
                companions,
            ),
            IoWork::ApplyRecoverable {
                context,
                plan,
                ticket,
                stop_requested,
            } => self.apply_recoverable(&context, &plan, ticket, &stop_requested),
            IoWork::AmendRecovery {
                root,
                ticket,
                group,
                folder,
                stem,
                cancelled,
            } => {
                self.amend_recovery(&root, &ticket, group, &folder, stem.as_deref(), &cancelled);
                Ok(())
            }
            IoWork::DiscardRecovery { root, ticket } => {
                self.discard_recovery(&root, &ticket);
                Ok(())
            }
            IoWork::PlanDecisions {
                catalog,
                decisions,
                companions,
                followers,
                created,
                tidy,
                policy,
                cancelled,
            } => self.plan_decisions(
                &catalog,
                &decisions,
                (companions, followers.as_ref(), &created),
                &tidy,
                policy,
                &cancelled,
            ),
            IoWork::PlanTidy {
                catalog,
                moves,
                trashed,
                cancelled,
            } => {
                let result =
                    operations::folders_emptied_by_trash(&catalog, &moves, &trashed, || {
                        self.stopped(&cancelled)
                    })
                    .map_err(|error| error.context("Could not check which folders are left empty"));
                self.send_plan(result, &cancelled)
            }
            IoWork::Apply(plan) => self.apply(&plan),
            IoWork::SavePreferences { path, preferences } => {
                self.save_preferences(&path, &preferences);
                Ok(())
            }
            IoWork::Checkpoint {
                root,
                session,
                views,
            } => {
                self.checkpoint(&root, &session, &views);
                Ok(())
            }
            IoWork::SaveViews { root, views } => {
                self.save_views(&root, &views);
                Ok(())
            }
            IoWork::ForgetViews(data) => {
                if let Err(e) = views::forget_all(&data) {
                    self.send(Event::ViewError(format!(
                        "Remembered folders could not be forgotten: {e}"
                    )));
                }
                Ok(())
            }
            IoWork::MergeSession { root, session } => {
                self.merge_session(&root, session);
                Ok(())
            }
            IoWork::Save { root, session } => {
                self.save(&root, &session);
                Ok(())
            }
        }
    }

    fn edit_groups(
        &self,
        scene: bool,
        cancelled: &AtomicBool,
        run: u64,
        catalog: &Catalog,
        (automatic, manual): (
            Arc<HashMap<MediaId, usize>>,
            Arc<manual_groups::ManualGroups>,
        ),
        edit: manual_groups::Edit,
    ) -> anyhow::Result<()> {
        let resolved = manual_groups::edit(run, automatic, manual, &catalog.media, edit, || {
            self.stopped(cancelled)
        });
        if !cancelled.load(Ordering::Acquire) && !self.cancelled() {
            let mut resolved = resolved?;
            if scene {
                resolved
                    .groups
                    .retain(|id, _| resolved.automatic.contains_key(id));
                resolved.choices =
                    manual_groups::group_choices(&resolved.groups, || self.stopped(cancelled))?;
            }
            self.send(Event::GroupsEdited(resolved));
        }
        Ok(())
    }

    fn scan(&self, root: PathBuf) -> anyhow::Result<()> {
        let epoch = self.epoch;
        tracing::info!(epoch, root = %root.display(), "folder discovery started");
        let root = match crate::core::catalog::open_root(&root) {
            Ok(root) => root,
            Err((problem, error)) => {
                tracing::warn!(epoch, root = %root.display(), ?problem, %error, "photo folder unavailable");
                self.send(Event::FolderUnavailable {
                    folder: root,
                    problem,
                });
                return Ok(());
            }
        };
        let (restored, unresumable) = operations::recovery::Store::standard()
            .restore_or_set_aside(&root, &|| self.cancelled())?;
        let catalog = Catalog::scan(
            &root,
            || self.cancelled(),
            |batch| self.send(Event::Batch(batch)),
        )?;
        let (mut session, damaged) = Session::load_or_set_aside(&catalog.root)?;
        // One line holds the notice, so two problems are told together.
        let mut notices = Vec::new();
        if let Some(lost) = unresumable {
            notices.push(unresumable_notice(&lost));
        }
        if let Some(kept) = damaged {
            notices.push(format!(
                "Staged choices for this folder could not be read and start empty. The damaged file was kept at {}",
                kept.display()
            ));
        }
        if !notices.is_empty() {
            self.send(Event::ViewError(notices.join(" ")));
        }
        session.reconcile_catalog(&catalog);
        let mut views = views::Saved::load(&catalog.root).unwrap_or_else(|e| {
            self.send(Event::ViewError(format!(
                "Saved view could not be restored: {e}"
            )));
            Default::default()
        });
        views.reconcile_catalog(&catalog);
        if let Some(restored) = restored {
            self.send(Event::OperationsRestored(restored));
        }
        self.send(Event::Opened(Arc::new(catalog), session, views));
        Ok(())
    }

    fn plan_trash(
        &self,
        catalog: &Catalog,
        request: &operations::TrashRequest,
        companions: bool,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<()> {
        let result =
            operations::trash_plan(catalog, request, companions, || self.stopped(cancelled));
        self.send_plan(result, cancelled)
    }

    /// `intent` is the reviewed destination folder and file name per photo.
    fn plan_organize(
        &self,
        cancelled: &AtomicBool,
        directories: &crate::core::organize::DirectoryEdits,
        catalog: &Catalog,
        (labels, names): (&HashMap<MediaId, String>, &HashMap<MediaId, String>),
        output: &std::path::Path,
        companions: bool,
    ) -> anyhow::Result<()> {
        let result = operations::organize_plan_with_layout_cancellable(
            catalog,
            labels,
            names,
            output,
            companions,
            directories,
            || self.stopped(cancelled),
        );
        self.send_plan(result, cancelled)
    }

    /// Reads each photo's own metadata from its file, so the plan never
    /// waits for the image worker, which a model can hold for minutes.
    fn plan_decisions(
        &self,
        catalog: &Catalog,
        decisions: &HashMap<MediaId, Decision>,
        (companions, followers, created): (
            bool,
            Option<&std::collections::HashSet<MediaId>>,
            &[PathBuf],
        ),
        tidy: &[(PathBuf, Vec<String>)],
        policy: crate::core::metadata::WritePolicy,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<()> {
        let stopped = || self.stopped(cancelled);
        let result = operations::decisions_plan_with_followers(
            catalog,
            decisions,
            companions,
            followers,
            policy,
            |media| {
                // A photo that is gone stops the plan, as it always did.
                Fingerprint::read(&media.path)?;
                crate::core::metadata::read_cancellable(&media.path, &stopped)
            },
            stopped,
        )
        .and_then(|mut plan| {
            operations::remove_folders_emptied_by_trash(
                &mut plan, catalog, created, tidy, stopped,
            )?;
            Ok(plan)
        })
        .map_err(|error| error.context("Could not plan file edits"));
        self.send_plan(result, cancelled)
    }

    /// A plan the caller cancelled is reported as such, never as a result or error.
    fn send_plan(
        &self,
        result: anyhow::Result<Plan>,
        cancelled: &AtomicBool,
    ) -> anyhow::Result<()> {
        if cancelled.load(Ordering::Acquire) {
            self.send(Event::PlanningCancelled);
        } else {
            self.send(Event::Planned(result?));
        }
        Ok(())
    }

    fn apply_recoverable(
        &self,
        context: &operations::recovery::Context,
        plan: &Plan,
        ticket: Option<operations::recovery::Ticket>,
        stop_requested: &AtomicBool,
    ) -> anyhow::Result<()> {
        let store = operations::recovery::Store::standard();
        let result = store.execute_stoppable(
            context,
            plan,
            ticket.as_ref(),
            || self.cancelled(),
            || stop_requested.load(Ordering::Acquire),
            |done, total| self.progress(done, total),
        );
        match result {
            Ok(report) => self.send(Event::Applied(report)),
            Err(error) if store.has_pending(&context.root) || ticket.is_some() => {
                self.send(Event::ApplyFailed(format!("{error:#}")))
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn amend_recovery(
        &self,
        root: &std::path::Path,
        ticket: &operations::recovery::Ticket,
        group: usize,
        folder: &std::path::Path,
        stem: Option<&str>,
        cancelled: &AtomicBool,
    ) {
        let result = operations::recovery::Store::standard()
            .amend_destination(root, ticket, group, folder, stem, &|| {
                self.stopped(cancelled)
            })
            .map_err(|e| format!("{e:#}"));
        self.send(Event::OperationsAmended(result));
    }

    fn discard_recovery(&self, root: &std::path::Path, ticket: &operations::recovery::Ticket) {
        match operations::recovery::Store::standard().discard(root, ticket) {
            Ok(()) => self.send(Event::OperationsDiscarded),
            Err(error) => self.send(Event::ApplyFailed(format!("{error:#}"))),
        }
    }

    fn apply(&self, plan: &Plan) -> anyhow::Result<()> {
        let report = operations::execute(
            plan,
            &operations::journal_dir(),
            || self.cancelled(),
            |done, total| self.progress(done, total),
        )?;
        self.send(Event::Applied(report));
        Ok(())
    }

    fn save_preferences(
        &self,
        path: &std::path::Path,
        preferences: &crate::core::preferences::Preferences,
    ) {
        if let Err(e) = preferences.save(path) {
            self.send(Event::PreferencesError(format!(
                "Preferences could not be saved: {e}"
            )));
        }
    }

    fn checkpoint(&self, root: &std::path::Path, session: &Session, views: &views::Saved) {
        let result = session.save(root).and_then(|()| views.save(root));
        self.send(Event::Checkpointed(
            result.map_err(|e| format!("Could not save before quitting: {e:#}")),
        ));
    }

    fn save_views(&self, root: &std::path::Path, views: &views::Saved) {
        if let Err(e) = views.save(root) {
            self.send(Event::ViewError(format!(
                "Saved view could not be written: {e}"
            )));
        }
    }

    fn merge_session(&self, root: &std::path::Path, session: Session) {
        let result = Session::load(root).and_then(|mut existing| {
            existing.merge(session);
            existing.save(root)
        });
        self.saved(result);
    }

    fn save(&self, root: &std::path::Path, session: &Session) {
        self.saved(session.save(root));
    }

    /// A failed save is reported on its own: it must not cancel analysis or
    /// other work the way a general error does.
    fn saved(&self, result: anyhow::Result<()>) {
        self.send(match result {
            Ok(()) => Event::Saved,
            Err(e) => Event::ViewError(format!("Staged choices could not be saved: {e:#}")),
        });
    }
}

/// The plan is gone or could not be read, so the folders it touched cannot be
/// named: the user is told what may have happened and where to look.
fn unresumable_notice(lost: &operations::recovery::Unresumable) -> String {
    if lost.left_in_place {
        // Nothing was dropped. Apply refuses to run over a plan it has not
        // restored, so the user must know why until a later open reads it.
        let record = match &lost.kept {
            Some(kept) => format!(" Its record is unchanged at {}.", kept.display()),
            None => String::new(),
        };
        return format!(
            "An interrupted file operation in this folder could not be read this time ({}).{record} PhotoSort will try again the next time this folder is opened; until then nothing can be applied here. Some files may already have been moved, renamed or sent to the Trash.",
            lost.reason
        );
    }
    let record = match &lost.kept {
        Some(kept) => format!(
            "its record could not be read and was kept at {}",
            kept.display()
        ),
        None => "its record is missing".to_owned(),
    };
    format!(
        "An interrupted file operation in this folder could not be resumed: {record}. Some files may already have been moved, renamed or sent to the Trash. Check this folder and any folder you were organizing into."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_library_whose_interrupted_operation_cannot_be_read_still_opens_and_says_so() {
        use operations::recovery::{Context, Store};
        let library = tempfile::tempdir().unwrap();
        let root = library.path().canonicalize().unwrap();
        std::fs::write(root.join("a.jpg"), b"photo").unwrap();
        let context = Context {
            root: root.clone(),
            rotation_only: false,
            media: vec![],
            session: Session::default(),
            views: Default::default(),
        };
        let plan = Plan {
            actions: vec![operations::Action::CreateDirectory {
                path: root.join("2024"),
            }],
            ..Default::default()
        };
        // Stopped before its first action: an unfinished plan with a journal.
        let report = Store::standard()
            .execute_stoppable(&context, &plan, None, || false, || true, |_, _| {})
            .unwrap();
        let journal = operations::journal_dir().join(report.recovery.unwrap().journal);

        let (tx, events) = std::sync::mpsc::sync_channel(64);
        let publisher = Publisher {
            tx,
            generation: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            stop: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(|| {}),
        };
        let (work, rx) = std::sync::mpsc::channel();
        let worker = spawn(publisher.clone(), rx);
        let scan = |notice: &mut Option<String>| {
            work.send(Message {
                epoch: 1,
                work: IoWork::Scan(root.clone()),
            })
            .unwrap();
            loop {
                match events.recv_timeout(Duration::from_secs(30)).unwrap().event {
                    Event::ViewError(message) => *notice = Some(message),
                    Event::Opened(catalog, ..) => return catalog.media.len(),
                    Event::Error(error) => panic!("the library did not open: {error}"),
                    _ => {}
                }
            }
        };
        // A journal that cannot be read right now (here: no permission) is
        // not damaged. It stays, the plan stays pending, and the user is told.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let allow = |mode| {
                std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(mode)).unwrap()
            };
            let saved = std::fs::read(&journal).unwrap();
            allow(0o000);
            // An administrator reads the file anyway: nothing to see then.
            if std::fs::File::open(&journal).is_err() {
                let mut notice = None;
                assert_eq!(scan(&mut notice), 1);
                let notice = notice.expect("the user is told the operation is still pending");
                assert!(
                    notice.contains("try again") && notice.contains(&journal.display().to_string()),
                    "{notice}"
                );
                assert!(Store::standard().has_pending(&root));
            }
            allow(0o600);
            assert_eq!(std::fs::read(&journal).unwrap(), saved);
        }
        // The user tidied the data folder by hand, or the disk damaged the file.
        std::fs::write(&journal, b"{\"version\":1,\"context\":{tru").unwrap();
        let mut notice = None;
        assert_eq!(scan(&mut notice), 1);
        let notice = notice.expect("the user is told the operation could not be resumed");
        assert!(notice.contains("could not be resumed"), "{notice}");
        // Renamed, not deleted, and the notice says where it is.
        let kept: Vec<_> = std::fs::read_dir(operations::journal_dir())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| std::fs::read(path).is_ok_and(|bytes| bytes.ends_with(b"{tru")))
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(!journal.exists() && notice.contains(&kept[0].display().to_string()));
        // The plan is dropped: the next open is quiet and nothing was run.
        assert!(!Store::standard().has_pending(&root));
        let mut again = None;
        assert_eq!(scan(&mut again), 1);
        assert!(again.is_none() && !root.join("2024").exists());
        publisher.stop.store(true, Ordering::Release);
        worker.join().unwrap();
    }

    #[test]
    fn a_failed_save_after_a_folder_change_still_reaches_the_user_without_cancelling_work() {
        let library = tempfile::tempdir().unwrap();
        // A folder where the session file belongs makes reading it fail.
        std::fs::create_dir_all(crate::core::session::session_path(library.path())).unwrap();
        let (tx, events) = std::sync::mpsc::sync_channel(8);
        let publisher = Publisher {
            tx,
            generation: Arc::new(std::sync::atomic::AtomicU64::new(2)),
            stop: Arc::new(AtomicBool::new(false)),
            wake: Arc::new(|| {}),
        };
        let (work, rx) = std::sync::mpsc::channel();
        let worker = spawn(publisher.clone(), rx);
        // Queued under folder 1, run after the user already moved on to folder 2.
        work.send(Message {
            epoch: 1,
            work: IoWork::MergeSession {
                root: library.path().to_path_buf(),
                session: Session::default(),
            },
        })
        .unwrap();
        let envelope = events.recv_timeout(Duration::from_secs(10)).unwrap();
        publisher.stop.store(true, Ordering::Release);
        worker.join().unwrap();
        assert_eq!(envelope.epoch, 2);
        assert!(
            matches!(&envelope.event, Event::ViewError(m) if m.contains("could not be saved")),
            "a save failure is its own notice, not an error that resets runs"
        );
    }
}
