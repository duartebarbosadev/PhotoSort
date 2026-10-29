use super::review_card::{
    Badge, CardEvent, CardPhoto, CardState, Fact, GridClick, Inset, ReviewCard, size_text,
    slot_ratio,
};

/// The long side, in pixels, a closed-eyes card's zoomed eyes are cut at.
const EYE_EDGE: f32 = 480.0;

use super::*;
use crate::core::delete_review::{self, Category, Entry, FormatChoice, Review};

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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
        self.view_data_ready.insert(Step::EasyDelete);
        self.stage_delete_suggestions();
        let saved = self
            .pending_bookmark
            .as_ref()
            .map(|v| v.delete_categories.iter().copied().collect());
        self.delete_categories = self.delete_tab_categories(saved.as_ref());
        self.delete_run = None;
        self.filter_dirty = true;
        let trash = self
            .delete_review
            .entries()
            .iter()
            .flat_map(Entry::candidates)
            .filter(|id| self.decision(*id).rejected)
            .count();
        // A closed-eyes shot with no better one is kept: a note, not a suggestion.
        let notes = self
            .delete_review
            .entries()
            .iter()
            .filter(|e| e.informational())
            .count();
        let kept = if notes > 0 {
            format!(
                " · {} kept with no better shot",
                count(notes, "closed-eyes photo", "closed-eyes photos")
            )
        } else {
            String::new()
        };
        self.set_workflow_status(
            Step::EasyDelete,
            match self.delete_review.entries().len() - notes {
                0 if notes == 0 => {
                    "Nothing to clean up · no duplicates or problem shots found".to_owned()
                }
                n => format!(
                    "{} · {} marked for Trash{kept} · nothing changes on disk until you apply Review changes",
                    count(n, "suggestion", "suggestions"),
                    count(trash, "photo", "photos")
                ),
            } + &self.failure_note(),
        );
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
            .stage_new_suggestions(&mut self.decisions);
        if !marked.is_empty() {
            self.changed_items(marked);
        }
    }
    /// The categories a tab shows: one group, or every group ("All"). A saved
    /// set of several groups, from before tabs, opens on All.
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
            self.active = Some(candidate);
            self.focused_slot = side as u64;
            self.changed_items([candidate]);
        }
    }
    /// Puts cards back to PhotoSort's suggestion.
    fn accept_delete_suggestions(&mut self, ids: &[MediaId]) {
        if !self.delete_enabled() {
            return;
        }
        self.delete_review.accept(ids, &mut self.decisions);
        self.changed_items(self.delete_candidates(ids));
    }
    fn delete_candidates(&self, ids: &[MediaId]) -> Vec<MediaId> {
        ids.iter()
            .filter_map(|id| self.delete_review.entry(*id))
            .flat_map(Entry::candidates)
            .collect()
    }
    /// Undoes the user's changes to the current card.
    pub(super) fn undo_delete(&mut self) {
        if let Some(id) = self.delete_current {
            self.accept_delete_suggestions(&[id]);
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
                    hover: Some(match tab {
                        None => "Every suggestion, grouped".to_owned(),
                        Some(c) => format!("Only {}", c.heading().to_lowercase()),
                    }),
                }
            })
            .collect();
        let selected = tabs.iter().position(|t| *t == self.delete_tab());
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
                .on_hover_text(&entry.reason);
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
                .on_hover_text("Back to PhotoSort's suggestion · R")
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
        let subjects = self.delete_review_ids();
        self.delete_summary(ui, &subjects);
        ui.add_space(S2);
        // The grid takes its final width at once and the panel slides over
        // the room it leaves, in or out: one reflow per open or close, never
        // one per frame of the slide. The grid keeps room for one card.
        let area = ui.available_rect_before_wrap();
        let shown = self.delete_panel && self.delete_current.is_some();
        let slide = ui
            .ctx()
            .animate_bool_responsive(egui::Id::new("delete_panel_slide"), shown);
        let widest = (area.width() - 460.0).max(360.0);
        self.delete_panel_width = self
            .delete_panel_width
            .unwrap_or(area.width() * 0.45)
            .clamp(360.0, widest)
            .into();
        let panel_width = self.delete_panel_width.unwrap_or(360.0);
        let grid = if shown {
            area.with_max_x(area.right() - panel_width - S3)
        } else {
            area
        };
        let mut sections: Vec<(Category, usize)> = Vec::new();
        for id in &subjects {
            let category = self.delete_review.entry(*id).map(|e| e.category);
            match (sections.last_mut(), category) {
                (Some((last, n)), Some(c)) if *last == c => *n += 1,
                (_, Some(c)) => sections.push((c, 1)),
                _ => {}
            }
        }
        let starts: Vec<usize> = sections
            .iter()
            .scan(0, |start, (_, n)| {
                let at = *start;
                *start += n;
                Some(at)
            })
            .collect();
        let current = self
            .delete_current
            .and_then(|id| subjects.iter().position(|other| *other == id));
        let lengths: Vec<usize> = sections.iter().map(|(_, n)| *n).collect();
        let clicked = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(grid)
                    .layout(egui::Layout::top_down(Align::Min)),
                |ui| {
                    self.sectioned_card_grid(
                        ui,
                        "delete",
                        &lengths,
                        |app, i| app.delete_card_aspects(subjects[i]),
                        |app, i| app.delete_card_has_facts(subjects[i]),
                        |app, ui, section| {
                            let (category, n) = sections[section];
                            let ids = &subjects[starts[section]..starts[section] + n];
                            app.delete_section_header(ui, category, ids);
                        },
                        |app, i| {
                            let entry = app.delete_review.entry(subjects[i])?.clone();
                            Some(app.delete_card(&entry))
                        },
                        current,
                    )
                },
            )
            .inner;
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
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2)
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
    fn delete_summary(&mut self, ui: &mut egui::Ui, subjects: &[MediaId]) {
        let candidates = self.delete_candidates(subjects);
        let trash: Vec<MediaId> = candidates
            .iter()
            .copied()
            .filter(|id| self.decision(*id).rejected)
            .collect();
        let freed: u64 = trash.iter().map(|id| self.delete_bytes(*id)).sum();
        let changed = self.delete_review.changed_count(subjects, &self.decisions);
        let note = if changed > 0 {
            format!(
                " · {} · nothing changes on disk until you apply Review changes",
                if self.preferences.easy_delete_auto {
                    format!("{} changed by you", count(changed, "card", "cards"))
                } else {
                    format!(
                        "{} not following the suggestion",
                        count(changed, "card", "cards")
                    )
                }
            )
        } else {
            " · nothing changes on disk until you apply Review changes".to_owned()
        };
        ui.horizontal(|ui| {
            // Accept all and the size slider are placed first, from the
            // right; the summary then gets only the room left and truncates.
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        changed > 0 && self.delete_enabled(),
                        egui::Button::new("Accept all suggestions"),
                    )
                    .on_hover_text(if changed > 0 {
                        "Puts every card shown back to PhotoSort's suggestion · A"
                    } else {
                        "Every card shown already follows PhotoSort's suggestion"
                    })
                    .clicked()
                {
                    self.accept_delete_defaults();
                }
                ui.add_space(S3);
                self.size_slider(ui);
                ui.add_space(S3);
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    let body = egui::TextStyle::Body.resolve(ui.style());
                    job.append(
                        &format!(
                            "{} going to Trash · frees {} · {} kept",
                            count(trash.len(), "photo", "photos"),
                            size_text(freed),
                            candidates.len() - trash.len()
                        ),
                        0.0,
                        egui::TextFormat::simple(body.clone(), INK),
                    );
                    job.append(&note, 0.0, egui::TextFormat::simple(body, MUTED));
                    let full = job.text.clone();
                    ui.add(egui::Label::new(job).truncate()).on_hover_text(full);
                });
            });
        });
    }

    /// A group's header: its name and size, and the action for all of it.
    fn delete_section_header(&mut self, ui: &mut egui::Ui, category: Category, ids: &[MediaId]) {
        ui.label(
            RichText::new(category.heading())
                .font(semibold(14.0))
                .color(INK),
        );
        ui.label(RichText::new(ids.len().to_string()).color(MUTED));
        let enabled = self.delete_enabled();
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if category == Category::Formats {
                // What each choice frees, measured on these files: which
                // format is smaller depends on the camera and its settings.
                let pairs = ids
                    .iter()
                    .filter_map(|id| self.delete_review.entry(*id))
                    .filter(|e| e.category == Category::Formats)
                    .count();
                let shots = if pairs == 1 {
                    "the shot".to_owned()
                } else {
                    format!("all {pairs} shots")
                };
                let segments: Vec<Segment> = FormatChoice::ALL
                    .into_iter()
                    .map(|choice| {
                        let freed = size_text(self.format_trash_bytes(ids, choice));
                        let (shown, hover) = match choice {
                            FormatChoice::Both => (
                                choice.title().to_owned(),
                                "Keeps the JPEG and the HEIC of every shot. Nothing goes to Trash."
                                    .to_owned(),
                            ),
                            FormatChoice::Heic => (
                                format!("{} · frees {freed}", choice.title()),
                                format!(
                                    "Moves the JPEG of {} to Trash and keeps the HEIC: frees {freed}.",
                                    shots
                                ),
                            ),
                            FormatChoice::Jpeg => (
                                format!("{} · frees {freed}", choice.title()),
                                format!(
                                    "Moves the HEIC of {} to Trash and keeps the JPEG: frees {freed}.",
                                    shots
                                ),
                            ),
                        };
                        Segment {
                            name: shown.clone(),
                            shown,
                            hover: Some(hover),
                        }
                    })
                    .collect();
                let selected = self
                    .delete_review
                    .format_choice(ids, &self.decisions)
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
                ui.label(RichText::new("For all of them:").color(MUTED));
                return;
            }
            // Only shots with a better one have a suggestion to act on; the
            // others are kept and stay kept.
            let ids: Vec<MediaId> = if category == Category::EyesClosed {
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
            };
            let ids = ids.as_slice();
            if ids.is_empty() {
                ui.label(RichText::new("No better shots found · kept").color(MUTED))
                    .on_hover_text(
                        "None of these has another shot of the same moment with everyone's eyes open, so they are kept",
                    );
                return;
            }
            let n = ids.len();
            let (label, hover) = match category {
                Category::Exact => (
                    if n == 1 {
                        "Trash the exact copy".to_owned()
                    } else {
                        format!("Trash all {n} exact copies")
                    },
                    "Moves the copy of each identical pair to Trash and keeps the other",
                ),
                Category::Near => (
                    if n == 1 {
                        "Keep the sharper one".to_owned()
                    } else {
                        format!("Keep the sharper one in all {n}")
                    },
                    "Keeps the sharper photo of each pair (or, when equally sharp, the one with more metadata) and moves the other to Trash",
                ),
                Category::EyesClosed => (
                    // Photos, not cards: a card can hold several shots.
                    match ids
                        .iter()
                        .filter_map(|id| self.delete_review.entry(*id))
                        .flat_map(|e| e.suggestion())
                        .filter(|keep| !keep)
                        .count()
                    {
                        1 => "Trash the closed-eyes shot".to_owned(),
                        n => format!("Trash all {n} closed-eyes shots"),
                    },
                    "Moves each closed-eyes shot that has a better shot of the same moment to Trash and keeps the better one. Shots with no better one stay kept",
                ),
                Category::Blur => (
                    if n == 1 {
                        "Trash the blurry photo".to_owned()
                    } else {
                        format!("Trash all {n} blurry photos")
                    },
                    "Moves every blurry photo here to Trash",
                ),
                Category::Dark => (
                    if n == 1 {
                        "Trash the dark photo".to_owned()
                    } else {
                        format!("Trash all {n} dark photos")
                    },
                    "Moves every very dark photo here to Trash",
                ),
                Category::Bright => (
                    if n == 1 {
                        "Trash the overexposed photo".to_owned()
                    } else {
                        format!("Trash all {n} overexposed photos")
                    },
                    "Moves every overexposed photo here to Trash",
                ),
                Category::Formats => unreachable!(),
            };
            let changed = self.delete_review.changed_count(ids, &self.decisions);
            let label = format!("{label} · frees {}", size_text(self.suggested_trash_bytes(ids)));
            if ui
                .add_enabled(enabled && changed > 0, egui::Button::new(label))
                .on_hover_text(if changed > 0 {
                    hover.to_owned()
                } else {
                    format!("Already the choice on every card here. {hover}.")
                })
                .clicked()
            {
                self.accept_delete_suggestions(ids);
            }
        });
    }

    /// Whether PhotoSort suggests keeping this photo over the other one of
    /// its card. It is marked beside the pill whatever the choice is now, so
    /// the suggestion stays visible after a change. Both formats of one shot
    /// are kept, so neither is marked.
    pub(super) fn delete_suggested_keeper(&self, id: MediaId) -> bool {
        self.delete_keep_why(id).is_some()
    }
    /// Why PhotoSort suggests keeping this photo, for its Suggested mark:
    /// the selected card's reason when it keeps the photo, as one photo can
    /// be the keeper of several cards (an identical pair and a closed-eyes
    /// shot, say), otherwise the first card's.
    pub(super) fn delete_keep_why(&self, id: MediaId) -> Option<String> {
        let current = self
            .delete_current
            .and_then(|c| self.delete_review.entry(c));
        let entry = current.filter(|e| keeps_for(e, id)).or_else(|| {
            self.delete_review
                .entries()
                .iter()
                .find(|e| keeps_for(e, id))
        })?;
        keep_why_text(entry)
    }
}

