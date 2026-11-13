//! The Review step: every change staged in the other steps, listed by what
//! happens to the photos, each one undoable, and applied with one button.
//! Nothing changes on disk before this step's Apply.
use super::plan_review::ReviewAction;
use super::states::{state_card, state_frame};
use super::*;
use egui_phosphor::regular as icon;
use std::path::{Path, PathBuf};

/// What the last Apply on the Review step did.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Outcome {
    /// "Moved 3 photos and sent 2 photos to the Trash."
    pub sentence: String,
    /// Every part finished, nothing is left to retry.
    pub done: bool,
    /// Listed photos that stayed in the folder they were listed in while
    /// the rest moved to another: their choices are still staged there.
    pub left_behind: Option<(usize, PathBuf)>,
    /// Folders the list sent to the Trash with their leftover files that
    /// could not go there, and so are still on disk.
    pub kept_folders: Vec<PathBuf>,
}

/// Why the Review step lists nothing and waits for Organize instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Blocked {
    /// Organize's automatic grouping has no result yet, and only Organize
    /// groups the photos.
    Ungrouped,
    /// Organize's folders come from metadata some photos' files could not
    /// give: reading it again from here would only fail again.
    MetadataUnread,
}

/// An Apply on the Review step whose moves come first: what it listed,
/// applied once the moves are done and the folder is read again.
#[derive(Clone, Debug)]
pub(super) struct Chain {
    /// The listed Trash marks, ratings and rotations, by each photo's id
    /// after the moves.
    pub decisions: HashMap<MediaId, Decision>,
    /// The RAW and XMP files listed as going to the Trash with those marks,
    /// by id after the moves. Nothing else follows a mark there.
    pub followers: HashSet<MediaId>,
    /// The folders the moves made: one left empty by the Trash marks is
    /// removed with them. Set once the moves are applied.
    pub created: Vec<PathBuf>,
    /// The folders listed as left empty by the Trash marks, each with the
    /// leftovers listed for it: removed after the marks are applied.
    pub tidy: Vec<(PathBuf, Vec<String>)>,
    /// The folder the moves were listed in.
    pub source: PathBuf,
    /// The folder read again after the moves; set once they are applied.
    pub root: Option<PathBuf>,
}

/// Asked by the Review step's Apply, before anything changes on disk: RAW
/// files the user rated, rotated or chose to keep would go to the Trash
/// with the JPEG that shares their name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RawQuestion {
    /// How many RAW files it is about.
    pub count: usize,
    /// "Don't ask again" is ticked: the answer is remembered.
    pub quiet: bool,
}

/// The same folder, however it is spelled: the scan resolves links, and a
/// folder name may differ only in case on a case-insensitive disk.
pub(super) fn same_folder(a: &Path, b: &Path) -> bool {
    a.components().eq(b.components())
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
        || same_directory(a, b)
}
/// One folder on disk under two spellings, links followed. The volume
/// answers (device and inode, or volume and file index on Windows).
fn same_directory(a: &Path, b: &Path) -> bool {
    a.is_dir() && same_file::is_same_file(a, b).unwrap_or(false)
}

/// The widest the Review step's column gets, so lines stay readable.
const COLUMN: f32 = 980.0;

impl PhotoSortApp {
    /// Shift+Enter, the bar's Review button and every "Review and apply"
    /// lead to the Review step.
    pub(super) fn review_changes(&mut self) {
        if self.step == Step::Review {
            if self.retry_plan.is_some() {
                self.review_remaining_operations();
            }
            return;
        }
        self.switch_step(Step::Review);
    }

