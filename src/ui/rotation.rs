use super::review_card::{Badge, CardButton, CardEvent, CardPhoto, GridClick, ReviewCard};
use super::*;
use crate::core::rotation::{set_turns, turns};

/// A number of clockwise quarter turns, in words, for tooltips.
fn turn_text(turns: u8) -> &'static str {
    match turns % 4 {
        1 => "90° right",
        2 => "180°",
        3 => "90° left",
        _ => "not at all",
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What Fix Rotation's grid is drawn from, kept between frames: a frame in
/// which nothing changed reads it instead of working it out for every photo.
#[derive(Default)]
pub(super) struct Deck {
    /// The list and the review the photos shown were taken from.
    listed: Option<(u64, usize, u64)>,
    shown: std::rc::Rc<Shown>,
    /// The metadata the shapes were taken from.
    sized: Option<(usize, u64)>,
    /// Each photo's width over its height, as it is on disk.
    shapes: Vec<f32>,
    /// The decisions the turns were counted from.
    counted: Option<u64>,
    /// Whether each photo has a turn staged: its card then shows it twice.
    turned: Vec<bool>,
    /// How many do.
    turns: usize,
    /// A certain suggestion is not taken yet.
    left: bool,
    /// Changes whenever a card's shape may have: the grid asks its cards
    /// again.
    stamp: u64,
}

/// The photos the grid shows, in order.
#[derive(Default)]
pub(super) struct Shown {
    ids: Vec<MediaId>,
    /// Each photo's place among them.
    places: HashMap<MediaId, usize>,
    /// How many, from the first, have a suggestion.
    suggested: usize,
}

/// The one-click action in a card's header, beside its rotate buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RotationLink {
    /// Back to where the photo started.
    Undo,
    /// Turns down a staged suggestion: the photo stays as it is on disk.
    KeepOriginal,
    /// Stages a suggestion that was not staged on its own.
    UseSuggestion,
}

