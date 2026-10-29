//! Application-owned review and departure guards, shared by all workflow pages.
use super::*;
#[derive(Clone)]
pub(super) enum Destination {
    Step(Step),
    Folder(PathBuf),
    Close,
}
pub(super) struct SmokeExit {
    delay: Duration,
    deadline: Option<Instant>,
}
impl PhotoSortApp {
    /// Request quit once after the first UI frame and the configured delay.
    /// Reuses all departure guards and normal worker shutdown; never forces exit.
    pub fn with_smoke_test(mut self, delay: Duration) -> Self {
        // Smoke runs must stay short and must not use up the first launch.
        self.intro = super::intro::Intro::Off;
        self.smoke_exit = Some(SmokeExit {
            delay,
            deadline: None,
        });
        self
    }
    pub(super) fn smoke_close_requested(&mut self, ctx: &egui::Context) -> bool {
        self.smoke_close_requested_at(ctx, Instant::now())
    }
    /// Uses the monotonic clock, not egui's input time: while the window is
    /// hidden, eframe runs no egui pass and input time stays at the last shown frame.
    pub(super) fn smoke_close_requested_at(&mut self, ctx: &egui::Context, now: Instant) -> bool {
        let Some(smoke) = &mut self.smoke_exit else {
            return false;
        };
        let deadline = *smoke.deadline.get_or_insert(now + smoke.delay);
        if now < deadline {
            ctx.request_repaint_after(deadline - now);
            return false;
        }
        tracing::info!(
            late_ms = (now - deadline).as_millis() as u64,
            "smoke test requesting quit"
        );
        // One request only: cancelling a departure dialog must not reopen it.
        self.smoke_exit = None;
        true
    }
    pub(super) fn handle_close_request(&mut self, ctx: &egui::Context, native_requested: bool) {
        let window_requested = ctx.input(|i| i.viewport().close_requested());
        if (window_requested || native_requested) && !self.close_authorized {
            if window_requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            self.request_departure(Destination::Close);
        }
        if self.close_authorized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    pub(super) fn review_remaining_operations(&mut self) -> bool {
        if let Some(plan) = self.retry_plan.clone() {
            if !self.busy && !self.scanning {
                self.set_review_plan(Some(plan));
                self.plan_open = true;
            }
            return true;
        }
        false
    }
    pub(super) fn discard_remaining_operations(&mut self) {
        if self.busy {
            return;
        }
        if let (Some(root), Some(ticket)) = (self.root.clone(), self.recovery_ticket.clone()) {
            match self.runtime.io(IoWork::DiscardRecovery { root, ticket }) {
                Ok(()) => {
                    self.busy = true;
                    self.set_status("Discarding saved file plan…".into());
                }
                Err(error) => {
                    self.error = Some(error);
                }
            }
            return;
        }
        let organize = self
            .retry_plan
            .as_ref()
            .or(self.plan.as_ref())
            .is_some_and(|plan| plan.output_root.is_some());
        self.retry_plan = None;
        self.set_review_plan(None);
        self.plan_open = false;
        self.rotation_plan = false;
        self.error = None;
        if organize {
            self.group_mode = GroupMode::Current;
            self.organize_drafts.clear();
            self.file_names.clear();
            self.output = self
                .root
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
        }
        self.dirty = Some(Instant::now());
        self.set_status(
            "Unfinished file plan discarded · completed operations remain applied".into(),
        );
        if let Some(destination) = self.after_apply.take() {
            self.continue_departure(destination);
        }
    }
    pub(super) fn apply_reviewed_plan(&mut self) {
        if self.busy || self.scanning {
            return;
        }
        if let Some(plan) = self.plan.take() {
            self.runtime.cancel_analysis();
            self.analysis_run = None;
            let Some(_) = &self.root else {
                self.set_review_plan(Some(plan));
                self.error = Some("Open a library before applying a file plan".into());
                return;
            };
            let mut views = self.saved_views.clone();
            if self.pending_bookmark.is_none() && self.view_data_ready.contains(&self.step) {
                views.step = Some(self.step);
                views.bookmarks.insert(self.step, self.capture_bookmark());
                if let Some(catalog) = &self.catalog {
                    views.capture_catalog_fingerprints(catalog, &self.index);
                }
            }
            let Some(catalog) = &self.catalog else {
                self.set_review_plan(Some(plan));
                self.error = Some("Wait for library discovery before applying changes".into());
                return;
            };
            let context = crate::core::operations::recovery::Context::capture_catalog(
                self.rotation_plan,
                catalog,
                &self.index,
                self.session_intent(),
                views.clone(),
            );
            let stop_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
            match self.runtime.io(IoWork::ApplyRecoverable {
                context: Box::new(context),
                plan: plan.clone(),
                ticket: self.recovery_ticket.clone(),
                stop_requested: stop_requested.clone(),
            }) {
                Ok(()) => {
                    self.saved_views = views;
                    self.applying_plan = Some(plan);
                    self.apply_stop = Some(stop_requested);
                    self.dirty = None;
                    self.busy = true;
                    self.plan_open = false;
                    self.error = None;
                    self.set_status("Applying reviewed changes…".into());
                }
                Err(error) => {
                    self.set_review_plan(Some(plan));
                    self.after_apply = None;
                    self.error = Some(error);
                }
            }
        }
    }
    pub(super) fn stop_file_operations(&mut self) {
        if self.busy
            && self.applying_plan.is_some()
            && let Some(stop) = &self.apply_stop
            && !stop.swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            self.set_status("Stopping file changes…".into());
        }
    }
    pub(super) fn allow_review_departure(&mut self) -> bool {
        let unfinished = match self.step {
            Step::PickBest => self.pick_review.unfinished(),
            _ => false,
        };
        if unfinished {
            self.review_notice = Some(self.step);
        }
        !unfinished
    }
    pub(super) fn has_unsaved_plan_extras(&self) -> bool {
        self.organize_changes_pending()
            && self
                .retry_plan
                .as_ref()
                .is_none_or(|plan| plan.output_root.is_some())
    }
    pub(super) fn save_plan_and_quit(&mut self) {
        if self.has_unsaved_plan_extras()
            || !self.plan_open
            || self.review_notice.is_some()
            || self.busy
            || self.scanning
            || self.retry_edit.is_some()
            || self.retry_plan.is_none()
            || self.recovery_ticket.is_none()
            || !matches!(self.after_apply, Some(Destination::Close))
        {
            return;
        }
        let Some(root) = self.root.clone().filter(|_| self.catalog.is_some()) else {
            return;
        };
        self.capture_saved_view();
        match self.runtime.io(IoWork::Checkpoint {
            root,
            session: self.session_snapshot(),
            views: self.saved_views.clone(),
        }) {
            Ok(()) => {
                self.close_save_pending = true;
                self.busy = true;
                self.set_status("Saving before quitting…".into());
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn request_departure(&mut self, destination: Destination) {
        // A second quit request can arrive while the durable plan is already open.
        // Unsaved plan/editor and in-flight mutation guards still take precedence.
        if matches!(destination, Destination::Close)
            && self.plan_open
            && !self.busy
            && !self.scanning
            && self.retry_edit.is_none()
            && self.retry_plan.is_some()
            && self.recovery_ticket.is_some()
            && self.organize_departure.is_none()
            && self.departure.is_none()
            && self.review_notice.is_none()
        {
            if self.allow_review_departure() {
                self.after_apply = Some(destination);
            } else {
                self.plan_open = false;
            }
            return;
        }
        if self.busy
            || self.plan_open
            || self.organize_departure.is_some()
            || self.departure.is_some()
            || self.review_notice.is_some()
            || !self.allow_review_departure()
        {
            return;
        }
        if self.review_remaining_operations() {
            self.after_apply = Some(destination);
            return;
        }
        if self.organize_changes_pending() {
            self.organize_departure = Some(destination);
            self.set_review_plan(None);
            self.organize();
            return;
        }
        self.continue_departure(destination);
    }
    pub(super) fn continue_departure(&mut self, destination: Destination) {
        if self.decisions.values().any(|d| !d.is_empty()) {
            self.departure = Some(destination);
        } else {
            self.finish_departure(destination);
        }
    }
    fn finish_departure(&mut self, destination: Destination) {
        match destination {
            Destination::Step(step) => self.set_step(step),
            Destination::Folder(path) => self.open_now(path),
            Destination::Close => self.close_authorized = true,
        }
    }
    pub(super) fn review_dialogs(&mut self, ctx: &egui::Context) {
        if let Some(step) = self.review_notice {
            let mut action = 0;
            modal(egui::Id::new("unfinished_review")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading("Confirm or reset this choice");
                ui.label("This review has unfinished choices. Confirm them or restore the suggestions before moving on.");
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Keep reviewing").clicked() { action = 1; }
                    if ui.button("Confirm").clicked() { action = 2; }
                    if ui.button("Reset default").clicked() { action = 3; }
                    if ui.button("Reset all").clicked() { action = 4; }
                });
            });
            if action > 0 {
                self.review_notice = None;
                match (step, action) {
                    (Step::PickBest, 2) => self.confirm_pick(false),
                    (Step::PickBest, 3 | 4) => self.reset_pick(action == 4),
                    _ => {}
                }
            }
        }
        if self.departure.is_some() {
            let mut action = 0;
            modal(egui::Id::new("pending_changes")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.heading("Resolve staged changes");
                ui.label(format!("{} photos have staged ratings, rotations or Trash marks.",self.decisions.values().filter(|d|!d.is_empty()).count()));
                ui.label("Discard clears these staged choices. Your files remain unchanged until you review and apply a plan.");
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { action = 1; }
                    if ui.button("Discard").clicked() { action = 2; }
                    if ui.button("Review and apply…").clicked() { action = 3; }
                });
            });
            if action > 0 {
                let destination = self.departure.take().unwrap();
                match action {
                    2 => {
                        self.discard_staged_choices();
                        self.continue_departure(destination);
                    }
                    3 => {
                        self.after_apply = Some(destination);
                        self.review_decisions();
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum OrganizeResolution {
    Cancel,
    Discard,
    Review,
}
impl PhotoSortApp {
    /// Drops every planned Organize change: folder edits and new folders in
    /// every grouping, file renames and another Organize into folder, so the
    /// folders are as they are on disk. Trash marks, ratings and rotations
    /// are decisions shared with the other steps and stay.
    pub(super) fn discard_organize_changes(&mut self) {
        self.dirty = Some(Instant::now());
        self.organize_drafts.clear();
        self.file_names.clear();
        self.group_mode = GroupMode::Current;
        self.output = self
            .root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        self.organize_preview_requested = false;
        self.set_review_plan(None);
    }
    pub(super) fn organize_changes_pending(&self) -> bool {
        if self.step != Step::Organize || self.scanning {
            return false;
        }
        let Some(catalog) = &self.catalog else {
            return false;
        };
        let draft = self.organize_drafts.get(&self.group_mode);
        let ready = self.organize_grouping_ready();
        if !ready && draft.is_none() && self.file_names.values().all(|s| s.trim().is_empty()) {
            return false;
        }
        let generated;
        let labels = if let Some(draft) = draft {
            draft.labels()
        } else {
            generated = self.organize_labels();
            &generated
        };
        let directories = draft.map(|d| d.directory_edits()).unwrap_or_default();
        crate::core::operations::organize_changes_pending(
            catalog,
            labels,
            &self.file_names,
            &PathBuf::from(&self.output),
            &directories,
        )
    }
    pub(super) fn resolve_organize_departure(&mut self, resolution: OrganizeResolution) {
        let Some(destination) = self.organize_departure.take() else {
            return;
        };
        match resolution {
            OrganizeResolution::Cancel => {
                self.set_review_plan(None);
                self.error = None;
            }
            OrganizeResolution::Discard => {
                self.runtime.cancel_analysis();
                self.analysis_run = None;
                self.organize_run = None;
                self.progress = None;
                self.discard_organize_changes();
                self.error = None;
                self.continue_departure(destination);
            }
            OrganizeResolution::Review => {
                if self.plan.as_ref().is_some_and(|p| !p.actions.is_empty()) {
                    self.after_apply = Some(destination);
                    self.plan_open = true;
                } else {
                    self.organize_departure = Some(destination);
                }
            }
        }
    }
    pub(super) fn organize_departure_dialog(&mut self, ctx: &egui::Context) {
        if self.organize_departure.is_none() || self.busy {
            return;
        }
        let mut resolution = None;
        modal(egui::Id::new("pending_organize_changes")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading("Resolve folder changes");
            if let Some(plan) = &self.plan {
                ui.label(format!("{} file and folder operations are waiting to be applied.", plan.actions.len()));
            }
            if let Some(error) = &self.error { ui.colored_label(RED, error); }
            if !self.organize_metadata_ready() { self.metadata_status(ui); }
            ui.label("Keep editing, discard the proposed organization, or review and apply it before leaving this step.");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Keep editing").clicked() { resolution = Some(OrganizeResolution::Cancel); }
                if ui.button("Discard folder changes").clicked() { resolution = Some(OrganizeResolution::Discard); }
                if ui.add_enabled(self.plan.as_ref().is_some_and(|p| !p.actions.is_empty()), egui::Button::new("Review and apply…")).clicked() { resolution = Some(OrganizeResolution::Review); }
            });
        });
        if let Some(resolution) = resolution {
            self.resolve_organize_departure(resolution);
        }
    }
}

#[cfg(test)]
#[path = "transitions/organize_departure_tests.rs"]
mod organize_departure_tests;
