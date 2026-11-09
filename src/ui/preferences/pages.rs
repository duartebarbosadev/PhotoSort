//! The Settings window's pages: a sidebar of categories, each a column of
//! cards whose rows put the setting on the left and its control on the right.
use super::*;
use crate::core::preferences::Preferences;
use crate::core::processing::{Backend, Check, Prediction};
use egui_phosphor::regular as icon;

/// Below this row width a control moves under its label.
const STACK_WIDTH: f32 = 440.0;
/// Below this dialog width the sidebar shows only icons.
pub(super) const COMPACT_WIDTH: f32 = 640.0;
pub(super) const SIDEBAR_WIDTH: f32 = 172.0;
pub(super) const SIDEBAR_COMPACT: f32 = 40.0;
const SLIDER_WIDTH: f32 = 132.0;
const ROW_PAD: f32 = 9.0;
const COMBO_WIDTH: f32 = 196.0;
/// Room kept for a row's control before its width is known: the widest, a
/// slider with its words, value and reset.
const CONTROL_GUESS: f32 = 320.0;

impl Section {
    pub(crate) const ALL: [Section; 5] = [
        Section::General,
        Section::Appearance,
        Section::Steps,
        Section::Performance,
        Section::Storage,
    ];
    pub(super) fn title(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Appearance => "Appearance",
            Section::Steps => "Steps",
            Section::Performance => "Performance",
            Section::Storage => "Storage",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Section::General => icon::GEAR_SIX,
            Section::Appearance => icon::PALETTE,
            Section::Steps => icon::SLIDERS_HORIZONTAL,
            Section::Performance => icon::CPU,
            Section::Storage => icon::HARD_DRIVES,
        }
    }
    /// Whether `draft` changes anything this page saves.
    pub(super) fn pending(self, draft: &Preferences, saved: &Preferences) -> bool {
        let (d, s) = (draft, saved);
        match self {
            Section::General => {
                d.updates.automatic != s.updates.automatic || d.metadata_writes != s.metadata_writes
            }
            Section::Appearance => {
                d.strip_position != s.strip_position
                    || d.show_filenames != s.show_filenames
                    || d.compact_strip != s.compact_strip
                    || d.raw_look != s.raw_look
                    || d.reduce_motion != s.reduce_motion
                    || d.show_shortcuts != s.show_shortcuts
            }
            Section::Steps => {
                d.hidden_steps != s.hidden_steps
                    || d.easy_delete_auto != s.easy_delete_auto
                    || d.pick_best_auto != s.pick_best_auto
                    || d.rotation_auto != s.rotation_auto
                    || d.easy_delete != s.easy_delete
                    || d.similarity_distance != s.similarity_distance
                    || d.subject_strictness != s.subject_strictness
                    || d.navigation != s.navigation
            }
            Section::Performance => {
                d.processing != s.processing
                    || d.cache.previews_mib != s.cache.previews_mib
                    || d.cache.originals_mib != s.cache.originals_mib
            }
            Section::Storage => d.cache.disk_mib != s.cache.disk_mib,
        }
    }
}

/// Whether Save would change anything. Fields the dialog does not own, such
/// as recent folders, never count.
pub(super) fn has_pending(draft: &Preferences, saved: &Preferences) -> bool {
    Section::ALL.iter().any(|s| s.pending(draft, saved))
}

/// The category list. Returns the category clicked.
pub(super) fn sidebar(
    ui: &mut egui::Ui,
    current: Section,
    compact: bool,
    draft: &Preferences,
    saved: &Preferences,
) -> Option<Section> {
    let width = if compact {
        SIDEBAR_COMPACT
    } else {
        SIDEBAR_WIDTH
    };
    let mut chosen = None;
    ui.vertical(|ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = 2.0;
        for section in Section::ALL {
            let pending = section.pending(draft, saved);
            if nav_item(ui, section, section == current, pending, compact, width).clicked() {
                chosen = Some(section);
            }
        }
    });
    chosen
}

