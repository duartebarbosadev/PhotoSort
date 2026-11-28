use super::review_card::{Badge, CardEvent, CardPhoto, Fact, GridClick, GroupLink, ReviewCard};
use super::*;
use crate::core::pick_review::{Cluster, Reason, Review};
use crate::core::text::{TO_TRASH, TRASH};

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
    /// Each group's card: its photos' widths over their heights, every
    /// photo of the group, best first.
    aspects: Vec<Vec<f32>>,
    /// Changes whenever a card's shape may have: the grid asks its cards
    /// again.
    stamp: u64,
    /// The review and the decisions the counts were taken from.
    counted: Option<(u64, u64)>,
    /// The groups' photos going to Trash, and their bytes.
    trash: usize,
    freed: u64,
    /// Photos and videos going to Trash that are in no group here: marked
    /// in another step. With `trash` and `not_shown`, every Trash mark
    /// Review lists.
    elsewhere: usize,
    /// Photos going to Trash in groups not shown: the other mode's, or
    /// hidden by the people filter.
    not_shown: usize,
    /// Groups Accept all suggestions would change.
    changed: usize,
}

/// Which kind of group Pick Best shows. Both stage into the one set of
/// decisions, so switching keeps every choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PickMode {
    /// Near-duplicate shots: a burst, or the same picture taken again.
    Similar,
    /// One subject from other angles or framings, which Similar does not
    /// group together. What Pick Best opens with.
    #[default]
    SameSubject,
}

/// Which groups Pick Best shows, by the faces found in their photos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum People {
    #[default]
    All,
    Without,
    Only,
}

/// Which groups Pick Best shows, by how their photos show: taller than
/// wide (portrait) or not (landscape, square too).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Orientation {
    #[default]
    All,
    /// Portrait and landscape never share a card: a mixed group shows as
    /// a portrait card and a landscape card, each with its own best.
    Same,
    /// Only groups with both portrait and landscape photos.
    Mixed,
}

/// Pick Best's groups of both modes, and how they are shown.
#[derive(Default)]
pub(super) struct PickModes {
    pub(super) mode: PickMode,
    pub(super) people: People,
    pub(super) orientation: Orientation,
    pub(super) similar: Vec<Cluster>,
    /// `None` until a Pick Best analysis found them; why, when it could not.
    subject: Option<Result<Vec<Cluster>, String>>,
    /// The Same subject groups of the analysis under way, when it sent them.
    pub(super) found: Option<Result<HashMap<MediaId, usize>, String>>,
    /// Groups of the mode shown that the filters hide, and those with a
    /// photo never checked for people.
    hidden: usize,
    unchecked: usize,
    /// Photos last chosen by hand in Same subject mode. The other photos
    /// chosen by hand belong to Similar (or to another step): each mode's
    /// Undo leaves the other's choices as they are.
    pub(super) subject_hand: HashSet<MediaId>,
}

impl PickModes {
    /// Whether an analysis found groups of either mode to show.
    pub(super) fn has_groups(&self) -> bool {
        !self.similar.is_empty() || self.subject.is_some()
    }
}

