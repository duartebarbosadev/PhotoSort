use super::review_card::{
    Badge, CardEvent, CardPhoto, Fact, GridClick, Inset, ReviewCard, size_text, slot_ratio,
    summary_label,
};
use super::*;
use crate::core::delete_review::{self, Category, Entry, FormatChoice, Review};

/// How the star on a suggested keeper explains itself.
pub(super) const SUGGESTED: &str = "Suggested by PhotoSort.";

/// On a photo PhotoSort suggests moving to Trash and left unmarked: every
/// other photo of its Pick Best group is going there already.
pub(super) const LAST_KEPT: &str = "Only photo kept from its group";

/// The long side, in pixels, a closed-eyes card's zoomed eyes are cut at.
const EYE_EDGE: f32 = 480.0;

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What Easy Delete's grid is drawn from, kept between frames: a frame in
/// which nothing changed reads it instead of working it out for every card.
#[derive(Default)]
pub(super) struct Deck {
    /// The review the cards were worked out from.
    review: u64,
    /// What the grid asks of each card to place it, by subject. Kept across
    /// tabs and searches, for as long as the review is.
    cards: HashMap<MediaId, DeckCard>,
    /// The metadata the cards were last checked against.
    metadata: (usize, u64),
    /// Changes whenever a card's shape, facts line or place may have: the
    /// grid asks its cards again.
    stamp: u64,
    /// The list the cards shown were taken from.
    listed: Option<(u64, usize)>,
    shown: std::rc::Rc<Shown>,
    /// The decisions the counts were taken from.
    counted: Option<u64>,
    counts: Counts,
    /// The Pick Best groups and the decisions `last_kept` was taken from.
    kept: Option<(u64, u64)>,
    /// The last photo still kept of each Pick Best group. A suggestion
    /// never marks one: its card says so and stays as it is.
    last_kept: HashSet<MediaId>,
    /// A card was drawn with a facts line the grid did not expect, or
    /// without one it did.
    redrawn: bool,
}

impl Deck {
    /// Notes whether a card just drawn had facts to show. They come from
    /// more than the review and the metadata (the analysis, the files'
    /// sizes), so what was worked out before can be out of date: the card
    /// on screen settles it.
    fn drew(&mut self, subject: MediaId, facts: bool) {
        if let Some(card) = self.cards.get_mut(&subject)
            && card.facts != facts
        {
            card.facts = facts;
            self.stamp = crate::core::revision::next();
            self.redrawn = true;
        }
    }
}

/// A photo's size, orientation and capture time, if known.
type Seen = Option<(u32, u32, u8, Option<i64>)>;

/// What the grid asks of one card to place it.
struct DeckCard {
    /// Each photo of the card, and its metadata when this was worked out.
    seen: Vec<(MediaId, Seen)>,
    /// Each photo's width over its height.
    aspects: Vec<f32>,
    /// It shows a line of facts.
    facts: bool,
    titled: bool,
}

/// The cards the grid shows, in order.
#[derive(Default)]
pub(super) struct Shown {
    subjects: Vec<MediaId>,
    /// Each subject's place among them.
    places: HashMap<MediaId, usize>,
    /// Each group shown and how many cards it has.
    sections: Vec<(Category, usize)>,
    /// Each group's first card.
    starts: Vec<usize>,
    lengths: Vec<usize>,
}

/// The numbers over the grid.
#[derive(Default)]
struct Counts {
    /// Photos going to Trash, and their bytes.
    trash: usize,
    freed: u64,
    /// Cards Accept all suggestions would change.
    changed: usize,
    sections: Vec<SectionCounts>,
}

/// The numbers in one group's header.
#[derive(Clone, Default)]
struct SectionCounts {
    /// Cards its Accept all acts on, how many it would change and what the
    /// suggestions free.
    acts_on: usize,
    changed: usize,
    frees: u64,
    /// For two formats of one shot: what each choice frees, and the choice
    /// every card has, if they agree.
    formats: [u64; 3],
    choice: Option<FormatChoice>,
}

impl PhotoSortApp {
    pub(super) fn finish_delete_analysis(&mut self) {
        let entries = delete_review::from_analysis(
            &self.media,
            &self.analysis,
            &self.metadata,
            &self.delete_pairs,
            &self.delete_blinks,
        );
        let mut changed: std::collections::HashSet<_> = self
            .duplicates
            .keys()
            .chain(self.near_duplicates.keys())
            .copied()
            .collect();
        self.duplicates.clear();
        self.near_duplicates.clear();
        for pair in &self.delete_pairs {
            let map = match pair.kind {
                crate::core::near_duplicates::batch::Kind::Exact => &mut self.duplicates,
                crate::core::near_duplicates::batch::Kind::Near => &mut self.near_duplicates,
                // One shot in two formats is neither a copy nor a similar frame.
                crate::core::near_duplicates::batch::Kind::Formats => continue,
            };
            map.insert(pair.discard, pair.keep);
            changed.insert(pair.discard);
        }
        for id in changed {
            self.update_browse_item(id);
        }
        self.delete_review.replace(entries);
        if self.step == Step::EasyDelete {
            self.trash_last = None;
        }
        self.view_data_ready.insert(Step::EasyDelete);
        self.stage_delete_suggestions();
        let saved = self
            .pending_bookmark
            .as_ref()
            .map(|v| v.delete_categories.iter().copied().collect());
        self.delete_categories = self.delete_tab_categories(saved.as_ref());
        self.delete_run = None;
        self.filter_dirty = true;
        self.finish_step_status(Step::EasyDelete);
    }
    /// With the setting on, suggestions are the default decisions: each
    /// photo's is staged once, when it is first found or when Easy Delete
    /// opens after staged choices were discarded.
    pub(super) fn stage_delete_suggestions(&mut self) {
        if !self.preferences.easy_delete_auto || !self.view_data_ready.contains(&Step::EasyDelete) {
            return;
        }
        let marked = self
            .delete_review
            .stage_new_suggestions_sparing(&self.pick_review, &mut self.decisions);
        if !marked.is_empty() {
            self.note_trash_steps(&marked, Step::EasyDelete);
            self.changed_items(marked);
        }
    }
    /// The categories a tab shows: one group, or every group ("All"). A saved
    /// set of several groups, or one group with no photos, opens on All.
    fn delete_tab_categories(&self, saved: Option<&HashSet<Category>>) -> HashSet<Category> {
        match saved.filter(|set| set.len() == 1) {
            Some(single)
                if self
                    .delete_review
                    .entries()
                    .iter()
                    .any(|e| single.contains(&e.category)) =>
            {
                single.clone()
            }
            _ => Category::ALL.into_iter().collect(),
        }
    }
    /// The selected tab's group, or `None` for All.
    pub(super) fn delete_tab(&self) -> Option<Category> {
        if self.delete_categories.len() == 1 {
            self.delete_categories.iter().next().copied()
        } else {
            None
        }
    }
    fn set_delete_tab(&mut self, tab: Option<Category>) {
        self.delete_categories = match tab {
            Some(category) => [category].into_iter().collect(),
            None => Category::ALL.into_iter().collect(),
        };
        self.filter_dirty = true;
    }
    pub(super) fn delete_review_ids(&self) -> Vec<MediaId> {
        self.filtered.iter().map(|i| self.media[*i].id).collect()
    }
    /// Why a card suggests what it does. The sentence names the better shot
    /// as the file was called when the suggestions were made: with a new
    /// name staged in Organize, it says that name, like every other view.
    fn delete_reason(&self, entry: &Entry) -> String {
        let media = entry
            .companion
            .and_then(|id| self.index.get(&id))
            .map(|&i| &self.media[i]);
        match media {
            Some(media) if self.file_names.contains_key(&media.id) => {
                entry.reason.replace(&media.name(), &self.shown_name(media))
            }
            _ => entry.reason.clone(),
        }
    }
    fn delete_enabled(&self) -> bool {
        !self.busy && self.delete_run.is_none()
    }
    /// Switches one photo of the current card between Keep and Trash.
    pub(super) fn toggle_delete_candidate(&mut self, side: usize) {
        if !self.delete_enabled() {
            return;
        }
        if let Some(review) = self.delete_current
            && let Some(candidate) = self
                .delete_review
                .entry(review)
                .and_then(|e| e.candidates().get(side).copied())
        {
            self.delete_review
                .toggle(review, candidate, &mut self.decisions);
            self.by_hand.insert(candidate);
            self.active = Some(candidate);
            self.focused_slot = side as u64;
            self.changed_items([candidate]);
        }
    }
    /// Left and Right decide the whole card shown beside the grid. Keep
    /// keeps every photo on it; Trash sends all but its keeper to Trash, so
    /// neither can leave a pair with both photos marked.
    pub(super) fn decide_delete_card(&mut self, trash: bool) {
        if !self.delete_enabled() {
            return;
        }
        if let Some(id) = self.delete_current {
            if trash {
                self.delete_review.trash(id, &mut self.decisions);
            } else {
                self.delete_review.keep(id, &mut self.decisions);
            }
            let decided = self.delete_candidates(&[id]);
            self.by_hand.extend(decided.iter().copied());
            self.changed_items(decided);
        }
    }
    /// Accept all suggestions, for these cards: every photo not decided by
    /// hand takes its suggestion, and a choice made by hand stays as it is.
    /// A photo that would be the last one kept of its card or of its Pick
    /// Best group is left as it is, so none ever ends with no photo kept.
    fn accept_delete_suggestions(&mut self, ids: &[MediaId]) {
        if !self.delete_enabled() {
            return;
        }
        let changed = self.delete_review.accept_sparing(
            ids,
            &self.pick_review,
            &mut self.decisions,
            &self.by_hand,
        );
        self.accepted_status(changed.len());
        self.changed_items(changed);
    }
    /// The photos of the cards of `ids`, each once: an Eyes closed card's
    /// better shot can be on another card too.
    fn delete_candidates(&self, ids: &[MediaId]) -> Vec<MediaId> {
        let mut seen = HashSet::new();
        ids.iter()
            .filter_map(|id| self.delete_review.entry(*id))
            .flat_map(Entry::candidates)
            .filter(|id| seen.insert(*id))
            .collect()
    }
    /// Undoes the user's changes to the current card. One card, asked for
    /// by hand: its suggestion applies in full, and its photos count as
    /// undecided again.
    pub(super) fn undo_delete(&mut self) {
        if let Some(id) = self.delete_current
            && self.delete_enabled()
        {
            self.delete_review.accept(&[id], &mut self.decisions);
            let photos = self.delete_candidates(&[id]);
            for photo in &photos {
                self.by_hand.remove(photo);
            }
            self.changed_items(photos);
        }
    }
    /// Every card shown back to its suggestion.
    pub(super) fn accept_delete_defaults(&mut self) {
        let ids = self.delete_review_ids();
        self.accept_delete_suggestions(&ids);
    }
    /// A click on the grid's empty space: no card is selected, and the
    /// panel beside the grid closes. An arrow key, Enter or a click selects
    /// again.
    pub(super) fn deselect_delete_card(&mut self) {
        self.delete_panel = false;
        self.delete_current = None;
        self.active = None;
    }
    /// Enter: the suggestion is already the decision, so move on.
    pub(super) fn next_delete_card(&mut self) {
        self.navigate_selection(1, false, false);
        self.delete_panel = self.delete_current.is_some();
    }
    fn choose_delete_formats(&mut self, ids: &[MediaId], choice: FormatChoice) {
        if !self.delete_enabled() {
            return;
        }
        self.delete_review
            .choose_formats(ids, choice, &mut self.decisions);
        // Only the two-format cards were decided, by hand.
        let pairs: Vec<MediaId> = ids
            .iter()
            .copied()
            .filter(|id| {
                self.delete_review
                    .entry(*id)
                    .is_some_and(|e| e.category == Category::Formats)
            })
            .collect();
        self.by_hand.extend(self.delete_candidates(&pairs));
        self.changed_items(self.delete_candidates(ids));
    }
    /// The group tabs, with their counts. Only groups with suggestions get one.
    pub(super) fn delete_controls(&mut self, ui: &mut egui::Ui) {
        let counts: Vec<(Category, usize)> = Category::ALL
            .into_iter()
            .map(|c| {
                let n = self
                    .delete_review
                    .entries()
                    .iter()
                    .filter(|e| e.category == c)
                    .count();
                (c, n)
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        if counts.is_empty() {
            return;
        }
        let tabs: Vec<Option<Category>> = std::iter::once(None)
            .chain(counts.iter().map(|(c, _)| Some(*c)))
            .collect();
        let segments: Vec<Segment> = tabs
            .iter()
            .map(|tab| {
                let (title, n) = match tab {
                    None => ("All", self.delete_review.entries().len()),
                    Some(c) => (c.title(), counts.iter().find(|(o, _)| o == c).unwrap().1),
                };
                Segment {
                    shown: format!("{title}  {n}"),
                    name: format!("{title}, {n}"),
                    hover: None,
                }
            })
            .collect();
        let selected = tabs.iter().position(|t| *t == self.delete_tab());
        // An overflowing bar picks the group from a menu instead.
        if self.bar_fit == super::top_bar::Fit::Overflow {
            let mut clicked = None;
            egui::ComboBox::from_id_salt("delete_tabs_menu")
                .selected_text(selected.map_or("All", |i| segments[i].shown.as_str()))
                .show_ui(ui, |ui| {
                    for (index, segment) in segments.iter().enumerate() {
                        if ui
                            .selectable_label(selected == Some(index), &segment.shown)
                            .clicked()
                        {
                            clicked = Some(index);
                        }
                    }
                });
            if let Some(clicked) = clicked
                && Some(clicked) != selected
            {
                self.set_delete_tab(tabs[clicked]);
            }
            return;
        }
        if let Some(clicked) = segmented(
            ui,
            ui.id().with("delete_tabs"),
            &segments,
            selected,
            ui.spacing().interact_size.y,
        ) && Some(clicked) != selected
        {
            self.set_delete_tab(tabs[clicked]);
        }
    }
    /// The current card's photos, large, beside the grid: its tag (the
    /// reason in its tooltip), then the comparison Review showed, with zoom,
    /// Keep/Trash and Undo. The grid is its strip.
    fn delete_compare_panel(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let Some(entry) = self
            .delete_current
            .and_then(|id| self.delete_review.entry(id))
            .cloned()
        else {
            self.delete_panel = false;
            return;
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(&entry.tag).font(semibold(13.0)).color(INK))
                .on_hover_text(self.delete_reason(&entry));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, icon::X, "Close · Esc").clicked() {
                    self.delete_panel = false;
                }
            });
        });
        ui.add_space(S1);
        self.review_canvas(ui);
    }
    /// With suggestions applied automatically, going back to one undoes the
    /// user's change; otherwise it applies the suggestion for the first time.
    fn delete_undo_label(&self) -> &'static str {
        if self.preferences.easy_delete_auto {
            "Undo"
        } else {
            "Use suggestion"
        }
    }
    /// The comparison panel's Undo, beside its Keep/Trash switches.
    pub(super) fn delete_undo_button(&mut self, ui: &mut egui::Ui) {
        if let Some(id) = self.delete_current {
            let changed = self.delete_review.changed(id, &self.decisions);
            if ui
                .add_enabled(
                    changed && self.delete_enabled(),
                    egui::Button::new(self.delete_undo_label()),
                )
                .on_hover_text("R")
                .clicked()
            {
                self.undo_delete();
            }
        }
    }
}