fn nav_item(
    ui: &mut egui::Ui,
    section: Section,
    selected: bool,
    pending: bool,
    compact: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 32.0), egui::Sense::click());
    let title = section.title();
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, title)
    });
    let response = match (compact, pending) {
        (true, true) => response.on_hover_text(format!("{title} · unsaved changes")),
        (true, false) => response.on_hover_text(title),
        (false, true) => response.on_hover_text("Unsaved changes"),
        (false, false) => response,
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if selected {
            SELECTED
        } else if response.hovered() {
            CARD_HOVER
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect, R_CONTROL, fill);
        if response.has_focus() {
            painter.rect_stroke(
                rect,
                R_CONTROL,
                Stroke::new(1.0, ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        let color = if selected || response.hovered() {
            INK
        } else {
            MUTED
        };
        let icon_x = if compact {
            rect.center().x
        } else {
            rect.left() + 20.0
        };
        painter.text(
            egui::pos2(icon_x, rect.center().y),
            egui::Align2::CENTER_CENTER,
            section.icon(),
            egui::FontId::proportional(16.0),
            if selected { ACCENT } else { color },
        );
        if !compact {
            painter.text(
                egui::pos2(rect.left() + 38.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                if selected {
                    medium(12.5)
                } else {
                    egui::FontId::proportional(12.5)
                },
                color,
            );
        }
        if pending {
            let at = if compact {
                egui::pos2(rect.right() - 6.0, rect.top() + 6.0)
            } else {
                egui::pos2(rect.right() - 12.0, rect.center().y)
            };
            painter.circle_filled(at, 3.0, ACCENT);
        }
    }
    response
}

/// The rows of one card, with a hairline between them.
struct Rows<'u> {
    ui: &'u mut egui::Ui,
    count: usize,
}
impl Rows<'_> {
    fn separate(&mut self) {
        if self.count > 0 {
            let ui = &mut *self.ui;
            let y = ui.cursor().top();
            ui.painter()
                .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, LINE));
        }
        self.count += 1;
    }
    fn row(
        &mut self,
        label: &str,
        hint: &str,
        info: Option<&str>,
        control: impl FnOnce(&mut egui::Ui),
    ) {
        self.row_with(None, label, hint, info, control);
    }
    /// One setting: `glyph` and `label` with a one-line `hint` under them,
    /// details in an "i" tooltip, and `control` at the right.
    fn row_with(
        &mut self,
        glyph: Option<&str>,
        label: &str,
        hint: &str,
        info: Option<&str>,
        control: impl FnOnce(&mut egui::Ui),
    ) {
        self.separate();
        let ui = &mut *self.ui;
        let id = ui.id().with(("settings_row", label));
        fixed_width(ui, |ui| {
            let width = ui.available_width();
            ui.add_space(ROW_PAD);
            if width < STACK_WIDTH {
                ui.vertical(|ui| text_block(ui, glyph, label, hint, info));
                ui.add_space(S1 + 2.0);
                ui.horizontal(|ui| trailing(ui, id.with("control"), control));
            } else {
                // The text's height last frame, so the control sits centred on it.
                let remembered = ui.data(|d| d.get_temp::<f32>(id));
                let height = remembered.unwrap_or(0.0).max(ui.spacing().interact_size.y);
                // Before the control has been measured, leave it generous room so
                // nothing overflows and widens the dialog.
                let control_width = ui
                    .data(|d| d.get_temp::<f32>(id.with("control")))
                    .unwrap_or(CONTROL_GUESS);
                let text_width = (width - control_width - S6).max(80.0);
                let layout = egui::Layout::left_to_right(egui::Align::Center);
                let text = ui
                    .allocate_ui_with_layout(egui::vec2(width, height), layout, |ui| {
                        let text = ui
                            .allocate_ui_with_layout(
                                egui::vec2(text_width, height),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(text_width);
                                    text_block(ui, glyph, label, hint, info);
                                },
                            )
                            .response
                            .rect
                            .height();
                        trailing(ui, id.with("control"), control);
                        text
                    })
                    .inner;
                if remembered.is_none() {
                    ui.data_mut(|d| d.insert_temp(id, text));
                    discard_layout(ui);
                } else if (text - height).abs() > 0.5 && text > ui.spacing().interact_size.y {
                    ui.data_mut(|d| d.insert_temp(id, text));
                    ui.ctx().request_repaint();
                }
            }
            ui.add_space(ROW_PAD);
        });
    }
    /// A small secondary line under the row before it.
    fn note(&mut self, text: &str) {
        let ui = &mut *self.ui;
        ui.add_space(-ROW_PAD + 2.0);
        ui.add(egui::Label::new(RichText::new(text).small().color(FAINT)).wrap());
        ui.add_space(ROW_PAD);
    }
}

