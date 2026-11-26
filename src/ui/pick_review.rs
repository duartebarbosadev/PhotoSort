use super::review_card::{Badge, CardEvent, CardPhoto, Fact, GridClick, GroupLink, ReviewCard};
use super::*;
use crate::core::pick_review::{Cluster, Reason, Review};
use crate::core::text::{TO_TRASH, TRASH};

/// Photos a card shows, best first; the panel beside the grid shows them all.
const CARD_PHOTOS: usize = 3;

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What the cards' shapes were taken from: the review, the photos shown
/// last (their stamp and how many) and the metadata.
type Shaped = (u64, u64, Option<usize>, (usize, u64));

/// What Pick Best's grid is drawn from, kept between frames: a frame in
/// which nothing changed reads it instead of working it out for every group.
#[derive(Default)]
pub(super) struct Deck {
    shaped: Option<Shaped>,
    /// Each group's card: its photos' widths over their heights, and how
    /// many it shows.
    aspects: Vec<([f32; CARD_PHOTOS], usize)>,
    /// Changes whenever a card's shape may have: the grid asks its cards
    /// again.
    stamp: u64,
    /// The review and the decisions the counts were taken from.
    counted: Option<(u64, u64)>,
    /// The groups' photos going to Trash, and their bytes.
    trash: usize,
    freed: u64,
    /// Photos and videos going to Trash that are in no group here: marked
    /// in another step. With `trash`, every Trash mark Review lists.
    elsewhere: usize,
    /// Groups Accept all suggestions would change.
    changed: usize,
}

impl PhotoSortApp {
    /// Easy Delete, Fix Rotation and Pick Best start their analysis on entry
    /// when they have nothing to show yet, so the step is never empty.
    /// It is also called when a run ends, for a step entered while another
    /// step's analysis was still running.
    pub(super) fn start_ready_analysis(&mut self) {
        let unanalysed = match self.step {
            Step::PickBest | Step::EasyDelete => self.step_unanalysed(),
            Step::Rotation => {
                !self.view_data_ready.contains(&Step::Rotation)
                    && self.rotation_review.ids().is_empty()
            }
            Step::Organize => self.organize_grouping_wanted(),
            _ => false,
        };
        if unanalysed
            && self.model_issue.is_none()
            && !self.model_dialog
            && self.catalog.is_some()
            && self.retry_plan.is_none()
            && !self.scanning
            && !self.busy
            && !self.close_authorized
            && self.departure.is_none()
            && self.after_apply.is_none()
            && !self.plan_open
            && self.analysis_run.is_none()
        {
            self.analyze_current_workflow();
        }
    }

    /// Easy Delete or Pick Best with no results because its own analysis has
    /// not run, or was cancelled: nothing found yet, which is not a result.
    pub(super) fn step_unanalysed(&self) -> bool {
        match self.step {
            Step::PickBest => {
                !self.view_data_ready.contains(&Step::PickBest)
                    && self.pick_run.is_none()
                    && self.pick_review.clusters().is_empty()
            }
            Step::EasyDelete => {
                !self.view_data_ready.contains(&Step::EasyDelete)
                    && self.delete_run.is_none()
                    && self.delete_review.entries().is_empty()
            }
            _ => false,
        }
    }