impl PhotoSortApp {
    /// Whether anyone is clearly in a group's photos, seen from any side
    /// (`persons`) or by a face found: `None` while a photo was never
    /// checked for people.
    fn people_in(&self, cluster: &Cluster) -> Option<bool> {
        let mut people = false;
        for id in &cluster.ordered {
            let analysis = self.analysis.get(id)?;
            people |= analysis.people? > 0 || analysis.faces.as_ref().is_some_and(|f| f.count > 0);
        }
        Some(people)
    }
    fn mode_groups(&self, mode: PickMode) -> &[Cluster] {
        match mode {
            PickMode::Similar => &self.pick_modes.similar,
            PickMode::SameSubject => match &self.pick_modes.subject {
                Some(Ok(clusters)) => clusters,
                _ => &[],
            },
        }
    }
    /// Whether the people filter can tell: a group of the mode shown was
    /// checked for people.
    pub(super) fn people_known(&self) -> bool {
        self.mode_groups(self.pick_modes.mode)
            .iter()
            .any(|c| self.people_in(c).is_some())
    }
    /// Whether a photo shows taller than wide, as its card shows it: after
    /// its EXIF orientation and the turns staged for it.
    fn shown_portrait(&self, id: MediaId) -> bool {
        self.photo_aspect(id, self.preview_turns(id)) < 1.0
    }
    /// A group's photos as they show, portrait then landscape, each in
    /// capture order. `None` when they all show one way.
    fn by_orientation(&self, cluster: &Cluster) -> Option<[Vec<MediaId>; 2]> {
        let (mut portrait, mut landscape): (Vec<MediaId>, Vec<MediaId>) = cluster
            .ordered
            .iter()
            .partition(|id| self.shown_portrait(**id));
        if portrait.is_empty() || landscape.is_empty() {
            return None;
        }
        for part in [&mut portrait, &mut landscape] {
            part.sort_by_cached_key(|id| {
                self.index.get(id).map(|i| {
                    crate::core::dates::capture_order(&self.media[*i], self.metadata.get(id))
                })
            });
        }
        Some([portrait, landscape])
    }
    /// The groups of the mode chosen as the orientation filter has them:
    /// each, a mixed one split into its portrait and landscape parts (a
    /// part of one photo is no group), or only the mixed ones. With the
    /// split parts, for `spare_split_parts`.
    fn oriented_groups(&self) -> (Vec<Cluster>, Vec<Cluster>, Vec<Vec<MediaId>>) {
        let (mut shown, mut hidden, mut parts) = (Vec::new(), Vec::new(), Vec::new());
        for cluster in self.mode_groups(self.pick_modes.mode) {
            let split = self.by_orientation(cluster);
            match (self.pick_modes.orientation, split) {
                (Orientation::Same, Some(split)) => {
                    for part in split.into_iter().filter(|part| part.len() >= 2) {
                        let mut part = crate::core::pick_review::ranked(
                            cluster.key,
                            &part,
                            &self.analysis,
                            &self.metadata,
                        );
                        self.arrange_pick_group(&mut part);
                        parts.push(part.ordered.clone());
                        shown.push(part);
                    }
                }
                (Orientation::Mixed, None) => hidden.push(cluster.clone()),
                _ => shown.push(cluster.clone()),
            }
        }
        (shown, hidden, parts)
    }
    /// Shows the groups of the mode chosen that pass the orientation and
    /// people filters. Nothing staged changes, but for `spare_split_parts`.
    pub(super) fn show_pick_groups(&mut self) {
        let mode = self.pick_modes.mode;
        let people = if self.people_known() {
            self.pick_modes.people
        } else {
            People::All
        };
        let (oriented, mut hidden, parts) = self.oriented_groups();
        let mut shown = Vec::new();
        let mut unchecked = 0;
        for cluster in oriented {
            let found = self.people_in(&cluster);
            unchecked += usize::from(found.is_none());
            let wanted = match (people, found) {
                (People::All, _) => true,
                (People::Without, Some(found)) => !found,
                (People::Only, Some(found)) => found,
                // Not checked: neither with nor without people.
                (_, None) => false,
            };
            if wanted { &mut shown } else { &mut hidden }.push(cluster);
        }
        let other = match mode {
            PickMode::Similar => PickMode::SameSubject,
            PickMode::SameSubject => PickMode::Similar,
        };
        let other = self
            .mode_groups(other)
            .iter()
            .map(|c| c.ordered.clone())
            .collect();
        self.pick_modes.hidden = hidden.len();
        self.pick_modes.unchecked = unchecked;
        self.pick_review.show(shown, hidden, other);
        self.spare_split_parts(parts);
        self.filter_dirty = true;
    }
    /// A part of a split group that shows with every photo in Trash, where
    /// none was chosen by hand: the whole group's suggestion kept a photo
    /// of the other orientation. The part keeps its own best instead, so a
    /// card never sends all of itself to Trash. Nothing else changes.
    fn spare_split_parts(&mut self, parts: Vec<Vec<MediaId>>) {
        let shown: HashSet<MediaId> = self
            .pick_review
            .clusters()
            .iter()
            .flat_map(|c| c.ordered.iter().copied())
            .collect();
        let kept: Vec<MediaId> = parts
            .iter()
            .filter(|part| part.iter().all(|id| shown.contains(id)))
            .filter(|part| {
                part.iter()
                    .all(|id| self.decision(*id).rejected && !self.by_hand.contains(id))
            })
            .map(|part| part[0])
            .collect();
        if !kept.is_empty() {
            crate::core::model::update_trash(
                &mut self.decisions,
                kept.iter().map(|id| (*id, false)),
            );
            self.changed_items(kept);
        }
    }
    pub(super) fn set_pick_mode(&mut self, mode: PickMode) {
        if self.pick_modes.mode == mode || !self.pick_enabled() {
            return;
        }
        self.pick_modes.mode = mode;
        self.pending_bookmark = None;
        self.deselect_pick_group();
        self.show_pick_groups();
        // Each mode stages its suggestions once it is shown.
        self.stage_pick_suggestions();
    }
    pub(super) fn set_pick_orientation(&mut self, orientation: Orientation) {
        if self.pick_modes.orientation == orientation || !self.pick_enabled() {
            return;
        }
        self.pick_modes.orientation = orientation;
        self.pending_bookmark = None;
        self.deselect_pick_group();
        self.show_pick_groups();
    }
    pub(super) fn set_pick_people(&mut self, people: People) {
        if self.pick_modes.people == people || !self.pick_enabled() {
            return;
        }
        self.pick_modes.people = people;
        self.pending_bookmark = None;
        self.show_pick_groups();
    }
    /// Notes the mode these photos were chosen by hand in.
    fn note_pick_hand(&mut self, ids: impl IntoIterator<Item = MediaId>) {
        for id in ids {
            if self.pick_modes.mode == PickMode::SameSubject {
                self.pick_modes.subject_hand.insert(id);
            } else {
                self.pick_modes.subject_hand.remove(&id);
            }
        }
    }
    /// Of these photos, those chosen by hand in the other mode (or, for
    /// Same subject, in another step): this mode's Undo leaves them alone.
    fn chosen_elsewhere(&self, ids: &[MediaId]) -> HashSet<MediaId> {
        let subject = self.pick_modes.mode == PickMode::SameSubject;
        ids.iter()
            .filter(|id| {
                self.by_hand.contains(id) && self.pick_modes.subject_hand.contains(id) != subject
            })
            .copied()
            .collect()
    }
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

