//! The review card shared by the guided steps' grids (Easy Delete, Fix
//! Rotation and Pick Best): the photos of one suggestion or group, what
//! PhotoSort suggests for each, whether the choice is settled, and the same
//! quick actions as Review. The steps describe cards; this module draws them
//! and reports what was clicked, so every grid looks and behaves alike.
use super::library::{WELL, reveal, ring, shimmer};
use super::*;

/// One photo on a card.
pub(super) struct CardPhoto {
    pub id: MediaId,
    /// Quarter turns to show it with.
    pub turns: u8,
    /// The pill over its top-right corner.
    pub badge: Badge,
    /// Marks PhotoSort's suggested keeper beside the pill.
    pub suggested: bool,
    /// Shown under the photo when the grid has captions.
    pub caption: String,
    /// Its accessible name and tooltip.
    pub name: String,
    /// Deciding facts in one line under it, when the layout has room.
    pub facts: Vec<Fact>,
    /// Edged in red: it goes to Trash.
    pub trash: bool,
    /// Makes the pill a Keep/Trash switch, with this tooltip.
    pub toggle: Option<String>,
    /// Why it is the suggested keeper, over its Suggested mark.
    pub suggested_hover: Option<String>,
    /// A zoomed detail over its lower-left corner.
    pub inset: Option<Inset>,
}

/// Zoomed parts of a card photo drawn over it, such as the eyes of each
/// person in a closed-eyes shot, with a quiet label under them.
pub(super) struct Inset {
    /// Each detail, cut sharp from the original (`None` while it decodes),
    /// with its width over its height.
    pub strips: Vec<(Option<super::people::FaceImage>, f32)>,
    pub label: String,
    /// Shut eyes rather than open ones: picks the label's icon.
    pub closed: bool,
}

/// One fact under a card photo, such as "4.1 MB".
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fact {
    pub text: String,
    /// Better than the other photo's value: highlighted.
    pub better: bool,
}

/// A pill over a photo: Keep or Trash, or none.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Badge {
    Keep(String),
    Trash(String),
    /// No pill: the photo speaks for itself.
    None,
}
impl Badge {
    pub fn keep(keep: bool) -> Self {
        if keep {
            Self::Keep("Keep".into())
        } else {
            Self::Trash("Trash".into())
        }
    }
}

/// How settled a card's choice is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CardState {
    /// The suggestion is the decision: nothing to say.
    Suggested,
}

/// A button on a card's action row.
pub(super) struct CardButton {
    pub text: String,
    pub hover: String,
    pub primary: bool,
    pub enabled: bool,
    /// Right-aligned with Confirm, instead of with the per-photo choices.
    pub trailing: bool,
}
impl CardButton {
    pub fn new(text: impl Into<String>, hover: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            hover: hover.into(),
            primary: false,
            enabled: true,
            trailing: false,
        }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

pub(super) struct ReviewCard {
    pub title: String,
    pub state: CardState,
    /// Replaces the state's own words, as in "Round 2 of 3".
    pub state_text: Option<String>,
    pub photos: Vec<CardPhoto>,
    /// Why PhotoSort suggests it, in one line.
    pub note: String,
    pub warning: bool,
    pub buttons: Vec<CardButton>,
    /// The card the keyboard shortcuts act on.
    pub current: bool,
    /// The title's tooltip: the full explanation behind a short tag.
    pub title_hover: Option<String>,
    /// Shown large beside the grid: a soft, neutral highlight that eases in.
    pub shown: bool,
    /// A link in the header back to the suggestion: its label (such as
    /// "Undo") and tooltip.
    pub undo: Option<(String, String)>,
}

/// What a card was asked to do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CardEvent {
    /// A click on the card, or on one of its photos.
    Select(usize),
    /// A double click: open it in Review on that photo.
    Open(usize),
    /// One of its buttons, by index.
    Button(usize),
    /// A photo's Keep/Trash pill.
    Toggle(usize),
    /// The header's Undo link.
    Undo,
}

/// What a sectioned grid was asked to do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GridClick {
    Card(usize, CardEvent),
    /// A click between and around the cards.
    Background,
}

/// The geometry of the cards in one grid row.
#[derive(Clone, Copy, Debug)]
pub(super) struct CardLayout {
    /// The height of its photos, shared by its row; each photo is as wide
    /// as its shape needs at that height.
    pub photo_height: f32,
    pub captions: bool,
    /// A line of facts under each photo.
    pub facts: bool,
    /// The note line and the action row under the photos.
    pub footer: bool,
}

/// The narrowest a card gets, so its header and pills still fit.
const MIN_CARD: f32 = 168.0;
const PAD: f32 = S2;
const HEADER: f32 = 18.0;
/// Between the header and the photos.
const HEADER_GAP: f32 = 6.0;
/// Between photos on one card.
const PHOTO_GAP: f32 = S1 + 2.0;
const CAPTION: f32 = 16.0;
const NOTE: f32 = 18.0;
const FACTS: f32 = 16.0;
/// A section header's row in a sectioned grid.
const SECTION: f32 = 36.0;
const ACTIONS: f32 = 28.0;
const RADIUS: f32 = 8.0;