    pub(super) fn finish_pick_analysis(&mut self) {
        self.view_data_ready.insert(Step::PickBest);
        let clusters = crate::core::pick_review::from_analysis(
            &self.media,
            &self.analysis,
            &self.cull_groups,
            &self.metadata,
        );
        self.pick_review.replace(clusters);
        if self.step == Step::PickBest {
            self.trash_last = None;
        }
        self.pick_run = None;
        self.stage_pick_suggestions();
        self.finish_step_status(Step::PickBest);
        self.filter_dirty = true;
        self.review_changed();
    }
    /// With the setting on, suggestions are the default decisions: each
    /// photo's is staged once, when it is first found or when Pick Best
    /// opens after staged choices were discarded.
    pub(super) fn stage_pick_suggestions(&mut self) {
        if !self.preferences.pick_best_auto || !self.view_data_ready.contains(&Step::PickBest) {
            return;
        }
        let marked = self.pick_review.stage_new_suggestions(&mut self.decisions);
        if !marked.is_empty() {
            self.note_trash_steps(&marked, Step::PickBest);
            self.changed_items(marked);
        }
    }
    pub(super) fn pick_enabled(&self) -> bool {
        !self.busy && self.pick_run.is_none()
    }
    /// The selected group.
    pub(super) fn pick_group(&self) -> Option<usize> {
        self.pick_current
            .and_then(|id| self.pick_review.group_of(id))
    }
    fn pick_cluster(&self) -> Option<&Cluster> {
        self.pick_group().map(|g| &self.pick_review.clusters()[g])
    }
    /// The selected group's photos, best first, as the panel shows them.
    pub(super) fn pick_panel_ids(&self) -> Vec<MediaId> {
        self.pick_cluster()
            .map(|c| self.pick_order(c))
            .unwrap_or_default()
    }
    /// A group's photos as its card and panel show them: best first, and
    /// those already marked for Trash when the list was built last.
    pub(super) fn pick_order(&self, cluster: &Cluster) -> Vec<MediaId> {
        let (last, first): (Vec<MediaId>, Vec<MediaId>) =
            cluster.ordered.iter().partition(|id| self.shown_last(**id));
        first.into_iter().chain(last).collect()
    }
    /// Switches one photo between Keep and Trash.
    pub(super) fn toggle_pick(&mut self, id: MediaId) {
        if !self.pick_enabled() {
            return;
        }
        self.pick_review.toggle(id, &mut self.decisions);
        self.by_hand.insert(id);
        self.set_active(id);
        self.changed_items([id]);
    }
    /// The number keys: the nth photo of the group shown beside the grid.
    pub(super) fn toggle_pick_slot(&mut self, slot: usize) {
        if let Some(id) = self.pick_panel_ids().get(slot).copied() {
            self.toggle_pick(id);
        }
    }
    /// The selected group's Accept or Undo: one group, asked for by hand.
    /// Accept takes its suggestion in full, and Undo takes it back to where
    /// the step started it. Its photos count as undecided again.
    pub(super) fn undo_pick(&mut self) {
        if let Some(group) = self.pick_group() {
            self.undo_pick_group(group);
        }
    }
    fn undo_pick_group(&mut self, group: usize) {
        if !self.pick_enabled() {
            return;
        }
        let Some(members) = self
            .pick_review
            .clusters()
            .get(group)
            .map(|c| c.ordered.clone())
        else {
            return;
        };
        let clear = !self.pick_undecided(group) && !self.preferences.pick_best_auto;
        if !clear {
            self.pick_review.accept(&[group], &mut self.decisions);
        }
        self.undo_group(members, clear);
    }
    /// Accept all suggestions: every photo not decided by hand takes its
    /// suggestion. A choice made by hand stays as it is, and a group whose
    /// suggested photo is marked for Trash is left alone, so no group ever
    /// ends with no photo kept.
    pub(super) fn accept_pick_defaults(&mut self) {
        if !self.pick_enabled() {
            return;
        }
        let photos: Vec<MediaId> = self
            .pick_review
            .clusters()
            .iter()
            .flat_map(|c| c.ordered.iter().copied())
            .collect();
        let before = self.accept_marks(&photos);
        let accepted = self
            .pick_review
            .accept_undecided(&mut self.decisions, &self.by_hand);
        self.note_accepted(Step::PickBest, before);
        self.accepted_status(accepted.suggestions);
        self.changed_items(accepted.photos);
    }
    /// A click on the grid's empty space: no group is selected, and the
    /// panel beside the grid closes.
    pub(super) fn deselect_pick_group(&mut self) {
        self.pick_panel = false;
        self.pick_current = None;
        self.active = None;
    }
    /// The previous or next group, shown beside the grid.
    pub(super) fn navigate_pick(&mut self, delta: isize) {
        let groups = self.pick_review.clusters().len();
        if groups == 0 {
            return;
        }
        let next = match self.pick_group() {
            Some(current) => (current as isize + delta).clamp(0, groups as isize - 1) as usize,
            None if delta < 0 => groups - 1,
            None => 0,
        };
        self.pending_bookmark = None;
        let best = self.pick_review.clusters()[next].best();
        self.set_active(best);
        self.pick_panel = true;
    }
    /// A: the selected group takes its suggestion, as its Accept suggestion
    /// button does. A group decided already stays as it is.
    pub(super) fn accept_pick_card(&mut self) {
        if let Some(group) = self.pick_group()
            && self.pick_link(group) == Some(GroupLink::Accept)
        {
            self.undo_pick_group(group);
        }
    }
    /// U: the selected group's Undo, as its button.
    pub(super) fn undo_pick_card(&mut self) {
        if let Some(group) = self.pick_group()
            && self.pick_link(group) == Some(GroupLink::Undo)
        {
            self.undo_pick_group(group);
        }
    }
    /// The panel's Undo, beside its Keep/Trash switches: Accept suggestion
    /// while the group is undecided, as on its card.
    pub(super) fn pick_undo_button(&mut self, ui: &mut egui::Ui) {
        if let Some(group) = self.pick_group() {
            let (button, key) = if self.pick_undecided(group) {
                (primary(ACCEPT), "A")
            } else {
                (egui::Button::new("Undo"), "U")
            };
            if ui
                .add_enabled(self.pick_link(group).is_some(), button)
                .on_hover_text(key)
                .clicked()
            {
                self.undo_pick();
            }
        }
    }
    /// What a group's header offers, if anything: Accept while nobody has
    /// decided it (an undecided group differs from its suggestion), then
    /// Undo.
    fn pick_link(&self, group: usize) -> Option<GroupLink> {
        let changed = self.pick_review.changed(group, &self.decisions);
        GroupLink::of(
            self.pick_enabled(),
            self.pick_undecided(group),
            changed,
            self.preferences.pick_best_auto,
            changed,
        )
    }
    /// A group nobody has decided yet: no photo of it marked for Trash and
    /// none chosen by hand. It shows its suggestion and nothing else.
    pub(super) fn pick_undecided(&self, group: usize) -> bool {
        !self
            .pick_review
            .decided(group, &self.decisions, &self.by_hand)
    }
}

/// The card's first decision. Its wording matches Accept all suggestions.
pub(super) const ACCEPT: &str = "Accept suggestion";
/// The line over the grid while nothing is marked: what to do.
fn hint() -> String {
    format!(
        "Nothing marked yet: accept a group's suggestion, or click a photo to mark it for the {TRASH}."
    )
}