fn text_block(ui: &mut egui::Ui, glyph: Option<&str>, label: &str, hint: &str, info: Option<&str>) {
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 2.0);
    ui.horizontal(|ui| {
        if let Some(glyph) = glyph {
            paint_glyph(ui, glyph, MUTED, 15.0);
        }
        ui.add(egui::Label::new(RichText::new(label).color(INK)).wrap());
        if let Some(info) = info {
            let response = paint_glyph(ui, icon::INFO, FAINT, 14.0);
            let response = named(response, egui::WidgetType::Label, "More about this setting");
            response.on_hover_ui(|ui| {
                ui.set_max_width(320.0);
                ui.label(info);
            });
        }
    });
    if !hint.is_empty() {
        ui.add(egui::Label::new(RichText::new(hint).small().color(MUTED)).wrap());
    }
}

/// Lays `add` out left to right against the right edge, so Tab order
/// follows reading order. Uses the width the group needed last frame.
pub(super) fn trailing<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let width = ui.data(|d| d.get_temp::<f32>(id));
    if let Some(width) = width {
        ui.add_space((ui.available_width() - width).max(0.0));
    }
    let inner = ui.horizontal(add);
    let used = inner.response.rect.width();
    // A new or resized group (Cancel cache task giving way to Refresh, say)
    // would sit out of place, or widen the dialog, for a frame.
    if width.is_none_or(|width| (used - width).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, used));
        discard_layout(ui);
    }
    inner.inner
}

/// Runs `add` in a child exactly as wide as the space left. Content that is
/// too wide for a frame is clipped instead of widening the dialog.
pub(super) fn fixed_width<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width = ui.available_width();
    let rect = egui::Rect::from_min_size(
        ui.cursor().min,
        egui::vec2(width, ui.available_height().max(0.0)),
    );
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
    let clip = ui.clip_rect();
    child.set_clip_rect(egui::Rect::from_x_y_ranges(
        rect.x_range().intersection(clip.x_range()),
        clip.y_range(),
    ));
    let inner = add(&mut child);
    let height = child.min_rect().height();
    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    inner
}

/// The frame only measured something new; egui lays it out again at once
/// instead of drawing it.
fn discard_layout(ui: &egui::Ui) {
    ui.ctx().request_discard("settings rows measured");
}

/// An icon painted without a label, so assistive technology never reads the
/// icon font's private-use glyph.
fn paint_glyph(ui: &mut egui::Ui, glyph: &str, color: Color32, size: f32) -> egui::Response {
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(size + 2.0, height), egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(size),
        color,
    );
    response
}

/// A titled card of rows. A `danger` card has a red edge, for actions that
/// take effect at once.
fn card(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    subtitle: Option<&str>,
    danger: bool,
    add: impl FnOnce(&mut Rows),
) {
    ui.add_space(S4);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        paint_glyph(ui, glyph, if danger { RED } else { ACCENT }, 15.0);
        ui.label(RichText::new(title).font(semibold(13.0)).color(INK));
    });
    if let Some(subtitle) = subtitle {
        ui.add(egui::Label::new(RichText::new(subtitle).small().color(MUTED)).wrap());
    }
    ui.add_space(S1 + 2.0);
    egui::Frame::new()
        .fill(ELEVATED)
        .corner_radius(R_TILE)
        .stroke(if danger {
            Stroke::new(1.0, RED.gamma_multiply(0.4))
        } else {
            Stroke::NONE
        })
        .inner_margin(egui::Margin::symmetric(14, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(&mut Rows { ui, count: 0 });
        });
}

fn switch(ui: &mut egui::Ui, on: &mut bool, name: &str) {
    ui.add(Switch::new(on, name).hide_label());
}

/// A segmented choice between a few values.
fn choice<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[(T, &str, Option<&str>)],
) {
    let segments: Vec<Segment> = options
        .iter()
        .map(|(_, label, hover)| Segment {
            shown: (*label).to_owned(),
            name: (*label).to_owned(),
            hover: hover.map(str::to_owned),
        })
        .collect();
    let selected = options.iter().position(|(v, _, _)| v == value);
    if let Some(clicked) = segmented(
        ui,
        ui.id().with(id),
        &segments,
        selected,
        ui.spacing().interact_size.y + 4.0,
    ) {
        *value = options[clicked].0;
    }
}