    /// Orders a group's photos so the most alike sit side by side. A burst
    /// is best told apart by when each shot was taken: a pose held for a
    /// few frames, then changed. So by capture time where every photo of the
    /// group has one (the file names settle shots of one millisecond), else
    /// by what they show (their embeddings), else best to worst. The
    /// embedding sees the whole frame, where a large scene can outweigh the
    /// person in it.
    fn arrange_pick_group(&self, cluster: &mut Cluster) {
        let taken = |id: MediaId| {
            let m = self.metadata.get(&id)?;
            Some(m.captured_ms? + i64::from(m.captured_subsec_ms.unwrap_or(0)))
        };
        let timed = cluster.ordered.iter().all(|id| taken(*id).is_some());
        let embedded = cluster
            .ordered
            .iter()
            .all(|id| self.embeddings.contains_key(id));
        // Each photo's place among the group's file names, for ties.
        let mut named = cluster.ordered.clone();
        named.sort_by_key(|id| self.index.get(id).map(|i| self.media[*i].name()));
        let place = |id: MediaId| named.iter().position(|n| *n == id).unwrap_or(0) as f64;
        cluster.arrange(|a, b| {
            if timed {
                let apart = (taken(a)? - taken(b)?).abs() as f64;
                Some(apart + (place(a) - place(b)).abs() * 0.001)
            } else if embedded {
                let (a, b) = (self.embeddings.get(&a)?, self.embeddings.get(&b)?);
                let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| f64::from(x * y)).sum();
                let norm = |v: &[f32]| v.iter().map(|x| f64::from(x * x)).sum::<f64>().sqrt();
                let length = norm(a) * norm(b);
                (length > 0.0).then(|| 1.0 - dot / length)
            } else {
                None
            }
        });
    }
    pub(super) fn finish_pick_analysis(&mut self) {
        self.view_data_ready.insert(Step::PickBest);
        let mut similar = crate::core::pick_review::from_analysis(
            &self.media,
            &self.analysis,
            &self.cull_groups,
            &self.metadata,
        );
        for cluster in &mut similar {
            self.arrange_pick_group(cluster);
        }
        // Same subject shows only what Similar does not group already: a
        // group whose photos are all in one Similar group is left to it.
        let similar_of: HashMap<MediaId, usize> = similar
            .iter()
            .enumerate()
            .flat_map(|(n, c)| c.ordered.iter().map(move |id| (*id, n)))
            .collect();
        let within_similar = |c: &Cluster| {
            let group = similar_of.get(&c.ordered[0]);
            group.is_some() && c.ordered.iter().all(|id| similar_of.get(id) == group)
        };
        let subject = self.pick_modes.found.clone().map(|found| {
            found.map(|groups| {
                let mut clusters = crate::core::pick_review::from_analysis(
                    &self.media,
                    &self.analysis,
                    &groups,
                    &self.metadata,
                );
                clusters.retain(|c| !within_similar(c));
                for cluster in &mut clusters {
                    self.arrange_pick_group(cluster);
                }
                clusters
            })
        });
        self.pick_modes.similar = similar;
        self.pick_modes.subject = subject;
        self.show_pick_groups();
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
    /// A group's photos as its card and panel show them: best first, the
    /// most alike side by side (`Cluster::arrange`), and those already
    /// marked for Trash when the list was built last.
    pub(super) fn pick_order(&self, cluster: &Cluster) -> Vec<MediaId> {
        let (last, first): (Vec<MediaId>, Vec<MediaId>) =
            cluster.shown.iter().partition(|id| self.shown_last(**id));
        first.into_iter().chain(last).collect()
    }
    /// Switches one photo between Keep and Trash.
    pub(super) fn toggle_pick(&mut self, id: MediaId) {
        if !self.pick_enabled() {
            return;
        }
        self.pick_review.toggle(id, &mut self.decisions);
        self.by_hand.insert(id);
        self.note_pick_hand([id]);
        self.set_active(id);
        self.changed_items([id]);
    }
    /// A photo's number, or a click on it: in a group nobody has started by
    /// hand, that photo is kept and the others go to Trash; once the group is
    /// started, it switches that photo between Keep and Trash. U starts over.
    pub(super) fn keep_pick(&mut self, id: MediaId) {
        if !self.pick_enabled() {
            return;
        }
        let Some(members) = self.pick_review.cluster_of(id).map(|c| c.ordered.clone()) else {
            return;
        };
        // In focus a number or click only shows that photo; D marks it.
        if self.focus_active() && self.pick_review.group_of(id) == self.pick_group() {
            self.set_active(id);
            return;
        }
        if self.card_started(&members) {
            let kept = !self.decision(id).rejected;
            crate::core::model::update_trash(&mut self.decisions, [(id, kept)]);
            self.by_hand.insert(id);
            self.note_pick_hand([id]);
        } else {
            self.pick_review.keep_only(id, &mut self.decisions);
            self.by_hand.extend(members.iter().copied());
            self.note_pick_hand(members.iter().copied());
        }
        self.set_active(id);
        self.changed_items(members);
    }
    /// The number keys: the nth photo of the group shown beside the grid.
    pub(super) fn keep_pick_slot(&mut self, slot: usize) {
        if let Some(id) = self.pick_panel_ids().get(slot).copied() {
            self.keep_pick(id);
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
        // A photo chosen by hand in the other mode stays as it was chosen.
        let spared = self.chosen_elsewhere(&members);
        if !clear {
            self.pick_review
                .accept(&[group], &spared, &mut self.decisions);
        }
        let members: Vec<MediaId> = members
            .into_iter()
            .filter(|id| !spared.contains(id))
            .collect();
        for id in &members {
            self.pick_modes.subject_hand.remove(id);
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
        "Nothing marked yet: accept a group's suggestion, or click the photo to keep and the others go to the {TRASH}."
    )
}

impl PhotoSortApp {
    /// Pick Best's only view: a summary with Accept all suggestions, then
    /// one card per group with its best photos. Clicking a card opens all of
    /// its photos large in a panel beside the grid.
    pub(super) fn pick_grid(&mut self, ui: &mut egui::Ui) {
        // Focus belongs to the panel: closed any way (X, Esc, a click on the
        // grid's background), it ends.
        if !(self.pick_panel && self.pick_current.is_some()) {
            self.panel_focus = false;
        }
        // Before the deck: a mode or filter chosen here changes the groups
        // the deck and the grid are drawn from this frame.
        self.pick_modes_bar(ui);
        ui.add_space(S2);
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
                            if let Some(aspects) = app.grids.pick.aspects.get(i) {
                                shapes.extend_from_slice(aspects);
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

    /// The bar over Pick Best's groups: which kind of group, and the people
    /// filter, with what that filter cannot tell.
    pub(super) fn pick_modes_bar(&mut self, ui: &mut egui::Ui) {
        let height = ui.spacing().interact_size.y;
        let segment = |shown: &str, hover: &str| Segment {
            shown: shown.to_owned(),
            name: shown.to_owned(),
            hover: Some(hover.to_owned()),
        };
        let enabled = self.pick_enabled();
        let modes = [PickMode::Similar, PickMode::SameSubject];
        let people = [People::All, People::Without, People::Only];
        let orientations = [Orientation::All, Orientation::Same, Orientation::Mixed];
        let known = self.people_known();
        let (mut mode, mut filter, mut orientation) = (None, None, None);
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let selected = modes.iter().position(|m| *m == self.pick_modes.mode);
                mode = segmented(
                    ui,
                    ui.id().with("pick_mode"),
                    &[
                        segment(
                            "Similar",
                            "For bursts and near-identical shots where only one is worth keeping.",
                        ),
                        segment(
                            "Same subject",
                            "For whenever you don't need 3 angles of the same stuff.",
                        ),
                    ],
                    selected,
                    height,
                );
            });
            ui.add_space(S3);
            ui.add_enabled_ui(enabled && known, |ui| {
                let selected = people.iter().position(|p| *p == self.pick_modes.people);
                filter = segmented(
                    ui,
                    ui.id().with("pick_people"),
                    &[
                        segment("All", "For going through everything, people or not."),
                        segment(
                            "Without people",
                            "For scenery and things, when you don't want to pick between photos of friends.",
                        ),
                        segment(
                            "Only people",
                            "For when you just want the best shot of your friends and family.",
                        ),
                    ],
                    // Never checked for people: the filter shows every group.
                    if known { selected } else { Some(0) },
                    height,
                );
            });
            ui.add_space(S3);
            ui.add_enabled_ui(enabled, |ui| {
                let selected = orientations
                    .iter()
                    .position(|o| *o == self.pick_modes.orientation);
                orientation = segmented(
                    ui,
                    ui.id().with("pick_orientation"),
                    &[
                        segment(
                            "All",
                            "For when you don't mind whether the one you keep is tall or wide.",
                        ),
                        segment(
                            "Same orientation",
                            "For keeping a tall and a wide one of each thing, say one for a story and one for a post.",
                        ),
                        segment(
                            "Mixed only",
                            "For when you shot the same thing both tall and wide and want to choose a format.",
                        ),
                    ],
                    selected,
                    height,
                );
            });
            let mut notes = Vec::new();
            if !known && !self.pick_review.clusters().is_empty() {
                notes.push("People not detected yet · Analyze again to filter by people".to_owned());
            } else {
                let filtered = self.pick_modes.people != People::All;
                let unchecked = self.pick_modes.unchecked;
                if filtered && unchecked > 0 {
                    notes.push(format!(
                        "{} not checked for people",
                        count(unchecked, "group", "groups")
                    ));
                }
                let hidden = self.pick_modes.hidden.saturating_sub(if filtered {
                    unchecked
                } else {
                    0
                });
                if hidden > 0 {
                    notes.push(format!("{} hidden", count(hidden, "group", "groups")));
                }
            }
            if !notes.is_empty() {
                ui.add_space(S3);
                super::review_card::summary_label(ui, &notes.join(" · "));
            }
        });
        if let Some(index) = mode {
            self.set_pick_mode(modes[index]);
        }
        if let Some(index) = filter.filter(|_| known) {
            self.set_pick_people(people[index]);
        }
        if let Some(index) = orientation {
            self.set_pick_orientation(orientations[index]);
        }
    }

    /// Whether the filters hide every group of the mode shown.
    pub(super) fn pick_filter_hides_all(&self) -> bool {
        self.pick_modes.hidden > 0 && self.pick_review.clusters().is_empty()
    }
    /// Shows every group of the mode again: both filters back to All.
    pub(super) fn clear_pick_filters(&mut self) {
        self.pick_modes.people = People::All;
        self.pick_modes.orientation = Orientation::All;
        self.deselect_pick_group();
        self.show_pick_groups();
    }

    /// Pick Best with no group to show, once it has analyzed: the title and
    /// the line under it, for the mode and the filter.
    pub(super) fn pick_empty_text(&self) -> (String, String) {
        let hidden = self.pick_modes.hidden;
        let filtered = self.pick_filter_hides_all();
        let shown = match (self.pick_modes.people, self.pick_modes.orientation) {
            (People::All, Orientation::Mixed) => "of portrait and landscape photos".to_owned(),
            (People::All, _) => "for these filters".to_owned(),
            (people, orientation) => format!(
                "{}{}",
                if people == People::Without {
                    "without people"
                } else {
                    "with people"
                },
                if orientation == Orientation::Mixed {
                    ", portrait and landscape"
                } else {
                    ""
                }
            ),
        };
        match (self.pick_modes.mode, &self.pick_modes.subject) {
            (PickMode::SameSubject, None) => (
                "Same subject groups not found yet".into(),
                "Analyze again to find photos of one subject from other angles.".into(),
            ),
            (PickMode::SameSubject, Some(Err(error))) => {
                ("Same subject groups could not be found".into(), error.clone())
            }
            (_, _) if filtered => (
                format!("No groups {shown}"),
                format!(
                    "The filters hide {}.",
                    count(hidden, "group", "groups")
                ),
            ),
            (PickMode::SameSubject, Some(Ok(_))) => (
                "No subject shot from other angles".into(),
                "Photos of one subject taken from other angles within 15 minutes show here, when Similar does not group them already.".into(),
            ),
            (PickMode::Similar, _) => ("No similar photos to compare".into(), String::new()),
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
        if deck.not_shown > 0 {
            summary.push_str(&format!(" · {} more in groups not shown", deck.not_shown));
        }
        if deck.elsewhere > 0 {
            summary.push_str(&format!(" · {} more marked in other steps", deck.elsewhere));
        }
        // Before anything is marked, the line says what to do instead.
        if trash == 0
            && deck.elsewhere == 0
            && deck.not_shown == 0
            && !self.pick_review.clusters().is_empty()
        {
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
                    self.pick_order(cluster)
                        .into_iter()
                        .map(|id| self.photo_aspect(id, 0))
                        .collect()
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
            deck.not_shown = self
                .pick_review
                .not_shown()
                .iter()
                .filter(|id| self.decisions.get(id).is_some_and(|d| d.rejected))
                .count();
            deck.elsewhere = self
                .decisions
                .iter()
                .filter(|(id, d)| d.rejected && self.index.contains_key(id))
                .count()
                .saturating_sub(deck.trash + deck.not_shown);
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
        let shown = self.pick_order(cluster);
        self.pick_facts(cluster, &shown)
            .iter()
            .any(|facts| !facts.is_empty())
    }

    /// The card for one group: every photo, best first, and their deciding facts,
    /// the suggested one marked. A group nobody has decided yet shows no
    /// Keep or Trash, only that mark and Accept suggestion; once anything
    /// in it is decided, each photo has its Keep/Trash pill, and Undo as
    /// `pick_link` says.
    fn pick_card(&self, group: usize) -> ReviewCard {
        let cluster = &self.pick_review.clusters()[group];
        let enabled = self.pick_enabled();
        let undecided = self.pick_undecided(group);
        let started = self.card_started(&cluster.ordered);
        let order = self.pick_order(cluster);
        // Every photo of the group, best first: a burst of many makes a
        // wide card, which its row shrinks to fit.
        let shown = order;
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
                    // Only Trash is marked: the photos left unmarked are the
                    // ones kept.
                    badge: if undecided || keep {
                        Badge::None
                    } else {
                        Badge::keep(false)
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
                    toggle: enabled.then(|| super::review_card::keep_text(started, keep, side)),
                }
            })
            .collect();
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
            // header holds only Accept or Undo.
            title: String::new(),
            state_text: None,
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
        // A click on another group's card leaves focus: the panel shows
        // that group whole.
        if self.pick_group() != Some(group) {
            self.panel_focus = false;
        }
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
                    self.keep_pick(*id);
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
    /// Switches the photo at `side` of a group's card by hand, as D would,
    /// where a test needs a single photo marked or unmarked.
    fn flip(app: &mut PhotoSortApp, group: usize, side: usize) {
        let id = app.pick_order(&app.pick_review.clusters()[group])[side];
        app.toggle_pick(id);
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
    fn modes_and_the_people_filter_change_what_is_shown_and_never_a_choice_made_elsewhere() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.preferences.pick_best_auto = false;
        app.media = (1..=6).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        // Someone stands in 4 with their back turned: no face, but a person.
        // 1 is the best of 1-2, 5 of 4-5.
        for (n, score, people) in [
            (1, 30., 0),
            (2, 20., 0),
            (3, 40., 0),
            (4, 10., 1),
            (5, 20., 0),
            (6, 5., 0),
        ] {
            app.analysis.insert(
                MediaId(n),
                crate::core::analysis::Analysis {
                    score,
                    aesthetic: Some(0.5),
                    faces: Some(FaceSummary::default()),
                    people: Some(people),
                    ..Default::default()
                },
            );
            // All shot landscape; 5 was held upright, as its EXIF says.
            app.metadata.insert(
                MediaId(n),
                Metadata {
                    width: 6000,
                    height: 4000,
                    orientation: if n == 5 { 6 } else { 1 },
                    ..Default::default()
                },
            );
        }
        let ids = |groups: &[(u64, usize)]| -> HashMap<MediaId, usize> {
            groups.iter().map(|(n, g)| (MediaId(*n), *g)).collect()
        };
        app.cull_groups = Arc::new(ids(&[(1, 1), (2, 1), (4, 2), (5, 2)]));
        // Same subject also joins 3 and 6, other angles of each.
        app.pick_modes.found = Some(Ok(ids(&[(1, 1), (2, 1), (3, 1), (4, 2), (5, 2), (6, 2)])));
        app.finish_pick_analysis();
        app.set_step(Step::PickBest);
        app.refilter();
        // Pick Best opens on Same subject.
        assert_eq!(app.pick_modes.mode, PickMode::SameSubject);
        app.set_pick_mode(PickMode::Similar);
        let shown = |app: &PhotoSortApp| -> Vec<Vec<u64>> {
            app.pick_review
                .clusters()
                .iter()
                .map(|c| {
                    let mut ids: Vec<u64> = c.ordered.iter().map(|id| id.0).collect();
                    ids.sort();
                    ids
                })
                .collect()
        };
        let trashed = |app: &PhotoSortApp| -> Vec<u64> {
            (1..=6)
                .filter(|n| app.decision(MediaId(*n)).rejected)
                .collect()
        };
        assert_eq!(shown(&app), vec![vec![1, 2], vec![4, 5]]);

        // Only people: Accept all takes the shown group's suggestion and
        // leaves the hidden one as it is, also once it shows again.
        app.set_pick_people(People::Only);
        assert_eq!(shown(&app), vec![vec![4, 5]]);
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![4]);
        app.set_pick_people(People::All);
        assert_eq!(trashed(&app), vec![4]);

        // Photo 2 chosen by hand in Similar: 1 goes to Trash.
        app.keep_pick(MediaId(2));
        assert_eq!(trashed(&app), vec![1, 4]);
        app.set_pick_mode(PickMode::SameSubject);
        assert_eq!(shown(&app), vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(trashed(&app), vec![1, 4]);
        // Same subject's Undo and Undo all leave Similar's choices alone.
        app.undo_pick_group(0);
        app.undo_accepted(Step::PickBest);
        assert_eq!(trashed(&app), vec![1, 4]);
        assert!(app.by_hand.contains(&MediaId(2)));
        app.set_pick_mode(PickMode::Similar);
        assert_eq!(shown(&app), vec![vec![1, 2], vec![4, 5]]);
        // Similar's own Undo all takes back its Accept all.
        app.undo_accepted(Step::PickBest);
        assert_eq!(trashed(&app), vec![1]);

        // 4-6 keeps the portrait 5 and sends 4 and 6 to Trash.
        app.set_pick_mode(PickMode::SameSubject);
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![1, 4, 6]);
        app.set_pick_orientation(Orientation::Mixed);
        assert_eq!(shown(&app), vec![vec![4, 5, 6]]);
        // Split apart, 5 is a card of one and no group; 4 and 6 make a
        // card whose own best, 4, stays rather than all of it going.
        app.set_pick_orientation(Orientation::Same);
        assert_eq!(shown(&app), vec![vec![1, 2, 3], vec![4, 6]]);
        assert_eq!(trashed(&app), vec![1, 6]);
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
        // Trash on the photos going there (the rest, unmarked, are kept),
        // and Undo, which takes it back to nothing marked.
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![1]);
        assert_eq!(app.staged_change_count(), 1);
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains(&keep) && shown.contains(&trash), "{shown}");
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
        flip(&mut app, 1, 1);
        flip(&mut app, 1, 1);
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
        flip(&mut app, 0, 0);
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
    fn a_click_keeps_that_photo_and_undo_restores_the_suggestion() {
        let mut app = app();
        // Its suggestion is applied (3, the best, kept; 1 and 2 in Trash):
        // a click switches only its photo, and the others stay as accepted.
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 4], "Photo 2 alone comes back");
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // A group nobody has touched (suggestions off, Undo clears it): a
        // click keeps that photo and sends the others to Trash.
        app.preferences.pick_best_auto = false;
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![4]);
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 3, 4], "Photo 2 alone is kept");
        assert_eq!(app.pick_group(), Some(0));
        assert!(app.pick_review.changed(0, &app.decisions));
        // Once started, a click switches that photo: back from Trash, then
        // to Trash again.
        app.pick_card_event(0, CardEvent::Toggle(0));
        assert_eq!(trashed(&app), vec![1, 4], "Photo 3 is kept too");
        app.pick_card_event(0, CardEvent::Toggle(0));
        assert_eq!(trashed(&app), vec![1, 3, 4], "a second click trashes it");
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 2, 3, 4], "so does one on photo 2");
        // Undo starts the group over, as suggested.
        app.preferences.pick_best_auto = true;
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // Accept all leaves a group the user has started on as it is: photos
        // 1 and 4 were kept by hand, so the rest of their groups stays too,
        // photo 2 included, which nobody decided.
        flip(&mut app, 1, 1);
        flip(&mut app, 0, 2);
        app.decisions.remove(&MediaId(2));
        assert_eq!(trashed(&app), Vec::<u64>::new());
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), Vec::<u64>::new());
        // A group whose suggested photo is marked for Trash, by hand or by
        // Easy Delete, is left as it is until its own Undo: marking the
        // undecided photo 4 would leave it with no photo kept.
        app.pick_card_event(1, CardEvent::Undo);
        app.decisions.remove(&MediaId(4));
        flip(&mut app, 1, 0);
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
        // A number key switches the nth photo of the group shown, best
        // first: its suggestion is applied, so the others stay as they are. The group's view stays as it
        // is: original detail already shown is not dropped to wait again.
        app.view_started = Instant::now() - Duration::from_secs(1);
        hold(&mut app, egui::Key::Num2, egui::Modifiers::NONE, 3);
        assert_eq!(
            trashed(&app),
            vec![1, 4],
            "2 brings back the second best alone"
        );
        assert!(
            app.view_started.elapsed() >= Duration::from_secs(1),
            "a switch restarted the wait for original detail"
        );
        key(&mut app, egui::Key::R, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 4], "R does nothing here");
        key(&mut app, egui::Key::U, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4], "U undoes the group");
        // F focuses one photo: the arrows flip between the group's three,
        // round from the last, never leaving the group or restarting the
        // wait for detail.
        key(&mut app, egui::Key::F, egui::Modifiers::NONE);
        assert!(app.focus_active());
        let first = app.focused_slot;
        // In focus a number only shows that photo, marking nothing; D
        // sends the one shown to Trash and back, alone. Photo 3 is the best.
        key(&mut app, egui::Key::Num1, egui::Modifiers::NONE);
        assert_eq!(app.focused_slot, 0);
        assert_eq!(trashed(&app), vec![1, 2, 4], "a number marked a photo");
        key(&mut app, egui::Key::D, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 3, 4], "D trashes the one shown");
        key(&mut app, egui::Key::D, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4], "D again keeps it");
        key(&mut app, egui::Key::Num2, egui::Modifiers::NONE);
        assert_eq!(app.focused_slot, 1);
        key(&mut app, egui::Key::ArrowRight, egui::Modifiers::NONE);
        assert_eq!(app.pick_group(), Some(0));
        assert_eq!(app.focused_slot, (first + 1) % 3);
        key(&mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        key(&mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        assert_eq!(app.focused_slot, (first + 2) % 3);
        assert!(app.view_started.elapsed() >= Duration::from_secs(1));
        // Only the focused photo is drawn, but every photo of the group is
        // asked for in its view, so flipping shows detail at once.
        for n in 1..=3 {
            app.metadata.insert(
                MediaId(n),
                Metadata {
                    width: 6000,
                    height: 4000,
                    ..Default::default()
                },
            );
        }
        app.inspection_wanted.clear();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        // Small previews, so the panel wants original detail.
        for n in 1..=3 {
            let handle = ctx.load_texture(
                "preview",
                egui::ColorImage::filled([600, 400], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            app.textures.insert(
                (MediaId(n), true),
                Texture {
                    handle,
                    bytes: 600 * 400 * 4,
                    used: 0,
                },
            );
        }
        let _ = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.review_canvas(ui));
            },
        );
        let mut asked: Vec<u64> = app.inspection_wanted.iter().map(|r| r.slot).collect();
        asked.sort_unstable();
        assert_eq!(asked, [0, 1, 2]);
        let regions: Vec<_> = app.inspection_wanted.iter().map(|r| r.region).collect();
        assert!(regions.windows(2).all(|pair| pair[0] == pair[1]));
        // Esc leaves focus first, then closes the panel.
        key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.focus_active() && app.pick_panel);
        // Closing the panel any other way (its X) ends focus too.
        key(&mut app, egui::Key::F, egui::Modifiers::NONE);
        assert!(app.focus_active());
        app.pick_panel = false;
        super::super::review_card::render_grid(&mut app);
        assert!(!app.panel_focus, "focus outlived the panel");
        app.pick_panel = true;
        // A click on another group's card leaves focus too.
        key(&mut app, egui::Key::F, egui::Modifiers::NONE);
        app.pick_card_event(1, CardEvent::Select(0));
        assert!(!app.panel_focus && app.pick_group() == Some(1));
        app.pick_card_event(0, CardEvent::Select(0));
        app.view_started = Instant::now() - Duration::from_secs(1);
        // Out of focus the three sit in one row, two in view at a time,
        // scrolling sideways: only those in view ask for detail.
        app.inspection_wanted.clear();
        let _ = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.review_canvas(ui));
            },
        );
        assert_eq!(app.inspection_wanted.len(), 2);
        // A two-finger swipe across the row, never quite level, scrolls it
        // to the third photo and leaves the zoom alone.
        let zoom = app.view_for(0).zoom;
        let mut slots = Vec::new();
        for step in 0..12 {
            app.inspection_wanted.clear();
            let events = if step < 4 {
                vec![
                    egui::Event::PointerMoved(egui::pos2(900.0, 400.0)),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(-200.0, 30.0),
                        modifiers: Default::default(),
                    },
                ]
            } else {
                Vec::new()
            };
            let _ = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.review_canvas(ui));
                },
            );
            slots = app.inspection_wanted.iter().map(|r| r.slot).collect();
        }
        assert!(slots.contains(&2), "the row did not scroll: {slots:?}");
        assert_eq!(app.view_for(0).zoom, zoom, "a sideways swipe zoomed");
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
    fn a_group_shows_its_shots_in_the_order_they_were_taken_from_the_best() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = (1..=3).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        // Standing (1, the best), sitting 16 s later (2), standing again
        // 14 s later (3). The whole-frame embeddings, ruled by the scene,
        // put the sitting shot closest to the best.
        for (n, at) in [(1, 0), (2, 16_000), (3, 14_000)] {
            app.metadata.insert(
                MediaId(n),
                Metadata {
                    captured_ms: Some(at),
                    ..Default::default()
                },
            );
        }
        for (n, embedding) in [(1, [1.0, 0.0]), (2, [1.0, 0.1]), (3, [0.0, 1.0])] {
            app.embeddings
                .insert(MediaId(n), Arc::new(embedding.to_vec()));
        }
        let ids: Vec<MediaId> = (1..=3).map(MediaId).collect();
        let scores = ids.iter().map(|id| (*id, 10.0 - id.0 as f64)).collect();
        let mut cluster = Cluster::new(1, ids, scores);
        app.arrange_pick_group(&mut cluster);
        assert_eq!(cluster.shown, [1, 3, 2].map(MediaId), "by capture time");
        // Without capture times, the embeddings decide.
        app.metadata.clear();
        app.arrange_pick_group(&mut cluster);
        assert_eq!(cluster.shown, [1, 2, 3].map(MediaId), "by embedding");
    }

    #[test]
    fn a_card_has_no_title_and_shows_every_photo_of_its_group() {
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
        // Every photo of the group is on its card, however many there are:
        // no "+N more" leaves some out.
        let card = app.pick_card(0);
        assert_eq!(card.photos.len(), 20);
        assert_eq!(card.state_text, None);
        assert!(card.title.is_empty());
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
        app.pick_modes.mode = PickMode::Similar;
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