impl CardLayout {
    pub fn height(&self) -> f32 {
        PAD + HEADER
            + HEADER_GAP
            + self.photo_height
            + if self.captions { CAPTION } else { 0.0 }
            + if self.facts { FACTS } else { 0.0 }
            + if self.footer {
                S1 + NOTE + S2 + ACTIONS
            } else {
                0.0
            }
            + PAD
    }
}

/// A card's place in a justified row: its photos' shapes side by side, the
/// padding around them, and the narrowest its header allows.
pub(super) fn card_tile(aspects: &[f32]) -> super::photo_rows::Tile {
    super::photo_rows::Tile {
        aspect: aspects.iter().sum(),
        extra: 2.0 * PAD + PHOTO_GAP * aspects.len().saturating_sub(1) as f32,
        min_width: MIN_CARD,
    }
}

impl CardState {
    fn label(self, text: Option<&str>) -> (String, Color32) {
        match self {
            Self::Suggested => (text.unwrap_or_default().to_owned(), MUTED),
        }
    }
}

impl PhotoSortApp {
    /// The grids' scroll area. It restores a saved offset, or follows the
    /// current card whenever it changes, as Review's strip follows its photo.
    /// `span` gives a card's top and height in the scrolled content.
    fn card_scroll(
        &mut self,
        ui: &egui::Ui,
        salt: &str,
        current: Option<usize>,
        span: impl Fn(usize) -> (f32, f32),
    ) -> egui::ScrollArea {
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("card_grid", salt))
            .auto_shrink([false, false]);
        if let Some(offset) = self.library_restore_offset.take() {
            return scroll.vertical_scroll_offset(offset);
        }
        let key = egui::Id::new(("card_grid_current", salt));
        let previous = ui.data(|d| d.get_temp::<Option<usize>>(key));
        ui.data_mut(|d| d.insert_temp(key, current));
        if previous != Some(current)
            && let Some(position) = current
        {
            let (top, height) = span(position);
            let offset = egui::scroll_area::State::load(
                ui.ctx(),
                ui.make_persistent_id(egui::IdSalt::new(("card_grid", salt))),
            )
            .map_or(0.0, |state| state.offset.y);
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + height > offset + ui.available_height() {
                scroll =
                    scroll.vertical_scroll_offset((top + height - ui.available_height()).max(0.0));
            }
        }
        scroll
    }

    /// A virtualized grid of review cards in sections, each under a header
    /// row that `header` draws. Cards sit in justified rows: the photos of
    /// a row share one height, near the shared photo size, and each photo
    /// is as wide as its shape needs, so it fills its slot. `aspects` gives
    /// a card's photos' widths over their heights, from what is known
    /// without drawing it, and `facts` whether it shows a line of facts
    /// under its photos; a row keeps that line only when one of its cards
    /// needs it. Cards are numbered across sections, `sections` holding each
    /// one's count. Returns the card and event clicked this frame, or a
    /// click on the empty space around the cards.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sectioned_card_grid(
        &mut self,
        ui: &mut egui::Ui,
        salt: &str,
        sections: &[usize],
        aspects: impl Fn(&Self, usize) -> Vec<f32>,
        facts: impl Fn(&Self, usize) -> bool,
        header: impl FnMut(&mut Self, &mut egui::Ui, usize),
        card: impl FnMut(&mut Self, usize) -> Option<ReviewCard>,
        current: Option<usize>,
    ) -> Option<GridClick> {
        self.rows_card_grid(
            ui, salt, sections, true, aspects, facts, header, card, current,
        )
    }

    /// `sectioned_card_grid` with one section and no header.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn flat_card_grid(
        &mut self,
        ui: &mut egui::Ui,
        salt: &str,
        count: usize,
        aspects: impl Fn(&Self, usize) -> Vec<f32>,
        facts: impl Fn(&Self, usize) -> bool,
        card: impl FnMut(&mut Self, usize) -> Option<ReviewCard>,
        current: Option<usize>,
    ) -> Option<GridClick> {
        self.rows_card_grid(
            ui,
            salt,
            &[count],
            false,
            aspects,
            facts,
            |_, _, _| {},
            card,
            current,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn rows_card_grid(
        &mut self,
        ui: &mut egui::Ui,
        salt: &str,
        sections: &[usize],
        headers: bool,
        aspects: impl Fn(&Self, usize) -> Vec<f32>,
        facts: impl Fn(&Self, usize) -> bool,
        mut header: impl FnMut(&mut Self, &mut egui::Ui, usize),
        mut card: impl FnMut(&mut Self, usize) -> Option<ReviewCard>,
        current: Option<usize>,
    ) -> Option<GridClick> {
        enum Row {
            Header(usize),
            Cards(super::photo_rows::PhotoRow, CardLayout),
        }
        let gap = S2;
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        let width = ui.available_width();
        let target = self.photo_size();
        let captions = self.preferences.show_filenames;

        // (top, height, row) in the scrolled content.
        let mut rows: Vec<(f32, f32, Row)> = Vec::new();
        let mut spans = Vec::new();
        let mut top = 0.0;
        let mut start = 0;
        for (section, &len) in sections.iter().enumerate() {
            let header_top = top;
            if headers {
                rows.push((top, SECTION, Row::Header(section)));
                top += SECTION;
            }
            let tiles: Vec<_> = (start..start + len)
                .map(|i| card_tile(&aspects(self, i)))
                .collect();
            for (n, mut row) in super::photo_rows::justify(&tiles, width, target, gap)
                .into_iter()
                .enumerate()
            {
                row.range = row.range.start + start..row.range.end + start;
                let layout = CardLayout {
                    photo_height: row.height,
                    captions,
                    facts: row.range.clone().any(|i| facts(self, i)),
                    footer: false,
                };
                let height = layout.height();
                for _ in row.range.clone() {
                    // The first row keeps its section's header in view.
                    spans.push(if n == 0 {
                        (header_top, top + height - header_top)
                    } else {
                        (top, height)
                    });
                }
                rows.push((top, height, Row::Cards(row, layout)));
                top += height + gap;
            }
            start += len;
        }
        let scroll = self.card_scroll(ui, salt, current, |position| {
            spans.get(position).copied().unwrap_or((0.0, 0.0))
        });
        let mut clicked = None;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            ui.set_min_height(top);
            let origin = ui.max_rect().min;
            // Under every card and header, which take their own clicks.
            let background = egui::Rect::from_min_max(
                origin + viewport.min.to_vec2(),
                origin + egui::vec2(width, viewport.max.y),
            );
            if ui
                .interact(background, ui.id().with("grid_background"), Sense::click())
                .clicked()
            {
                clicked = Some(GridClick::Background);
            }
            for (row_top, height, row) in &rows {
                if row_top + height < viewport.min.y || *row_top > viewport.max.y {
                    continue;
                }
                let y = origin.y + row_top;
                match row {
                    Row::Header(section) => {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(origin.x, y),
                            egui::vec2(width, SECTION),
                        );
                        ui.scope_builder(
                            egui::UiBuilder::new()
                                .id_salt((salt, "section", *section))
                                .max_rect(rect)
                                .layout(egui::Layout::left_to_right(Align::Center)),
                            |ui| header(self, ui, *section),
                        );
                    }
                    Row::Cards(row, layout) => {
                        for (index, &(left, card_width)) in row.range.clone().zip(&row.tiles) {
                            let Some(spec) = card(self, index) else {
                                continue;
                            };
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(origin.x + left, y),
                                egui::vec2(card_width, *height),
                            );
                            let shapes = aspects(self, index);
                            let event = ui
                                .scope_builder(
                                    egui::UiBuilder::new().id_salt((salt, index)).max_rect(rect),
                                    |ui| self.review_card(ui, rect, &spec, *layout, &shapes),
                                )
                                .inner;
                            if let Some(event) = event {
                                clicked = Some(GridClick::Card(index, event));
                            }
                        }
                    }
                }
            }
        });
        self.library_offset = output.state.offset.y;
        clicked
    }

    /// Draws one card in `rect`. Returns what was clicked on it.
    pub(super) fn review_card(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        card: &ReviewCard,
        layout: CardLayout,
        // Each photo's width over its height; a photo without one is 4:3.
        aspects: &[f32],
    ) -> Option<CardEvent> {
        let mut event = None;
        let response = ui.interact(rect, ui.id().with("card"), Sense::click());
        let photos = card
            .photos
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>()
            .join(" and ");
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                card.current,
                format!("{}: {photos}", card.title),
            )
        });
        let hover = ui
            .ctx()
            .animate_bool_responsive(response.id.with("hover"), response.hovered());
        let shown = ui
            .ctx()
            .animate_bool_responsive(response.id.with("shown"), card.shown);
        let fill = lerp_color(CARD, CARD_HOVER, hover.max(shown));
        let painter = ui.painter_at(rect.expand(4.0));
        painter.rect_filled(rect, R_TILE, fill);
        if shown > 0.0 {
            painter.rect_stroke(
                rect,
                R_TILE,
                Stroke::new(1.5, INK.gamma_multiply(0.35 * shown)),
                egui::StrokeKind::Inside,
            );
        }
        ring(ui, response.id, rect, R_TILE, card.current, false);
        if response.double_clicked() {
            event = Some(CardEvent::Open(0));
        } else if response.clicked() {
            event = Some(CardEvent::Select(0));
        }

        // Header: what the suggestion is, and how settled.
        let inner = rect.shrink(PAD);
        let header = egui::Rect::from_min_size(inner.min, egui::vec2(inner.width(), HEADER));
        let mut right = header.right();
        // Without a footer, a card's buttons are icons at the header's right.
        if !layout.footer {
            for (index, button) in card.buttons.iter().enumerate().rev() {
                let ink = if button.enabled { MUTED } else { FAINT };
                let glyph =
                    painter.layout_no_wrap(button.text.clone(), FontId::proportional(15.0), ink);
                let rect = Align2::RIGHT_CENTER.anchor_size(
                    egui::pos2(right, header.center().y),
                    egui::vec2(glyph.size().x + 10.0, HEADER + 4.0),
                );
                let response = ui
                    .interact(rect, ui.id().with(("button", index)), Sense::click())
                    .on_hover_text(&button.hover);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        button.enabled,
                        &button.hover,
                    )
                });
                if button.enabled && response.hovered() {
                    painter.rect_filled(rect, R_CONTROL, ACCENT_SOFT);
                }
                painter.galley(rect.center() - glyph.size() / 2.0, glyph, ink);
                if button.enabled && response.clicked() {
                    event = Some(CardEvent::Button(index));
                }
                right = rect.left() - 2.0;
            }
            if !card.buttons.is_empty() {
                right -= S1;
            }
        }
        if let Some((label, hover)) = &card.undo {
            let undo = painter.layout_no_wrap(
                format!("{}  {label}", egui_phosphor::regular::ARROW_U_UP_LEFT),
                medium(11.0),
                ACCENT,
            );
            let link = Align2::RIGHT_CENTER.anchor_size(
                egui::pos2(right, header.center().y),
                undo.size() + egui::vec2(8.0, 4.0),
            );
            let response = ui
                .interact(link, ui.id().with("undo"), Sense::click())
                .on_hover_text(hover)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            if response.hovered() {
                painter.rect_filled(link, R_CONTROL, ACCENT_SOFT);
            }
            painter.galley(link.min + egui::vec2(4.0, 2.0), undo, ACCENT);
            if response.clicked() {
                event = Some(CardEvent::Undo);
            }
            right = link.left() - S2;
        }
        let (state, color) = card.state.label(card.state_text.as_deref());
        let state = painter.layout_no_wrap(state, medium(11.0), color);
        painter.galley(
            egui::pos2(
                right - state.size().x,
                header.center().y - state.size().y / 2.0,
            ),
            state.clone(),
            color,
        );
        let mut title =
            egui::text::LayoutJob::simple_singleline(card.title.clone(), semibold(13.0), INK);
        title.wrap = egui::text::TextWrapping::truncate_at_width(
            (right - header.left() - state.size().x - S3).max(S6),
        );
        let title = painter.layout_job(title);
        let title_rect = egui::Rect::from_min_size(
            header.left_center() - egui::vec2(0.0, title.size().y / 2.0),
            title.size(),
        );
        painter.galley(title_rect.min, title, INK);
        if let Some(hover) = &card.title_hover {
            ui.interact(title_rect, ui.id().with("title"), Sense::hover())
                .on_hover_text(hover);
        }

        // Photos, side by side, each as wide as its shape needs; on a card
        // wider than its photos, as for a header, they are centred.
        let top = header.bottom() + HEADER_GAP;
        let widths: Vec<f32> = (0..card.photos.len())
            .map(|i| aspects.get(i).copied().unwrap_or(4.0 / 3.0) * layout.photo_height)
            .collect();
        let used = widths.iter().sum::<f32>() + PHOTO_GAP * widths.len().saturating_sub(1) as f32;
        // Rounding can leave the photos a hair wider than the card: they
        // share what there is.
        let scale = (inner.width() / used).min(1.0);
        let mut left = inner.left() + ((inner.width() - used) / 2.0).max(0.0);
        for (side, photo) in card.photos.iter().enumerate() {
            let slot = egui::Rect::from_min_size(
                egui::pos2(left, top),
                egui::vec2(widths[side] * scale, layout.photo_height),
            );
            left = slot.right() + PHOTO_GAP;
            let (pill, mark) = self.card_photo(ui, &painter, slot, fill, photo);
            let response = ui.interact(slot, ui.id().with(("photo", side)), Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::Button,
                    true,
                    format!(
                        "Photo {}: {} · {}",
                        side + 1,
                        photo.name,
                        match &photo.badge {
                            Badge::Keep(text) | Badge::Trash(text) => text,
                            Badge::None => "",
                        }
                    ),
                )
            });
            // A photo with a Keep/Trash switch is the switch itself; the
            // card around it selects the card.
            if let Some(toggle) = &photo.toggle {
                let response = response
                    .on_hover_text(format!("{}\n{toggle}", photo.name))
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if response.clicked() {
                    event = Some(CardEvent::Toggle(side));
                }
            } else {
                let response = response.on_hover_text(&photo.name);
                if response.double_clicked() {
                    event = Some(CardEvent::Open(side));
                } else if response.clicked() {
                    event = Some(CardEvent::Select(side));
                }
            }
            // Over the photo's own hover area, so its tooltip wins there.
            if let (Some(mark), Some(hover)) = (mark, &photo.suggested_hover) {
                ui.interact(mark, ui.id().with(("suggested", side)), Sense::hover())
                    .on_hover_text(hover);
            }
            if let Some(hover) = &photo.toggle {
                let response = ui
                    .interact(pill, ui.id().with(("pill", side)), Sense::click())
                    .on_hover_text(hover)
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, hover)
                });
                if response.hovered() {
                    painter.rect_stroke(
                        pill.expand(1.0),
                        R_CONTROL,
                        Stroke::new(1.5, Color32::WHITE),
                        egui::StrokeKind::Outside,
                    );
                }
                if response.clicked() {
                    event = Some(CardEvent::Toggle(side));
                }
            }
            if layout.facts && !photo.facts.is_empty() {
                let top = slot.bottom() + if layout.captions { CAPTION } else { 0.0 };
                let mut job = egui::text::LayoutJob::default();
                for (i, fact) in photo.facts.iter().enumerate() {
                    if i > 0 {
                        job.append(
                            " · ",
                            0.0,
                            egui::TextFormat::simple(FontId::proportional(11.0), FAINT),
                        );
                    }
                    job.append(
                        &fact.text,
                        0.0,
                        if fact.better {
                            egui::TextFormat::simple(medium(11.0), GREEN)
                        } else {
                            egui::TextFormat::simple(FontId::proportional(11.0), MUTED)
                        },
                    );
                }
                job.wrap = egui::text::TextWrapping::truncate_at_width(slot.width());
                let galley = painter.layout_job(job);
                let at = egui::pos2(
                    slot.center().x - galley.size().x / 2.0,
                    top + (FACTS - galley.size().y) / 2.0,
                );
                let rect = egui::Rect::from_min_size(at, galley.size());
                painter.galley(at, galley, MUTED);
                let full = photo
                    .facts
                    .iter()
                    .map(|f| f.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" · ");
                ui.interact(rect, ui.id().with(("facts", side)), Sense::hover())
                    .on_hover_text(full);
            }
            if layout.captions && !photo.caption.is_empty() {
                let mut job = egui::text::LayoutJob::simple_singleline(
                    photo.caption.clone(),
                    FontId::proportional(11.0),
                    MUTED,
                );
                // A lone photo's name may use the card's width.
                job.wrap = egui::text::TextWrapping::truncate_at_width(if card.photos.len() == 1 {
                    inner.width()
                } else {
                    slot.width()
                });
                let galley = painter.layout_job(job);
                painter.galley(
                    egui::pos2(
                        slot.center().x - galley.size().x / 2.0,
                        slot.bottom() + (CAPTION - galley.size().y) / 2.0,
                    ),
                    galley,
                    MUTED,
                );
            }
        }

        if !layout.footer {
            return event;
        }
        // Why: the reason or confidence, in one line.
        let note = egui::Rect::from_min_size(
            egui::pos2(
                inner.left(),
                top + layout.photo_height
                    + if layout.captions { CAPTION } else { 0.0 }
                    + if layout.facts { FACTS } else { 0.0 }
                    + S1,
            ),
            egui::vec2(inner.width(), NOTE),
        );
        let text = if card.warning {
            format!("{}  {}", egui_phosphor::regular::WARNING, card.note)
        } else {
            card.note.clone()
        };
        ui.put(
            note,
            egui::Label::new(RichText::new(text).size(11.5).color(if card.warning {
                ACCENT
            } else {
                MUTED
            }))
            .truncate(),
        )
        .on_hover_text(&card.note);

        // The same quick actions as Review's bar.
        let actions = egui::Rect::from_min_size(
            egui::pos2(inner.left(), note.bottom() + S2),
            egui::vec2(inner.width(), ACTIONS),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(actions)
                .layout(egui::Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = S1 + 2.0;
                let mut add = |ui: &mut egui::Ui, index: usize, button: &CardButton| {
                    let widget = if button.primary {
                        primary(button.text.clone())
                    } else {
                        egui::Button::new(button.text.clone())
                    };
                    if ui
                        .add_enabled(button.enabled, widget)
                        .on_hover_text(&button.hover)
                        .clicked()
                    {
                        event = Some(CardEvent::Button(index));
                    }
                };
                for (index, button) in card.buttons.iter().enumerate() {
                    if !button.trailing {
                        add(ui, index, button);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    // Right to left: the last trailing button sits rightmost.
                    for (index, button) in card.buttons.iter().enumerate().rev() {
                        if button.trailing {
                            add(ui, index, button);
                        }
                    }
                });
            },
        );
        event
    }

    /// A card's photo, fitted to its slot on a dark well, with its pill.
    /// A photo going to Trash is edged in red, so a page of cards can be
    /// scanned at a glance, and stays at full brightness so it can still be
    /// compared with the keeper. Returns the pill's rect.
    fn card_photo(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        slot: egui::Rect,
        fill: Color32,
        photo: &CardPhoto,
    ) -> (egui::Rect, Option<egui::Rect>) {
        use egui_phosphor::regular as icon;
        let id = photo.id;
        // A large card slot, as on a Retina screen, outgrows the grid
        // thumbnail: it shows the display preview once that has loaded.
        let pixels = slot.width().max(slot.height()) * ui.ctx().pixels_per_point();
        let texture = if pixels > crate::core::pipeline::THUMB_EDGE as f32 * 1.15 {
            self.texture(id, true).or_else(|| self.texture(id, false))
        } else {
            self.texture(id, false)
        };
        let shown = reveal(ui, id, texture.is_some());
        painter.rect_filled(slot, RADIUS, BG);
        if texture.is_none() || shown < 1.0 {
            painter.rect_filled(slot, RADIUS, WELL);
        }
        let trash = ui
            .ctx()
            .animate_bool_responsive(ui.id().with(("trash", id, photo.turns)), photo.trash);
        if let Some(texture) = texture {
            let drawn = paint_image_tinted(
                ui,
                &texture,
                slot,
                1.0,
                Vec2::ZERO,
                photo.turns,
                Color32::from_white_alpha((shown * 255.0) as u8),
            );
            round_corners(painter, drawn, RADIUS, BG);
            round_corners(painter, slot, RADIUS, fill);
        } else if self.requested.contains(&(id, false)) {
            shimmer(ui, slot);
        } else {
            painter.text(
                slot.center(),
                Align2::CENTER_CENTER,
                if self.failures.contains_key(&id) {
                    icon::IMAGE_BROKEN
                } else {
                    icon::IMAGE
                },
                FontId::proportional(22.0),
                FAINT,
            );
        }
        if trash > 0.0 {
            painter.rect_stroke(
                slot,
                RADIUS,
                Stroke::new(TRASH_EDGE, RED.gamma_multiply(trash)),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(inset) = &photo.inset {
            paint_inset(painter, slot, inset);
        }
        let pill = badge_pill(painter, slot, &photo.badge);
        let mark = photo.suggested.then(|| suggested_mark(painter, slot, pill));
        (pill, mark)
    }
}

/// An inset over a card photo's lower-left corner: the zoomed details,
/// stacked, on a dark plate with their label under them. Neutral, so the
/// photo's Keep/Trash pill stays the only colour that decides.
fn paint_inset(painter: &egui::Painter, slot: egui::Rect, inset: &Inset) {
    use egui_phosphor::regular as icon;
    if inset.strips.is_empty() {
        return;
    }
    let label = painter.layout_no_wrap(
        format!(
            "{}  {}",
            if inset.closed {
                icon::EYE_CLOSED
            } else {
                icon::EYE
            },
            inset.label
        ),
        FontId::proportional(10.5),
        INK.gamma_multiply(0.9),
    );
    let (margin, pad, gap) = (6.0, 3.0, 2.0);
    let aspects: Vec<f32> = inset
        .strips
        .iter()
        .map(|(_, aspect)| aspect.clamp(1.5, 6.0))
        .collect();
    let band = label.size().y + 4.0;
    // As wide as fits, then narrower until the stack fits under the pill.
    let mut width = (slot.width() * 0.42)
        .min(170.0)
        .max(label.size().x)
        .min(slot.width() - 2.0 * (margin + pad));
    let stack = |width: f32| {
        aspects.iter().map(|a| width / a).sum::<f32>() + gap * (aspects.len() - 1) as f32
    };
    let room = slot.height() * 0.45 - band - 2.0 * pad;
    if stack(width) > room {
        width *= room / stack(width);
    }
    let rect = egui::Rect::from_min_size(
        egui::pos2(
            slot.left() + margin,
            slot.bottom() - margin - stack(width) - band - 2.0 * pad,
        ),
        egui::vec2(
            width.max(label.size().x) + 2.0 * pad,
            stack(width) + band + 2.0 * pad,
        ),
    );
    painter.rect_filled(rect, R_CONTROL, Color32::from_black_alpha(150));
    let mut top = rect.top() + pad;
    for ((image, _), aspect) in inset.strips.iter().zip(&aspects) {
        let strip = egui::Rect::from_min_size(
            egui::pos2(rect.left() + pad, top),
            egui::vec2(width, width / aspect),
        );
        match image {
            Some(face) => painter.image(face.texture, strip, face.uv, Color32::WHITE),
            None => painter.rect_filled(strip, 2.0, WELL),
        };
        top = strip.bottom() + gap;
    }
    painter.galley(
        egui::pos2(rect.left() + pad, rect.bottom() - pad - label.size().y),
        label,
        INK,
    );
}

/// The pill over a card photo's top-right corner. Returns its rect.
fn badge_pill(painter: &egui::Painter, slot: egui::Rect, badge: &Badge) -> egui::Rect {
    use egui_phosphor::regular as icon;
    let (text, fill, ink) = match badge {
        Badge::Keep(text) => (format!("{}  {text}", icon::CHECK), GREEN, Color32::BLACK),
        Badge::Trash(text) => (format!("{}  {text}", icon::TRASH), RED, Color32::BLACK),
        Badge::None => return Align2::RIGHT_TOP.anchor_size(slot.right_top(), Vec2::ZERO),
    };
    let galley = painter.layout_no_wrap(text, medium(11.0), ink);
    let size = galley.size() + egui::vec2(16.0, 6.0);
    let rect =
        Align2::RIGHT_TOP.anchor_size(slot.right_top() + egui::vec2(-S1 - 2.0, S1 + 2.0), size);
    painter.rect_filled(rect, R_CONTROL, fill);
    painter.galley(rect.min + egui::vec2(8.0, 3.0), galley, ink);
    rect
}

/// A star beside the pill of the photo PhotoSort suggests keeping, or
/// under it when the photo is too narrow for both side by side, and only
/// the star when even that does not fit. Returns its rect.
fn suggested_mark(painter: &egui::Painter, slot: egui::Rect, pill: egui::Rect) -> egui::Rect {
    let star = egui_phosphor::regular::STAR;
    let mark = |text: String| {
        let galley = painter.layout_no_wrap(text, medium(11.0), ACCENT);
        let size = galley.size() + egui::vec2(14.0, 6.0);
        (galley, size)
    };
    let (mut galley, size) = mark(format!("{star}  Suggested"));
    let mut rect = Align2::RIGHT_TOP.anchor_size(pill.left_top() - egui::vec2(S1, 0.0), size);
    if rect.left() < slot.left() + S1 {
        rect = Align2::RIGHT_TOP.anchor_size(pill.right_bottom() + egui::vec2(0.0, S1), size);
    }
    if rect.left() < slot.left() + S1 {
        let (icon, size) = mark(star.to_owned());
        galley = icon;
        rect = Align2::RIGHT_TOP.anchor_size(pill.right_bottom() + egui::vec2(0.0, S1), size);
    }
    painter.rect_filled(rect, R_CONTROL, Color32::from_black_alpha(170));
    painter.galley(rect.min + egui::vec2(7.0, 3.0), galley, ACCENT);
    rect
}

/// A photo slot's height over its width for one photo: portrait photos get
/// taller cells instead of dark bars beside them.
pub(super) fn slot_ratio(width: u32, height: u32, orientation: u8) -> f32 {
    if width == 0 || height == 0 {
        return 0.75;
    }
    let (w, h) = if (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    };
    (h as f32 / w as f32).clamp(0.56, 1.5)
}

pub(super) fn size_text(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_048_576.0;
    if megabytes >= 1024.0 {
        format!("{:.1} GB", megabytes / 1024.0)
    } else {
        format!("{megabytes:.1} MB")
    }
}

impl PhotoSortApp {
    /// A photo's width over its height as a grid shows it, from its
    /// metadata, with `turns` more quarter turns. Never from its texture,
    /// so rows do not move as thumbnails arrive.
    pub(super) fn photo_aspect(&self, id: MediaId, turns: u8) -> f32 {
        self.metadata.get(&id).map_or(4.0 / 3.0, |m| {
            super::photo_rows::aspect(m.width, m.height, m.orientation, turns)
        })
    }

    /// The photo size every grid shares: the height of its photos.
    pub(super) fn photo_size(&self) -> f32 {
        self.preferences.photo_size
    }

    /// The photo size slider every grid shows over it: small image, slider,
    /// large image. One size for every step, kept in preferences: changing
    /// it in one step changes it in all of them.
    pub(super) fn size_slider(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        ui.label(RichText::new(icon::IMAGE).size(16.0).color(MUTED));
        let mut value = self.photo_size();
        ui.spacing_mut().slider_width = 90.0;
        ui.spacing_mut().interact_size.y = 14.0;
        let response = named(
            ui.add(
                egui::Slider::new(&mut value, crate::core::preferences::PHOTO_SIZES)
                    .show_value(false),
            ),
            egui::WidgetType::Slider,
            "Photo size",
        )
        .on_hover_text("Photo size, the same in every step");
        if response.changed() {
            self.preferences.photo_size = value;
        }
        // Saved once a drag ends, not on every frame of it.
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            self.save_preferences();
        }
        ui.label(RichText::new(icon::IMAGE).size(11.0).color(MUTED));
    }
}