    /// Lists the changes again when they may have changed: Organize's moves
    /// need a dry-run plan, which comes back from the file worker.
    pub(super) fn refresh_review(&mut self) {
        // A guard's Keep editing or Discard, or a discarded retry, drops the
        // list without an error: it is listed again.
        if self.step == Step::Review
            && !self.busy
            && self.plan.is_none()
            && self.error.is_none()
            && self.review_blocked.is_none()
        {
            self.review_stale = true;
        }
        if self.step != Step::Review
            || !self.review_stale
            || self.busy
            || self.scanning
            || self.catalog.is_none()
            || self.retry_plan.is_some()
            || self.review_needs_metadata()
            || self.close_authorized
        {
            return;
        }
        // An automatic grouping still running lists its folders when done.
        if self
            .analysis_run
            .is_some_and(|run| self.organize_run.is_some_and(|(owner, _)| owner == run))
        {
            return;
        }
        self.review_stale = false;
        self.review_targets = false;
        self.review_tidy = false;
        self.review_blocked = None;
        // Organize's folders need every photo's metadata; Organize says
        // which files could not be read and reads them again.
        if self.metadata_run.is_none()
            && !self.organize_metadata_ready()
            && self.media.iter().all(|m| {
                self.metadata.contains_key(&m.id) || self.metadata_failures.contains_key(&m.id)
            })
        {
            self.set_review_plan(None);
            self.review_blocked = Some(Blocked::MetadataUnread);
            return;
        }
        let grouping =
            self.automatic_organize_mode() && !self.organize_drafts.contains_key(&self.group_mode);
        // New names need Organize's folders first. Only Organize groups its
        // photos: an analysis started here would run another step's profile.
        if grouping
            && !self.organize_grouping_ready()
            && self.file_names.values().any(|name| !name.trim().is_empty())
        {
            self.set_review_plan(None);
            self.review_blocked = Some(Blocked::Ungrouped);
            return;
        }
        if !self.organize_changes_pending() {
            self.list_review(Plan::default());
            return;
        }
        // Another analysis comes first; Organize's plan waits for it.
        if grouping && self.analysis_run.is_some() {
            self.review_stale = true;
            return;
        }
        self.set_review_plan(None);
        self.review_preview = true;
        self.organize();
        if !self.busy {
            self.review_preview = false;
            // Metadata or a grouping comes first; the list waits for it.
            if self.metadata_run.is_some() || self.analysis_run.is_some() {
                self.review_stale = true;
            } else if self.error.is_none() {
                // Never a list without Organize's changes: say so instead.
                self.error = Some("Organize's folder changes could not be planned.".into());
            }
        }
    }

    /// Organize's folders come from the photos' metadata still being read:
    /// the list waits for it, and says how far it is.
    fn review_needs_metadata(&self) -> bool {
        self.metadata_run.is_some()
            && self.metadata_organize_mode()
            && !self.organize_drafts.contains_key(&self.group_mode)
    }

    /// The changes the Review step lists right now, added up as its
    /// sentence says them. `None` while there is no current list: on another
    /// step, while it is being made again, or during an Apply.
    pub(super) fn review_listed_count(&self) -> Option<usize> {
        (self.step == Step::Review
            && self.plan.is_some()
            && self.plan_rows.editable
            && !self.review_stale
            && !self.busy
            && !self.review_applying()
            && self.retry_plan.is_none())
        .then(|| self.plan_rows.total())
    }

    /// How many photos `staged_choices` lists, without listing them: what
    /// the Review badge counts off the Review step, and the folder and
    /// Close guards.
    pub(super) fn staged_choice_count(&self) -> usize {
        self.decisions
            .iter()
            .filter(|(id, d)| !d.is_empty() && self.index.contains_key(id))
            .count()
    }

    /// The same count in words, for the folder and Close guards: "1 photo",
    /// "3 photos and 1 video".
    pub(super) fn staged_choice_tally(&self) -> super::plan_review::Tally {
        let mut tally = super::plan_review::Tally::default();
        for (id, decision) in &self.decisions {
            match self.index.get(id) {
                Some(i) if !decision.is_empty() => {
                    if self.media[*i].kind == MediaKind::Video {
                        tally.videos += 1;
                    } else {
                        tally.photos += 1;
                    }
                }
                _ => {}
            }
        }
        tally
    }

    /// The staged Trash marks, ratings and rotations of photos in this
    /// folder, in folder order: what the Review step lists.
    pub(super) fn staged_choices(&self) -> Vec<(MediaId, Decision)> {
        let mut staged: Vec<(MediaId, Decision)> = self
            .decisions
            .iter()
            .filter(|(id, d)| !d.is_empty() && self.index.contains_key(id))
            .map(|(id, d)| (*id, d.clone()))
            .collect();
        staged.sort_by_key(|(id, _)| self.index[id]);
        staged
    }

    /// Lists Organize's `plan` with every staged choice. Two details come
    /// from the file worker a moment later, with the list already shown:
    /// which folders the Trash marks leave empty, and which rotations would
    /// re-save the image.
    pub(super) fn list_review(&mut self, plan: Plan) {
        self.set_review_plan(Some(plan));
        self.review_listed = self.staged_choices().into_iter().collect();
        if !self.ask_review_tidy() {
            self.ask_review_targets();
        }
    }