/// Whether `entry` suggests keeping `id` over the other photo of its card.
fn keeps_for(entry: &Entry, id: MediaId) -> bool {
    entry.companion == Some(id) && entry.subject != id && entry.category != Category::Formats
}

/// The Suggested mark's tooltip for `entry`'s keeper.
fn keep_why_text(entry: &Entry) -> Option<String> {
    Some(if entry.keep_why.is_empty() {
        "PhotoSort suggests keeping this photo and moving the other to Trash.".to_owned()
    } else {
        format!("Suggested to keep. {}", entry.keep_why)
    })
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
                    let time = chrono::DateTime::from_timestamp_millis(ms)?;
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

    /// One photo's facts on its card, in one line, for Review.
    pub(super) fn delete_photo_facts(&self, id: MediaId) -> Option<String> {
        let entry = self.delete_review.entry(self.delete_current?)?;
        let ids = entry.candidates();
        let side = ids.iter().position(|other| *other == id)?;
        let facts = self.delete_facts(entry, &ids).swap_remove(side);
        (!facts.is_empty()).then(|| {
            facts
                .iter()
                .map(|f| f.text.as_str())
                .collect::<Vec<_>>()
                .join(" · ")
        })
    }

    /// The card for one suggestion: a short tag (the full reason in its
    /// tooltip), its photos with a Keep/Trash pill each and their deciding
    /// facts, and Undo once it differs from the suggestion.
    fn delete_card(&mut self, entry: &Entry) -> ReviewCard {
        let subject = entry.subject;
        let enabled = self.delete_enabled();
        let leaders = entry.candidates();
        let facts = self.delete_facts(entry, &leaders);
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
                    caption: media.map(Media::name).unwrap_or_default(),
                    name: media
                        .map(|m| m.path.display().to_string())
                        .unwrap_or_default(),
                    facts,
                    dim: !keep,
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
            title: entry.tag.clone(),
            state: CardState::Suggested,
            state_text: None,
            photos,
            note: String::new(),
            warning: false,
            buttons: Vec::new(),
            // The selected card shows in the panel beside the grid, which is
            // enough: no outline that could be read as Keep or Trash.
            current: false,
            shown: self.delete_panel && self.delete_current == Some(subject),
            title_hover: Some(entry.reason.clone()),
            undo: (enabled && self.delete_review.changed(subject, &self.decisions)).then(|| {
                (
                    self.delete_undo_label().to_owned(),
                    "Back to PhotoSort's suggestion · R on the selected card".to_owned(),
                )
            }),
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
        let ctx = egui::Context::default();
        theme_install(&ctx);
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
    #[test]
    fn suggestions_are_the_staged_decisions_and_review_changes_lists_them() {
        let mut app = app();
        // The copy and the dark photo go to Trash; both formats stay.
        assert!(trashed(&app, 1) && trashed(&app, 3));
        assert!(!trashed(&app, 2) && !trashed(&app, 4) && !trashed(&app, 5));
        assert_eq!(app.staged_change_count(), 2);
        // Nothing waits on a confirmation: moving between cards is free.
        app.navigate_selection(1, false, false);
        // Leaving the step offers Review changes for the staged marks.
        app.request_departure(Destination::Step(Step::Cull));
        assert!(matches!(app.departure, Some(Destination::Step(Step::Cull))));
        assert_eq!(app.step, Step::EasyDelete);
    }
    #[test]
    fn a_finished_analysis_stages_its_suggestions_once() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
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
        assert!(
            app.status.contains("1 photo marked for Trash"),
            "{}",
            app.status
        );
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
        assert!(
            shown.contains("2 cards not following the suggestion"),
            "{shown}"
        );
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
        app.apply_preferences(on);
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
            Some("Suggested to keep. Why this one is kept")
        );
        assert_eq!(app.delete_keep_why(MediaId(1)), None);
        let entry = app.delete_review.entry(MediaId(1)).unwrap().clone();
        let card = app.delete_card(&entry);
        assert_eq!(
            card.photos[1].suggested_hover,
            app.delete_keep_why(MediaId(2))
        );
        assert_eq!(card.photos[0].suggested_hover, None);
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Suggested").count(), 1, "{shown}");
        // Moving the keeper to Trash by hand keeps the mark.
        app.delete_card_event(MediaId(1), CardEvent::Toggle(1));
        app.delete_panel = false;
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Suggested").count(), 1, "{shown}");
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
    fn bulk_actions_apply_to_their_group_and_accept_all_restores_every_card() {
        let mut app = app();
        for id in [1, 3] {
            app.delete_current = Some(MediaId(id));
            app.toggle_delete_candidate(0);
        }
        assert!(!trashed(&app, 1) && !trashed(&app, 3));
        // "Trash all N exact copies" is the Exact group's suggestion only.
        app.accept_delete_suggestions(&[MediaId(1)]);
        assert!(trashed(&app, 1) && !trashed(&app, 3));
        // One choice for every JPEG + HEIC pair.
        app.choose_delete_formats(&app.delete_review_ids(), FormatChoice::Heic);
        assert!(trashed(&app, 4) && !trashed(&app, 5));
        assert_eq!(
            app.delete_review
                .format_choice(&app.delete_review_ids(), &app.decisions),
            Some(FormatChoice::Heic)
        );
        // Accept all suggestions: back to every suggestion, both formats kept.
        app.accept_delete_defaults();
        assert!(trashed(&app, 1) && trashed(&app, 3));
        assert!(!trashed(&app, 4) && !trashed(&app, 5));
        assert_eq!(
            app.delete_review
                .changed_count(&app.delete_review_ids(), &app.decisions),
            0
        );
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
            "Keep the shot with eyes open",
            "No shot with eyes open - kept",
            "Eyes open",
            "Eyes closed",
            "10.jpg",
            // Photos, not cards; the note is kept, so it is not counted.
            "Trash all 2 closed-eyes shots · frees 16.0 MB",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        assert!(!shown.contains("Identical copy"), "{shown}");
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
            Some("Suggested to keep. Everyone's eyes are open here.")
        );
        let copy = app.delete_review.entry(MediaId(9)).unwrap().clone();
        assert_eq!(
            app.delete_card(&copy).photos[1].suggested_hover.as_deref(),
            Some("Suggested to keep. Why this one is kept")
        );
        app.set_active(MediaId(6));
        assert_eq!(
            app.delete_keep_why(MediaId(7)).as_deref(),
            Some("Suggested to keep. Everyone's eyes are open here.")
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
    }
    #[test]
    fn grid_shows_short_tags_facts_and_group_actions_without_confirm() {
        let mut app = app();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "Identical copy",
            "JPEG + HEIC",
            "Too dark",
            "Exact copies",
            "Same photo, two formats",
            "Keep both",
            "Keep HEIC",
            "Keep JPEG",
            "Trash the exact copy · frees 1.0 MB",
            "Trash the dark photo · frees 3.0 MB",
            "Keep HEIC · frees 4.0 MB",
            "Keep JPEG · frees 5.0 MB",
            "Accept all suggestions",
            "HEIC",
            "2 photos going to Trash · frees 4.0 MB",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        for gone in ["Provisional", "Confirm", "explained", "1 Trash", "2 Keep"] {
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
        assert!(shown.contains("Too dark") && !shown.contains("Identical copy"));
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
        // Review shows the same facts beside each file name.
        app.delete_current = Some(MediaId(1));
        assert_eq!(
            app.delete_photo_facts(MediaId(2)).as_deref(),
            Some("2025/sorted")
        );
    }
    #[test]
    fn saved_views_from_before_tabs_open_on_all_and_tabs_select_one_group() {
        let mut app = app();
        let legacy: HashSet<_> = [Category::Exact, Category::Dark].into_iter().collect();
        app.delete_categories = app.delete_tab_categories(Some(&legacy));
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
        assert_eq!(shown.matches("Identical copy").count(), 1, "{shown}");
        // The card's photos open beside the grid under its tag, with Undo;
        // the reason stays in the tag's tooltip.
        app.delete_card_event(MediaId(1), CardEvent::Select(0));
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Identical copy").count(), 2, "{shown}");
        assert!(
            shown.contains("Undo") && !shown.contains("explained"),
            "{shown}"
        );
        // Esc closes it; the grid stays.
        key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.delete_panel);
        let shown = super::super::review_card::render_grid(&mut app);
        assert_eq!(shown.matches("Identical copy").count(), 1, "{shown}");
    }
    #[test]
    fn the_size_slider_sets_how_many_cards_fit_at_every_window_width() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
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
                .map(|i| entry(2 * i + 1, Some(2 * i + 2), Category::Exact))
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
                    egui::Shape::Text(text) if text.galley.text() == "Identical copy" => {
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
        assert!(text.matches("Identical copy").count() >= 2, "{text}");
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
        app.review = false;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.global_style_mut(|style| style.animation_time = 1.0);
        // Where each "Identical copy" title was painted: the card's first,
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
                    egui::Shape::Text(text) if text.galley.text() == "Identical copy" => {
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
        // Enter shows the first card; the grid still outlines none.
        key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert!(app.delete_panel);
        let entry = app.delete_review.entry(MediaId(1)).unwrap().clone();
        assert!(!app.delete_card(&entry).current);
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
        assert_eq!(shown.matches("JPEG + HEIC").count(), 1, "{shown}");
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
        // A click on the card around the photos opens the panel.
        click(&mut app, egui::pos2(300.0, 300.0));
        assert!(app.delete_panel);
        assert!(!trashed(&app, 1), "a card click changed a photo");
        app.deselect_delete_card();
        // An arrow key selects the first card again and shows it.
        key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert!(app.delete_panel);
    }

    #[test]
    fn keyboard_switches_photos_moves_on_with_enter_and_undoes_with_r() {
        let mut app = app();
        app.delete_card_event(MediaId(1), CardEvent::Select(0));
        app.review = false;
        key(&mut app, egui::Key::Num2, egui::Modifiers::NONE);
        assert!(trashed(&app, 2), "2 switches the second photo");
        key(&mut app, egui::Key::ArrowLeft, egui::Modifiers::NONE);
        assert!(!trashed(&app, 1), "Left switches the first photo");
        key(&mut app, egui::Key::R, egui::Modifiers::NONE);
        assert!(trashed(&app, 1) && !trashed(&app, 2), "R undoes the card");
        key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(4)), "Enter moves on");
        key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(3)));
        key(&mut app, egui::Key::ArrowUp, egui::Modifiers::NONE);
        assert_eq!(app.delete_current, Some(MediaId(4)));
        key(&mut app, egui::Key::Num1, egui::Modifiers::NONE);
        assert!(trashed(&app, 4));
        key(&mut app, egui::Key::A, egui::Modifiers::NONE);
        assert!(!trashed(&app, 4), "A accepts every suggestion");
        assert!(preferences::workflow_hint(Step::EasyDelete).contains("Enter · Next"));
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