impl PhotoSortApp {
    /// Whether detection ran, even if every photo looked upright.
    pub(super) fn rotation_detected(&self) -> bool {
        !self.rotations.is_empty() || !self.rotation_review.ids().is_empty()
    }
    /// A detection run that stopped before it finished leaves no partial
    /// results: the step offers to detect again.
    pub(super) fn drop_rotation_run(&mut self) {
        if self.rotation_run.take().is_some() {
            self.rotations.clear();
            self.filter_dirty = true;
        }
    }
    fn rotation_enabled(&self) -> bool {
        !self.busy && self.rotation_run.is_none()
    }
    /// With the setting on, certain suggestions are the default decisions:
    /// each photo's is staged once, when it is first found or when Fix
    /// Rotation opens after staged choices were discarded.
    pub(super) fn stage_rotation_suggestions(&mut self) {
        if !self.preferences.rotation_auto || !self.view_data_ready.contains(&Step::Rotation) {
            return;
        }
        let turned = self
            .rotation_review
            .stage_new_suggestions(&mut self.decisions);
        if !turned.is_empty() {
            self.changed_items(turned);
        }
    }
    pub(super) fn detect_rotations(&mut self) {
        if !self.can_start_analysis() || !self.model_install_ready() {
            return;
        }
        let Some(model) = crate::core::models::orientation_model() else {
            self.model_issue = Some(crate::core::model_assets::Issue {
                missing: vec![crate::core::model_assets::Id::Rotation],
                ..Default::default()
            });
            self.remember_model_retry();
            self.open_model_setup();
            return;
        };
        match self.runtime.orient(Arc::new(self.media.clone()), model) {
            Ok(run) => {
                self.clear_model_retry();
                self.resume_suspended = false;
                self.rotations.clear();
                self.rotation_run = Some(run);
                self.analysis_run = Some(run);
                self.analysis_failures.clear();
                self.analysis_successes = 0;
                self.progress = Some((0, self.media.len()));
                // The progress card says what runs.
                self.set_status(String::new());
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn finish_rotation_detection(&mut self) {
        let mut rows: Vec<_> = self
            .media
            .iter()
            .filter_map(|m| {
                self.rotations
                    .get(&m.id)
                    .filter(|s| s.turns % 4 != 0)
                    .map(|s| {
                        let order = crate::core::dates::capture_order(m, self.metadata.get(&m.id));
                        (order, m.id, s.turns, s.uncertain)
                    })
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.0.cmp(&b.1.0)));
        if self.step == Step::Rotation {
            self.trash_last = None;
        }
        self.rotation_review.replace_predictions(
            rows.into_iter()
                .map(|(_, id, turns, uncertain)| (id, turns, uncertain))
                .collect(),
        );
        self.rotation_run = None;
        self.view_data_ready.insert(Step::Rotation);
        self.rotation_queue_only = true;
        self.filter_dirty = true;
        self.stage_rotation_suggestions();
        self.finish_step_status(Step::Rotation);
    }
    /// The grid's photos: only the suggestions, or every photo with the
    /// suggestions first. In each part, photos marked for Trash go last.
    /// A card hidden by the search stops being selected.
    pub(super) fn refilter_rotations(&mut self) {
        let queue_only = self.rotation_queue_only;
        let review = &self.rotation_review;
        let media = &self.media;
        self.filtered.retain(|i| {
            media[*i].kind != MediaKind::Video
                && (!queue_only || review.suggestion(media[*i].id).is_some())
        });
        let order: HashMap<_, _> = review
            .ids()
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i))
            .collect();
        let mut keyed: Vec<_> = std::mem::take(&mut self.filtered)
            .into_iter()
            .map(|i| {
                let id = self.media[i].id;
                let at = order.get(&id).copied();
                ((at.is_none(), self.shown_last(id), at), i)
            })
            .collect();
        // Stable: every other photo keeps the browsing order.
        keyed.sort_by_key(|(key, _)| *key);
        self.filtered = keyed.into_iter().map(|(_, i)| i).collect();
        if self
            .active
            .is_some_and(|id| !self.filtered.iter().any(|i| self.media[*i].id == id))
        {
            self.deselect_rotation_card();
        }
    }
    /// The bar's only control: suggestions or every photo.
    pub(super) fn rotation_controls(&mut self, ui: &mut egui::Ui) {
        let mut queue_only = self.rotation_queue_only;
        if ui
            .add(Switch::new(&mut queue_only, "Suggestions only"))
            .changed()
        {
            self.rotation_queue_only = queue_only;
            self.filter_dirty = true;
        }
    }
    /// Turns the photo by `delta` quarter turns clockwise.
    pub(super) fn rotate_card(&mut self, id: MediaId, delta: u8) {
        if self.rotation_enabled() {
            self.rotate(id, delta);
        }
    }
    /// The header action a photo offers now, if any.
    pub(super) fn rotation_link(&self, id: MediaId) -> Option<RotationLink> {
        let staged = turns(&self.decisions, id);
        if self.rotation_review.changed(id, &self.decisions) {
            Some(RotationLink::Undo)
        } else if staged != 0 && self.rotation_review.suggestion(id).is_some() {
            Some(RotationLink::KeepOriginal)
        } else if self.rotation_review.suggestion(id).is_some() {
            Some(RotationLink::UseSuggestion)
        } else {
            None
        }
    }
    fn rotation_link_text(&self, id: MediaId, link: RotationLink) -> (&'static str, String) {
        match link {
            RotationLink::Undo => ("Undo", "R".to_owned()),
            RotationLink::KeepOriginal => ("Keep original", String::new()),
            RotationLink::UseSuggestion => ("Use suggestion", self.rotation_suggestion_text(id)),
        }
    }
    /// What PhotoSort suggests for a photo and how sure it is, for tooltips.
    fn rotation_suggestion_text(&self, id: MediaId) -> String {
        let Some(suggestion) = self.rotation_review.suggestion(id) else {
            return String::new();
        };
        let mut text = format!("Suggested: {}", turn_text(suggestion.turns));
        if let Some(found) = self.rotations.get(&id) {
            text.push_str(&format!(" · {:.0}% confident", found.confidence * 100.0));
        }
        if suggestion.uncertain {
            text.push_str("\nUnsure, so not turned until you choose");
        }
        text
    }
    pub(super) fn follow_rotation_link(&mut self, id: MediaId, link: RotationLink) {
        if !self.rotation_enabled() {
            return;
        }
        match link {
            RotationLink::Undo => self.rotation_review.undo(id, &mut self.decisions),
            RotationLink::KeepOriginal => set_turns(&mut self.decisions, id, 0),
            RotationLink::UseSuggestion => {
                if let Some(suggestion) = self.rotation_review.suggestion(id) {
                    set_turns(&mut self.decisions, id, suggestion.turns);
                }
            }
        }
        self.pending_bookmark = None;
        self.changed_item(id);
    }
    /// The certain suggestions not taken yet. Uncertain ones are left to
    /// the user, as when suggestions apply automatically.
    fn rotation_suggestions_left(&self) -> Vec<MediaId> {
        self.rotation_review
            .ids()
            .iter()
            .copied()
            .filter(|id| {
                self.rotation_review
                    .suggestion(*id)
                    .is_some_and(|s| !s.uncertain && turns(&self.decisions, *id) != s.turns)
            })
            .collect()
    }
    /// Accept all suggestions: every certain suggestion is staged.
    pub(super) fn accept_rotation_suggestions(&mut self) {
        if !self.rotation_enabled() {
            return;
        }
        let ids = self.rotation_suggestions_left();
        for id in &ids {
            if let Some(suggestion) = self.rotation_review.suggestion(*id) {
                set_turns(&mut self.decisions, *id, suggestion.turns);
            }
        }
        if !ids.is_empty() {
            self.pending_bookmark = None;
            self.changed_items(ids);
        }
    }
    /// R: the photo shown beside the grid goes back to where it started.
    pub(super) fn undo_rotation(&mut self) {
        if let Some(id) = self.active
            && self.rotation_review.changed(id, &self.decisions)
        {
            self.follow_rotation_link(id, RotationLink::Undo);
        }
    }
    /// A click on the grid's empty space: no card is selected, and the
    /// panel beside the grid closes.
    pub(super) fn deselect_rotation_card(&mut self) {
        self.rotation_panel = false;
        self.active = None;
    }
    /// Enter: the choice is already staged, so move on.
    pub(super) fn next_rotation_card(&mut self) {
        self.navigate_selection(1, false, false);
        self.rotation_panel = self.active.is_some();
    }
    /// What the panel beside the grid shows: the photo as it is, and turned
    /// when a turn is staged.
    pub(super) fn rotation_canvas_ids(&self, id: MediaId) -> Vec<MediaId> {
        if turns(&self.decisions, id) == 0 {
            vec![id]
        } else {
            vec![id, id]
        }
    }
    /// Rotate and Undo in the floating bar under the panel's photos.
    pub(super) fn rotation_bar_buttons(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let Some(id) = self.active else {
            return;
        };
        let enabled = self.rotation_enabled();
        ui.add_enabled_ui(enabled, |ui| {
            if icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, "Rotate left · Q").clicked() {
                self.rotate_card(id, 3);
            }
            if icon_button(ui, icon::ARROW_CLOCKWISE, "Rotate right · E").clicked() {
                self.rotate_card(id, 1);
            }
            if let Some(link) = self.rotation_link(id) {
                let (label, hover) = self.rotation_link_text(id, link);
                let mut response = ui.button(label);
                if !hover.is_empty() {
                    response = response.on_hover_text(hover);
                }
                if response.clicked() {
                    self.follow_rotation_link(id, link);
                }
            }
        });
    }
}

