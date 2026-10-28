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
    /// Drawn dimmed and red-edged: it goes to Trash.
    pub dim: bool,
    /// Makes the pill a Keep/Trash switch, with this tooltip.
    pub toggle: Option<String>,
    /// Why it is the suggested keeper, over its Suggested mark.
    pub suggested_hover: Option<String>,
}

/// One fact under a card photo, such as "4.1 MB".
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fact {
    pub text: String,
    /// Better than the other photo's value: highlighted.
    pub better: bool,
}

/// A pill over a photo: Keep or Trash, or a neutral label.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Badge {
    Keep(String),
    Trash(String),
    Plain(String),
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
    /// PhotoSort's suggestion, nothing changed or confirmed yet.
    Provisional,
    /// Changed but not confirmed: leaving it asks first.
    Changed,
    Confirmed,
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
    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn trailing(mut self) -> Self {
        self.trailing = true;
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

/// The fixed geometry of one grid's cards, so rows can be virtualized.
#[derive(Clone, Copy, Debug)]
pub(super) struct CardLayout {
    pub width: f32,
    /// Photo slots in a row; a card with fewer photos centres them.
    pub slots: usize,
    pub captions: bool,
    /// A photo slot's height over its width.
    pub ratio: f32,
    /// A line of facts under each photo.
    pub facts: bool,
    /// The note line and the action row under the photos.
    pub footer: bool,
}

/// The narrowest photo slot before a grid drops a column.
pub(super) const MIN_SLOT: f32 = 200.0;
const PAD: f32 = S3;
const HEADER: f32 = 18.0;
const CAPTION: f32 = 18.0;
const NOTE: f32 = 18.0;
const FACTS: f32 = 16.0;
/// A section header's row in a sectioned grid.
const SECTION: f32 = 40.0;
const ACTIONS: f32 = 28.0;
const RADIUS: f32 = 8.0;

impl CardLayout {
    fn image_height(&self) -> f32 {
        self.slot_width() * self.ratio
    }
    fn slot_width(&self) -> f32 {
        (self.width - 2.0 * PAD - S2 * (self.slots.saturating_sub(1)) as f32) / self.slots as f32
    }
    pub fn height(&self) -> f32 {
        PAD + HEADER
            + S2
            + self.image_height()
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

impl CardState {
    fn label(self, text: Option<&str>) -> (String, Color32) {
        use egui_phosphor::regular as icon;
        let (glyph, words, color) = match self {
            Self::Confirmed => (icon::CHECK_CIRCLE, "Confirmed", GREEN),
            Self::Changed => (icon::PENCIL_SIMPLE, "Provisional · changed", ACCENT),
            Self::Provisional => (icon::CIRCLE_DASHED, "Provisional", MUTED),
            Self::Suggested => return (text.unwrap_or_default().to_owned(), MUTED),
        };
        (format!("{glyph}  {}", text.unwrap_or(words)), color)
    }
}

impl PhotoSortApp {
    /// A virtualized grid of `count` review cards of `slots` photos each.
    /// `card` describes card `i`; the current card is scrolled into view
    /// whenever it changes. Returns the card and event clicked this frame.
    pub(super) fn card_grid(
        &mut self,
        ui: &mut egui::Ui,
        salt: &str,
        count: usize,
        slots: usize,
        mut card: impl FnMut(&mut Self, usize) -> Option<ReviewCard>,
        current: Option<usize>,
    ) -> Option<(usize, CardEvent)> {
        let gap = S3;
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        let slots = slots.max(1);
        let min = 2.0 * PAD + slots as f32 * MIN_SLOT + S2 * (slots - 1) as f32;
        let width = ui.available_width();
        let columns = ((width + gap) / (min + gap)).floor().max(1.0) as usize;
        let layout = CardLayout {
            width: (width - gap * (columns - 1) as f32) / columns as f32,
            slots,
            captions: self.preferences.show_filenames,
            ratio: 0.75,
            facts: false,
            footer: true,
        };
        let height = layout.height();
        let rows = count.div_ceil(columns);
        let scroll = self.card_scroll(ui, salt, current, |position| {
            ((position / columns) as f32 * (height + gap), height)
        });
        let mut clicked = None;
        let output = scroll.show_rows(ui, height, rows, |ui, range| {
            let origin = ui.max_rect().min;
            for row in range.clone() {
                for column in 0..columns {
                    let index = row * columns + column;
                    if index >= count {
                        break;
                    }
                    let Some(spec) = card(self, index) else {
                        continue;
                    };
                    let rect = egui::Rect::from_min_size(
                        origin
                            + egui::vec2(
                                column as f32 * (layout.width + gap),
                                (row - range.start) as f32 * (height + gap),
                            ),
                        egui::vec2(layout.width, height),
                    );
                    let event = ui
                        .scope_builder(
                            egui::UiBuilder::new().id_salt((salt, index)).max_rect(rect),
                            |ui| self.review_card(ui, rect, &spec, layout),
                        )
                        .inner;
                    if let Some(event) = event {
                        clicked = Some((index, event));
                    }
                }
            }
        });
        self.library_offset = output.state.offset.y;
        clicked
    }

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
    /// row that `header` draws. Unlike `card_grid`, rows differ in height:
    /// each is as tall as its tallest photo needs (`ratio` gives a card's
    /// slot height over width), so portrait photos fill their cells. Cards
    /// are numbered across sections, `sections` holding each one's count.
    /// Returns the card and event clicked this frame, or a click on the
    /// empty space around the cards.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sectioned_card_grid(
        &mut self,
        ui: &mut egui::Ui,
        salt: &str,
        sections: &[usize],
        slots: usize,
        // The narrowest a photo slot gets before the grid drops a column.
        min_slot: f32,
        ratio: impl Fn(&Self, usize) -> f32,
        mut header: impl FnMut(&mut Self, &mut egui::Ui, usize),
        mut card: impl FnMut(&mut Self, usize) -> Option<ReviewCard>,
        current: Option<usize>,
    ) -> Option<GridClick> {
        enum Row {
            Header(usize),
            Cards(std::ops::Range<usize>, CardLayout),
        }
        let gap = S3;
        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
        let slots = slots.max(1);
        let min = 2.0 * PAD + slots as f32 * min_slot + S2 * (slots - 1) as f32;
        let width = ui.available_width();
        let columns = ((width + gap) / (min + gap)).floor().max(1.0) as usize;
        let base = CardLayout {
            width: (width - gap * (columns - 1) as f32) / columns as f32,
            slots,
            captions: self.preferences.show_filenames,
            ratio: 0.75,
            facts: true,
            footer: false,
        };
        // (top, height, row) in the scrolled content.
        let mut rows: Vec<(f32, f32, Row)> = Vec::new();
        let mut spans = Vec::new();
        let mut top = 0.0;
        let mut start = 0;
        for (section, &len) in sections.iter().enumerate() {
            let header_top = top;
            rows.push((top, SECTION, Row::Header(section)));
            top += SECTION;
            let mut first = start;
            while first < start + len {
                let end = (first + columns).min(start + len);
                let layout = CardLayout {
                    ratio: (first..end).map(|i| ratio(self, i)).fold(0.0, f32::max),
                    ..base
                };
                let height = layout.height();
                for _ in first..end {
                    // The first row keeps its section's header in view.
                    spans.push(if first == start {
                        (header_top, top + height - header_top)
                    } else {
                        (top, height)
                    });
                }
                rows.push((top, height, Row::Cards(first..end, layout)));
                top += height + gap;
                first = end;
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
                    Row::Cards(range, layout) => {
                        for index in range.clone() {
                            let Some(spec) = card(self, index) else {
                                continue;
                            };
                            let column = index - range.start;
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(origin.x + column as f32 * (layout.width + gap), y),
                                egui::vec2(layout.width, *height),
                            );
                            let event = ui
                                .scope_builder(
                                    egui::UiBuilder::new().id_salt((salt, index)).max_rect(rect),
                                    |ui| self.review_card(ui, rect, &spec, *layout),
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
        if let Some((label, hover)) = &card.undo {
            let undo = painter.layout_no_wrap(
                format!("{}  {label}", egui_phosphor::regular::ARROW_U_UP_LEFT),
                medium(11.0),
                ACCENT,
            );
            let link = Align2::RIGHT_CENTER
                .anchor_size(header.right_center(), undo.size() + egui::vec2(8.0, 4.0));
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

        // Photos, side by side; fewer than the grid's slots are centred.
        let slot_width = layout.slot_width();
        let top = header.bottom() + S2;
        let used = card.photos.len() as f32 * (slot_width + S2) - S2;
        let lead = ((inner.width() - used) / 2.0).max(0.0);
        for (side, photo) in card.photos.iter().enumerate() {
            let slot = egui::Rect::from_min_size(
                egui::pos2(inner.left() + lead + side as f32 * (slot_width + S2), top),
                egui::vec2(slot_width, layout.image_height()),
            );
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
                            Badge::Keep(text) | Badge::Trash(text) | Badge::Plain(text) => text,
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
                job.wrap = egui::text::TextWrapping::truncate_at_width(slot.width());
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
                top + layout.image_height()
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
    /// A photo going to Trash is dimmed and edged in red, so a page of
    /// cards can be scanned at a glance. Returns the pill's rect.
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
        let dim = ui
            .ctx()
            .animate_bool_responsive(ui.id().with(("dim", id, photo.turns)), photo.dim);
        if let Some(texture) = texture {
            let light = shown * (1.0 - 0.6 * dim);
            let alpha = (shown * 255.0) as u8;
            let gray = (light * 255.0) as u8;
            let drawn = paint_image_tinted(
                ui,
                &texture,
                slot,
                1.0,
                Vec2::ZERO,
                photo.turns,
                Color32::from_rgba_premultiplied(gray, gray, gray, alpha),
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
        if dim > 0.0 {
            painter.rect_stroke(
                slot.shrink(1.0),
                RADIUS,
                Stroke::new(2.0, RED.gamma_multiply(0.85 * dim)),
                egui::StrokeKind::Inside,
            );
        }
        let pill = badge_pill(painter, slot, &photo.badge);
        let mark = photo.suggested.then(|| suggested_mark(painter, pill));
        (pill, mark)
    }
}

/// The pill over a card photo's top-right corner. Returns its rect.
fn badge_pill(painter: &egui::Painter, slot: egui::Rect, badge: &Badge) -> egui::Rect {
    use egui_phosphor::regular as icon;
    let (text, fill, ink) = match badge {
        Badge::Keep(text) => (format!("{}  {text}", icon::CHECK), GREEN, Color32::BLACK),
        Badge::Trash(text) => (format!("{}  {text}", icon::TRASH), RED, Color32::BLACK),
        Badge::Plain(text) => (text.clone(), Color32::from_black_alpha(170), INK),
    };
    let galley = painter.layout_no_wrap(text, medium(11.0), ink);
    let size = galley.size() + egui::vec2(16.0, 6.0);
    let rect =
        Align2::RIGHT_TOP.anchor_size(slot.right_top() + egui::vec2(-S1 - 2.0, S1 + 2.0), size);
    painter.rect_filled(rect, R_CONTROL, fill);
    painter.galley(rect.min + egui::vec2(8.0, 3.0), galley, ink);
    rect
}

/// A star beside the pill of the photo PhotoSort suggests keeping.
/// Returns its rect.
fn suggested_mark(painter: &egui::Painter, pill: egui::Rect) -> egui::Rect {
    let galley = painter.layout_no_wrap(
        format!("{}  Suggested", egui_phosphor::regular::STAR),
        medium(11.0),
        ACCENT,
    );
    let size = galley.size() + egui::vec2(14.0, 6.0);
    let rect = Align2::RIGHT_TOP.anchor_size(pill.left_top() - egui::vec2(S1, 0.0), size);
    painter.rect_filled(rect, R_CONTROL, Color32::from_black_alpha(170));
    painter.galley(rect.min + egui::vec2(7.0, 3.0), galley, ACCENT);
    rect
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