impl PhotoSortApp {
    /// Pick Best's only view: a summary with Accept all suggestions, then
    /// one card per group with its best photos. Clicking a card opens all of
    /// its photos large in a panel beside the grid.
    pub(super) fn pick_grid(&mut self, ui: &mut egui::Ui) {
        self.pick_deck();
        self.pick_summary(ui);
        ui.add_space(S2);
        let shown = self.pick_panel && self.pick_current.is_some();
        let layout = super::review_card::panel_layout(
            ui,
            "pick_panel_slide",
            shown,
            &mut self.pick_panel_width,
        );
        let groups = self.pick_review.clusters().len();
        let current = self.pick_group();
        let stamp = self.grids.pick.stamp;
        let clicked = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(layout.grid)
                    .layout(egui::Layout::top_down(Align::Min)),
                |ui| {
                    self.flat_card_grid(
                        ui,
                        "pick",
                        groups,
                        stamp,
                        |app, i, shapes| {
                            if let Some((aspects, shown)) = app.grids.pick.aspects.get(i) {
                                shapes.extend_from_slice(&aspects[..*shown]);
                            }
                        },
                        |app, i| app.pick_card_has_facts(i),
                        |app, i| Some(app.pick_card(i)),
                        current,
                    )
                },
            )
            .inner;
        if layout.slide > 0.0
            && let Some(width) =
                super::review_card::side_panel(ui, &layout, "pick_side_panel", |ui| {
                    self.pick_compare_panel(ui)
                })
        {
            self.pick_panel_width = Some(width);
        }
        ui.advance_cursor_after_rect(layout.area);
        match clicked {
            Some(GridClick::Card(i, event)) => self.pick_card_event(i, event),
            Some(GridClick::Background) => self.deselect_pick_group(),
            None => {}
        }
    }

    /// One line over the grid: what is staged, the photo size, and Accept
    /// all suggestions.
    fn pick_summary(&mut self, ui: &mut egui::Ui) {
        let deck = &self.grids.pick;
        let (trash, freed, changed) = (deck.trash, deck.freed, deck.changed);
        // The line counts this step's groups and says so: Review's own
        // count takes in what other steps marked, named after it here.
        let mut summary = format!(
            "{} in these groups going {TO_TRASH} · frees {}",
            count(trash, "photo", "photos"),
            super::review_card::size_text(freed),
        );
        if deck.elsewhere > 0 {
            summary.push_str(&format!(" · {} more marked in other steps", deck.elsewhere));
        }
        // Before anything is marked, the line says what to do instead.
        if trash == 0 && deck.elsewhere == 0 && !self.pick_review.clusters().is_empty() {
            summary = hint();
        }
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                match self.accept_or_undo_button(ui, changed > 0, self.pick_enabled()) {
                    Some(true) => self.undo_accepted(Step::PickBest),
                    Some(false) => self.accept_pick_defaults(),
                    None => {}
                }
                ui.add_space(S3);
                self.size_slider(ui);
                ui.add_space(S3);
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    super::review_card::summary_label(ui, &summary);
                });
            });
        });
    }

    /// The selected group's photos, large, beside the grid, with zoom,
    /// Keep/Trash and Undo. The grid is its strip.
    fn pick_compare_panel(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        if self.pick_group().is_none() {
            self.pick_panel = false;
            return;
        }
        // No title: the selected card beside the panel has it.
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, icon::X, "Close · Esc").clicked() {
                    self.pick_panel = false;
                }
            });
        });
        ui.add_space(S1);
        self.review_canvas(ui);
    }

    /// What the grid is drawn from, brought up to date: each card's photos'
    /// shapes when the groups, the photos shown last or a photo's metadata
    /// changed, and the counts when a decision did. A frame in which none
    /// of that happened looks up the stamps.
    fn pick_deck(&mut self) {
        let mut deck = std::mem::take(&mut self.grids.pick);
        let review = self.pick_review.revision();
        let shaped = (
            review,
            self.grids.last_noted,
            self.trash_last.as_ref().map(HashSet::len),
            self.metadata_stamp(),
        );
        if deck.shaped != Some(shaped) {
            deck.aspects.clear();
            deck.aspects
                .extend(self.pick_review.clusters().iter().map(|cluster| {
                    let mut aspects = [0.0; CARD_PHOTOS];
                    let mut shown = 0;
                    for (slot, id) in aspects.iter_mut().zip(self.pick_order(cluster)) {
                        *slot = self.photo_aspect(id, 0);
                        shown += 1;
                    }
                    (aspects, shown)
                }));
            deck.shaped = Some(shaped);
            deck.stamp = crate::core::revision::next();
        }
        let counted = (review, self.decisions.revision());
        if deck.counted != Some(counted) {
            let trash = self
                .pick_review
                .clusters()
                .iter()
                .flat_map(|c| c.ordered.iter())
                .filter(|id| self.decisions.get(id).is_some_and(|d| d.rejected));
            (deck.trash, deck.freed) = trash.fold((0, 0), |(count, bytes), id| {
                let size = self
                    .index
                    .get(id)
                    .map_or(0, |i| self.media[*i].fingerprint.bytes);
                (count + 1, bytes + size)
            });
            deck.elsewhere = self
                .decisions
                .iter()
                .filter(|(id, d)| d.rejected && self.index.contains_key(id))
                .count()
                .saturating_sub(deck.trash);
            deck.changed = (0..self.pick_review.clusters().len())
                .filter(|group| {
                    self.pick_review
                        .pending(*group, &self.decisions, &self.by_hand)
                })
                .count();
            deck.counted = Some(counted);
        }
        self.grids.pick = deck;
    }

    /// Why PhotoSort suggests keeping the group's best photo, for its
    /// Suggested mark: what put it ahead of the next best one.
    pub(super) fn pick_keep_why(&self, cluster: &Cluster) -> String {
        let best = cluster.best();
        let next = cluster.ordered[1];
        let why = match cluster.reason {
            Reason::FullSize => {
                "Full size; the next best is a smaller or cropped copy of it.".to_owned()
            }
            Reason::Sharper => match cluster.soft.len() {
                1 => "Sharp; 1 other photo is softer.".to_owned(),
                n => format!("Sharp; {n} others are softer."),
            },
            Reason::EyesOpen => match cluster.blinks(next) {
                1 => "Eyes open; closed for someone in the next best.".to_owned(),
                n => format!("Eyes open; closed for {n} people in the next best."),
            },
            Reason::Score => format!(
                "Best overall score: {:.0}, next best {:.0}.",
                cluster.scores[&best], cluster.scores[&next]
            ),
        };
        format!("{} {why}", super::delete_review::SUGGESTED)
    }

    /// What a card says under each of the photos given: only whose eyes
    /// are closed, and that the winner's are open. Dates, sharpness, sizes
    /// and the like are left out as clutter; the Suggested mark's tooltip
    /// says why its photo won.
    fn pick_facts(&self, cluster: &Cluster, ids: &[MediaId]) -> Vec<Vec<Fact>> {
        let best = cluster.best();
        ids.iter()
            .map(|id| {
                let fact = cluster.eyes.get(id).and_then(|eyes| {
                    if eyes.closed > 0 {
                        Some(Fact {
                            text: match eyes.closed {
                                1 => "Eyes closed".to_owned(),
                                n => format!("{n} with eyes closed"),
                            },
                            better: false,
                        })
                    } else {
                        (eyes.open > 0).then(|| Fact {
                            text: "Eyes open".to_owned(),
                            better: *id == best,
                        })
                    }
                });
                fact.into_iter().collect()
            })
            .collect()
    }

    /// Whether a group's card has a line under its photos.
    fn pick_card_has_facts(&self, group: usize) -> bool {
        let cluster = &self.pick_review.clusters()[group];
        let shown: Vec<MediaId> = self
            .pick_order(cluster)
            .into_iter()
            .take(CARD_PHOTOS)
            .collect();
        self.pick_facts(cluster, &shown)
            .iter()
            .any(|facts| !facts.is_empty())
    }

    /// The card for one group: its best photos and their deciding facts,
    /// the suggested one marked. A group nobody has decided yet shows no
    /// Keep or Trash, only that mark and Accept suggestion; once anything
    /// in it is decided, each photo has its Keep/Trash pill, and Undo as
    /// `pick_link` says.
    fn pick_card(&self, group: usize) -> ReviewCard {
        let cluster = &self.pick_review.clusters()[group];
        let enabled = self.pick_enabled();
        let undecided = self.pick_undecided(group);
        let order = self.pick_order(cluster);
        let shown: Vec<MediaId> = order.iter().take(CARD_PHOTOS).copied().collect();
        let facts = self.pick_facts(cluster, &shown);
        let photos = shown
            .iter()
            .zip(facts)
            .enumerate()
            .map(|(side, (id, facts))| {
                let keep = Review::keeps(&self.decisions, *id);
                let media = self.index.get(id).map(|i| &self.media[*i]);
                let suggested = *id == cluster.best();
                CardPhoto {
                    id: *id,
                    turns: self.preview_turns(*id),
                    badge: if undecided {
                        Badge::None
                    } else {
                        Badge::keep(keep)
                    },
                    suggested,
                    suggested_hover: suggested.then(|| self.pick_keep_why(cluster)),
                    caption: media.map(|m| self.shown_name(m)).unwrap_or_default(),
                    name: media
                        .map(|m| crate::core::text::shown(&m.path))
                        .unwrap_or_default(),
                    facts,
                    trash: !keep,
                    dimmed: self.trashed_elsewhere(*id),
                    inset: None,
                    toggle: enabled.then(|| {
                        format!(
                            "{} · {}",
                            if undecided {
                                "Mark for Trash"
                            } else if keep {
                                "Move to Trash instead"
                            } else {
                                "Keep instead"
                            },
                            side + 1
                        )
                    }),
                }
            })
            .collect();
        // Only the photos the card leaves out, and any of them kept, which
        // means something once the group is decided.
        let hidden = &order[shown.len()..];
        let state_text = (!hidden.is_empty()).then(|| {
            let kept = hidden
                .iter()
                .filter(|id| Review::keeps(&self.decisions, **id))
                .count();
            if kept > 0 && !undecided {
                format!("+{} more, {kept} kept", hidden.len())
            } else {
                format!("+{} more", hidden.len())
            }
        });
        let others = cluster.ordered.len() - 1;
        let accept = format!(
            "Keep the starred photo and mark the {} for Trash · A",
            if others == 1 {
                "other one".to_owned()
            } else {
                format!("other {others}")
            }
        );
        ReviewCard {
            // No date over the photos: the card is its photos, and the
            // header holds only Accept or Undo and how many more it has.
            title: String::new(),
            state_text,
            photos,
            buttons: Vec::new(),
            // The selected group, outlined while the panel is hidden too:
            // the keys still act on it.
            shown: self.pick_group() == Some(group),
            title_hover: None,
            undo: self.pick_link(group).map(|link| match link {
                GroupLink::Accept => (ACCEPT.to_owned(), accept),
                GroupLink::Undo => ("Undo".to_owned(), String::new()),
            }),
            undo_icon: None,
            filled: undecided,
        }
    }

    /// Applies a card's event to its group, after making it the selected
    /// group so the keyboard follows the mouse.
    pub(super) fn pick_card_event(&mut self, group: usize, event: CardEvent) {
        self.pending_bookmark = None;
        let Some(ordered) = self
            .pick_review
            .clusters()
            .get(group)
            .map(|c| self.pick_order(c))
        else {
            return;
        };
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                if let Some(id) = ordered.get(side) {
                    self.set_active(*id);
                }
                // A click shows the group's photos large beside the grid.
                self.pick_panel = true;
            }
            CardEvent::Toggle(side) => {
                if let Some(id) = ordered.get(side) {
                    self.toggle_pick(*id);
                }
            }
            CardEvent::Undo => self.undo_pick_group(group),
            CardEvent::Button(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::faces::{EyeState, FaceEyes, FaceSummary};
    use crate::core::pick_review::Cluster;
    fn photo(n: u64) -> Media {
        Media {
            id: MediaId(n),
            path: format!("/{n}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        }
    }
    /// Two groups: photos 1-3 (3 is best) and 4-5 (5 is best).
    fn app() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        // These tests cover suggestions applied automatically.
        app.preferences.easy_delete_auto = true;
        app.preferences.pick_best_auto = true;
        app.preferences.rotation_auto = true;
        app.media = (1..=5).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.pick_review.replace(vec![
            Cluster::new(
                1,
                vec![MediaId(1), MediaId(2), MediaId(3)],
                HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.), (MediaId(3), 30.)]),
            ),
            Cluster::new(
                2,
                vec![MediaId(4), MediaId(5)],
                HashMap::from([(MediaId(4), 10.), (MediaId(5), 20.)]),
            ),
        ]);
        app.view_data_ready.insert(Step::PickBest);
        app.set_step(Step::PickBest);
        app.refilter();
        app
    }
    fn trashed(app: &PhotoSortApp) -> Vec<u64> {
        let mut ids: Vec<u64> = (1..=5)
            .filter(|n| app.decision(MediaId(*n)).rejected)
            .collect();
        ids.sort();
        ids
    }
    fn key(app: &mut PhotoSortApp, key: egui::Key, modifiers: egui::Modifiers) {
        hold(app, key, modifiers, 0);
    }
    /// A key press, then `repeats` more of it with no release between, as
    /// a held key sends them: egui marks those as repeats itself.
    fn hold(app: &mut PhotoSortApp, key: egui::Key, modifiers: egui::Modifiers, repeats: usize) {
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for _ in 0..=repeats {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            };
            let _ = ctx.run_test(input, |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            });
        }
    }

    #[test]
    fn entering_pick_best_stages_its_suggestions_once() {
        let mut app = app();
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // The user keeps photo 1; leaving and coming back never re-trashes it.
        app.toggle_pick(MediaId(1));
        app.set_step(Step::Cull);
        app.set_step(Step::PickBest);
        assert_eq!(trashed(&app), vec![2, 4]);
        // Review changes lists them: no "complete every comparison" gate.
        app.review_changes();
        assert!(!app.status.contains("Complete every comparison"));
    }

    #[test]
    fn with_automatic_suggestions_off_nothing_is_staged_until_accepted() {
        use egui_phosphor::regular as icon;
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.preferences.pick_best_auto = false;
        app.media = (1..=5).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.pick_review.replace(vec![
            Cluster::new(
                1,
                vec![MediaId(1), MediaId(2)],
                HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.)]),
            ),
            Cluster::new(
                2,
                vec![MediaId(3), MediaId(4), MediaId(5)],
                HashMap::from([(MediaId(3), 10.), (MediaId(4), 30.), (MediaId(5), 20.)]),
            ),
        ]);
        app.view_data_ready.insert(Step::PickBest);
        app.set_step(Step::PickBest);
        let (keep, trash) = (
            format!("{}  Keep", icon::CHECK),
            format!("{}  Trash", icon::TRASH),
        );
        // Nobody has decided anything: no Keep or Trash on any photo, only
        // the suggested one's star and each group's Accept suggestion, and
        // the line says what to do. Nothing counts as a change.
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains(&keep) && !shown.contains(&trash), "{shown}");
        assert!(
            shown.contains(icon::STAR) && shown.contains(&hint()),
            "{shown}"
        );
        assert_eq!(shown.matches(ACCEPT).count(), 2, "{shown}");
        assert!(trashed(&app).is_empty());
        assert_eq!(app.staged_change_count(), 0);
        // Accepting one group stages that group only, which then shows
        // Keep and Trash, and Undo, which takes it back to nothing marked.
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![1]);
        assert_eq!(app.staged_change_count(), 1);
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(shown.contains(&keep) && shown.contains(&trash), "{shown}");
        assert_eq!(shown.matches(ACCEPT).count(), 1, "{shown}");
        assert!(shown.contains("Undo"), "{shown}");
        app.pick_card_event(0, CardEvent::Undo);
        assert!(trashed(&app).is_empty() && app.pick_undecided(0));
        assert_eq!(app.staged_change_count(), 0);
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains(&keep) && !shown.contains(&trash), "{shown}");
        assert_eq!(shown.matches(ACCEPT).count(), 2, "{shown}");
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![1]);
        // A photo of the other group marked and unmarked by hand: that group
        // is decided, every photo kept, and Accept all leaves it so.
        app.pick_card_event(1, CardEvent::Toggle(1));
        app.pick_card_event(1, CardEvent::Toggle(1));
        assert!(!app.pick_undecided(1));
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![1]);
        // Nobody's choice again, that group takes its suggestion: two photos
        // marked, and the status counts the one suggestion, not the photos.
        for id in [3, 4, 5] {
            app.by_hand.remove(&MediaId(id));
        }
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![1, 3, 5]);
        assert_eq!(app.status, "Accepted 1 suggestion");
    }

    /// Photos already marked for Trash show last in their group, and stay
    /// where they are while the user marks or unmarks them.
    #[test]
    fn photos_marked_for_trash_show_last_until_pick_best_is_entered_again() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = (1..=3).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.pick_review.replace(vec![Cluster::new(
            1,
            vec![MediaId(1), MediaId(2), MediaId(3)],
            HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.), (MediaId(3), 30.)]),
        )]);
        app.view_data_ready.insert(Step::PickBest);
        // The best photo was marked for Trash in Cull.
        app.decisions.insert(
            MediaId(3),
            Decision {
                rejected: true,
                ..Default::default()
            },
        );
        app.set_step(Step::PickBest);
        app.refilter();
        let shown = |app: &PhotoSortApp| -> Vec<u64> {
            app.pick_card(0).photos.iter().map(|p| p.id.0).collect()
        };
        assert_eq!(shown(&app), vec![2, 1, 3]);
        // Marking photo 2 leaves it in place.
        app.pick_card_event(0, CardEvent::Toggle(0));
        app.refilter();
        assert!(app.decision(MediaId(2)).rejected);
        assert_eq!(shown(&app), vec![2, 1, 3]);
        assert_eq!(app.pick_panel_ids(), [2, 1, 3].map(MediaId));
        // Coming back to the step puts it last too.
        app.set_step(Step::Cull);
        app.set_step(Step::PickBest);
        app.refilter();
        assert_eq!(shown(&app), vec![1, 3, 2]);
    }

    #[test]
    fn a_click_toggles_and_undo_restores_the_suggestion() {
        let mut app = app();
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 4], "Photo 2 is kept too");
        assert_eq!(app.pick_group(), Some(0));
        assert!(app.pick_review.changed(0, &app.decisions));
        // Toggling back by hand is no change.
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert!(!app.pick_review.changed(0, &app.decisions));
        // Pick another photo as the keeper, then Undo.
        app.pick_card_event(0, CardEvent::Toggle(0));
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 3, 4]);
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // Accept all leaves a group the user has started on as it is: photos
        // 1 and 4 were kept by hand, so the rest of their groups stays too,
        // photo 2 included, which nobody decided.
        app.pick_card_event(1, CardEvent::Toggle(1));
        app.pick_card_event(0, CardEvent::Toggle(2));
        app.decisions.remove(&MediaId(2));
        assert_eq!(trashed(&app), Vec::<u64>::new());
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), Vec::<u64>::new());
        // A group whose suggested photo is marked for Trash, by hand or by
        // Easy Delete, is left as it is until its own Undo: marking the
        // undecided photo 4 would leave it with no photo kept.
        app.pick_card_event(1, CardEvent::Undo);
        app.decisions.remove(&MediaId(4));
        app.pick_card_event(1, CardEvent::Toggle(0));
        assert_eq!(trashed(&app), vec![5]);
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![5]);
        app.pick_deck();
        assert_eq!(app.grids.pick.changed, 0, "Accept all has nothing to do");
        app.pick_card_event(1, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![4]);
    }

    #[test]
    fn pick_best_opens_with_nothing_selected_and_the_keyboard_selects_and_shows() {
        let mut app = app();
        assert_eq!(app.pick_current, None);
        assert!(!app.pick_panel);
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains("Round"), "{shown}");
        // Enter shows the first group beside the grid.
        key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.pick_group(), Some(0));
        assert!(app.pick_panel);
        // D never guesses which photo of the group the user means.
        key(&mut app, egui::Key::D, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4], "D changed the group");
        // Number keys toggle the nth photo of the group shown, best first,
        // once however long the key is held.
        hold(&mut app, egui::Key::Num2, egui::Modifiers::NONE, 3);
        assert_eq!(trashed(&app), vec![1, 4], "Photo 2 is the second best");
        key(&mut app, egui::Key::R, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 4], "R does nothing here");
        key(&mut app, egui::Key::U, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4], "U undoes the group");
        // Arrows move to the next group; Esc closes the panel.
        key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.pick_group(), Some(1));
        key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.pick_panel);
        // With the panel closed, number keys change nothing.
        key(&mut app, egui::Key::Num1, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // A click on the grid background deselects.
        app.deselect_pick_group();
        assert_eq!(app.pick_current, None);
    }

    #[test]
    fn the_grid_shows_one_card_per_group_with_its_suggested_photo_and_no_confirm() {
        let mut app = app();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [egui_phosphor::regular::STAR, "Accept all suggestions"] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        // No heading over a group: neither its date nor its number.
        for text in [
            "Group 1",
            "Group 2",
            "Confirm",
            "Provisional",
            "Keep all",
            "Round",
        ] {
            assert!(!shown.contains(text), "{text} in:\n{shown}");
        }
        assert!(
            shown.contains(&format!("3 photos in these groups going {TO_TRASH}")),
            "{shown}"
        );
        assert!(!shown.contains("more marked"), "{shown}");
        // A photo marked in another step is in no group. The line names it
        // apart, so the two add up to what Review counts.
        app.media.push(photo(6));
        app.index.insert(MediaId(6), 5);
        crate::core::model::update_trash(&mut app.decisions, [(MediaId(6), true)]);
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(
            shown.contains(&format!("3 photos in these groups going {TO_TRASH}"))
                && shown.contains("1 more marked in other steps"),
            "{shown}"
        );
    }

    #[test]
    fn a_card_has_no_title_and_never_draws_the_count_over_its_undo() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = (1..=20).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.pick_review.replace(vec![Cluster::new(
            1,
            (1..=20).map(MediaId).collect(),
            (1..=20).map(|n| (MediaId(n), n as f64)).collect(),
        )]);
        app.view_data_ready.insert(Step::PickBest);
        app.set_step(Step::PickBest);
        app.refilter();
        app.review = false;
        // Decided by hand, kept as it is: its count says how many it keeps.
        app.by_hand.insert(MediaId(20));
        // The header as painted for photos of a shape: the count beside
        // Undo, and whether the two overlap. No title over them.
        let mut header = |width: u32, height: u32| {
            for n in 1..=20 {
                app.metadata.insert(
                    MediaId(n),
                    Metadata {
                        width,
                        height,
                        ..Default::default()
                    },
                );
            }
            app.metadata_shapes += 1;
            app.preferences.photo_size = *crate::core::preferences::PHOTO_SIZES.start();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let output = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.library(ui));
                },
            );
            fn walk<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::TextShape>) {
                match shape {
                    egui::Shape::Text(text) => out.push(text),
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
                    _ => {}
                }
            }
            let mut texts = Vec::new();
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut texts);
            }
            let span = |t: &egui::epaint::TextShape| (t.pos.x, t.pos.x + t.galley.size().x);
            assert!(!texts.iter().any(|t| t.galley.text().contains("Group")));
            let undo = texts
                .iter()
                .find(|t| t.galley.text().ends_with("Undo"))
                .expect("the card's Undo");
            let count = texts.iter().find(|t| t.galley.text().starts_with("+17"));
            (
                count.map(|t| t.galley.text().to_owned()),
                count.is_some_and(|t| span(t).1 > span(undo).0),
            )
        };
        // Wide photos: room for the whole count.
        assert_eq!(
            header(3000, 2000),
            (Some("+17 more, 17 kept".to_owned()), false)
        );
        // Narrower cards, down to the narrowest: the count stays clear of
        // Undo, shortened where it must be.
        let mut counts = Vec::new();
        for width in [1600, 1400, 1000, 500] {
            let (count, over) = header(width, 2000);
            assert!(!over, "{width}: {count:?}");
            counts.push(count.expect("the count beside Undo"));
        }
        assert!(counts.iter().all(|c| c.starts_with("+17 more")));
        assert!(counts.iter().any(|c| c == "+17 more"), "{counts:?}");
    }

    #[test]
    fn cards_say_who_has_eyes_closed_and_the_winner_has_eyes_open() {
        let mut app = app();
        let face = |state, ratio| FaceEyes {
            centre: [0.5, 0.4],
            size: [0.1, 0.12],
            state,
            ratio,
        };
        let analysis = |score: f64, eyes: FaceEyes| crate::core::analysis::Analysis {
            // Sharpness and looks that differ: the card says neither.
            sharpness: score * 5.0,
            quality_sharpness: Some(score * 5.0),
            aesthetic: Some(score as f32 / 100.0),
            faces: Some(FaceSummary {
                count: 1,
                eyes: vec![eyes],
                ..Default::default()
            }),
            score,
            ..Default::default()
        };
        app.analysis = HashMap::from([
            (
                MediaId(4),
                analysis(80.0, face(EyeState::Closed, Some(0.05))),
            ),
            (MediaId(5), analysis(60.0, face(EyeState::Open, Some(0.3)))),
        ]);
        app.cull_groups = Arc::new(HashMap::from([(MediaId(4), 7), (MediaId(5), 7)]));
        app.pick_review = Default::default();
        app.decisions.clear();
        app.finish_pick_analysis();
        let cluster = &app.pick_review.clusters()[0];
        assert_eq!(
            cluster.best(),
            MediaId(5),
            "Open eyes win over a higher score"
        );
        assert_eq!(trashed(&app), vec![4]);
        let facts = app.pick_facts(cluster, &[MediaId(5), MediaId(4)]);
        assert_eq!(
            facts[0][0],
            Fact {
                text: "Eyes open".into(),
                better: true
            }
        );
        assert_eq!(
            facts[1][0],
            Fact {
                text: "Eyes closed".into(),
                better: false
            }
        );
        assert!(app.pick_keep_why(cluster).contains("closed for someone"));
        assert_eq!((facts[0].len(), facts[1].len()), (1, 1));
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(
            shown.contains("Eyes closed") && shown.contains("Eyes open"),
            "{shown}"
        );
        for clutter in ["Sharpness", "Soft", "Aesthetic"] {
            assert!(!shown.contains(clutter), "{clutter} in:\n{shown}");
        }
    }

    #[test]
    fn background_pick_results_do_not_change_another_workflows_active_photo() {
        let mut app = app();
        app.set_step(Step::Cull);
        app.active = Some(MediaId(3));
        app.finish_pick_analysis();
        assert_eq!(app.active, Some(MediaId(3)));
        assert_eq!(app.step, Step::Cull);
    }
    fn loaded_folder(app: &mut PhotoSortApp) {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.runtime.events = receiver;
        sender
            .send(crate::core::jobs::Envelope {
                epoch: app.runtime.epoch(),
                event: Event::Opened(
                    Arc::new(Catalog {
                        media: app.media.clone(),
                        ..Default::default()
                    }),
                    Session::default(),
                    Default::default(),
                ),
            })
            .unwrap();
        app.poll(&egui::Context::default());
    }
    #[test]
    fn folder_completion_starts_pick_analysis_without_reentering_the_step() {
        let mut app = app();
        app.pick_review = Default::default();
        app.view_data_ready.clear();
        app.catalog = None;
        app.scanning = true;
        app.set_step(Step::PickBest);
        assert!(app.analysis_run.is_none(), "Do not analyze a partial scan");
        loaded_folder(&mut app);
        assert!(!app.scanning);
        assert!(
            app.pick_run.is_some(),
            "The completed scan must start quality analysis"
        );
        assert_eq!(app.pick_run, app.analysis_run);
        let run = app.pick_run;
        app.set_step(Step::PickBest);
        assert_eq!(
            run, app.pick_run,
            "Step reentry must reuse the existing run"
        );
    }
    /// Easy Delete and Pick Best each entered while the other's analysis is
    /// still running: the step waits, runs its own when that one ends, and
    /// says "nothing found" only as a result.
    #[test]
    fn a_step_entered_during_another_steps_analysis_waits_then_runs_its_own() {
        fn shown(app: &mut PhotoSortApp) -> String {
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let output = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.empty_state(ui));
                },
            );
            super::super::review_card::painted_text(&output)
        }
        let own_run = |app: &PhotoSortApp| match app.step {
            Step::PickBest => app.pick_run,
            _ => app.delete_run,
        };
        for (first, second, invitation, nothing) in [
            (
                Step::EasyDelete,
                Step::PickBest,
                "Find similar photos",
                "No similar photos to compare",
            ),
            (
                Step::PickBest,
                Step::EasyDelete,
                "Find photos to clean up",
                "Nothing to clean up",
            ),
        ] {
            let mut app = app();
            app.pick_review = Default::default();
            app.view_data_ready.clear();
            app.set_step(first);
            loaded_folder(&mut app);
            let running = own_run(&app).expect("The first step analyzes on entry");
            app.set_step(second);
            app.refilter();
            assert_eq!(app.analysis_run, Some(running), "{second:?}");
            let waiting = shown(&mut app);
            assert!(
                waiting.contains("Waiting for the current analysis to finish")
                    && !waiting.contains(nothing),
                "{second:?}: {waiting}"
            );
            // The first run ends: the step on screen starts its own.
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            app.runtime.events = receiver;
            sender
                .send(crate::core::jobs::Envelope {
                    epoch: app.runtime.epoch(),
                    event: Event::AnalysisDone {
                        run: running,
                        stats: Default::default(),
                    },
                })
                .unwrap();
            app.poll(&egui::Context::default());
            assert!(app.view_data_ready.contains(&first), "{first:?}");
            let own = own_run(&app).expect("The step on screen runs its own analysis");
            assert_eq!(app.analysis_run, Some(own), "{second:?}");
            assert_ne!(own, running);
            // Cancelled, it offers Analyze instead of starting over, and
            // still claims no result.
            app.cancel_analysis();
            assert!(app.analysis_run.is_none(), "{second:?}");
            let cancelled = shown(&mut app);
            assert!(
                cancelled.contains(invitation)
                    && cancelled.contains("Analyze library")
                    && !cancelled.contains(nothing),
                "{second:?}: {cancelled}"
            );
        }
    }
    #[test]
    fn post_apply_departure_does_not_start_unwanted_pick_analysis() {
        for destination in [Destination::Step(Step::Cull), Destination::Close] {
            let mut app = app();
            app.pick_review = Default::default();
            app.view_data_ready.clear();
            app.after_apply = Some(destination);
            loaded_folder(&mut app);
            assert!(app.pick_run.is_none());
            assert!(app.analysis_run.is_none());
            assert!(app.step == Step::Cull || app.close_authorized);
        }
    }
}