impl PhotoSortApp {
    /// Easy Delete's only view: a summary with Accept all suggestions, then
    /// one section per group with its bulk action, and one card per
    /// suggestion, a pair side by side or a single photo. Clicking a card
    /// opens its photos large in a panel beside the grid. The keyboard acts
    /// on the current card in both.
    pub(super) fn delete_grid(&mut self, ui: &mut egui::Ui) {
        let shown = self.delete_deck();
        self.delete_summary(ui);
        ui.add_space(S2);
        // The grid takes its final width at once and the panel slides over
        // the room it leaves, in or out: one reflow per open or close, never
        // one per frame of the slide. The grid keeps room for one card.
        let area = ui.available_rect_before_wrap();
        let panel_shown = self.delete_panel && self.delete_current.is_some();
        let slide = ui
            .ctx()
            .animate_bool_responsive(egui::Id::new("delete_panel_slide"), panel_shown);
        let widest = (area.width() - 460.0).max(360.0);
        self.delete_panel_width = self
            .delete_panel_width
            .unwrap_or(area.width() * 0.45)
            .clamp(360.0, widest)
            .into();
        let panel_width = self.delete_panel_width.unwrap_or(360.0);
        let grid = if panel_shown {
            area.with_max_x(area.right() - panel_width - S3)
        } else {
            area
        };
        let current = self
            .delete_current
            .and_then(|id| shown.places.get(&id).copied());
        let stamp = self.grids.delete.stamp;
        let subjects = &shown.subjects;
        let clicked = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(grid)
                    .layout(egui::Layout::top_down(Align::Min)),
                |ui| {
                    self.sectioned_card_grid(
                        ui,
                        "delete",
                        &shown.lengths,
                        stamp,
                        |app, i, shapes| {
                            if let Some(card) = app.grids.delete.cards.get(&subjects[i]) {
                                shapes.extend_from_slice(&card.aspects);
                            }
                        },
                        |app, i| {
                            app.grids
                                .delete
                                .cards
                                .get(&subjects[i])
                                .is_some_and(|card| card.facts)
                        },
                        |app, i| {
                            app.grids
                                .delete
                                .cards
                                .get(&subjects[i])
                                .is_some_and(|card| card.titled)
                        },
                        |app, ui, section| app.delete_section_header(ui, &shown, section),
                        |app, i| {
                            let entry = app.delete_review.entry(subjects[i])?.clone();
                            let card = app.delete_card(&entry);
                            let facts = card.photos.iter().any(|photo| !photo.facts.is_empty());
                            app.grids.delete.drew(subjects[i], facts);
                            Some(card)
                        },
                        current,
                    )
                },
            )
            .inner;
        // A card drawn with a facts line its row has no room for, or without
        // the one it keeps room for: its rows are laid out again next frame.
        if std::mem::take(&mut self.grids.delete.redrawn) {
            ui.ctx().request_repaint();
        }
        if slide > 0.0 {
            self.delete_side_panel(ui, area, panel_width, slide);
        }
        ui.advance_cursor_after_rect(area);
        match clicked {
            Some(GridClick::Card(i, event)) => self.delete_card_event(subjects[i], event),
            Some(GridClick::Background) => self.deselect_delete_card(),
            None => {}
        }
    }

    /// What the grid is drawn from, brought up to date: the cards shown
    /// when the list was built again, each card's shape and facts line when
    /// the review was replaced or one of its photos' details arrived, and
    /// the counts and the photos held back when a decision or Pick Best's
    /// groups changed. A frame in which none of that happened looks up the
    /// stamps.
    fn delete_deck(&mut self) -> std::rc::Rc<Shown> {
        let mut deck = std::mem::take(&mut self.grids.delete);
        let review = self.delete_review.revision();
        if deck.review != review {
            deck.review = review;
            deck.cards.clear();
            deck.listed = None;
        }
        let listed = (self.grids.listed, self.filtered.len());
        let metadata = self.metadata_stamp();
        if deck.listed != Some(listed) {
            let shown = self.delete_shown();
            for subject in &shown.subjects {
                if !deck.cards.contains_key(subject)
                    && let Some(card) = self.delete_deck_card(*subject)
                {
                    deck.cards.insert(*subject, card);
                }
            }
            deck.shown = std::rc::Rc::new(shown);
            deck.listed = Some(listed);
            deck.counted = None;
            deck.stamp = crate::core::revision::next();
        }
        if deck.metadata != metadata {
            // Only the cards one of whose photos' details changed.
            for (subject, card) in &mut deck.cards {
                if card
                    .seen
                    .iter()
                    .any(|(id, seen)| self.delete_seen(*id) != *seen)
                    && let Some(again) = self.delete_deck_card(*subject)
                {
                    *card = again;
                    deck.stamp = crate::core::revision::next();
                }
            }
            deck.metadata = metadata;
        }
        let decisions = self.decisions.revision();
        let kept = (self.pick_review.revision(), decisions);
        if deck.kept != Some(kept) {
            deck.last_kept = self.pick_review.last_kept(&self.decisions);
            deck.kept = Some(kept);
            deck.counted = None;
        }
        if deck.counted != Some(decisions) {
            deck.counts = self.delete_counts(&deck.shown, &deck.last_kept);
            deck.counted = Some(decisions);
        }
        let shown = deck.shown.clone();
        self.grids.delete = deck;
        shown
    }

    /// The cards shown, in order, and the groups they fall in.
    fn delete_shown(&self) -> Shown {
        let subjects = self.delete_review_ids();
        let mut sections: Vec<(Category, usize)> = Vec::new();
        for id in &subjects {
            let category = self.delete_review.entry(*id).map(|e| e.category);
            match (sections.last_mut(), category) {
                (Some((last, n)), Some(c)) if *last == c => *n += 1,
                (_, Some(c)) => sections.push((c, 1)),
                _ => {}
            }
        }
        let starts = sections
            .iter()
            .scan(0, |start, (_, n)| {
                let at = *start;
                *start += n;
                Some(at)
            })
            .collect();
        Shown {
            places: subjects
                .iter()
                .enumerate()
                .map(|(place, id)| (*id, place))
                .collect(),
            lengths: sections.iter().map(|(_, n)| *n).collect(),
            subjects,
            sections,
            starts,
        }
    }

    /// What the grid asks of `subject`'s card to place it.
    fn delete_deck_card(&self, subject: MediaId) -> Option<DeckCard> {
        let entry = self.delete_review.entry(subject)?;
        Some(DeckCard {
            seen: entry
                .candidates()
                .into_iter()
                .map(|id| (id, self.delete_seen(id)))
                .collect(),
            aspects: self.delete_card_aspects(subject),
            facts: self.delete_card_has_facts(subject),
            titled: !card_title(entry).is_empty(),
        })
    }

    /// What a card's shape and facts take from a photo's metadata.
    fn delete_seen(&self, id: MediaId) -> Seen {
        self.metadata
            .get(&id)
            .map(|m| (m.width, m.height, m.orientation, m.captured_ms))
    }

    /// The numbers over the grid and in its groups' headers.
    fn delete_counts(&self, shown: &Shown, last_kept: &HashSet<MediaId>) -> Counts {
        let trash: Vec<MediaId> = self
            .delete_candidates(&shown.subjects)
            .into_iter()
            .filter(|id| self.decisions.get(id).is_some_and(|d| d.rejected))
            .collect();
        let changed = |ids: &[MediaId]| {
            ids.iter()
                .filter(|id| {
                    self.delete_review
                        .pending(**id, &self.decisions, last_kept, &self.by_hand)
                })
                .count()
        };
        // Every card Accept all would change, counted with its group.
        let mut differ = 0;
        let sections = shown
            .sections
            .iter()
            .zip(&shown.starts)
            .map(|((category, n), start)| {
                let ids = &shown.subjects[*start..start + n];
                if *category == Category::Formats {
                    differ += changed(ids);
                    return SectionCounts {
                        formats: FormatChoice::ALL
                            .map(|choice| self.format_trash_bytes(ids, choice)),
                        choice: self.delete_review.format_choice(ids, &self.decisions),
                        ..Default::default()
                    };
                }
                let acts_on = self.delete_section_ids(*category, ids);
                let counts = SectionCounts {
                    acts_on: acts_on.len(),
                    changed: changed(&acts_on),
                    frees: self.suggested_trash_bytes(&acts_on),
                    ..Default::default()
                };
                differ += if acts_on.len() == ids.len() {
                    counts.changed
                } else {
                    changed(ids)
                };
                counts
            })
            .collect();
        Counts {
            trash: trash.len(),
            freed: trash.iter().map(|id| self.delete_bytes(*id)).sum(),
            changed: differ,
            sections,
        }
    }

    /// The cards of a group its Accept all acts on. Only closed-eyes shots
    /// with a better one have a suggestion to act on; the others are kept
    /// and stay kept.
    fn delete_section_ids(&self, category: Category, ids: &[MediaId]) -> Vec<MediaId> {
        if category == Category::EyesClosed {
            ids.iter()
                .copied()
                .filter(|id| {
                    self.delete_review
                        .entry(*id)
                        .is_some_and(|e| !e.informational())
                })
                .collect()
        } else {
            ids.to_vec()
        }
    }

    /// The panel beside the grid, `slide` of the way in from the right edge
    /// of `area`, over the grid's cards while it slides out. Its left edge
    /// resizes it.
    fn delete_side_panel(&mut self, ui: &mut egui::Ui, area: egui::Rect, width: f32, slide: f32) {
        let rect = egui::Rect::from_min_size(
            egui::pos2(area.right() - (width + S3) * slide + S3, area.top()),
            egui::vec2(width, area.height()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("delete_side_panel")
                .max_rect(rect)
                .layout(egui::Layout::top_down(Align::Min)),
            |ui| {
                ui.set_clip_rect(area.intersect(ui.clip_rect()));
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, EDGE))
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2 - 1.0)
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        self.delete_compare_panel(ui);
                    });
            },
        );
        let handle = egui::Rect::from_x_y_ranges(
            rect.left() - 4.0..=rect.left() + 2.0,
            rect.top()..=rect.bottom(),
        );
        let response = ui
            .interact(handle, egui::Id::new("delete_panel_resize"), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            self.delete_panel_width = Some(width - response.drag_delta().x);
        }
    }

    /// One line over the grid: what is staged, the photo size, and Accept
    /// all suggestions.
    fn delete_summary(&mut self, ui: &mut egui::Ui) {
        let counts = &self.grids.delete.counts;
        let (trash, freed, changed) = (counts.trash, counts.freed, counts.changed);
        ui.horizontal(|ui| {
            // Accept all and the size slider are placed first, from the
            // right; the summary then gets only the room left and truncates.
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        changed > 0 && self.delete_enabled(),
                        egui::Button::new("Accept all suggestions"),
                    )
                    .on_hover_text("A")
                    .clicked()
                {
                    self.accept_delete_defaults();
                }
                ui.add_space(S3);
                self.size_slider(ui);
                ui.add_space(S3);
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    summary_label(
                        ui,
                        &format!(
                            "{} going to Trash · frees {}",
                            count(trash, "photo", "photos"),
                            size_text(freed),
                        ),
                    );
                });
            });
        });
    }

    /// A group's header: its name, and the action for all of it. The tab
    /// over the grid has its size.
    fn delete_section_header(&mut self, ui: &mut egui::Ui, shown: &Shown, section: usize) {
        let (category, n) = shown.sections[section];
        let ids = &shown.subjects[shown.starts[section]..shown.starts[section] + n];
        let counts = self
            .grids
            .delete
            .counts
            .sections
            .get(section)
            .cloned()
            .unwrap_or_default();
        ui.label(
            RichText::new(category.heading())
                .font(semibold(14.0))
                .color(INK),
        );
        let enabled = self.delete_enabled();
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if category == Category::Formats {
                // What each choice frees, measured on these files: which
                // format is smaller depends on the camera and its settings.
                let segments: Vec<Segment> = FormatChoice::ALL
                    .into_iter()
                    .zip(counts.formats)
                    .map(|(choice, frees)| {
                        let shown = match choice {
                            FormatChoice::Both => choice.title().to_owned(),
                            _ => format!("{} · frees {}", choice.title(), size_text(frees)),
                        };
                        Segment {
                            name: shown.clone(),
                            shown,
                            hover: None,
                        }
                    })
                    .collect();
                let selected = counts
                    .choice
                    .and_then(|c| FormatChoice::ALL.iter().position(|o| *o == c));
                ui.add_enabled_ui(enabled, |ui| {
                    if let Some(clicked) = segmented(
                        ui,
                        ui.id().with("format_choice"),
                        &segments,
                        selected,
                        ui.spacing().interact_size.y,
                    ) {
                        self.choose_delete_formats(ids, FormatChoice::ALL[clicked]);
                    }
                });
                return;
            }
            if counts.acts_on == 0 {
                ui.label(RichText::new("No better shots · kept").color(MUTED));
                return;
            }
            let hover = match category {
                Category::Exact => "Trashes one file of each identical pair",
                Category::Near => "Trashes the weaker photo of each pair",
                Category::EyesClosed => "Trashes each closed-eyes shot that has a better one",
                Category::Blur | Category::Dark | Category::Bright => "Trashes every photo here",
                Category::Formats => unreachable!(),
            };
            let label = format!("Accept all · frees {}", size_text(counts.frees));
            if ui
                .add_enabled(enabled && counts.changed > 0, egui::Button::new(label))
                .on_hover_text(hover)
                .on_disabled_hover_text(hover)
                .clicked()
            {
                let ids = self.delete_section_ids(category, ids);
                self.accept_delete_suggestions(&ids);
            }
        });
    }

    /// Whether PhotoSort suggests keeping this photo over the other one of
    /// its card. It is marked beside the pill whatever the choice is now, so
    /// the suggestion stays visible after a change. Both formats of one shot
    /// are kept, so neither is marked.
    pub(super) fn delete_suggested_keeper(&self, id: MediaId) -> bool {
        self.delete_keeper_entry(id).is_some()
    }
    /// Why PhotoSort suggests keeping this photo, for its Suggested mark.
    pub(super) fn delete_keep_why(&self, id: MediaId) -> Option<String> {
        keep_why_text(self.delete_keeper_entry(id)?)
    }
    /// The card that suggests keeping this photo: the selected card when it
    /// does, as one photo can be the keeper of several cards (an identical
    /// pair and a closed-eyes shot, say), otherwise the first one.
    fn delete_keeper_entry(&self, id: MediaId) -> Option<&Entry> {
        let current = self
            .delete_current
            .and_then(|c| self.delete_review.entry(c));
        current.filter(|e| keeps_for(e, id)).or_else(|| {
            self.delete_review
                .entries()
                .iter()
                .find(|e| keeps_for(e, id))
        })
    }
}