impl PhotoSortApp {
    /// Fix Rotation's only view: a summary with the photo size, then one
    /// card per photo, as it is beside how it will be. Clicking a card
    /// opens its photos large in a panel beside the grid.
    pub(super) fn rotation_grid(&mut self, ui: &mut egui::Ui) {
        let shown = self.rotation_deck();
        let ids = &shown.ids;
        self.rotation_summary(ui);
        ui.add_space(S2);
        // As in Easy Delete: the grid takes its final width at once and the
        // panel slides over the room it leaves.
        let area = ui.available_rect_before_wrap();
        let panel_shown = self.rotation_panel && self.active.is_some();
        let slide = ui
            .ctx()
            .animate_bool_responsive(egui::Id::new("rotation_panel_slide"), panel_shown);
        let widest = (area.width() - 460.0).max(360.0);
        self.rotation_panel_width = self
            .rotation_panel_width
            .unwrap_or(area.width() * 0.45)
            .clamp(360.0, widest)
            .into();
        let panel_width = self.rotation_panel_width.unwrap_or(360.0);
        let grid = if panel_shown {
            area.with_max_x(area.right() - panel_width - S3)
        } else {
            area
        };
        // Suggestions first, then, with every photo shown, the others.
        let suggested = shown.suggested;
        let sections: Vec<(bool, usize)> = [(true, suggested), (false, ids.len() - suggested)]
            .into_iter()
            .filter(|(_, n)| *n > 0)
            .collect();
        let lengths: Vec<usize> = sections.iter().map(|(_, n)| *n).collect();
        let current = self.active.and_then(|id| shown.places.get(&id).copied());
        let stamp = self.grids.rotation.stamp;
        let clicked = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(grid)
                    .layout(egui::Layout::top_down(Align::Min)),
                |ui| {
                    self.sectioned_card_grid(
                        ui,
                        "rotation",
                        &lengths,
                        stamp,
                        // The photo as it is and, once a turn is staged, as
                        // it will be. The turned photo keeps the first one's
                        // slot, fitted inside it, so turning again does not
                        // move the card or its rotate buttons.
                        |app, i, shapes| {
                            let deck = &app.grids.rotation;
                            if let Some(shape) = deck.shapes.get(i) {
                                shapes.push(*shape);
                                if deck.turned.get(i) == Some(&true) {
                                    shapes.push(*shape);
                                }
                            }
                        },
                        |_, _| false,
                        // The rotate buttons sit in every card's header.
                        |_, _| true,
                        |_, ui, section| {
                            // The status bar counts the suggestions.
                            let (suggestions, _) = sections[section];
                            ui.label(
                                RichText::new(if suggestions {
                                    "Sideways or upside down"
                                } else {
                                    "Other photos"
                                })
                                .font(semibold(14.0))
                                .color(INK),
                            );
                        },
                        |app, i| Some(app.rotation_card(ids[i])),
                        current,
                    )
                },
            )
            .inner;
        if slide > 0.0 {
            self.rotation_side_panel(ui, area, panel_width, slide);
        }
        ui.advance_cursor_after_rect(area);
        match clicked {
            Some(GridClick::Card(i, event)) => self.rotation_card_event(ids[i], event),
            Some(GridClick::Background) => self.deselect_rotation_card(),
            None => {}
        }
    }

    /// One line over the grid: what is staged, the photo size, and Accept
    /// all suggestions.
    fn rotation_summary(&mut self, ui: &mut egui::Ui) {
        let (turned, left) = (self.grids.rotation.turns, self.grids.rotation.left);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        left && self.rotation_enabled(),
                        egui::Button::new("Accept all suggestions"),
                    )
                    .on_hover_text("A")
                    .clicked()
                {
                    self.accept_rotation_suggestions();
                }
                ui.add_space(S3);
                self.size_slider(ui);
                ui.add_space(S3);
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    super::review_card::summary_label(
                        ui,
                        &format!("{} turned", count(turned, "photo", "photos")),
                    );
                });
            });
        });
    }

    /// The panel beside the grid, as in Easy Delete: it slides in over the
    /// grid's right edge, and its left edge resizes it.
    fn rotation_side_panel(&mut self, ui: &mut egui::Ui, area: egui::Rect, width: f32, slide: f32) {
        use egui_phosphor::regular as icon;
        let rect = egui::Rect::from_min_size(
            egui::pos2(area.right() - (width + S3) * slide + S3, area.top()),
            egui::vec2(width, area.height()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .id_salt("rotation_side_panel")
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
                        // No title: the selected card beside the panel names
                        // the photo.
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if icon_button(ui, icon::X, "Close · Esc").clicked() {
                                    self.rotation_panel = false;
                                }
                            });
                        });
                        ui.add_space(S1);
                        self.review_canvas(ui);
                    });
            },
        );
        let handle = egui::Rect::from_x_y_ranges(
            rect.left() - 4.0..=rect.left() + 2.0,
            rect.top()..=rect.bottom(),
        );
        let response = ui
            .interact(
                handle,
                egui::Id::new("rotation_panel_resize"),
                Sense::drag(),
            )
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            self.rotation_panel_width = Some(width - response.drag_delta().x);
        }
    }

    /// What the grid is drawn from, brought up to date: the photos shown
    /// when the list was built again with other photos or in another order,
    /// their shapes when a photo's metadata arrived, and which are turned
    /// when a decision changed. A frame in which none of that happened
    /// looks up three stamps.
    fn rotation_deck(&mut self) -> std::rc::Rc<Shown> {
        let mut deck = std::mem::take(&mut self.grids.rotation);
        let review = self.rotation_review.revision();
        let listed = (self.grids.listed, self.filtered.len(), review);
        if deck.listed != Some(listed) {
            let ids: Vec<MediaId> = self.filtered.iter().map(|i| self.media[*i].id).collect();
            // A turn builds the list again, most often to the same photos
            // in the same order: everything worked out for them stands.
            let same = deck.listed.is_some_and(|(_, _, of)| of == review) && deck.shown.ids == ids;
            if !same {
                deck.shown = std::rc::Rc::new(Shown {
                    suggested: ids
                        .iter()
                        .take_while(|id| self.rotation_review.suggestion(**id).is_some())
                        .count(),
                    places: ids
                        .iter()
                        .enumerate()
                        .map(|(place, id)| (*id, place))
                        .collect(),
                    ids,
                });
                deck.sized = None;
                deck.counted = None;
            }
            deck.listed = Some(listed);
        }
        let metadata = self.metadata_stamp();
        if deck.sized != Some(metadata) {
            deck.shapes.clear();
            deck.shapes
                .extend(deck.shown.ids.iter().map(|id| self.photo_aspect(*id, 0)));
            deck.sized = Some(metadata);
            deck.stamp = crate::core::revision::next();
        }
        let decisions = self.decisions.revision();
        if deck.counted != Some(decisions) {
            // From the staged turns, which are few, not from every photo.
            deck.turned.clear();
            deck.turned.resize(deck.shown.ids.len(), false);
            deck.turns = 0;
            for (id, decision) in self.decisions.iter() {
                if decision.turns % 4 != 0
                    && let Some(place) = deck.shown.places.get(id)
                {
                    deck.turned[*place] = true;
                    deck.turns += 1;
                }
            }
            deck.left = !self.rotation_suggestions_left().is_empty();
            deck.counted = Some(decisions);
            deck.stamp = crate::core::revision::next();
        }
        let shown = deck.shown.clone();
        self.grids.rotation = deck;
        shown
    }

    /// The slot height over width a card's photos need: a quarter turn
    /// makes a landscape photo portrait.
    pub(super) fn rotation_card_ratio(&self, id: MediaId) -> f32 {
        let Some(m) = self
            .metadata
            .get(&id)
            .filter(|m| m.width > 0 && m.height > 0)
        else {
            return 0.75;
        };
        let (w, h) = if (5..=8).contains(&m.orientation) {
            (m.height, m.width)
        } else {
            (m.width, m.height)
        };
        let upright = h as f32 / w as f32;
        let ratio = if turns(&self.decisions, id) % 2 == 1 {
            upright.max(1.0 / upright)
        } else {
            upright
        };
        ratio.clamp(0.56, 1.5)
    }

    /// The card for one photo: as it is now and, once a turn is staged, as
    /// it will be. No labels: the photos say it. Rotate buttons and the one
    /// action that applies (Undo, Keep original or Use suggestion) sit in
    /// its header; how sure PhotoSort is lives in their tooltips.
    pub(super) fn rotation_card(&self, id: MediaId) -> ReviewCard {
        use egui_phosphor::regular as icon;
        let media = self.index.get(&id).map(|i| &self.media[*i]);
        let caption = media.map(|m| self.shown_name(m)).unwrap_or_default();
        let path = media
            .map(|m| m.path.display().to_string())
            .unwrap_or_default();
        let staged = turns(&self.decisions, id);
        let enabled = self.rotation_enabled();
        // Marked for Trash in another step: dimmed, its pill takes it back.
        let trash = self.decision(id).rejected;
        // One photo twice: its name once, under the first.
        let photo = |turns: u8, name: String| CardPhoto {
            id,
            turns,
            badge: if trash && turns == 0 {
                Badge::Trash("Trash".into())
            } else {
                Badge::None
            },
            suggested: false,
            caption: if turns == 0 {
                caption.clone()
            } else {
                String::new()
            },
            name,
            facts: Vec::new(),
            trash,
            dimmed: trash,
            toggle: (trash && turns == 0 && enabled).then(|| "Keep instead".to_owned()),
            suggested_hover: None,
            inset: None,
        };
        let mut photos = vec![photo(0, path.clone())];
        if staged != 0 {
            // The suggestion only when the turn differs from it.
            let mut name = format!("{path}\nTurned {}", turn_text(staged));
            match self.rotation_review.suggestion(id) {
                Some(s) if s.turns % 4 != staged % 4 => {
                    name.push('\n');
                    name.push_str(&self.rotation_suggestion_text(id));
                }
                Some(_) => {
                    if let Some(found) = self.rotations.get(&id) {
                        name.push_str(&format!(" · {:.0}% confident", found.confidence * 100.0));
                    }
                }
                None => {}
            }
            photos.push(photo(staged, name));
        }
        ReviewCard {
            title: String::new(),
            state_text: None,
            photos,
            buttons: vec![
                CardButton::new(icon::ARROW_COUNTER_CLOCKWISE, "Rotate left").enabled(enabled),
                CardButton::new(icon::ARROW_CLOCKWISE, "Rotate right").enabled(enabled),
            ],
            // The selected card shows in the panel beside the grid.
            shown: self.rotation_panel && self.active == Some(id),
            title_hover: None,
            undo: self.rotation_link(id).filter(|_| enabled).map(|link| {
                let (label, hover) = self.rotation_link_text(id, link);
                (label.to_owned(), hover)
            }),
        }
    }

    /// Applies a card's event to its photo, which first becomes the current
    /// photo so the keyboard follows the mouse.
    pub(super) fn rotation_card_event(&mut self, id: MediaId, event: CardEvent) {
        self.pending_bookmark = None;
        if self.active != Some(id) {
            self.set_active(id);
        }
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                self.focused_slot = side as u64;
                // A click shows the card's photos large beside the grid.
                self.rotation_panel = true;
            }
            CardEvent::Button(0) => self.rotate_card(id, 3),
            CardEvent::Button(_) => self.rotate_card(id, 1),
            CardEvent::Undo => {
                if let Some(link) = self.rotation_link(id) {
                    self.follow_rotation_link(id, link);
                }
            }
            // Only a photo marked for Trash elsewhere has a pill: Keep.
            CardEvent::Toggle(_) => {
                self.stage_manual([id], crate::core::decisions::Change::Trash(false));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four photos: 1 sideways, 2 upside down but uncertain, 3 and 4
    /// upright (the model would keep 3 as it is).
    fn fixture(auto: bool) -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.preferences.rotation_auto = auto;
        app.media = (1..=4)
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
        for (n, turns, uncertain) in [(1, 1, false), (2, 2, true), (3, 0, false)] {
            app.rotations.insert(
                MediaId(n),
                crate::core::models::RotationSuggestion {
                    turns,
                    confidence: 0.9,
                    uncertain,
                    views: 3,
                },
            );
        }
        app.set_step(Step::Rotation);
        app.finish_rotation_detection();
        app.refilter();
        app
    }
    fn shown(app: &PhotoSortApp) -> Vec<MediaId> {
        app.filtered.iter().map(|i| app.media[*i].id).collect()
    }
    fn staged(app: &PhotoSortApp, n: u64) -> u8 {
        turns(&app.decisions, MediaId(n))
    }

    #[test]
    fn certain_suggestions_are_staged_at_once_without_confirm() {
        let app = fixture(true);
        assert_eq!(staged(&app, 1), 1);
        // Uncertain: listed, but not turned until the user chooses.
        assert_eq!(staged(&app, 2), 0);
        assert_eq!(app.staged_change_count(), 1);
        assert_eq!(
            app.rotation_link(MediaId(1)),
            Some(RotationLink::KeepOriginal)
        );
        assert_eq!(
            app.rotation_link(MediaId(2)),
            Some(RotationLink::UseSuggestion)
        );
        assert_eq!(app.rotation_link(MediaId(4)), None);
        // What Review changes plans: the staged decisions.
        assert!(app.decisions.values().all(|d| !d.is_empty()));
        assert_eq!(app.decisions.len(), 1);
    }

    #[test]
    fn with_the_setting_off_nothing_is_staged_until_a_suggestion_is_used() {
        let mut app = fixture(false);
        assert!(app.decisions.is_empty());
        app.rotation_card_event(MediaId(1), CardEvent::Undo);
        assert_eq!(staged(&app, 1), 1);
        assert_eq!(app.rotation_link(MediaId(1)), Some(RotationLink::Undo));
        app.rotation_card_event(MediaId(1), CardEvent::Undo);
        assert!(app.decisions.is_empty());
        // Accept all suggestions turns the certain one only.
        app.accept_rotation_suggestions();
        assert_eq!((staged(&app, 1), staged(&app, 2)), (1, 0));
    }

    #[test]
    fn suggestions_leave_out_photos_the_model_would_keep() {
        let mut app = fixture(true);
        assert_eq!(shown(&app), [MediaId(1), MediaId(2)]);
        // A photo turned by hand in all photos is still no suggestion.
        app.rotation_queue_only = false;
        app.filter_dirty = true;
        app.refilter();
        assert_eq!(
            shown(&app),
            [MediaId(1), MediaId(2), MediaId(3), MediaId(4)]
        );
        app.rotation_card_event(MediaId(3), CardEvent::Button(1));
        assert_eq!(staged(&app, 3), 1);
        app.rotation_queue_only = true;
        app.filter_dirty = true;
        app.refilter();
        assert_eq!(shown(&app), [MediaId(1), MediaId(2)]);
    }

    #[test]
    fn turning_back_to_where_it_started_is_no_change_in_both_modes() {
        for queue_only in [true, false] {
            let mut app = fixture(true);
            app.rotation_queue_only = queue_only;
            app.filter_dirty = true;
            app.refilter();
            // A suggestion: right, then back left, is the suggestion again.
            app.rotation_card_event(MediaId(1), CardEvent::Button(1));
            assert_eq!(app.rotation_link(MediaId(1)), Some(RotationLink::Undo));
            app.rotation_card_event(MediaId(1), CardEvent::Button(0));
            assert!(!app.rotation_review.changed(MediaId(1), &app.decisions));
            assert_eq!(
                app.rotation_link(MediaId(1)),
                Some(RotationLink::KeepOriginal)
            );
            // An uncertain suggestion turned and turned back stages nothing.
            app.rotation_card_event(MediaId(2), CardEvent::Button(1));
            app.rotation_card_event(MediaId(2), CardEvent::Button(0));
            assert!(!app.decisions.contains_key(&MediaId(2)));
            if !queue_only {
                // A photo without a suggestion: no Undo, nothing staged.
                for _ in 0..4 {
                    app.rotation_card_event(MediaId(4), CardEvent::Button(1));
                }
                assert!(!app.decisions.contains_key(&MediaId(4)));
                assert_eq!(app.rotation_link(MediaId(4)), None);
                app.rotation_card_event(MediaId(4), CardEvent::Button(1));
                app.rotation_card_event(MediaId(4), CardEvent::Button(0));
                assert!(!app.decisions.contains_key(&MediaId(4)));
                assert!(app.rotation_card(MediaId(4)).undo.is_none());
                assert_eq!(app.rotation_card(MediaId(4)).photos.len(), 1);
            }
            assert_eq!(app.staged_change_count(), 1, "{queue_only}");
        }
    }

    #[test]
    fn keep_original_and_undo_are_one_click_each() {
        let mut app = fixture(true);
        app.rotation_card_event(MediaId(1), CardEvent::Undo);
        assert!(app.decisions.is_empty());
        assert_eq!(app.rotation_link(MediaId(1)), Some(RotationLink::Undo));
        assert_eq!(app.rotation_card(MediaId(1)).photos.len(), 1);
        app.rotation_card_event(MediaId(1), CardEvent::Undo);
        assert_eq!(staged(&app, 1), 1);
        // A new detection does not stage a kept photo again.
        app.rotation_card_event(MediaId(1), CardEvent::Undo);
        app.finish_rotation_detection();
        assert!(app.decisions.is_empty());
    }

    #[test]
    fn cards_show_the_photo_and_its_fix_without_labels_or_confirm() {
        let mut app = fixture(true);
        let card = app.rotation_card(MediaId(1));
        assert_eq!(card.photos.len(), 2);
        assert_eq!((card.photos[0].turns, card.photos[1].turns), (0, 1));
        assert!(card.photos.iter().all(|p| p.badge == Badge::None));
        assert!(card.photos[1].name.contains("90% confident"));
        let painted = super::super::review_card::render_grid(&mut app);
        for gone in [
            "Turn 90°",
            "Turn 180°",
            "Original",
            "Corrected preview",
            "Suggested:",
            "Provisional",
            "Confirm",
            "Keep as is",
        ] {
            assert!(!painted.contains(gone), "{gone} in:\n{painted}");
        }
        assert!(painted.contains("Keep original"), "{painted}");
        assert!(painted.contains("Use suggestion"), "{painted}");
    }

    #[test]
    fn the_grid_keeps_its_rows_until_a_turn_a_new_shape_or_the_width_changes_them() {
        use super::super::review_card::{LaidRows as Rows, render_grid_at};
        // The rows a grid that kept nothing lays out from the same state.
        fn fresh(app: &mut PhotoSortApp, width: f32) -> Rows {
            let kept = std::mem::take(&mut app.grids);
            render_grid_at(app, width);
            let rows = app.grids.rows.laid().1;
            app.grids = kept;
            rows
        }
        // One frame as the workspace draws it: how many times the rows
        // were laid out so far, and the rows, which are always those a
        // fresh grid would show.
        fn draw(app: &mut PhotoSortApp, width: f32) -> (usize, Rows) {
            app.refilter();
            render_grid_at(app, width);
            let (layouts, rows) = app.grids.rows.laid();
            assert_eq!(rows, fresh(app, width), "at {width}");
            (layouts, rows)
        }
        let mut app = fixture(false);
        app.rotation_queue_only = false;
        app.filter_dirty = true;
        let (first, rows) = draw(&mut app, 1200.0);
        assert_eq!(shown(&app).len(), 4);
        // Nothing changed, then a choice that turns nothing: the rows stand.
        assert_eq!(draw(&mut app, 1200.0).0, first);
        app.decisions.entry(MediaId(4)).or_default().rating = Some(3);
        assert_eq!(draw(&mut app, 1200.0), (first, rows.clone()));
        // A turn shows the photo twice: its card is wider in the next frame.
        app.rotate_card(MediaId(3), 1);
        let (second, turned) = draw(&mut app, 1200.0);
        assert!(second == first + 1 && turned != rows);
        // A photo's size arrives: its card takes its shape.
        app.metadata.insert(
            MediaId(4),
            Metadata {
                width: 3000,
                height: 4000,
                ..Default::default()
            },
        );
        let (third, shaped) = draw(&mut app, 1200.0);
        assert!(third == second + 1 && shaped != turned);
        // A window too narrow for two cards side by side.
        let (fourth, narrow) = draw(&mut app, 320.0);
        assert!(fourth == third + 1 && narrow != shaped);
        assert_eq!(draw(&mut app, 320.0).0, fourth);
    }
    #[test]
    fn a_click_opens_the_card_in_the_panel_and_keys_act_only_on_it() {
        let mut app = fixture(true);
        assert!(!app.rotation_panel);
        assert_eq!(app.active, None);
        app.rotation_card_event(MediaId(2), CardEvent::Open(0));
        assert!(app.rotation_panel && !app.review);
        assert_eq!(app.active, Some(MediaId(2)));
        assert_eq!(app.rotation_canvas_ids(MediaId(2)), [MediaId(2)]);
        app.rotation_card_event(MediaId(2), CardEvent::Undo);
        assert_eq!(
            app.rotation_canvas_ids(MediaId(2)),
            [MediaId(2), MediaId(2)]
        );
        app.deselect_rotation_card();
        assert!(!app.rotation_panel && app.active.is_none());
    }

    #[test]
    fn staged_rotations_survive_leaving_the_step_and_a_session_reload() {
        let mut app = fixture(true);
        app.set_step(Step::Cull);
        assert_eq!(staged(&app, 1), 1);
        assert_eq!(app.preview_turns(MediaId(1)), 1);
        let session = app.session_snapshot();
        assert!(session.rotations_staged.contains(&MediaId(1)));
        // A restart restores what was staged and does not stage it again.
        let mut again = fixture(true);
        again.decisions = session.decisions.clone().into();
        again
            .rotation_review
            .restore_staged(session.rotations_staged);
        set_turns(&mut again.decisions, MediaId(1), 0);
        again.finish_rotation_detection();
        assert_eq!(staged(&again, 1), 0);
        // Discarding staged choices brings the suggestions back.
        again.discard_staged_choices();
        again.set_step(Step::Cull);
        again.set_step(Step::Rotation);
        assert_eq!(staged(&again, 1), 1);
    }

    #[test]
    fn leaving_keeps_staged_turns_and_coming_back_never_turns_a_kept_photo() {
        let mut app = fixture(true);
        app.switch_step(Step::Cull);
        // Nothing is asked: the turn waits for the Review step.
        assert_eq!(app.step, Step::Cull);
        assert!(app.departure.is_none());
        assert_eq!(staged(&app, 1), 1);
        // The user keeps photo 1 as it is, from Cull.
        app.rotate(MediaId(1), 3);
        assert_eq!(staged(&app, 1), 0);
        app.switch_step(Step::Rotation);
        assert_eq!(staged(&app, 1), 0, "its suggestion is not staged again");
    }

    #[test]
    fn applied_rotations_leave_the_suggestions_and_are_not_staged_again() {
        let mut app = fixture(true);
        let applied: HashSet<_> = [MediaId(1)].into();
        app.decisions.remove(&MediaId(1));
        app.rotation_review.complete(&applied);
        assert_eq!(app.rotation_review.ids(), &[MediaId(2)]);
        app.stage_rotation_suggestions();
        assert!(app.decisions.is_empty());
    }
}