    /// Asks which folders that were there before hold nothing once the
    /// listed moves and Trash marks are applied, so "Tidying up" can name
    /// them before anything changes. True when the question was sent.
    fn ask_review_tidy(&mut self) -> bool {
        let trashed = self.plan_rows.trash_paths();
        let (Some(catalog), Some(plan)) = (self.catalog.clone(), &self.plan) else {
            return false;
        };
        // Marks on photos organized into another folder are applied there,
        // to folders this Apply makes.
        let elsewhere = plan
            .output_root
            .as_ref()
            .is_some_and(|output| *output != catalog.root);
        if trashed.is_empty() || elsewhere {
            return false;
        }
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let asked = self
            .runtime
            .io(IoWork::PlanTidy {
                catalog,
                moves: plan.clone(),
                trashed,
                cancelled: cancelled.clone(),
            })
            .is_ok();
        if asked {
            self.busy = true;
            self.review_tidy = true;
            // Cancelling leaves the list as it is: no folder is removed.
            self.plan_stop = Some(cancelled);
        }
        asked
    }

    /// Asks which listed rotations would re-save the image, to say so on
    /// their tiles.
    pub(super) fn ask_review_targets(&mut self) {
        let policy = self.preferences.metadata_writes;
        let updates: HashMap<MediaId, Decision> = self
            .review_listed
            .iter()
            .filter(|(_, d)| !d.rejected && d.turns % 4 != 0)
            .map(|(id, d)| (*id, d.clone()))
            .collect();
        // Only this policy ever re-saves an image to turn it.
        if policy != crate::core::metadata::WritePolicy::EmbedOrRotatePixels || updates.is_empty() {
            return;
        }
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if self
            .runtime
            .io(IoWork::PlanDecisions {
                catalog,
                decisions: updates,
                companions: self.companions,
                followers: None,
                created: Vec::new(),
                tidy: Vec::new(),
                policy,
                cancelled: cancelled.clone(),
            })
            .is_ok()
        {
            self.busy = true;
            self.review_targets = true;
            // Cancelling leaves the list as it is, without this detail.
            self.plan_stop = Some(cancelled);
            self.set_status("Checking how the rotations are saved…".into());
        }
    }

    /// Applies everything listed: Organize's moves first, then, once the
    /// folder is read again, the Trash marks, ratings and rotations of the
    /// moved photos, with no second review.
    pub(super) fn apply_review(&mut self) {
        if self.busy
            || self.scanning
            || self.review_stale
            || self.retry_plan.is_some()
            || self.raw_question.is_some()
        {
            return;
        }
        let Some(plan) = &self.plan else {
            return;
        };
        if self.plan_rows.is_empty() {
            return;
        }
        // A choice staged or taken back since the list was made, by a
        // finished analysis say, is listed before anything is applied.
        let live: HashMap<MediaId, Decision> = self.staged_choices().into_iter().collect();
        if live != self.review_listed {
            self.review_stale = true;
            return;
        }
        // RAW files the user kept would follow their JPEG to the Trash:
        // that is theirs to say, once, unless they answered for good.
        let wanted = self.plan_rows.wanted_raws();
        if wanted > 0
            && self.raw_answer.is_none()
            && self.preferences.limit_prompts.wanted_raw.is_none()
        {
            self.raw_question = Some(RawQuestion {
                count: wanted,
                quiet: false,
            });
            return;
        }
        let listed = live;
        let moves = !plan.actions.is_empty();
        // Apply's own plan is never taken for the list's write targets.
        self.review_targets = false;
        self.review_tidy = false;
        // Only the folders "Tidying up" names are removed after the marks.
        let tidy = self.plan_rows.emptied();
        self.review_outcome = Some(review_step::Outcome {
            sentence: self.plan_rows.applied_sentence(),
            done: false,
            left_behind: None,
            kept_folders: Vec::new(),
        });
        self.error = None;
        if moves {
            self.review_after_moves = (!listed.is_empty()).then(|| Chain {
                decisions: listed,
                followers: self.plan_rows.follower_ids(),
                created: Vec::new(),
                tidy,
                source: self.root.clone().unwrap_or_default(),
                root: None,
            });
            self.apply_reviewed_plan();
            if !self.busy {
                self.review_after_moves = None;
                self.review_outcome = None;
            }
        } else if !listed.is_empty() {
            self.apply_planned = true;
            let followers = self.plan_rows.follower_ids();
            self.plan_decisions(listed, Some(followers), Vec::new(), tidy);
            if !self.busy {
                self.apply_planned = false;
                self.review_outcome = None;
            }
        }
    }