/// A slider with plain words at its ends and its value after it. With
/// `reset`, a reset button (shown only when the value is not the default)
/// comes last. Returns whether reset was clicked.
#[allow(clippy::too_many_arguments)]
fn scale(
    ui: &mut egui::Ui,
    slider: egui::Slider<'_>,
    name: &str,
    value: f64,
    ends: (&str, &str),
    shown: &str,
    reset: Option<bool>,
) -> bool {
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.label(RichText::new(ends.0).small().color(MUTED));
    // A visible rail on the card, filled with the accent up to the knob.
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = SLIDER_WIDTH;
        ui.spacing_mut().slider_rail_height = 4.0;
        ui.spacing_mut().interact_size.y = 16.0;
        let visuals = ui.visuals_mut();
        visuals.widgets.inactive.bg_fill = SELECTED;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.5, INK);
        visuals.selection.bg_fill = ACCENT;
        let response = ui.add(slider.show_value(false).trailing_fill(true));
        let enabled = response.enabled();
        response.widget_info(|| egui::WidgetInfo::slider(enabled, value, name));
    });
    ui.label(RichText::new(ends.1).small().color(MUTED));
    ui.add_sized(
        [56.0, 18.0],
        egui::Label::new(RichText::new(shown).font(medium(12.0)).color(INK)),
    );
    let Some(differs) = reset else {
        return false;
    };
    let response = ui
        .add_visible(
            differs,
            egui::Button::new(RichText::new(icon::ARROW_COUNTER_CLOCKWISE).size(13.0))
                .frame(false)
                .min_size(egui::vec2(20.0, 20.0)),
        )
        .on_hover_text("Back to the default");
    named(
        response,
        egui::WidgetType::Button,
        &format!("Reset {}", name.to_lowercase()),
    )
    .clicked()
}

fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_owned()).color(RED))
        .stroke(Stroke::new(1.0, RED.gamma_multiply(0.45)))
}

/// A size in MiB in plain words.
fn size_text(mib: f64) -> String {
    if mib >= 1024.0 {
        format!("{:.1} GB", mib / 1024.0)
    } else {
        format!("{mib:.0} MB")
    }
}

fn rating_text(rating: Option<u8>) -> String {
    match rating {
        None => "Any photo".into(),
        Some(0) => "Unrated".into(),
        Some(n) => format!("{n} ★"),
    }
}

/// The page heading at the top of each category.
fn page_heading(ui: &mut egui::Ui, section: Section) {
    ui.label(
        RichText::new(section.title())
            .font(semibold(18.0))
            .color(INK),
    );
}

impl PhotoSortApp {
    pub(super) fn settings_page(
        &mut self,
        ui: &mut egui::Ui,
        section: Section,
        draft: &mut Preferences,
    ) {
        page_heading(ui, section);
        match section {
            Section::General => general_page(ui, draft),
            Section::Appearance => appearance_page(ui, draft),
            Section::Steps => steps_page(ui, draft),
            Section::Performance => self.performance_page(ui, draft),
            Section::Storage => self.storage_page(ui, draft),
        }
        ui.add_space(S4);
    }

    /// The line under Run AI on for the backend chosen in the dialog. Once
    /// models have loaded under the setting in effect, it says where they
    /// run; otherwise it says where they will run, from the accelerator check,
    /// so a choice is explained before it is applied or used.
    pub(super) fn running_on_note(&self, choice: Backend) -> String {
        let processing = &self.runtime.processing;
        let applied = self.preferences.processing.backend;
        let prediction = self.prediction(choice);
        match (processing.placement(), prediction) {
            (Some(placement), _) if choice == applied && processing.prepared_for() == applied => {
                placement.label()
            }
            (Some(placement), Ok(next)) => {
                format!("{} · next analysis on {}", placement.label(), next.target())
            }
            (Some(placement), Err(note)) => format!("{} · {note}", placement.label()),
            (None, Ok(next)) => next.label(),
            (None, Err(note)) => note,
        }
    }

    /// Where `choice` would run, or what stands in while the accelerator
    /// check runs or when it could not load ONNX Runtime.
    fn prediction(&self, choice: Backend) -> Result<Prediction, String> {
        match self.runtime.processing.accelerators() {
            Check::Done(Ok(accelerators)) => Ok(Prediction::of(choice, &accelerators)),
            Check::Done(Err(error)) => Err(format!("Could not check this computer: {error}")),
            Check::NotStarted | Check::Running => {
                Err("Checking this computer's accelerators…".into())
            }
        }
    }