/// A grid with a panel that slides over it from the right.
pub(super) struct PanelLayout {
    /// The grid and the panel together.
    pub area: egui::Rect,
    /// The grid: its final width at once, so it reflows once per open or
    /// close, never once per frame of the slide.
    pub grid: egui::Rect,
    pub width: f32,
    /// How far in the panel is, 0 to 1.
    pub slide: f32,
}

/// Lays out a grid beside a panel that is `shown`, keeping room for one
/// card. `width` is the panel's width once seen or resized.
pub(super) fn panel_layout(
    ui: &egui::Ui,
    salt: &str,
    shown: bool,
    width: &mut Option<f32>,
) -> PanelLayout {
    let area = ui.available_rect_before_wrap();
    let slide = ui.ctx().animate_bool_responsive(egui::Id::new(salt), shown);
    let widest = (area.width() - 460.0).max(360.0);
    let panel = width.unwrap_or(area.width() * 0.45).clamp(360.0, widest);
    *width = Some(panel);
    PanelLayout {
        area,
        grid: if shown {
            area.with_max_x(area.right() - panel - S3)
        } else {
            area
        },
        width: panel,
        slide,
    }
}

/// The panel beside a grid, `slide` of the way in from the right edge of
/// its area, over the grid's cards while it slides out. Its left edge
/// resizes it: returns the new width while it is dragged.
pub(super) fn side_panel(
    ui: &mut egui::Ui,
    layout: &PanelLayout,
    salt: &str,
    content: impl FnOnce(&mut egui::Ui),
) -> Option<f32> {
    let (area, width) = (layout.area, layout.width);
    let rect = egui::Rect::from_min_size(
        egui::pos2(area.right() - (width + S3) * layout.slide + S3, area.top()),
        egui::vec2(width, area.height()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .id_salt(salt)
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
                    content(ui);
                });
        },
    );
    let handle = egui::Rect::from_x_y_ranges(
        rect.left() - 4.0..=rect.left() + 2.0,
        rect.top()..=rect.bottom(),
    );
    let response = ui
        .interact(handle, egui::Id::new((salt, "resize")), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    response.dragged().then(|| width - response.drag_delta().x)
}