    /// Ends the RAW question. With an answer, the changes are listed with
    /// it and applied, and "Don't ask again" remembers it; without one,
    /// nothing is applied.
    pub(super) fn answer_raw_question(
        &mut self,
        answer: Option<crate::core::preferences::WantedRaw>,
    ) {
        let Some(question) = self.raw_question.take() else {
            return;
        };
        let Some(answer) = answer else {
            return;
        };
        if question.quiet {
            self.preferences.limit_prompts.wanted_raw = Some(answer);
            self.save_preferences();
        }
        // The list, and the result said once applied, follow the answer. The
        // folders already named in "Tidying up" stay named: the answer only
        // adds files to the Trash, and no folder goes that was not listed.
        self.raw_answer = Some(answer);
        let tidy = self.plan_rows.emptied();
        self.set_review_plan(self.plan.clone());
        self.plan_rows.restore_emptied(tidy);
        self.apply_review();
        self.raw_answer = None;
    }

    /// The RAW question, over the Review step until it is answered.
    pub(super) fn raw_question_dialog(&mut self, ctx: &egui::Context) {
        use crate::core::preferences::WantedRaw;
        let Some(question) = &mut self.raw_question else {
            return;
        };
        let one = question.count == 1;
        let mut answer = None;
        let mut cancel = false;
        let response = modal(egui::Id::new("wanted_raw")).show(ctx, |ui| {
            ui.set_max_width(440.0_f32.min((ctx.content_rect().width() - 60.0).max(180.0)));
            ui.heading(if one {
                "Trash the RAW file you kept?"
            } else {
                "Trash the RAW files you kept?"
            });
            let body = if one {
                "1 RAW file that you rated, rotated or chose to keep shares its name with a photo marked for Trash. RAW and JPEG files stay together, so it would go to the Trash too.".to_owned()
            } else {
                format!(
                    "{} RAW files that you rated, rotated or chose to keep share their names with photos marked for Trash. RAW and JPEG files stay together, so they would go to the Trash too.",
                    question.count
                )
            };
            ui.add(egui::Label::new(body).wrap());
            ui.add(egui::Label::new(RichText::new("Nothing has changed yet.").color(MUTED)).wrap());
            ui.add_space(S1);
            ui.checkbox(&mut question.quiet, "Don't ask again")
                .on_hover_text("Remembers your answer. Settings can bring the question back.");
            ui.add_space(S1);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
                if ui
                    .button(if one { "Trash it too" } else { "Trash them too" })
                    .on_hover_text("Each goes to the Trash with its JPEG")
                    .clicked()
                {
                    answer = Some(WantedRaw::Trash);
                }
                if ui
                    .add(primary(if one {
                        "Keep the RAW file"
                    } else {
                        "Keep the RAW files"
                    }))
                    .on_hover_text("Only the JPEGs go to the Trash")
                    .clicked()
                {
                    answer = Some(WantedRaw::Keep);
                }
            });
        });
        if answer.is_some() || cancel || response.should_close() {
            self.answer_raw_question(answer);
        }
    }

    /// Gives up an Apply from the Review step that cannot go on: an error,
    /// a failed apply, a folder that is gone or a stopped worker. Without
    /// this, its lock on the steps and folders would stay until a restart.
    pub(super) fn abandon_review_apply(&mut self) {
        self.review_after_moves = None;
        self.apply_planned = false;
        self.review_targets = false;
        self.review_tidy = false;
        self.review_outcome = None;
    }

    /// An Apply from the Review step is still on its way: its moves, the
    /// folder read again after them, or the choices applied next.
    pub(super) fn review_applying(&self) -> bool {
        self.review_after_moves.is_some() || self.apply_planned
    }

    /// Leaves items where they are on disk with their names: the Review
    /// step's Undo on a move or rename.
    pub(super) fn keep_organize_items_in_place(&mut self, ids: &HashSet<MediaId>) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        if !self.organize_drafts.contains_key(&self.group_mode) {
            let draft =
                crate::core::organize::Draft::from_catalog(&catalog, self.organize_labels());
            self.organize_drafts.insert(self.group_mode, draft);
        }
        if let Some(draft) = self.organize_drafts.get_mut(&self.group_mode) {
            draft.keep_in_place(ids, &self.media, false);
        }
        for id in ids {
            self.file_names.remove(id);
        }
        self.board_epoch += 1;
        self.dirty = Some(Instant::now());
        self.review_stale = true;
    }

    /// The Review step's page.
    pub(super) fn review_page(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_rect_before_wrap();
        let width = full.width().min(COLUMN);
        let column = egui::Rect::from_min_size(
            egui::pos2(full.center().x - width / 2.0, full.top()),
            egui::vec2(width, full.height()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(column)
                .layout(egui::Layout::top_down(Align::Min)),
            |ui| self.review_column(ui),
        );
        ui.allocate_rect(full, Sense::hover());
        if let Some(action) = self.review_action.take() {
            match action {
                ReviewAction::Undo(list, item) => self.undo_review_item(list, item),
                ReviewAction::Open(step) => self.switch_step(step),
            }
        }
    }

    fn review_column(&mut self, ui: &mut egui::Ui) {
        let applying = self.applying_plan.is_some()
            || self.apply_planned
            || self.review_outcome.as_ref().is_some_and(|o| !o.done)
                && (self.busy || self.scanning);
        if self.retry_plan.is_some() {
            let mut open = false;
            state_card(
                ui,
                icon::WARNING_CIRCLE,
                "Some changes did not finish",
                "The changes that finished are applied. Retry the rest, choose another destination or discard them.",
                |ui| {
                    open = ui
                        .add_enabled(!self.busy, primary("Review unfinished changes…"))
                        .clicked()
                },
            );
            if open {
                self.review_remaining_operations();
            }
            return;
        }
        if applying {
            let total = self.applying_plan.as_ref().map(|plan| plan.actions.len());
            state_frame(ui, |ui| {
                ui.add(egui::Spinner::new().size(28.0).color(ACCENT));
                ui.add_space(S3);
                ui.label(RichText::new("Applying your changes").font(semibold(18.0)));
                ui.add_space(S1);
                let detail = match total {
                    Some(n) => format!(
                        "{n} file {} on the way. Stop finishes the current one and keeps the rest for later.",
                        if n == 1 { "operation" } else { "operations" }
                    ),
                    None => "Reading the folder again.".into(),
                };
                ui.add(egui::Label::new(RichText::new(detail).color(MUTED)).wrap());
            });
            return;
        }
        // The list stays on screen while only a detail of it is being
        // checked: an Undo answers at once, Apply waits for the detail.
        let detail = self.review_tidy || self.review_targets;
        if self.review_stale || (self.busy && !detail) {
            state_frame(ui, |ui| {
                ui.add(egui::Spinner::new().size(28.0).color(ACCENT));
                ui.add_space(S3);
                ui.label(RichText::new("Listing your changes").font(semibold(18.0)));
                if let Some((done, total)) = self.metadata_progress
                    && self.review_needs_metadata()
                {
                    ui.add_space(S1);
                    ui.label(
                        RichText::new(format!("Reading photo metadata · {done} / {total}"))
                            .color(MUTED),
                    );
                }
            });
            return;
        }
        if let Some(blocked) = self.review_blocked {
            let (title, body) = match blocked {
                Blocked::Ungrouped => (
                    "Organize's folders are not grouped yet",
                    "Renamed photos need Organize's grouping first, so nothing can be applied yet.",
                ),
                Blocked::MetadataUnread => (
                    "Some photos' metadata could not be read",
                    "Organize's folders depend on it, so nothing can be applied yet.",
                ),
            };
            let mut open = false;
            state_card(ui, icon::FOLDERS, title, body, |ui| {
                open = ui.add(primary("Open Organize")).clicked()
            });
            if open {
                self.switch_step(Step::Organize);
            }
            return;
        }
        if self.plan.is_none() {
            let mut retry = false;
            state_card(
                ui,
                icon::WARNING_CIRCLE,
                "The changes could not be listed",
                "Nothing was changed. Try again, or go back to Organize to check its folders.",
                |ui| retry = ui.add(primary("Try again")).clicked(),
            );
            if retry {
                self.error = None;
                self.review_stale = true;
            }
            return;
        }
        if self.plan_rows.is_empty() {
            self.review_empty(ui);
            return;
        }
        let Some(plan) = self.plan.take() else {
            return;
        };
        let mut rows = std::mem::take(&mut self.plan_rows);
        let mut apply = false;
        ui.add_space(S2);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Review").font(semibold(22.0)).color(INK));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                apply = ui
                    .add_enabled(
                        !self.busy,
                        primary(rows.apply_label()).min_size(egui::vec2(0.0, 30.0)),
                    )
                    .on_hover_text("Apply every change listed here")
                    .clicked();
            });
        });
        ui.add_space(S1);
        // An Apply just finished, and this folder had choices of its own.
        if let Some(outcome) = self.review_outcome.as_ref().filter(|o| o.done) {
            ui.add(egui::Label::new(RichText::new(&outcome.sentence).color(GREEN)).wrap());
            if let Some(text) = left_behind_text(outcome) {
                ui.add(egui::Label::new(RichText::new(text).color(MUTED)).wrap());
            }
            ui.add_space(S1);
        }
        ui.add(
            egui::Label::new(
                RichText::new(self.review_sentence(&plan, &rows))
                    .font(medium(15.0))
                    .color(INK),
            )
            .wrap(),
        );
        ui.add_space(S2);
        ui.separator();
        let height = ui.available_height();
        self.plain_view(ui, &plan, &mut rows, height);
        self.plan = Some(plan);
        self.plan_rows = rows;
        if apply {
            self.apply_review();
        }
    }

    /// Nothing staged: what was just applied, or how changes get here.
    fn review_empty(&mut self, ui: &mut egui::Ui) {
        let back = self
            .preferences
            .neighbor(Step::Review, false)
            .filter(|step| *step != Step::Review)
            .unwrap_or(Step::Cull);
        let back = if self.preferences.visible(self.review_from) && self.review_from != Step::Review
        {
            self.review_from
        } else {
            back
        };
        let mut open = None;
        match &self.review_outcome {
            Some(outcome) if outcome.done => {
                let sentence = outcome.sentence.clone();
                let left = left_behind_text(outcome);
                state_frame(ui, |ui| {
                    ui.label(RichText::new(icon::CHECK_CIRCLE).size(40.0).color(GREEN));
                    ui.add_space(S3);
                    let title = if left.is_some() {
                        "Changes applied"
                    } else {
                        "All changes applied"
                    };
                    ui.label(RichText::new(title).font(semibold(18.0)));
                    ui.add_space(S1);
                    ui.add(egui::Label::new(RichText::new(sentence).color(INK)).wrap());
                    if let Some(left) = left {
                        ui.add(egui::Label::new(RichText::new(left).color(MUTED)).wrap());
                    }
                    ui.add_space(S4);
                    if ui
                        .add(primary(format!("Back to {}", back.title())))
                        .clicked()
                    {
                        open = Some(back);
                    }
                });
            }
            _ => state_card(ui, icon::LIST_CHECKS, "Nothing to apply yet", "", |ui| {
                if ui
                    .add(primary(format!("Back to {}", back.title())))
                    .clicked()
                {
                    open = Some(back);
                }
            }),
        }
        if let Some(step) = open {
            self.switch_step(step);
        }
    }
}

/// "2 photos stayed in Holidays, still staged there."
fn left_behind_text(outcome: &Outcome) -> Option<String> {
    let name = |folder: &Path| {
        folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder.display().to_string())
    };
    let mut lines = Vec::new();
    if let Some((n, folder)) = &outcome.left_behind {
        lines.push(format!(
            "{n} {} in {}, still staged there.",
            if *n == 1 {
                "photo stayed"
            } else {
                "photos stayed"
            },
            name(folder)
        ));
    }
    match outcome.kept_folders.as_slice() {
        [] => {}
        [folder] => lines.push(format!(
            "{} could not be moved to the Trash, so it is still there with its leftover files.",
            name(folder)
        )),
        folders => lines.push(format!(
            "{} folders could not be moved to the Trash, so they are still there with their leftover files: {}.",
            folders.len(),
            folders.iter().map(|f| name(f)).collect::<Vec<_>>().join(", ")
        )),
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}