    fn performance_page(&mut self, ui: &mut egui::Ui, draft: &mut Preferences) {
        use crate::core::processing::Mode;
        let ui_ctx = ui.ctx().clone();
        card(ui, icon::CPU, "AI processing", None, false, |rows| {
            rows.row(
                "Run AI on",
                "",
                Some("The first run on an accelerator can take minutes while models are prepared."),
                |ui| {
                    egui::ComboBox::from_id_salt("inference-backend")
                        .width(COMBO_WIDTH)
                        .selected_text(draft.processing.backend.label())
                        .show_ui(ui, |ui| {
                            let current = draft.processing.backend;
                            for backend in Backend::ALL
                                .into_iter()
                                .filter(|b| b.on_this_platform() || *b == current)
                            {
                                ui.selectable_value(
                                    &mut draft.processing.backend,
                                    backend,
                                    backend.label(),
                                );
                            }
                        });
                },
            );
            // The check loads ONNX Runtime once per launch, in the background.
            self.runtime.processing.check_accelerators();
            if self.runtime.processing.accelerators() == Check::Running {
                ui_ctx.request_repaint_after(std::time::Duration::from_millis(250));
            }
            let chosen = draft.processing.backend;
            rows.note(&self.running_on_note(chosen));
            let placed = self.runtime.processing.placement().is_some()
                && chosen == self.preferences.processing.backend
                && self.runtime.processing.prepared_for() == chosen;
            if !placed && let Some(reasons) = self.prediction(chosen).ok().and_then(|p| p.reasons())
            {
                rows.note(&reasons);
            }
            // Models still preparing, or stopped; the line above covers the rest.
            for status in self
                .runtime
                .processing
                .status()
                .into_iter()
                .filter(|s| s.preparing || s.detail.starts_with("Initialization stopped"))
            {
                rows.note(&format!(
                    "{} · {} · {}",
                    status.model,
                    status.backend.label(),
                    status.detail
                ));
            }
            let available = self.runtime.processing.available();
            let hint = format!(
                "{} of {} threads go to AI.",
                draft.processing.threads(available),
                available
            );
            rows.row("AI CPU use", &hint, None, |ui| {
                choice(
                    ui,
                    "cpu-mode",
                    &mut draft.processing.mode,
                    &[
                        (Mode::Balanced, "Balanced", None),
                        (Mode::Maximum, "Maximum", None),
                        (Mode::Custom, "Custom", None),
                    ],
                );
            });
            if draft.processing.mode == Mode::Custom {
                let threads = draft.processing.custom_threads;
                rows.row("AI threads", "", None, |ui| {
                    scale(
                        ui,
                        egui::Slider::new(&mut draft.processing.custom_threads, 1..=available),
                        "AI threads",
                        threads as f64,
                        ("Fewer", "More"),
                        &threads.to_string(),
                        None,
                    );
                });
            }
        });
        let defaults = crate::core::cache::Limits::default();
        card(ui, icon::MEMORY, "Memory", None, false, |rows| {
            let previews = draft.cache.previews_mib;
            rows.row("Preview memory", "", None, |ui| {
                if scale(
                    ui,
                    egui::Slider::new(
                        &mut draft.cache.previews_mib,
                        crate::core::cache::PREVIEWS_MIB,
                    )
                    .logarithmic(true),
                    "Preview memory",
                    f64::from(previews),
                    ("Less", "More"),
                    &size_text(f64::from(previews)),
                    Some(previews != defaults.previews_mib),
                ) {
                    draft.cache.previews_mib = defaults.previews_mib;
                }
            });
            let originals = draft.cache.originals_mib;
            rows.row("Full-size detail memory", "", None, |ui| {
                if scale(
                    ui,
                    egui::Slider::new(
                        &mut draft.cache.originals_mib,
                        crate::core::cache::Limits::originals_range(),
                    )
                    .logarithmic(true),
                    "Original detail memory",
                    f64::from(originals),
                    ("Less", "More"),
                    &size_text(f64::from(originals)),
                    Some(originals != defaults.originals_mib),
                ) {
                    draft.cache.originals_mib = defaults.originals_mib;
                }
            });
            rows.row("Prompts you turned off", "", None, |ui| {
                if ui
                    .add_enabled(draft.limit_prompts.any(), egui::Button::new("Show again"))
                    .clicked()
                {
                    draft.limit_prompts = Default::default();
                }
            });
        });
    }

