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
        // A queued session save belongs to its explicit root and is safe after navigation.
        if publisher.cancelled(epoch)
            && !matches!(
                message.work,
                IoWork::Save { .. }
                    | IoWork::MergeSession { .. }
                    | IoWork::Checkpoint { .. }
                    | IoWork::SaveViews { .. }
                    | IoWork::ForgetViews(_)
                    | IoWork::SavePreferences { .. }
            )
        {
            continue;
        }
        let _ = publisher.guard(super::Worker::Files, || {
            let files = Files { publisher, epoch };
            if let Err(e) = files.handle(message.work) {
                publisher.send(epoch, Event::Error(format!("{e:#}")));
            }
        });
    }
}

struct Files<'a> {
    publisher: &'a Publisher,
    epoch: u64,
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
        self.publisher.send(self.epoch, event);
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
            IoWork::MergeSession { root, session } => self.merge_session(&root, session),
            IoWork::Save { root, session } => self.save(&root, &session),
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
        let restored =
            operations::recovery::Store::standard().restore(&root, &|| self.cancelled())?;
        let catalog = Catalog::scan(
            &root,
            || self.cancelled(),
            |batch| self.send(Event::Batch(batch)),
        )?;
        let mut session = Session::load(&catalog.root)?;
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

    fn merge_session(&self, root: &std::path::Path, session: Session) -> anyhow::Result<()> {
        let mut existing = Session::load(root)?;
        existing.merge(session);
        existing.save(root)?;
        self.send(Event::Saved);
        Ok(())
    }

    fn save(&self, root: &std::path::Path, session: &Session) -> anyhow::Result<()> {
        session.save(root)?;
        self.send(Event::Saved);
        Ok(())
    }
}