/// Every piece of text a frame painted, one per line, for tests.
#[cfg(test)]
pub(super) fn painted_text(output: &egui::FullOutput) -> String {
    fn collect(shape: &egui::Shape, out: &mut String) {
        match shape {
            egui::Shape::Text(text) => {
                out.push_str(text.galley.text());
                out.push('\n');
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    for shape in &output.shapes {
        collect(&shape.shape, &mut out);
    }
    out
}

/// Checks how a frame drew a photo marked for Trash: `texture` at full
/// brightness, nothing filled over its middle afterwards, and a red edge.
#[cfg(test)]
pub(super) fn assert_trash_drawn_bright(output: &egui::FullOutput, texture: egui::TextureId) {
    fn flatten<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
        match shape {
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| flatten(s, out)),
            shape => out.push(shape),
        }
    }
    let mut shapes = Vec::new();
    for clipped in &output.shapes {
        flatten(&clipped.shape, &mut shapes);
    }
    let at = shapes
        .iter()
        .position(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id == texture))
        .expect("the photo was drawn");
    let egui::Shape::Mesh(mesh) = shapes[at] else {
        unreachable!()
    };
    assert!(
        mesh.vertices.iter().all(|v| v.color == Color32::WHITE),
        "the photo was tinted {:?}",
        mesh.vertices[0].color
    );
    let middle = mesh.calc_bounds().center();
    assert!(
        !shapes[at + 1..].iter().any(|s| matches!(
            s,
            egui::Shape::Rect(r) if r.fill.a() > 0 && r.rect.contains(middle)
        )),
        "something was filled over the photo"
    );
    assert!(
        shapes.iter().any(|s| matches!(
            s,
            egui::Shape::Rect(r) if r.stroke.color == RED && r.stroke.width >= TRASH_EDGE
        )),
        "no red edge"
    );
}