    fn storage_page(&mut self, ui: &mut egui::Ui, draft: &mut Preferences) {
        let defaults = crate::core::cache::Limits::default();
        card(ui, icon::HARD_DRIVES, "Disk cache", None, false, |rows| {
            let disk = draft.cache.disk_mib;
            rows.row(
                "Disk cache limit",
                "Previews and AI results kept between sessions.",
                None,
                |ui| {
                    if scale(
                        ui,
                        egui::Slider::new(
                            &mut draft.cache.disk_mib,
                            0..=crate::core::cache::DISK_MAX_MIB,
                        )
                        .logarithmic(true),
                        "Disk cache",
                        f64::from(disk),
                        ("Off", "More"),
                        &if disk == 0 {
                            "Off".to_owned()
                        } else {
                            size_text(f64::from(disk))
                        },
                        Some(disk != defaults.disk_mib),
                    ) {
                        draft.cache.disk_mib = defaults.disk_mib;
                    }
                },
            );
            let cache = self.runtime.cache.status();
            // Prepared models sit outside the limit, so each gets its own
            // row and the cache reads against the limit alone.
            let limit = self.preferences.cache.disk_mib;
            let (hint, details, models) = match (&cache.usage, cache.busy) {
                (_, true) => ("Updating caches…".to_owned(), None, None),
                (Some(usage), false) => {
                    let details = format!(
                        "Cached pixels: {:.1} MiB previews · {:.1} MiB originals\n{} metadata entries · {} analysis entries\n{} files in all",
                        usage.preview_bytes as f64 / 1048576.0,
                        usage.original_bytes as f64 / 1048576.0,
                        usage.metadata_entries,
                        usage.analysis_entries,
                        usage.files
                    );
                    let cached = usage.disk_bytes.saturating_sub(usage.model_bytes);
                    let hint = if limit == 0 {
                        "Off".to_owned()
                    } else {
                        format!(
                            "{} of the {} limit.",
                            size_text(cached as f64 / 1048576.0),
                            size_text(f64::from(limit))
                        )
                    };
                    (hint, Some(details), Some(usage))
                }
                (None, false) => ("Not measured yet.".to_owned(), None, None),
            };
            rows.row("Cache space used", &hint, details.as_deref(), |ui| {
                if cache.busy {
                    if ui.button("Cancel cache task").clicked() {
                        self.runtime.cache.cancel();
                    }
                } else {
                    if icon_button(ui, icon::ARROW_CLOCKWISE, "Refresh cache usage").clicked() {
                        self.runtime.cache.request(false);
                    }
                    self.cache_folder_button(ui);
                }
            });
            // Only Core ML prepares models; elsewhere there is nothing to show.
            if let Some(usage) = models.filter(|usage| usage.model_bytes > 0) {
                let hint = format!(
                    "{}, not counted in the limit. Clear caches removes them.",
                    size_text(usage.model_bytes as f64 / 1048576.0)
                );
                rows.row("Prepared AI models", &hint, None, |_| {});
            }
            if let Some(message) = &cache.message {
                rows.note(message);
            }
        });
        let idle = !self.busy && !self.scanning;
        card(
            ui,
            icon::WARNING,
            "Clean up",
            Some("These happen right away. Cancel doesn't undo them."),
            true,
            |rows| {
                rows.row(
                    "Clear reusable caches",
                    "Reviews, staged choices and downloaded models are kept.",
                    None,
                    |ui| {
                        if ui
                            .add_enabled(
                                idle && !self.runtime.cache.status().busy,
                                danger_button("Clear caches"),
                            )
                            .clicked()
                        {
                            self.runtime.cache.request(true);
                        }
                    },
                );
                if self.forget_confirm {
                    rows.row(
                        "Forget remembered folders",
                        "Forget the step and views saved for every folder?",
                        None,
                        |ui| {
                            if ui.button("Keep them").clicked() {
                                self.forget_confirm = false;
                            }
                            if ui.add_enabled(idle, danger_button("Forget")).clicked() {
                                self.forget_remembered_folders();
                            }
                        },
                    );
                } else {
                    rows.row(
                        "Forget remembered folders",
                        "Every folder opens fresh in Organize. Staged changes are kept.",
                        None,
                        |ui| {
                            if ui.add_enabled(idle, danger_button("Forget all")).clicked() {
                                self.forget_confirm = true;
                                self.forget_message = None;
                            }
                        },
                    );
                }
                let any_recent = !self.preferences.recent_folders.is_empty();
                rows.row(
                    "Clear recent folders",
                    "The folders themselves are not touched.",
                    None,
                    |ui| {
                        if ui
                            .add_enabled(any_recent, danger_button("Clear list"))
                            .clicked()
                        {
                            self.clear_recent_folders();
                        }
                    },
                );
                if let Some(message) = self.forget_message.clone() {
                    rows.note(&message);
                }
            },
        );
    }
}