/// Whether `entry` suggests keeping `id` over the other photo of its card.
fn keeps_for(entry: &Entry, id: MediaId) -> bool {
    entry.companion == Some(id) && entry.subject != id && entry.category != Category::Formats
}

/// The suggested keeper's star tooltip for `entry`: whose choice it is,
/// and why when the card says.
fn keep_why_text(entry: &Entry) -> Option<String> {
    Some(if entry.keep_why.is_empty() {
        SUGGESTED.to_owned()
    } else {
        format!("{SUGGESTED} {}", entry.keep_why)
    })
}

/// A card's title: its tag, only where it adds to the group's heading and
/// the Suggested mark, as in why a near-duplicate is kept, or that a
/// closed-eyes shot is kept for want of a better one.
fn card_title(entry: &Entry) -> String {
    if entry.category == Category::Near || entry.informational() {
        entry.tag.clone()
    } else {
        String::new()
    }
}

impl PhotoSortApp {
    /// Each photo's width over its height on `subject`'s card, in order.
    fn delete_card_aspects(&self, subject: MediaId) -> Vec<f32> {
        self.delete_review
            .entry(subject)
            .map(|entry| {
                entry
                    .candidates()
                    .iter()
                    .map(|id| self.photo_aspect(*id, 0))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether `subject`'s card shows a line of facts: exact copies in one
    /// folder have nothing that tells them apart.
    fn delete_card_has_facts(&self, subject: MediaId) -> bool {
        self.delete_review.entry(subject).is_some_and(|entry| {
            self.delete_facts(entry, &entry.candidates())
                .iter()
                .any(|facts| !facts.is_empty())
        })
    }

    /// The slot height over width a card's photos need.
    pub(super) fn delete_card_ratio(&self, subject: MediaId) -> f32 {
        let Some(entry) = self.delete_review.entry(subject) else {
            return 0.75;
        };
        entry
            .candidates()
            .iter()
            .map(|id| {
                self.metadata
                    .get(id)
                    .map_or(0.75, |m| slot_ratio(m.width, m.height, m.orientation))
            })
            .fold(0.0, f32::max)
    }

    /// The facts that tell a card's photos apart, the better value of each
    /// highlighted where "better" is unambiguous. A fact both photos share
    /// is left out: it cannot help to choose, and it would push the ones
    /// that can out of the line. The folder joins in when the copies live in
    /// different places, which for exact copies is often the only difference.
    /// A single photo shows its deciding fact first, then what it is.
    fn delete_facts(&self, entry: &Entry, ids: &[MediaId]) -> Vec<Vec<Fact>> {
        // Two photos or more, compared; or one photo alone.
        let pair = ids.len() >= 2;
        let media = |id: &MediaId| self.index.get(id).map(|i| &self.media[*i]);
        let meta = |id: &MediaId| self.metadata.get(id);
        let sharpness: Vec<Option<f64>> = ids
            .iter()
            .map(|id| {
                self.analysis
                    .get(id)
                    .map(|a| a.sharpness)
                    .filter(|v| v.is_finite())
            })
            .collect();
        let pixels: Vec<f64> = ids
            .iter()
            .map(|id| meta(id).map_or(0.0, |m| f64::from(m.width) * f64::from(m.height)))
            .collect();
        let bytes: Vec<u64> = ids.iter().map(|id| self.delete_bytes(*id)).collect();
        // Higher than every other photo's known value by more than `margin`.
        let higher = |values: &[f64], i: usize, margin: f64| {
            pair && (0..values.len())
                .filter(|j| *j != i)
                .all(|j| values[j] > 0.0 && values[i] > values[j] * (1.0 + margin))
        };
        // One row per kind of fact: its text for each photo, and which is better.
        let mut rows: Vec<(Vec<Option<String>>, Vec<bool>)> = Vec::new();
        let none = vec![false; ids.len()];
        let folder = ids
            .iter()
            .map(|id| media(id).map(|m| self.delete_folder(&m.path)))
            .collect();
        let format = ids
            .iter()
            .map(|id| {
                let path = &media(id)?.path;
                crate::core::near_duplicates::formats::Family::of(path)
                    .map(|f| f.title().to_owned())
                    .or_else(|| path.extension().map(|e| e.to_string_lossy().to_uppercase()))
            })
            .collect();
        let size = (
            ids.iter()
                .zip(&bytes)
                .map(|(id, b)| media(id).map(|_| size_text(*b)))
                .collect(),
            (0..ids.len())
                // Two formats of one shot: the smaller file saves space.
                .map(|i| {
                    entry.category == Category::Formats
                        && pair
                        && (0..ids.len()).all(|j| j == i || bytes[i] < bytes[j])
                })
                .collect(),
        );
        let resolution = (
            ids.iter()
                .map(|id| {
                    let m = meta(id).filter(|m| m.width > 0 && m.height > 0)?;
                    let (w, h) = if (5..=8).contains(&m.orientation) {
                        (m.height, m.width)
                    } else {
                        (m.width, m.height)
                    };
                    Some(format!("{w}×{h}"))
                })
                .collect(),
            (0..ids.len()).map(|i| higher(&pixels, i, 0.0)).collect(),
        );
        let sharp = (
            sharpness
                .iter()
                .map(|v| v.map(|v| format!("Sharpness {v:.0}")))
                .collect(),
            (0..ids.len())
                .map(|i| {
                    let values: Vec<f64> = sharpness.iter().map(|v| v.unwrap_or(0.0)).collect();
                    entry.category != Category::Formats && higher(&values, i, 0.05)
                })
                .collect(),
        );
        if pair {
            rows.push((folder, none.clone()));
            rows.push((format, none.clone()));
            rows.push(size);
            rows.push(resolution);
            rows.push(sharp);
            let times: Vec<Option<String>> = ids
                .iter()
                .map(|id| {
                    let ms = meta(id)?.captured_ms?;
                    let time = crate::core::dates::clock(ms)?;
                    Some(time.format("%H:%M:%S").to_string())
                })
                .collect();
            rows.push((times, none.clone()));
            rows.retain(|(texts, _)| texts.iter().any(|t| *t != texts[0]));
        } else {
            let brightness = ids
                .iter()
                .map(|id| {
                    let a = self.analysis.get(id)?;
                    Some(format!("Brightness {:.0}/255", a.brightness))
                })
                .collect();
            match entry.category {
                Category::Dark | Category::Bright => rows.push((brightness, none.clone())),
                _ => rows.push(sharp),
            }
            rows.push((format, none.clone()));
            rows.push(size);
            rows.push(resolution);
        }
        (0..ids.len())
            .map(|i| {
                rows.iter()
                    .filter_map(|(texts, better)| {
                        Some(Fact {
                            text: texts[i].clone()?,
                            better: better[i],
                        })
                    })
                    .collect()
            })
            .collect()
    }

    /// A photo's folder, relative to the opened folder.
    fn delete_folder(&self, path: &std::path::Path) -> String {
        let parent = path.parent().unwrap_or(path);
        match self
            .root
            .as_deref()
            .and_then(|root| parent.strip_prefix(root).ok())
        {
            Some(relative) if relative.as_os_str().is_empty() => self
                .root
                .as_deref()
                .and_then(|root| root.file_name())
                .map_or_else(
                    || parent.display().to_string(),
                    |n| n.to_string_lossy().into(),
                ),
            Some(relative) => relative
                .iter()
                .map(|c| c.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            None => parent.display().to_string(),
        }
    }

    fn delete_bytes(&self, id: MediaId) -> u64 {
        self.index
            .get(&id)
            .map_or(0, |i| self.media[*i].fingerprint.bytes)
    }

    /// The bytes PhotoSort's suggestions for these cards move to Trash.
    fn suggested_trash_bytes(&self, ids: &[MediaId]) -> u64 {
        ids.iter()
            .filter_map(|id| self.delete_review.entry(*id))
            .flat_map(|e| e.candidates().into_iter().zip(e.suggestion()))
            .filter(|(_, keep)| !keep)
            .map(|(id, _)| self.delete_bytes(id))
            .sum()
    }

    /// The bytes one format choice moves to Trash across these cards.
    fn format_trash_bytes(&self, ids: &[MediaId], choice: FormatChoice) -> u64 {
        ids.iter()
            .filter_map(|id| self.delete_review.entry(*id))
            .filter(|e| e.category == Category::Formats)
            .filter_map(|e| match choice {
                FormatChoice::Both => None,
                FormatChoice::Heic => Some(e.subject),
                FormatChoice::Jpeg => e.companion,
            })
            .map(|id| self.delete_bytes(id))
            .sum()
    }

    /// The card for one suggestion: a short tag (the full reason in its
    /// tooltip), its photos with a Keep/Trash pill each and their deciding
    /// facts, and Undo once it differs from the suggestion.
    fn delete_card(&mut self, entry: &Entry) -> ReviewCard {
        let subject = entry.subject;
        let enabled = self.delete_enabled();
        let leaders = entry.candidates();
        let mut facts = self.delete_facts(entry, &leaders);
        // A suggestion held back says why, first, where it is not cut off.
        for held in self
            .delete_review
            .held(subject, &self.grids.delete.last_kept)
        {
            if let Some(side) = leaders.iter().position(|id| *id == held) {
                facts[side].insert(
                    0,
                    Fact {
                        text: LAST_KEPT.into(),
                        better: false,
                    },
                );
            }
        }
        let mut insets: Vec<Option<Inset>> = leaders
            .iter()
            .map(|id| self.delete_eye_inset(entry, *id))
            .collect();
        let photos = leaders
            .iter()
            .zip(facts)
            .enumerate()
            .map(|(side, (id, facts))| {
                let keep = Review::keeps(&self.decisions, *id);
                let media = self.index.get(id).map(|i| &self.media[*i]);
                CardPhoto {
                    id: *id,
                    turns: self.preview_turns(*id),
                    badge: Badge::keep(keep),
                    // This card's own reason: its keeper may keep others too.
                    suggested: keeps_for(entry, *id),
                    suggested_hover: keeps_for(entry, *id)
                        .then(|| keep_why_text(entry))
                        .flatten(),
                    caption: media.map(|m| self.shown_name(m)).unwrap_or_default(),
                    name: media
                        .map(|m| m.path.display().to_string())
                        .unwrap_or_default(),
                    facts,
                    trash: !keep,
                    dimmed: self.trashed_elsewhere(*id),
                    toggle: enabled.then(|| {
                        format!(
                            "{} · {}",
                            if keep {
                                "Move to Trash instead"
                            } else {
                                "Keep instead"
                            },
                            side + 1
                        )
                    }),
                    inset: insets[side].take(),
                }
            })
            .collect();
        ReviewCard {
            title: card_title(entry),
            state_text: None,
            photos,
            buttons: Vec::new(),
            shown: self.delete_panel && self.delete_current == Some(subject),
            title_hover: Some(self.delete_reason(entry)),
            undo: (enabled && self.delete_review.changed(subject, &self.decisions))
                .then(|| (self.delete_undo_label().to_owned(), String::new())),
        }
    }

    /// The eyes of a closed-eyes card's photo, zoomed and cut sharp from the
    /// original: one strip per person, as many as the card names. Not on a
    /// photo shown turned: the crops follow the photo as it is stored.
    fn delete_eye_inset(&mut self, entry: &Entry, id: MediaId) -> Option<Inset> {
        let crops: Vec<_> = entry.eyes.iter().filter(|c| c.id == id).copied().collect();
        if crops.is_empty() || self.preview_turns(id) != 0 {
            return None;
        }
        let (w, h) = self.shown_size(id)?;
        let strips = crops
            .iter()
            .map(|crop| {
                let [left, top, right, bottom] = crop.rect;
                let uv = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom));
                let aspect = uv.width() * w as f32 / (uv.height() * h as f32).max(1.0);
                (self.sharp_region(id, uv, EYE_EDGE), aspect)
            })
            .collect();
        let closed = crops[0].closed;
        Some(Inset {
            strips,
            label: if closed { "Eyes closed" } else { "Eyes open" }.into(),
            closed,
        })
    }

    /// Applies a card's event to its suggestion, after making it the
    /// current suggestion so the keyboard follows the mouse.
    pub(super) fn delete_card_event(&mut self, subject: MediaId, event: CardEvent) {
        self.pending_bookmark = None;
        if self.delete_current != Some(subject) && self.delete_review.entry(subject).is_some() {
            self.set_active(subject);
        }
        if self.delete_current != Some(subject) {
            return;
        }
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                if let Some(id) = self
                    .delete_review
                    .entry(subject)
                    .and_then(|e| e.candidates().get(side).copied())
                {
                    self.active = Some(id);
                    self.focused_slot = side as u64;
                }
                // A click shows the card's photos large beside the grid.
                self.delete_panel = true;
            }
            CardEvent::Toggle(side) => self.toggle_delete_candidate(side),
            CardEvent::Undo => self.undo_delete(),
            CardEvent::Button(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(subject: u64, companion: Option<u64>, category: Category) -> Entry {
        Entry {
            subject: MediaId(subject),
            companion: companion.map(MediaId),
            category,
            tag: match category {
                Category::Exact => "Identical copy",
                Category::Formats => "JPEG + HEIC",
                Category::Near => "Sharper kept",
                _ => category.title(),
            }
            .into(),
            reason: format!("{} explained", category.title()),
            keep_why: if companion.is_some() && category != Category::Formats {
                "Why this one is kept".into()
            } else {
                String::new()
            },
            eyes: Vec::new(),
            extra: Vec::new(),
        }
    }
    /// An exact copy (1 + 2), a dark photo (3) and a JPEG + HEIC of one shot
    /// (4 + 5), with the suggestions staged as after an analysis.
    fn app() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        // These tests cover suggestions applied automatically.
        app.preferences.easy_delete_auto = true;
        app.preferences.pick_best_auto = true;
        app.preferences.rotation_auto = true;
        app.media = (1..=5)
            .map(|n| Media {
                id: MediaId(n),
                path: match n {
                    4 => "/shot.jpeg".into(),
                    5 => "/shot.heic".into(),
                    n => format!("/{n}.jpg").into(),
                },
                kind: MediaKind::Photo,
                fingerprint: Fingerprint {
                    bytes: n * 1_048_576,
                    ..Default::default()
                },
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.delete_review.replace(vec![
            entry(1, Some(2), Category::Exact),
            entry(4, Some(5), Category::Formats),
            entry(3, None, Category::Dark),
        ]);
        app.delete_review.stage_new_suggestions(&mut app.decisions);
        app.set_step(Step::EasyDelete);
        app.refilter();
        // Easy Delete opens with no card selected; most tests start on the first.
        assert_eq!(app.delete_current, None);
        app.set_active(MediaId(1));
        app
    }
    fn trashed(app: &PhotoSortApp, n: u64) -> bool {
        app.decision(MediaId(n)).rejected
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
            let _ = ctx.run_test(input, |ui| app.update_workspace(ui, false));
        }
    }
    #[test]
    fn suggestions_are_the_staged_decisions_and_survive_leaving_and_coming_back() {
        let mut app = app();
        // The copy and the dark photo go to Trash; both formats stay.
        assert!(trashed(&app, 1) && trashed(&app, 3));
        assert!(!trashed(&app, 2) && !trashed(&app, 4) && !trashed(&app, 5));
        assert_eq!(app.staged_change_count(), 2);
        // Nothing waits on a confirmation: moving between cards is free.
        app.navigate_selection(1, false, false);
        // The user keeps the copy after all.
        app.set_active(MediaId(1));
        app.toggle_delete_candidate(0);
        assert!(!trashed(&app, 1));
        // Leaving the step asks nothing and keeps every mark for Review.
        app.switch_step(Step::Cull);
        assert!(app.departure.is_none() && app.organize_departure.is_none());
        assert_eq!(app.step, Step::Cull);
        assert_eq!(app.staged_change_count(), 1);
        // Coming back never marks the kept photo again.
        app.switch_step(Step::EasyDelete);
        assert!(!trashed(&app, 1) && trashed(&app, 3));
        assert_eq!(app.staged_change_count(), 1);
    }
    #[test]
    fn a_finished_analysis_stages_its_suggestions_once() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        // These tests cover suggestions applied automatically.
        app.preferences.easy_delete_auto = true;
        app.preferences.pick_best_auto = true;
        app.preferences.rotation_auto = true;
        app.media = super::tests::app().media;
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        for media in &app.media {
            app.analysis.insert(media.id, Default::default());
        }
        app.set_step(Step::EasyDelete);
        let exact = crate::core::near_duplicates::batch::Pair {
            discard: MediaId(2),
            keep: MediaId(1),
            kind: crate::core::near_duplicates::batch::Kind::Exact,
            keeper: crate::core::near_duplicates::batch::Keeper::First,
            reason: "Byte-for-byte identical".into(),
            assessment: crate::core::near_duplicates::Comparator::default().assess(
                None,
                None,
                true,
                |_| None,
                || false,
            ),
        };
        app.delete_pairs = vec![exact];
        app.finish_delete_analysis();
        assert!(trashed(&app, 2) && !trashed(&app, 1));
        // The grid and the Review button show what was found and staged.
        assert_eq!(app.status, "");
        // The user keeps it; the same result arriving again leaves it kept.
        app.delete_current = Some(MediaId(2));
        app.toggle_delete_candidate(0);
        assert!(!trashed(&app, 2));
        app.finish_delete_analysis();
        assert!(!trashed(&app, 2));
        // And the session remembers it for the next launch.
        assert!(app.session_snapshot().suggested.contains(&MediaId(2)));
    }
    #[test]
    fn discarding_staged_choices_brings_the_suggestions_back_next_time() {
        let mut app = app();
        app.view_data_ready.insert(Step::EasyDelete);
        app.toggle_delete_candidate(0);
        assert!(!trashed(&app, 1));
        // Leaving with Discard clears every staged choice, the user's too.
        app.discard_staged_choices();
        assert!(app.decisions.values().all(|d| !d.rejected));
        app.set_step(Step::Cull);
        app.set_step(Step::EasyDelete);
        assert!(trashed(&app, 1) && trashed(&app, 3));
        assert!(!trashed(&app, 2) && !trashed(&app, 4) && !trashed(&app, 5));
        // A change survives leaving and coming back.
        app.delete_current = Some(MediaId(3));
        app.toggle_delete_candidate(0);
        app.set_step(Step::Cull);
        app.set_step(Step::EasyDelete);
        assert!(!trashed(&app, 3));
        // So does clearing every Trash mark: back on the next visit.
        app.root = Some("/".into());
        app.catalog = Some(Default::default());
        app.clear_all_trash_marks();
        assert!(!trashed(&app, 1));
        app.set_step(Step::Cull);
        app.set_step(Step::EasyDelete);
        assert!(trashed(&app, 1) && trashed(&app, 3));
    }
    #[test]
    fn with_automatic_suggestions_off_nothing_is_staged_until_accepted() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.preferences.easy_delete_auto = false;
        app.media = super::tests::app().media;
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.delete_review.replace(vec![
            entry(1, Some(2), Category::Exact),
            entry(4, Some(5), Category::Formats),
            entry(3, None, Category::Dark),
        ]);
        app.view_data_ready.insert(Step::EasyDelete);
        app.set_step(Step::EasyDelete);
        app.refilter();
        app.set_active(MediaId(1));
        assert!(app.decisions.values().all(|d| !d.rejected));
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(shown.contains("Use suggestion"), "{shown}");
        assert!(!shown.contains("Undo"), "{shown}");
        // One card at a time, or all at once.
        app.undo_delete();
        assert!(trashed(&app, 1));
        app.accept_delete_defaults();
        assert!(trashed(&app, 3));
        // Turning the setting on stages what is left at once.
        app.delete_current = Some(MediaId(3));
        app.toggle_delete_candidate(0);
        let mut on = app.preferences.clone();
        on.easy_delete_auto = true;
        app.save_settings(on);
        assert!(trashed(&app, 1));
    }
    #[test]
    fn large_cards_on_a_retina_screen_ask_for_the_display_preview() {
        let mut app = app();
        // Clicks and preview sizes below assume photos 160 points tall.
        app.preferences.photo_size = 160.0;
        app.review = false;
        let draw = |app: &mut PhotoSortApp, scale: f32| {
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.set_pixels_per_point(scale);
            let _ = ctx.run_test(
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
        };
        // At 1x a card's slot is about the size of the grid thumbnail.
        draw(&mut app, 1.0);
        assert!(app.requested.contains(&(MediaId(1), false)));
        assert!(!app.requested.contains(&(MediaId(1), true)));
        // At 2x it is twice as many pixels: the display preview is asked for.
        draw(&mut app, 2.0);
        assert!(app.requested.contains(&(MediaId(1), true)));
    }
    #[test]
    fn the_suggested_keeper_is_marked_whatever_the_choice() {
        let mut app = app();
        assert!(app.delete_suggested_keeper(MediaId(2)));
        assert!(!app.delete_suggested_keeper(MediaId(1)));
        // Both formats are kept, so neither is singled out.
        assert!(!app.delete_suggested_keeper(MediaId(4)));
        assert!(!app.delete_suggested_keeper(MediaId(5)));
        // Its tooltip says why.
        assert_eq!(
            app.delete_keep_why(MediaId(2)).as_deref(),
            Some("Suggested by PhotoSort. Why this one is kept")
        );
        assert_eq!(app.delete_keep_why(MediaId(1)), None);
        let entry = app.delete_review.entry(MediaId(1)).unwrap().clone();
        let card = app.delete_card(&entry);
        assert_eq!(
            card.photos[1].suggested_hover,
            app.delete_keep_why(MediaId(2))
        );
        assert_eq!(card.photos[0].suggested_hover, None);
        // One badge says the choice; a star beside it, whose it was.
        let star = egui_phosphor::regular::STAR;
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches(star).count(), 1, "{shown}");
        assert!(!shown.contains("Suggested"), "{shown}");
        // Moving the keeper to Trash by hand keeps the mark.
        app.delete_card_event(MediaId(1), CardEvent::Toggle(1));
        app.delete_panel = false;
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches(star).count(), 1, "{shown}");
    }
    #[test]
    fn changing_a_card_stages_it_at_once_and_undo_restores_the_suggestion() {
        let mut app = app();
        assert_eq!(app.delete_current, Some(MediaId(1)));
        app.toggle_delete_candidate(0);
        assert!(!trashed(&app, 1));
        assert!(app.delete_review.changed(MediaId(1), &app.decisions));
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(shown.contains("Undo"), "{shown}");
        app.undo_delete();
        assert!(trashed(&app, 1));
        assert!(!app.delete_review.changed(MediaId(1), &app.decisions));
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains("Undo"), "{shown}");
    }
    #[test]
    fn accept_all_fills_in_what_is_undecided_and_never_changes_a_choice_made_by_hand() {
        let mut app = app();
        // Nothing decided yet for the copy and the dark photo, as when
        // suggestions are not applied on their own.
        for id in [1, 3] {
            app.decisions.remove(&MediaId(id));
        }
        assert!(!trashed(&app, 1) && !trashed(&app, 3));
        // "Trash all N exact copies" is the Exact group's suggestion only.
        app.accept_delete_suggestions(&[MediaId(1)]);
        assert!(trashed(&app, 1) && !trashed(&app, 3));
        // One choice for every JPEG + HEIC pair, made by hand.
        app.choose_delete_formats(&app.delete_review_ids(), FormatChoice::Heic);
        assert!(trashed(&app, 4) && !trashed(&app, 5));
        assert_eq!(
            app.delete_review
                .format_choice(&app.delete_review_ids(), &app.decisions),
            Some(FormatChoice::Heic)
        );
        // By hand: the copy is kept and the photo it copies goes instead.
        app.delete_current = Some(MediaId(1));
        app.toggle_delete_candidate(0);
        app.toggle_delete_candidate(1);
        assert!(!trashed(&app, 1) && trashed(&app, 2));
        // Accept all suggestions marks the dark photo, which nobody decided,
        // and leaves both choices made by hand: the pair's and the formats'.
        app.accept_delete_defaults();
        assert!(trashed(&app, 3));
        assert!(!trashed(&app, 1) && trashed(&app, 2));
        assert!(trashed(&app, 4) && !trashed(&app, 5));
        assert_eq!(app.status, "Accepted 1 suggestion");
        // The card's own Undo is asked for by hand: its suggestion in full.
        app.undo_delete();
        assert!(trashed(&app, 1) && !trashed(&app, 2));
        // The copy undecided again and the photo it copies marked by hand:
        // the suggestion would send the pair's last photo to Trash, so it
        // is skipped and the copy stays undecided.
        app.decisions.remove(&MediaId(1));
        app.toggle_delete_candidate(1);
        app.accept_delete_defaults();
        assert!(!trashed(&app, 1) && trashed(&app, 2));
        // Accept all has nothing left to do, and says so.
        assert!(app.status.starts_with("No suggestion left to accept"));
        app.delete_deck();
        assert_eq!(app.grids.delete.counts.changed, 0);
    }
    #[test]
    fn suggestions_hold_back_the_last_photo_kept_of_a_pick_best_group_and_say_so() {
        use super::super::review_card::render_grid;
        let mut app = app();
        // Pick Best keeps the dark photo 3 over 1, which is going to Trash.
        app.delete_current = Some(MediaId(3));
        app.toggle_delete_candidate(0);
        app.pick_review
            .replace(vec![crate::core::pick_review::Cluster::new(
                1,
                vec![MediaId(1), MediaId(3)],
                HashMap::from([(MediaId(1), 10.), (MediaId(3), 20.)]),
            )]);
        // Staged on their own or accepted all at once, suggestions leave it.
        app.delete_review.restore_suggested(Default::default());
        app.stage_delete_suggestions();
        app.accept_delete_defaults();
        assert!(trashed(&app, 1) && !trashed(&app, 3));
        // Its card says why, and Accept all has nothing left to do.
        let shown = render_grid(&mut app);
        assert!(shown.contains(LAST_KEPT), "{shown}");
        assert_eq!(app.grids.delete.counts.changed, 0);
        // Marked by hand, it goes, and the note with it.
        app.toggle_delete_candidate(0);
        assert!(trashed(&app, 3));
        assert!(!render_grid(&mut app).contains(LAST_KEPT));
    }
    #[test]
    fn eyes_closed_cards_show_the_eyes_zoomed_and_only_trash_shots_with_a_better_one() {
        let mut app = app();
        for n in 6..=10 {
            app.media.push(Media {
                id: MediaId(n),
                path: format!("/{n}.jpg").into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint {
                    bytes: n * 1_048_576,
                    ..Default::default()
                },
            });
            app.index.insert(MediaId(n), app.media.len() - 1);
            app.metadata.insert(
                MediaId(n),
                crate::core::model::Metadata {
                    width: 4000,
                    height: 3000,
                    ..Default::default()
                },
            );
        }
        let crop = |n, closed| delete_review::EyeCrop {
            id: MediaId(n),
            rect: [0.4, 0.4, 0.6, 0.45],
            closed,
        };
        let mut entries = app.delete_review.entries().to_vec();
        // 7 is the better shot and also the keeper of an identical pair.
        entries.push(entry(9, Some(7), Category::Exact));
        // 10 is an identical copy of 6: both go to Trash, each on the card.
        entries.push(Entry {
            tag: "Keep the shot with eyes open".into(),
            keep_why: "Everyone's eyes are open here.".into(),
            eyes: vec![crop(6, true), crop(6, true), crop(7, false)],
            extra: vec![MediaId(10)],
            ..entry(6, Some(7), Category::EyesClosed)
        });
        app.delete_pairs
            .push(crate::core::near_duplicates::batch::Pair {
                discard: MediaId(10),
                keep: MediaId(6),
                kind: crate::core::near_duplicates::batch::Kind::Exact,
                keeper: crate::core::near_duplicates::batch::Keeper::First,
                assessment: crate::core::near_duplicates::Comparator::default().assess(
                    None,
                    None,
                    true,
                    |_| None,
                    || false,
                ),
                reason: String::new(),
            });
        entries.push(Entry {
            tag: "No shot with eyes open - kept".into(),
            eyes: vec![crop(8, true)],
            ..entry(8, None, Category::EyesClosed)
        });
        app.delete_review.replace(entries);
        app.delete_review.stage_new_suggestions(&mut app.decisions);
        assert!(trashed(&app, 6) && trashed(&app, 10) && !trashed(&app, 7) && !trashed(&app, 8));
        app.set_delete_tab(Some(Category::EyesClosed));
        app.refilter();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "No shot with eyes open - kept",
            "Eyes open",
            "Eyes closed",
            "10.jpg",
            // Photos, not cards; the note is kept, so it is not counted.
            "Accept all · frees 16.0 MB",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        // The heading says what the cards with a better shot are.
        assert!(!shown.contains("Keep the shot with eyes open"), "{shown}");
        // The zoomed eyes are cut sharp from the originals, off the UI thread.
        let wanted: Vec<_> = app.board.people.wanted.iter().map(|r| r.media.id).collect();
        for n in [6, 7, 8] {
            assert!(
                wanted.contains(&MediaId(n)),
                "{n} not requested: {wanted:?}"
            );
        }
        // Each card explains its own keeper, though 7 keeps two cards.
        let eyes = app.delete_review.entry(MediaId(6)).unwrap().clone();
        let card = app.delete_card(&eyes);
        assert_eq!(
            card.photos[2].suggested_hover.as_deref(),
            Some("Suggested by PhotoSort. Everyone's eyes are open here.")
        );
        let copy = app.delete_review.entry(MediaId(9)).unwrap().clone();
        assert_eq!(
            app.delete_card(&copy).photos[1].suggested_hover.as_deref(),
            Some("Suggested by PhotoSort. Why this one is kept")
        );
        app.set_active(MediaId(6));
        assert_eq!(
            app.delete_keep_why(MediaId(7)).as_deref(),
            Some("Suggested by PhotoSort. Everyone's eyes are open here.")
        );
        // Every photo of the card is shown: both closed-eyes shots, then the
        // better one on the right, with two people's eyes on the first.
        let card = app.delete_card(&eyes);
        let shown: Vec<_> = card.photos.iter().map(|p| p.id.0).collect();
        assert_eq!(shown, vec![6, 10, 7]);
        let strips: Vec<_> = card
            .photos
            .iter()
            .map(|p| p.inset.as_ref().map_or(0, |i| i.strips.len()))
            .collect();
        assert_eq!(strips, vec![2, 0, 1]);
        // Each pill switches its own photo; the panel shows all three.
        app.delete_card_event(MediaId(6), CardEvent::Toggle(1));
        assert!(trashed(&app, 6) && !trashed(&app, 10));
        assert_eq!(app.comparison_ids().len(), 3);
        // Every card shown: the summary counts 7 once, though it keeps two.
        app.set_delete_tab(None);
        app.refilter();
        let cards = app.delete_review_ids();
        assert!(cards.contains(&MediaId(9)) && cards.contains(&MediaId(6)));
        let candidates = app.delete_candidates(&cards);
        assert_eq!(candidates.iter().filter(|id| id.0 == 7).count(), 1);
    }
    #[test]
    fn grid_shows_short_tags_facts_and_group_actions_without_confirm() {
        let mut app = app();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "Too dark",
            "Exact copies",
            "Same photo, two formats",
            "Keep both",
            "Keep HEIC",
            "Keep JPEG",
            "Accept all · frees 1.0 MB",
            "Accept all · frees 3.0 MB",
            "Keep HEIC · frees 4.0 MB",
            "Keep JPEG · frees 5.0 MB",
            "Accept all suggestions",
            "HEIC",
            "2 photos going to Trash · frees 4.0 MB",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        // A card under a heading that says what it is has no title of its own.
        for gone in [
            "Provisional",
            "Confirm",
            "explained",
            "1 Trash",
            "2 Keep",
            "Identical copy",
            "JPEG + HEIC",
            "For all of them",
        ] {
            assert!(!shown.contains(gone), "{gone} still shown:\n{shown}");
        }
        assert_eq!(
            app.staged_change_count(),
            2,
            "Drawing the grid changed a decision"
        );
        // A tab shows one group.
        app.set_delete_tab(Some(Category::Dark));
        app.refilter();
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(shown.contains("Too dark") && !shown.contains("Same photo, two formats"));
    }
    #[test]
    fn cards_show_only_what_differs_and_the_folder_when_copies_live_apart() {
        let mut app = app();
        app.root = Some("/library".into());
        for (n, path) in [
            (1, "/library/2025/a.jpg"),
            (2, "/library/2025/sorted/a.jpg"),
        ] {
            let media = &mut app.media[n - 1];
            media.path = path.into();
            media.fingerprint.bytes = 1_048_576;
        }
        let same = Metadata {
            width: 4000,
            height: 3000,
            captured_ms: Some(1_700_000_000_000),
            ..Default::default()
        };
        for n in 1..=5 {
            app.metadata.insert(MediaId(n), same.clone());
        }
        let text = |facts: &[Fact]| facts.iter().map(|f| f.text.clone()).collect::<Vec<_>>();
        // Identical files: only where each one lives.
        let exact = app.delete_review.entry(MediaId(1)).unwrap().clone();
        let facts = app.delete_facts(&exact, &exact.candidates());
        assert_eq!(text(&facts[0]), vec!["2025"]);
        assert_eq!(text(&facts[1]), vec!["2025/sorted"]);
        // Two formats in one folder: format and size, the smaller highlighted;
        // the shared resolution and capture time are left out.
        let formats = app.delete_review.entry(MediaId(4)).unwrap().clone();
        let facts = app.delete_facts(&formats, &formats.candidates());
        assert_eq!(text(&facts[0]), vec!["JPEG", "4.0 MB"]);
        assert_eq!(text(&facts[1]), vec!["HEIC", "5.0 MB"]);
        assert!(facts[0][1].better && !facts[1][1].better);
        // Copies taken a second apart show the capture clock, as Details does.
        let noon = crate::core::dates::timestamp("2024:07:01 12:34:56").unwrap();
        for (n, ms) in [(1, noon), (2, noon + 1000)] {
            app.metadata.get_mut(&MediaId(n)).unwrap().captured_ms = Some(ms);
        }
        let facts = app.delete_facts(&exact, &exact.candidates());
        assert_eq!(text(&facts[0]), vec!["2025", "12:34:56"]);
        assert_eq!(text(&facts[1]), vec!["2025/sorted", "12:34:57"]);
        // A single photo keeps its deciding fact first, then what it is.
        app.analysis.insert(
            MediaId(3),
            crate::core::analysis::Analysis {
                brightness: 9.0,
                ..Default::default()
            },
        );
        let dark = app.delete_review.entry(MediaId(3)).unwrap().clone();
        assert_eq!(
            text(&app.delete_facts(&dark, &dark.candidates())[0]),
            vec!["Brightness 9/255", "JPEG", "3.0 MB", "4000×3000"]
        );
    }
    #[test]
    fn a_cards_facts_and_the_counts_follow_what_arrives_and_what_is_decided() {
        use super::super::review_card::render_grid;
        let mut app = app();
        // Identical copies of one size in one folder: nothing tells them
        // apart, so their card has no line of facts.
        app.media[1].fingerprint.bytes = app.media[0].fingerprint.bytes;
        let shown = render_grid(&mut app);
        assert!(shown.contains("2 photos going to Trash · frees 4.0 MB"));
        assert!(!shown.contains("Sharpness"));
        // An analysis tells them apart. The grid does not watch it: the
        // card on screen notices, and its row has the line a frame later.
        for (n, sharpness) in [(1, 120.0), (2, 80.0)] {
            app.analysis.insert(
                MediaId(n),
                Analysis {
                    sharpness,
                    ..Default::default()
                },
            );
        }
        render_grid(&mut app);
        let shown = render_grid(&mut app);
        assert!(shown.contains("Sharpness 120") && shown.contains("Sharpness 80"));
        // Their sizes arrive: the card says so in the same frame.
        for (n, width) in [(1, 4000), (2, 2000)] {
            app.metadata.insert(
                MediaId(n),
                Metadata {
                    width,
                    height: width * 3 / 4,
                    ..Default::default()
                },
            );
        }
        let shown = render_grid(&mut app);
        assert!(shown.contains("4000×3000") && shown.contains("2000×1500"));
        // A decision made anywhere is counted in the next frame.
        app.decisions.entry(MediaId(5)).or_default().rejected = true;
        let shown = render_grid(&mut app);
        assert!(shown.contains("3 photos going to Trash · frees 9.0 MB"));
    }
    #[test]
    fn saved_groups_other_than_one_tab_open_on_all_and_tabs_select_one_group() {
        let mut app = app();
        let several: HashSet<_> = [Category::Exact, Category::Dark].into_iter().collect();
        app.delete_categories = app.delete_tab_categories(Some(&several));
        assert_eq!(app.delete_tab(), None);
        app.refilter();
        assert_eq!(app.delete_review_ids().len(), 3);
        let formats: HashSet<_> = [Category::Formats].into_iter().collect();
        app.delete_categories = app.delete_tab_categories(Some(&formats));
        assert_eq!(app.delete_tab(), Some(Category::Formats));
        app.filter_dirty = true;
        app.refilter();
        assert_eq!(app.delete_review_ids(), vec![MediaId(4)]);
        // A group with no suggestions left falls back to All.
        let blur: HashSet<_> = [Category::Blur].into_iter().collect();
        assert_eq!(
            app.delete_tab_categories(Some(&blur)).len(),
            Category::ALL.len()
        );
    }
    #[test]
    fn a_trash_filter_never_hides_cards_whose_suggestion_is_staged() {
        let mut app = app();
        app.filter = crate::core::browsing::Filter::Unmarked;
        app.filter_dirty = true;
        app.refilter();
        assert_eq!(app.delete_review_ids().len(), 3);
    }
    #[test]
    fn card_pills_and_undo_act_on_their_card() {
        let mut app = app();
        // The dark photo's pill keeps it; its card becomes the current one.
        app.delete_card_event(MediaId(3), CardEvent::Toggle(0));
        assert_eq!(app.delete_current, Some(MediaId(3)));
        assert!(!trashed(&app, 3));
        // The HEIC of the pair goes to Trash from its own pill.
        app.delete_card_event(MediaId(4), CardEvent::Toggle(1));
        assert!(trashed(&app, 5) && !trashed(&app, 4));
        app.delete_card_event(MediaId(3), CardEvent::Undo);
        assert!(trashed(&app, 3));
        // Pills act on their card without opening the panel.
        assert!(!app.delete_panel);
        // Clicking a card's second photo shows the card large beside the grid.
        app.delete_card_event(MediaId(1), CardEvent::Select(1));
        assert!(app.delete_panel);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert_eq!(app.active, Some(MediaId(2)));
        assert_eq!(app.comparison_ids(), vec![MediaId(1), MediaId(2)]);
    }
    #[test]
    fn easy_delete_is_one_grid_with_the_clicked_card_in_a_side_panel() {
        let mut app = app();
        // Entering the step, or a saved view, never opens a separate Review.
        assert!(app.review);
        key(&mut app, egui::Key::F19, egui::Modifiers::NONE);
        assert!(!app.review);
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Identical copy").count(), 0, "{shown}");
        // The card's photos open beside the grid under its tag, with Undo;
        // the reason stays in the tag's tooltip.
        app.delete_card_event(MediaId(1), CardEvent::Select(0));
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Identical copy").count(), 1, "{shown}");
        assert!(
            shown.contains("Undo") && !shown.contains("explained"),
            "{shown}"
        );
        // Esc closes it; the grid stays.
        key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.delete_panel);
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Identical copy").count(), 0, "{shown}");
    }
    #[test]
    fn the_size_slider_sets_how_many_cards_fit_at_every_window_width() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        // These tests cover suggestions applied automatically.
        app.preferences.easy_delete_auto = true;
        app.preferences.pick_best_auto = true;
        app.preferences.rotation_auto = true;
        app.media = (1..=12)
            .map(|n| Media {
                id: MediaId(n),
                path: format!("/{n}.jpg").into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.delete_review.replace(
            (0..6)
                .map(|i| entry(2 * i + 1, Some(2 * i + 2), Category::Near))
                .collect(),
        );
        app.view_data_ready.insert(Step::EasyDelete);
        app.set_step(Step::EasyDelete);
        app.refilter();
        app.review = false;
        // The x of every card title in the first row, at a window width.
        let titles = |app: &mut PhotoSortApp, width: f32| {
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let output = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.library(ui));
                },
            );
            let mut titles = Vec::new();
            fn walk(shape: &egui::Shape, titles: &mut Vec<egui::Pos2>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == "Sharper kept" => {
                        titles.push(text.pos)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, titles)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut titles);
            }
            let text = super::super::review_card::painted_text(&output);
            // The cards of the first row.
            let top = titles.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let mut xs: Vec<f32> = titles.iter().filter(|p| p.y == top).map(|p| p.x).collect();
            xs.sort_by(f32::total_cmp);
            xs.dedup();
            (xs, text)
        };
        // The default fits five pairs of 4:3 photos in a wide window.
        let (columns, text) = titles(&mut app, 1600.0);
        assert_eq!(columns.len(), 5, "{columns:?}");
        assert!(text.contains("Accept all suggestions"), "{text}");
        // Smallest: more cards in a row.
        let sizes = crate::core::preferences::PHOTO_SIZES;
        app.preferences.photo_size = *sizes.start();
        assert_eq!(titles(&mut app, 1600.0).0.len(), 6);
        // Largest: one card a row, still inside the window, even narrow.
        app.preferences.photo_size = *sizes.end();
        for width in [1600.0, 700.0] {
            let (columns, text) = titles(&mut app, width);
            assert_eq!(columns.len(), 1, "at {width}: {columns:?}");
            assert!(columns[0] < width, "at {width}");
            assert!(text.contains("Accept all suggestions"), "at {width}");
        }
        // The size survives leaving the step, and the panel and keys work.
        app.set_step(Step::Cull);
        app.set_step(Step::EasyDelete);
        assert_eq!(app.preferences.photo_size, *sizes.end());
        app.delete_card_event(MediaId(3), CardEvent::Select(0));
        let (_, text) = titles(&mut app, 1200.0);
        assert!(text.matches("Sharper kept").count() >= 2, "{text}");
        // Its suggestion was staged on entry; 1 switches the copy to Keep.
        assert!(app.decision(MediaId(3)).rejected);
        key(&mut app, egui::Key::Num1, egui::Modifiers::NONE);
        assert!(!app.decision(MediaId(3)).rejected);
        key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(5)));
        assert!(app.delete_panel);
    }
    #[test]
    fn the_panel_slides_over_the_grid_without_reflowing_it_every_frame() {
        let mut app = app();
        // A near-duplicate card keeps its tag as its title.
        app.delete_review.replace(vec![
            entry(1, Some(2), Category::Near),
            entry(3, None, Category::Dark),
        ]);
        app.refilter();
        app.review = false;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.global_style_mut(|style| style.animation_time = 1.0);
        // Where each "Sharper kept" title was painted: the card's first,
        // then the panel's.
        let titles = |app: &mut PhotoSortApp, time: f64| {
            let output = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    time: Some(time),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.library(ui));
                },
            );
            let mut found = Vec::new();
            fn walk(shape: &egui::Shape, found: &mut Vec<egui::Pos2>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == "Sharper kept" => {
                        found.push(text.pos)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, found)),
                    _ => {}
                }
            }
            for clipped in &output.shapes {
                walk(&clipped.shape, &mut found);
            }
            found
        };
        titles(&mut app, 0.0);
        app.delete_card_event(MediaId(1), CardEvent::Select(0));
        let early = titles(&mut app, 0.1);
        let later = titles(&mut app, 0.4);
        let done = titles(&mut app, 2.0);
        // The card stays where the grid put it as the panel opened...
        assert_eq!(early[0], later[0]);
        assert_eq!(later[0], done[0]);
        // ...while the panel's own title moves in from the right.
        assert_eq!((early.len(), later.len(), done.len()), (2, 2, 2));
        assert!(
            early[1].x > later[1].x && later[1].x > done[1].x,
            "{early:?} {later:?} {done:?}"
        );
    }
    #[test]
    fn easy_delete_opens_with_nothing_selected_and_no_card_outlined() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = super::tests::app().media;
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.delete_review.replace(vec![
            entry(1, Some(2), Category::Exact),
            entry(3, None, Category::Dark),
        ]);
        app.view_data_ready.insert(Step::EasyDelete);
        app.set_step(Step::EasyDelete);
        app.refilter();
        assert_eq!((app.delete_current, app.delete_panel), (None, false));
        // The keys that change a card wait until one is shown.
        let staged = app.decisions.clone();
        for k in [egui::Key::Num1, egui::Key::ArrowLeft, egui::Key::R] {
            key(&mut app, k, egui::Modifiers::NONE);
        }
        assert_eq!(app.decisions, staged);
        assert_eq!(app.delete_current, None);
        // Enter shows the first card in the panel.
        key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert!(app.delete_panel);
        let entry = app.delete_review.entry(MediaId(1)).unwrap().clone();
        // Only a soft highlight marks the card shown in the panel.
        assert!(app.delete_card(&entry).shown);
        app.delete_panel = false;
        assert!(!app.delete_card(&entry).shown);
    }
    #[test]
    fn a_click_on_the_grid_background_deselects_and_closes_the_panel() {
        let mut app = app();
        // Clicks and preview sizes below assume photos 160 points tall.
        app.preferences.photo_size = 160.0;
        app.delete_card_event(MediaId(4), CardEvent::Select(0));
        assert!(app.delete_panel);
        app.deselect_delete_card();
        assert!(!app.delete_panel);
        assert_eq!((app.delete_current, app.active), (None, None));
        // Nothing is picked again on its own, even as the grid refreshes,
        // and the keys that act on a card do nothing.
        app.accept_delete_defaults();
        app.refilter();
        assert_eq!(app.delete_current, None);
        let before = app.decisions.clone();
        key(&mut app, egui::Key::Num1, egui::Modifiers::NONE);
        assert_eq!(app.decisions, before);
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("JPEG + HEIC").count(), 0, "{shown}");
        // The same through a real click beside the first group's only card.
        app.delete_card_event(MediaId(1), CardEvent::Select(0));
        app.delete_panel = false;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        app.review = false;
        let click = |app: &mut PhotoSortApp, at: egui::Pos2| {
            for events in [
                vec![egui::Event::PointerMoved(at)],
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            ] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run_test(input, |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.library(ui));
                });
            }
        };
        click(&mut app, egui::pos2(900.0, 300.0));
        assert_eq!(app.delete_current, None);
        assert!(!app.delete_panel);
        // A click on a photo switches it between Keep and Trash, and leaves
        // the panel closed.
        assert!(trashed(&app, 1));
        click(&mut app, egui::pos2(150.0, 200.0));
        assert!(!trashed(&app, 1));
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert!(!app.delete_panel);
        // A click on the card around the photos, here on the second
        // photo's name under it, opens the panel.
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
        fn find(shape: &egui::Shape, name: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == name => {
                    Some(text.visual_bounding_rect().center())
                }
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, name)),
                _ => None,
            }
        }
        let caption = output
            .shapes
            .iter()
            .find_map(|clipped| find(&clipped.shape, "2.jpg"))
            .expect("the second photo's name");
        click(&mut app, caption);
        assert!(app.delete_panel);
        assert!(!trashed(&app, 1), "a card click changed a photo");
        app.deselect_delete_card();
        // An arrow key selects the first card again and shows it.
        key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert!(app.delete_panel);
    }

    #[test]
    fn keyboard_decides_cards_switches_photos_moves_on_with_enter_and_undoes_with_r() {
        let mut app = app();
        app.delete_card_event(MediaId(1), CardEvent::Select(0));
        app.review = false;
        let none = egui::Modifiers::NONE;
        // Left keeps the pair and Right sends only its copy to Trash,
        // however often either is pressed.
        for _ in 0..2 {
            key(&mut app, egui::Key::ArrowLeft, none);
            assert!(!trashed(&app, 1) && !trashed(&app, 2), "Left keeps both");
        }
        for _ in 0..2 {
            key(&mut app, egui::Key::ArrowRight, none);
            assert!(
                trashed(&app, 1) && !trashed(&app, 2),
                "Right spares the keeper"
            );
        }
        // A held key decides once: its repeats never switch the photo back.
        hold(&mut app, egui::Key::Num2, none, 3);
        assert!(trashed(&app, 2), "2 switches the second photo once");
        key(&mut app, egui::Key::ArrowRight, none);
        assert!(
            trashed(&app, 1) && !trashed(&app, 2),
            "Right keeps the keeper"
        );
        key(&mut app, egui::Key::ArrowLeft, none);
        key(&mut app, egui::Key::R, none);
        assert!(trashed(&app, 1) && !trashed(&app, 2), "R undoes the card");
        key(&mut app, egui::Key::Enter, none);
        assert_eq!(app.delete_current, Some(MediaId(4)), "Enter moves on");
        // Two formats of one shot: Right keeps the HEIC.
        key(&mut app, egui::Key::ArrowRight, none);
        assert!(trashed(&app, 4) && !trashed(&app, 5));
        // A photo alone on its card: Left keeps it and Right marks it.
        key(&mut app, egui::Key::ArrowDown, none);
        assert_eq!(app.delete_current, Some(MediaId(3)));
        key(&mut app, egui::Key::ArrowLeft, none);
        assert!(!trashed(&app, 3));
        key(&mut app, egui::Key::ArrowRight, none);
        assert!(trashed(&app, 3));
        key(&mut app, egui::Key::ArrowUp, none);
        assert_eq!(app.delete_current, Some(MediaId(4)));
        key(&mut app, egui::Key::A, none);
        assert!(trashed(&app, 4), "A leaves a choice made by hand");
    }
    #[test]
    fn cancelled_duplicate_pass_preserves_review_and_rejects_late_pairs() {
        let mut app = app();
        let original = app.delete_review.entries().to_vec();
        let staged = app.decisions.clone();
        app.analysis_run = Some(41);
        app.delete_run = Some(41);
        app.cancel_analysis();
        assert_eq!(app.delete_review.entries(), original);
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        app.runtime.events = rx;
        for event in [
            Event::DuplicatePairs {
                run: 41,
                pairs: Vec::new(),
                blinks: Vec::new(),
            },
            Event::AnalysisDone {
                run: 41,
                stats: Default::default(),
            },
        ] {
            tx.send(crate::core::jobs::Envelope {
                epoch: app.runtime.epoch(),
                event,
            })
            .unwrap();
        }
        app.poll(&egui::Context::default());
        assert_eq!(app.delete_review.entries(), original);
        assert_eq!(app.decisions, staged);
    }
    #[test]
    fn quitting_with_staged_suggestions_asks_to_review_them() {
        let mut app = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        app.request_departure(Destination::Close);
        assert!(matches!(app.departure, Some(Destination::Close)));
        assert!(!app.close_authorized);
        let _ = ctx.run_test(egui::RawInput::default(), |ui| {
            app.update_workspace(ui, true)
        });
        assert!(!app.close_authorized);
    }
}