/// Draws one frame of a guided step's grid, as the workspace would.
#[cfg(test)]
pub(super) fn render_grid(app: &mut PhotoSortApp) -> String {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    app.review = false;
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
    painted_text(&output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_reserve_lines_only_for_what_they_show() {
        let bare = CardLayout {
            photo_height: 160.0,
            captions: false,
            facts: false,
            footer: false,
        };
        // Header, the photo and padding: nothing under the photo.
        assert_eq!(bare.height(), PAD + HEADER + HEADER_GAP + 160.0 + PAD);
        let named = CardLayout {
            captions: true,
            ..bare
        };
        assert_eq!(named.height(), bare.height() + CAPTION);
        let full = CardLayout {
            facts: true,
            ..named
        };
        assert_eq!(full.height(), bare.height() + CAPTION + FACTS);
    }

    #[test]
    fn a_card_is_as_wide_as_its_photos_or_its_header() {
        let tile = card_tile(&[1.5, 2.0 / 3.0]);
        // A pair of a landscape and a portrait photo, 200 tall.
        assert!((tile.width(200.0) - (2.0 * PAD + PHOTO_GAP + 300.0 + 133.333)).abs() < 0.01);
        // One small portrait photo: the header keeps the card readable.
        assert_eq!(card_tile(&[2.0 / 3.0]).width(90.0), MIN_CARD);
    }

    #[test]
    fn the_photo_size_slider_is_shared_and_saved_once_released() {
        use std::time::{Duration, Instant};
        let root = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.path().join("cache")), None);
        let path = root.path().join("preferences.json");
        app.preferences_path = Some(path.clone());
        assert_eq!(app.photo_size(), crate::core::preferences::PHOTO_SIZE);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
            ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 100.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.horizontal(|ui| app.size_slider(ui));
                    });
                },
            )
        };
        let output = frame(&mut app, vec![]);
        let slider = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .map(|(_, node)| node)
            .find(|node| node.role() == egui::accesskit::Role::Slider)
            .unwrap();
        assert_eq!(slider.label(), Some("Photo size"));
        assert_eq!(
            slider.numeric_value(),
            Some(f64::from(crate::core::preferences::PHOTO_SIZE))
        );
        let bounds = slider.bounds().unwrap();
        let end = egui::pos2(
            bounds.x1 as f32 - 1.0,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        let press = |pressed| egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(end), press(true)]);
        let largest = *crate::core::preferences::PHOTO_SIZES.end();
        assert_eq!(app.preferences.photo_size, largest);
        // Not saved while the pointer is down ...
        assert!(!path.exists());
        frame(&mut app, vec![press(false)]);
        frame(&mut app, vec![]);
        // ... and saved once it is released, for the next launch.
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            // A missing file loads as the defaults.
            if path.exists()
                && let Ok(saved) = crate::core::preferences::Preferences::load(&path)
            {
                assert_eq!(saved.photo_size, largest);
                break;
            }
            assert!(Instant::now() < until, "the photo size was never saved");
            std::thread::sleep(Duration::from_millis(5));
        }
        app.runtime.shutdown();
    }
}