fn general_page(ui: &mut egui::Ui, draft: &mut Preferences) {
    use crate::core::metadata::WritePolicy;
    card(ui, icon::DOWNLOAD_SIMPLE, "Updates", None, false, |rows| {
        rows.row("Check for updates daily", "", None, |ui| {
            switch(ui, &mut draft.updates.automatic, "Check for updates daily")
        });
        if let Err(reason) = crate::core::updates::Source::configured() {
            rows.note(&reason);
        }
    });
    card(
        ui,
        icon::FLOPPY_DISK,
        "Saving ratings and rotation",
        None,
        false,
        |rows| {
            let hint = match draft.metadata_writes {
                WritePolicy::EmbedWhenSupported => {
                    "In an XMP sidecar when the format can't hold it or one already exists."
                }
                WritePolicy::SidecarOnly => "",
                WritePolicy::EmbedOrRotatePixels => {
                    "Files that can't store rotation, such as BMP, are re-saved turned; other data inside them is lost."
                }
            };
            rows.row("Save changes to", hint, None, |ui| {
                let label = |p: WritePolicy| match p {
                    WritePolicy::EmbedWhenSupported => "Embed where supported",
                    WritePolicy::SidecarOnly => "Sidecars only",
                    WritePolicy::EmbedOrRotatePixels => "Embed, or rotate pixels",
                };
                egui::ComboBox::from_id_salt("metadata-writes")
                    .width(COMBO_WIDTH)
                    .selected_text(label(draft.metadata_writes))
                    .show_ui(ui, |ui| {
                        for policy in [
                            WritePolicy::EmbedWhenSupported,
                            WritePolicy::SidecarOnly,
                            WritePolicy::EmbedOrRotatePixels,
                        ] {
                            ui.selectable_value(&mut draft.metadata_writes, policy, label(policy));
                        }
                    });
            });
        },
    );
}

fn appearance_page(ui: &mut egui::Ui, draft: &mut Preferences) {
    use crate::core::preferences::StripPosition;
    use crate::core::raw_look::RawLook;
    card(ui, icon::IMAGE, "Photos", None, false, |rows| {
        rows.row("Photo strip in Review", "", None, |ui| {
            choice(
                ui,
                "strip-position",
                &mut draft.strip_position,
                &[
                    (StripPosition::Bottom, "Bottom", None),
                    (StripPosition::Left, "Left", None),
                ],
            );
        });
        rows.row("Left strip", "", None, |ui| {
            choice(
                ui,
                "strip-style",
                &mut draft.compact_strip,
                &[(true, "Rows", None), (false, "Thumbnails", None)],
            );
        });
        rows.row("File names under thumbnails", "", None, |ui| {
            switch(
                ui,
                &mut draft.show_filenames,
                "Show file names under thumbnails",
            )
        });
        rows.row(
            "RAW previews",
            "Enhanced looks more like a camera's JPEG. Files are not changed.",
            None,
            |ui| {
                choice(
                    ui,
                    "raw-look",
                    &mut draft.raw_look,
                    &[
                        (RawLook::Neutral, "Neutral", None),
                        (RawLook::Enhanced, "Enhanced", None),
                    ],
                );
            },
        );
    });
    card(ui, icon::APP_WINDOW, "Interface", None, false, |rows| {
        rows.row("Keyboard shortcut hints", "", None, |ui| {
            switch(
                ui,
                &mut draft.show_shortcuts,
                "Show workflow shortcut hints",
            )
        });
        rows.row("Reduce motion", "", None, |ui| {
            switch(ui, &mut draft.reduce_motion, "Reduce motion")
        });
    });
}

