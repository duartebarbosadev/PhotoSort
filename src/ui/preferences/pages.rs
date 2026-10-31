//! The Settings window's pages: a sidebar of categories, each a column of
//! cards whose rows put the setting on the left and its control on the right.
use super::*;
use crate::core::preferences::Preferences;
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
    fn subtitle(self) -> &'static str {
        match self {
            Section::General => "Updates, and where ratings and rotation are saved.",
            Section::Appearance => "How photos and the interface look.",
            Section::Steps => "Which steps you use, and how each one suggests changes.",
            Section::Performance => "How much of this computer PhotoSort uses.",
            Section::Storage => "Space used on disk, and clean-up.",
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
    ui.add(egui::Label::new(RichText::new(section.subtitle()).color(MUTED)).wrap());
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

    fn performance_page(&mut self, ui: &mut egui::Ui, draft: &mut Preferences) {
        use crate::core::processing::{Backend, Mode};
        card(ui, icon::CPU, "AI processing", None, false, |rows| {
            rows.row(
                "Run AI on",
                "Automatic picks the fastest option for this computer.",
                Some(
                    "Applies to the next analysis in every workflow. Automatic chooses a platform provider and is the default on Mac. Missing or failing providers fall back to CPU per model.\n\nAcceleration requires a compatible ONNX Runtime and driver. First model preparation can take minutes and extra memory. Native compilation finishes before cancellation can release it; cached photo results remain reusable.\n\nSession providers can still use CPU for unsupported operations; they do not prove that every operation ran on a GPU.",
                ),
                |ui| {
                    egui::ComboBox::from_id_salt("inference-backend")
                        .width(COMBO_WIDTH)
                        .selected_text(draft.processing.backend.label())
                        .show_ui(ui, |ui| {
                            for backend in Backend::ALL {
                                ui.selectable_value(
                                    &mut draft.processing.backend,
                                    backend,
                                    backend.label(),
                                );
                            }
                        });
                },
            );
            for status in self.runtime.processing.status() {
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
            rows.row(
                "AI CPU use",
                &hint,
                Some(
                    "Balanced uses 85% of the available thread count for AI. Applies to the next analysis, including rotation. Cached results stay available.\n\nThis controls AI calculations, not a limit on total app CPU use. Browsing and file tasks remain in the background.",
                ),
                |ui| {
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
                },
            );
            if draft.processing.mode == Mode::Custom {
                let threads = draft.processing.custom_threads;
                rows.row("AI threads", "How many threads AI may use.", None, |ui| {
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
        card(
            ui,
            icon::MEMORY,
            "Memory",
            Some(
                "More memory keeps photos ready to show. Displayed images and AI models use extra memory.",
            ),
            false,
            |rows| {
                let previews = draft.cache.previews_mib;
                rows.row(
                    "Preview memory",
                    "Recent previews kept ready.",
                    None,
                    |ui| {
                        if scale(
                            ui,
                            egui::Slider::new(&mut draft.cache.previews_mib, 32..=1024),
                            "Preview memory",
                            f64::from(previews),
                            ("Less", "More"),
                            &size_text(f64::from(previews)),
                            Some(previews != defaults.previews_mib),
                        ) {
                            draft.cache.previews_mib = defaults.previews_mib;
                        }
                    },
                );
                let originals = draft.cache.originals_mib;
                rows.row(
                    "Full-size detail memory",
                    "Originals kept ready for zooming in.",
                    None,
                    |ui| {
                        if scale(
                            ui,
                            egui::Slider::new(&mut draft.cache.originals_mib, 64..=2048),
                            "Original detail memory",
                            f64::from(originals),
                            ("Less", "More"),
                            &size_text(f64::from(originals)),
                            Some(originals != defaults.originals_mib),
                        ) {
                            draft.cache.originals_mib = defaults.originals_mib;
                        }
                    },
                );
            },
        );
    }

    fn storage_page(&mut self, ui: &mut egui::Ui, draft: &mut Preferences) {
        let defaults = crate::core::cache::Limits::default();
        card(ui, icon::HARD_DRIVES, "Disk cache", None, false, |rows| {
            let disk = draft.cache.disk_mib;
            rows.row(
                "Disk cache limit",
                "Previews and AI results kept between sessions.",
                Some("Limits apply when settings are saved. Zero disables persistent previews and AI evidence. AI models prepared for this computer are kept separately, whatever this limit. A model, runtime or macOS update replaces them, and clearing caches removes them. Displayed images and models use additional memory."),
                |ui| {
                    if scale(
                        ui,
                        egui::Slider::new(&mut draft.cache.disk_mib, 0..=32768).logarithmic(true),
                        "Disk cache",
                        f64::from(disk),
                        ("Off", "More"),
                        &if disk == 0 { "Off".to_owned() } else { size_text(f64::from(disk)) },
                        Some(disk != defaults.disk_mib),
                    ) {
                        draft.cache.disk_mib = defaults.disk_mib;
                    }
                },
            );
            let cache = self.runtime.cache.status();
            let (hint, details) = match (&cache.usage, cache.busy) {
                (_, true) => ("Updating caches…".to_owned(), None),
                (Some(usage), false) => {
                    let mut details = format!(
                        "Cached pixels: {:.1} MiB previews · {:.1} MiB originals\n{} metadata entries · {} analysis entries",
                        usage.preview_bytes as f64 / 1048576.0,
                        usage.original_bytes as f64 / 1048576.0,
                        usage.metadata_entries,
                        usage.analysis_entries
                    );
                    if usage.pinned_disk_bytes > 0 {
                        details.push_str(&format!(
                            "\n{:.1} MiB held by active model sessions",
                            usage.pinned_disk_bytes as f64 / 1048576.0
                        ));
                    }
                    (
                        format!(
                            "{} in {} files.",
                            size_text(usage.disk_bytes as f64 / 1048576.0),
                            usage.files
                        ),
                        Some(details),
                    )
                }
                (None, false) => ("Not measured yet.".to_owned(), None),
            };
            rows.row("Space used", &hint, details.as_deref(), |ui| {
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
            if let Some(message) = &cache.message {
                rows.note(message);
            }
        });
        let idle = !self.busy && !self.scanning;
        card(
            ui,
            icon::WARNING,
            "Clean up",
            Some("These happen right away. Cancel does not undo them."),
            true,
            |rows| {
                rows.row(
                    "Clear reusable caches",
                    "Reviews, staged choices and downloaded models are kept.",
                    Some("Clear runs immediately in the background. Current reviews, staged choices, saved positions and downloaded models are kept."),
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
                let info = "Every folder opens fresh in Organize. Staged changes and preferences are kept. The open folder keeps its views until you open it again.";
                if self.forget_confirm {
                    rows.row(
                        "Forget remembered folders",
                        "Forget the step and views saved for every folder?",
                        Some(info),
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
                        "Every folder opens fresh in Organize.",
                        Some(info),
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
                    "Empties the Recent folders list. The folders are not touched.",
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
        rows.row(
            "Check for updates daily",
            "Looks for a new version of PhotoSort once a day.",
            None,
            |ui| switch(ui, &mut draft.updates.automatic, "Check for updates daily"),
        );
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
            let (hint, info) = match draft.metadata_writes {
                WritePolicy::EmbedWhenSupported => (
                    "Inside the photo when its format allows, otherwise in a sidecar.",
                    "Existing XMP sidecars remain authoritative. Unsupported formats use a sidecar. Review shows the destination before applying; photo pixels are not re-encoded.",
                ),
                WritePolicy::SidecarOnly => (
                    "Always in an XMP sidecar next to the photo.",
                    "Existing XMP sidecars remain authoritative. Unsupported formats use a sidecar. Review shows the destination before applying; photo pixels are not re-encoded.",
                ),
                WritePolicy::EmbedOrRotatePixels => (
                    "Like embedding, but turns the pixels of files that can't store rotation.",
                    "Like Embed where supported, but rotating a file that cannot store orientation (such as BMP) rewrites its pixels instead of using a sidecar. Pixels and colour profile are kept exactly; other data stored inside those files is not. Review lists each one before applying.",
                ),
            };
            rows.row("Save changes to", hint, Some(info), |ui| {
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
        rows.row(
            "Photo strip in Review",
            "Where the filmstrip sits while you review.",
            None,
            |ui| {
                choice(
                    ui,
                    "strip-position",
                    &mut draft.strip_position,
                    &[
                        (StripPosition::Bottom, "Bottom", None),
                        (StripPosition::Left, "Left", None),
                    ],
                );
            },
        );
        rows.row(
            "File names under thumbnails",
            "In every step's grid, photo strip and Review.",
            Some("Cull's View options change the same settings."),
            |ui| {
                switch(
                    ui,
                    &mut draft.show_filenames,
                    "Show file names under thumbnails",
                )
            },
        );
        rows.row(
            "RAW previews",
            "Enhanced looks more like a camera's JPEG.",
            Some("Enhanced stretches contrast and adds a little colour to RAW previews, like a camera's JPEG. It only changes how photos look here: files, scores and suggestions are unchanged."),
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
        rows.row(
            "Keyboard shortcut hints",
            "Shows the current step's keys at the bottom of the window.",
            None,
            |ui| {
                switch(
                    ui,
                    &mut draft.show_shortcuts,
                    "Show workflow shortcut hints",
                )
            },
        );
        rows.row(
            "Reduce motion",
            "Panels, tabs and photos appear instantly instead of sliding and fading.",
            None,
            |ui| switch(ui, &mut draft.reduce_motion, "Reduce motion"),
        );
    });
}

fn steps_page(ui: &mut egui::Ui, draft: &mut Preferences) {
    use crate::core::subjects::Strictness;
    card(
        ui,
        icon::LIST_CHECKS,
        "Steps in your workflow",
        Some("Nothing changes on disk until you apply Review changes."),
        false,
        |rows| {
            for step in Step::ALL {
                let hint = match step {
                    Step::Organize => "Sort photos into folders by date, place or people.",
                    Step::EasyDelete => "Find blurry, dark and duplicate shots.",
                    Step::Rotation => "Turn photos that are on their side.",
                    Step::PickBest => "Keep the best photo of each similar group.",
                    Step::Cull => "Rate, mark and review every photo.",
                };
                rows.row_with(Some(step.icon()), step.title(), hint, None, |ui| {
                    if matches!(step, Step::Organize | Step::Cull) {
                        ui.label(RichText::new("Always on").small().color(FAINT))
                            .on_hover_text("Organize and Cull always remain available.");
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
            Some("On: what Easy Delete suggests is staged as your choice, and you change only the ones you disagree with. Off: every photo stays kept until you accept suggestions. Nothing changes on disk until you apply Review changes."),
            |ui| switch(ui, &mut draft.easy_delete_auto, "Easy Delete: apply suggestions automatically"),
        );
            let info = "Applies to future Easy Delete analyses. Current reviews stay unchanged.";
            let blur = draft.easy_delete.blur;
            rows.row("Blurry photos", "Soft or shaky shots.", Some(info), |ui| {
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
            rows.row(
            "Dark photos",
            "Shots that are almost entirely black.",
            Some("Near-black suggestions still require an almost entirely black preview. Applies to future Easy Delete analyses."),
            |ui| {
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
            },
        );
            let bright = draft.easy_delete.bright;
            rows.row(
                "Bright photos",
                "Blown-out, overexposed shots.",
                Some(info),
                |ui| {
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
                },
            );
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
            Some("On: photos Fix Rotation is sure about are staged turned, and you change only the ones you disagree with. Off: every photo stays as it is until you use a suggestion. Nothing changes on disk until you apply Review changes."),
            |ui| switch(ui, &mut draft.rotation_auto, "Fix Rotation: apply suggestions automatically"),
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
            "The best of each group is kept, the rest start marked for Trash.",
            Some("On: the best photo of each group is kept and the others are staged for Trash, and you change only the groups you disagree with. Off: every photo stays kept until you accept suggestions."),
            |ui| switch(ui, &mut draft.pick_best_auto, "Pick Best: apply suggestions automatically"),
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
            "Used by Organize and Easy Delete.",
            Some("Lower values make smaller, stricter groups. Used by the next Organize or Easy Delete analysis; current reviews stay unchanged."),
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
            rows.row(
            "Same-subject groups",
            "Used by Cull and Pick Best.",
            Some("Used by the next Cull or Pick Best analysis. Conservative keeps uncertain matches apart; Broad accepts more matches."),
            |ui| {
                choice(
                    ui,
                    "subject-strictness",
                    &mut draft.subject_strictness,
                    &[
                        (Strictness::Conservative, "Conservative", Some("Keeps uncertain matches apart")),
                        (Strictness::Standard, "Standard", None),
                        (Strictness::Broad, "Broad", Some("Accepts more matches")),
                    ],
                );
            },
        );
        },
    );
    card(ui, Step::Cull.icon(), "Cull", None, false, |rows| {
        rows.row(
            "Up/Down stops at",
            "Jump only to photos with this rating.",
            Some("Rating jumps take priority. Ctrl/Cmd bypasses movement filters and includes Trash marks. Group options use Cull analysis results; photos remain visible."),
            |ui| {
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
                            )
                            .on_hover_text(if rating == 0 {
                                "Jump to unrated photos".into()
                            } else {
                                format!("Jump to photos rated exactly {rating} stars")
                            });
                        }
                    });
            },
        );
        rows.row(
            "Skip single-photo groups",
            "Up/Down passes over groups of one.",
            None,
            |ui| {
                switch(
                    ui,
                    &mut draft.navigation.skip_singletons,
                    "Skip single-photo groups with Up/Down",
                )
            },
        );
        rows.row(
            "Stay inside similar groups",
            "Left/Right cycles within the current group.",
            None,
            |ui| {
                switch(
                    ui,
                    &mut draft.navigation.cycle_similarity,
                    "Cycle Left/Right within similarity groups",
                )
            },
        );
    });
}