fn steps_page(ui: &mut egui::Ui, draft: &mut Preferences) {
    use crate::core::subjects::Strictness;
    card(
        ui,
        icon::LIST_CHECKS,
        "Steps in your workflow",
        None,
        false,
        |rows| {
            for step in Step::ALL {
                rows.row_with(Some(step.icon()), step.title(), "", None, |ui| {
                    if matches!(step, Step::Organize | Step::Cull) {
                        ui.label(RichText::new("Always on").small().color(FAINT));
                    } else {
                        let mut visible = draft.visible(step);
                        ui.add(
                            Switch::new(&mut visible, format!("Show {}", step.title()))
                                .hide_label(),
                        );
                        if visible {
                            draft.hidden_steps.remove(&step);
                        } else {
                            draft.hidden_steps.insert(step);
                        }
                    }
                });
            }
        },
    );
    let defaults = crate::core::analysis::Thresholds::default();
    card(
        ui,
        Step::EasyDelete.icon(),
        "Easy Delete",
        None,
        false,
        |rows| {
            rows.row(
                "Apply suggestions automatically",
                "Suggested photos start marked for Trash.",
                None,
                |ui| {
                    switch(
                        ui,
                        &mut draft.easy_delete_auto,
                        "Easy Delete: apply suggestions automatically",
                    )
                },
            );
            let blur = draft.easy_delete.blur;
            rows.row("Blurry photos", "", None, |ui| {
                if scale(
                    ui,
                    egui::Slider::new(&mut draft.easy_delete.blur, 10.0..=1000.0),
                    "Blur threshold",
                    blur,
                    ("Fewer", "More"),
                    &format!("{blur:.0}"),
                    Some(blur != defaults.blur),
                ) {
                    draft.easy_delete.blur = defaults.blur;
                }
            });
            let dark = draft.easy_delete.dark;
            rows.row("Dark photos", "", None, |ui| {
                if scale(
                    ui,
                    egui::Slider::new(&mut draft.easy_delete.dark, 0.0..=128.0),
                    "Near-black ceiling",
                    dark,
                    ("Fewer", "More"),
                    &format!("{dark:.0}"),
                    Some(dark != defaults.dark),
                ) {
                    draft.easy_delete.dark = defaults.dark;
                }
            });
            let bright = draft.easy_delete.bright;
            rows.row("Bright photos", "", None, |ui| {
                // A lower threshold flags more photos, so this slider runs high to low.
                if scale(
                    ui,
                    egui::Slider::new(&mut draft.easy_delete.bright, 255.0..=128.0),
                    "Overexposed threshold",
                    bright,
                    ("Fewer", "More"),
                    &format!("{bright:.0}"),
                    Some(bright != defaults.bright),
                ) {
                    draft.easy_delete.bright = defaults.bright;
                }
            });
            rows.note("Thresholds apply from the next analysis.");
        },
    );
    card(
        ui,
        Step::Rotation.icon(),
        "Fix Rotation",
        None,
        false,
        |rows| {
            rows.row(
                "Apply suggestions automatically",
                "Photos it is sure about start turned.",
                None,
                |ui| {
                    switch(
                        ui,
                        &mut draft.rotation_auto,
                        "Fix Rotation: apply suggestions automatically",
                    )
                },
            );
        },
    );
    card(
        ui,
        Step::PickBest.icon(),
        "Pick Best",
        None,
        false,
        |rows| {
            rows.row(
                "Apply suggestions automatically",
                "All but the best of each group start marked for Trash.",
                None,
                |ui| {
                    switch(
                        ui,
                        &mut draft.pick_best_auto,
                        "Pick Best: apply suggestions automatically",
                    )
                },
            );
        },
    );
    card(
        ui,
        icon::SQUARES_FOUR,
        "Grouping similar photos",
        None,
        false,
        |rows| {
            let distance = draft.similarity_distance;
            let default = crate::core::regional::DEFAULT_EPS;
            rows.row(
                "Look-alike groups",
                "Organize and Easy Delete.",
                None,
                |ui| {
                    if scale(
                        ui,
                        egui::Slider::new(&mut draft.similarity_distance, 0.02..=0.20),
                        "Similarity distance",
                        f64::from(distance),
                        ("Stricter", "Looser"),
                        &format!("{distance:.3}"),
                        Some(distance != default),
                    ) {
                        draft.similarity_distance = default;
                    }
                },
            );
            rows.row("Same-subject groups", "Cull and Pick Best.", None, |ui| {
                choice(
                    ui,
                    "subject-strictness",
                    &mut draft.subject_strictness,
                    &[
                        (
                            Strictness::Conservative,
                            "Conservative",
                            Some("Keeps uncertain matches apart"),
                        ),
                        (Strictness::Standard, "Standard", None),
                        (Strictness::Broad, "Broad", Some("Accepts more matches")),
                    ],
                );
            });
            rows.note("Applies from the next analysis.");
        },
    );
    card(ui, Step::Cull.icon(), "Cull", None, false, |rows| {
        rows.row("Up/Down stops at", "", None, |ui| {
            egui::ComboBox::from_id_salt("navigation-rating")
                .width(COMBO_WIDTH)
                .selected_text(rating_text(draft.navigation.rating))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut draft.navigation.rating, None, rating_text(None));
                    for rating in 0..=5 {
                        ui.selectable_value(
                            &mut draft.navigation.rating,
                            Some(rating),
                            rating_text(Some(rating)),
                        );
                    }
                });
        });
        rows.row("Up/Down skips groups of one", "", None, |ui| {
            switch(
                ui,
                &mut draft.navigation.skip_singletons,
                "Up/Down skips groups of one",
            )
        });
        rows.row("Left/Right stays in a similar group", "", None, |ui| {
            switch(
                ui,
                &mut draft.navigation.cycle_similarity,
                "Left/Right stays in a similar group",
            )
        });
    });
}
