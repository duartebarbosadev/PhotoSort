//! Keyboard hints in the bottom bar and the shortcut sheet behind its
//! Shortcuts button. Keys are drawn as small keycaps; each step lists its
//! hints most useful first, so a narrow bar keeps the ones that matter.
use super::*;

/// One key on a keycap. Modifiers and arrows are drawn as the platform's
/// glyphs; everything else as its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cap {
    Alt,
    Shift,
    /// Cmd on macOS, Ctrl elsewhere.
    Command,
    Enter,
    Esc,
    Left,
    Right,
    Up,
    Down,
    Key(&'static str),
}
use Cap::*;

/// A shortcut: one or more alternative key combinations and what they do.
pub(super) struct Hint {
    pub keys: &'static [&'static [Cap]],
    pub action: &'static str,
}
const fn hint(keys: &'static [&'static [Cap]], action: &'static str) -> Hint {
    Hint { keys, action }
}

const ARROWS: &[Cap] = &[Left, Right, Up, Down];
const REVIEW: Hint = hint(&[&[Shift, Enter]], "Review");
const CLOSE: Hint = hint(&[&[Esc]], "Close photos");
const COPY: Hint = hint(&[&[Command, Key("C")]], "Copy");

const ORGANIZE: &[Hint] = &[
    hint(&[ARROWS], "Browse"),
    REVIEW,
    hint(&[&[Key("F2")]], "Rename"),
    hint(&[&[Key("\\")]], "Hold for original"),
    hint(&[&[Shift, Left, Right, Up, Down]], "Compare"),
    hint(&[&[Alt, Key("1–5")]], "Grouping mode"),
    COPY,
];
const EASY_DELETE: &[Hint] = &[
    hint(&[&[Key("1")], &[Left]], "Keep"),
    hint(&[&[Key("2")], &[Right]], "Trash"),
    hint(&[&[Up, Down]], "Browse"),
    hint(&[&[Enter]], "Next"),
    hint(&[&[Key("R")]], "Undo"),
    REVIEW,
    hint(&[&[Key("A")]], "Accept all suggestions"),
    CLOSE,
    COPY,
];
const ROTATION: &[Hint] = &[
    hint(&[&[Key("Q")], &[Key("E")]], "Rotate"),
    hint(&[ARROWS], "Browse"),
    hint(&[&[Enter]], "Next"),
    hint(&[&[Key("R")]], "Undo"),
    REVIEW,
    CLOSE,
    COPY,
];
const PICK_BEST: &[Hint] = &[
    hint(&[&[Key("1–9")]], "Keep or trash"),
    hint(&[ARROWS], "Group"),
    hint(&[&[Enter]], "Show or next"),
    hint(&[&[Key("R")]], "Undo"),
    REVIEW,
    hint(&[&[Key("A")]], "Accept all suggestions"),
    CLOSE,
    COPY,
];
const CULL: &[Hint] = &[
    hint(&[&[Left, Right]], "Cycle"),
    hint(&[&[Up, Down]], "Browse"),
    hint(&[&[Key("D")]], "Mark"),
    hint(&[&[Command, Key("0–5")]], "Rate"),
    hint(&[&[Key("R")]], "Rotate"),
    REVIEW,
    hint(&[&[Command, Left, Right, Up, Down]], "Bypass filters"),
    COPY,
];
/// Shortcuts that work on every step, for the sheet.
const STEPS: &[Hint] = &[
    hint(&[&[Command, Alt, Key("1–5")]], "Switch to a step"),
    hint(&[&[Shift, Enter]], "Review changes"),
    hint(&[&[Command, Key("C")]], "Copy photos"),
];
const ANYWHERE: &[Hint] = &[
    hint(&[&[Command, Key("O")]], "Open folder"),
    hint(&[&[Command, Key("F")]], "Search"),
    hint(&[&[Command, Key("S")]], "Analyze"),
    hint(&[&[Key("I")]], "Photo details"),
    hint(&[&[Command, Key(",")]], "Settings"),
];

/// The step's shortcuts, most useful first.
pub(super) fn step_hints(step: Step) -> &'static [Hint] {
    match step {
        Step::Organize => ORGANIZE,
        Step::EasyDelete => EASY_DELETE,
        Step::Rotation => ROTATION,
        Step::PickBest => PICK_BEST,
        Step::Cull => CULL,
    }
}

impl Cap {
    /// The readable name, as used in plain text and accessible names.
    fn text(self) -> &'static str {
        match self {
            Alt => "Alt",
            Shift => "Shift",
            Command => "Ctrl/Cmd",
            Enter => "Enter",
            Esc => "Esc",
            Left => "Left",
            Right => "Right",
            Up => "Up",
            Down => "Down",
            Key(key) => key,
        }
    }
    /// What the keycap shows, and whether it is an icon glyph.
    fn glyph(self) -> (&'static str, bool) {
        use egui_phosphor::regular as icon;
        let mac = cfg!(target_os = "macos");
        match self {
            Alt if mac => (icon::OPTION, true),
            Shift if mac => (icon::ARROW_FAT_UP, true),
            Command if mac => (icon::COMMAND, true),
            Enter if mac => (icon::KEY_RETURN, true),
            Esc if mac => ("esc", false),
            Command => ("Ctrl", false),
            Left => (icon::ARROW_LEFT, true),
            Right => (icon::ARROW_RIGHT, true),
            Up => (icon::ARROW_UP, true),
            Down => (icon::ARROW_DOWN, true),
            other => (other.text(), false),
        }
    }
}
impl Hint {
    /// The keys in plain text, such as "Shift+Enter" or "1/Left".
    pub fn keys_text(&self) -> String {
        self.keys
            .iter()
            .map(|combo| {
                if combo == &ARROWS {
                    "Arrows".to_owned()
                } else if combo.len() == 5 && combo[1..] == *ARROWS {
                    format!("{}+arrows", combo[0].text())
                } else {
                    combo.iter().map(|c| c.text()).collect::<Vec<_>>().join("+")
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }
    /// "Shift+Enter · Review", the hint's accessible name.
    pub fn text(&self) -> String {
        format!("{} · {}", self.keys_text(), self.action)
    }
}
/// The step's hints on one line, for the keyboard guide.
pub(super) fn workflow_hint(step: Step) -> String {
    step_hints(step)
        .iter()
        .map(Hint::text)
        .collect::<Vec<_>>()
        .join("   ")
}

const CAP_HEIGHT: f32 = 17.0;
const CAP_GAP: f32 = 2.0;
/// Space between a hint's keys and its action word.
const LABEL_GAP: f32 = 5.0;
/// Space between hints.
pub(super) const HINT_GAP: f32 = 14.0;

impl Cap {
    fn arrow(self) -> bool {
        matches!(self, Left | Right | Up | Down)
    }
}
/// The keycaps of one combination, laid out: neighbouring arrow keys share
/// one cap, so "Arrows" stays as short as a word.
fn combo_galleys(ui: &egui::Ui, combo: &[Cap]) -> Vec<std::sync::Arc<egui::Galley>> {
    let mut caps = Vec::new();
    let mut i = 0;
    while i < combo.len() {
        let cap = combo[i];
        if cap.arrow() {
            let mut glyphs = String::new();
            while i < combo.len() && combo[i].arrow() {
                glyphs.push_str(combo[i].glyph().0);
                i += 1;
            }
            caps.push(
                ui.painter()
                    .layout_no_wrap(glyphs, FontId::proportional(10.0), INK),
            );
            continue;
        }
        let (glyph, icon) = cap.glyph();
        let font = if icon {
            FontId::proportional(11.0)
        } else {
            medium(10.0)
        };
        caps.push(ui.painter().layout_no_wrap(glyph.into(), font, INK));
        i += 1;
    }
    caps
}
fn cap_width(galley: &egui::Galley) -> f32 {
    (galley.size().x + 8.0).max(CAP_HEIGHT)
}
fn separator_galley(ui: &egui::Ui) -> std::sync::Arc<egui::Galley> {
    ui.painter()
        .layout_no_wrap("/".into(), FontId::proportional(10.0), FAINT)
}
fn action_galley(ui: &egui::Ui, action: &str, color: Color32) -> std::sync::Arc<egui::Galley> {
    ui.painter()
        .layout_no_wrap(action.into(), FontId::proportional(11.0), color)
}
/// Width of the keycaps alone.
fn keys_width(ui: &egui::Ui, hint: &Hint) -> f32 {
    let mut width = 0.0;
    for (i, combo) in hint.keys.iter().enumerate() {
        if i > 0 {
            width += separator_galley(ui).size().x + 2.0 * CAP_GAP;
        }
        for (j, galley) in combo_galleys(ui, combo).iter().enumerate() {
            if j > 0 {
                width += CAP_GAP;
            }
            width += cap_width(galley);
        }
    }
    width
}
/// Width of one hint: its keycaps, then its action.
pub(super) fn hint_width(ui: &egui::Ui, hint: &Hint) -> f32 {
    keys_width(ui, hint) + LABEL_GAP + action_galley(ui, hint.action, MUTED).size().x
}
/// Paints the keycaps from `left`, centred on `y`; returns where they end.
fn paint_keys(ui: &egui::Ui, hint: &Hint, left: f32, y: f32) -> f32 {
    let painter = ui.painter();
    let mut x = left;
    for (i, combo) in hint.keys.iter().enumerate() {
        if i > 0 {
            let slash = separator_galley(ui);
            x += CAP_GAP;
            painter.galley(
                egui::pos2(x, y - slash.size().y / 2.0),
                slash.clone(),
                FAINT,
            );
            x += slash.size().x + CAP_GAP;
        }
        for (j, galley) in combo_galleys(ui, combo).into_iter().enumerate() {
            if j > 0 {
                x += CAP_GAP;
            }
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, y - CAP_HEIGHT / 2.0),
                egui::vec2(cap_width(&galley), CAP_HEIGHT),
            );
            // A keycap: a raised face over a slightly darker lip.
            painter.rect_filled(rect.translate(egui::vec2(0.0, 1.0)), 4, BG);
            painter.rect(
                rect,
                4,
                CARD,
                Stroke::new(1.0, LINE),
                egui::StrokeKind::Inside,
            );
            painter.galley(rect.center() - galley.size() / 2.0, galley, INK);
            x = rect.right();
        }
    }
    x
}
/// One hint as a widget: keycaps, then the action word.
pub(super) fn hint_row(ui: &mut egui::Ui, hint: &Hint, action_color: Color32) -> egui::Response {
    let width = hint_width(ui, hint);
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, CAP_HEIGHT + 2.0), Sense::hover());
    let name = hint.text();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &name));
    if ui.is_rect_visible(rect) {
        let y = rect.center().y - 0.5;
        let end = paint_keys(ui, hint, rect.left(), y);
        let action = action_galley(ui, hint.action, action_color);
        ui.painter().galley(
            egui::pos2(end + LABEL_GAP, y - action.size().y / 2.0),
            action,
            action_color,
        );
    }
    response
}

impl PhotoSortApp {
    /// The Shortcuts button and the sheet it opens above the bar: the step's
    /// keys, the keys every step shares, and the app's own.
    pub(super) fn shortcuts_button(&mut self, ui: &mut egui::Ui, labelled: bool) {
        let text = shortcuts_label(labelled);
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Id::new(SHORTCUT_SHEET));
        let button = ui
            .add(
                egui::Button::new(RichText::new(text).size(11.5).color(if open {
                    INK
                } else {
                    MUTED
                }))
                .fill(if open { CARD_HOVER } else { PANEL })
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(R_CONTROL)
                .min_size(egui::vec2(26.0, 22.0)),
            )
            .on_hover_text("Keyboard shortcuts");
        let button = named(button, egui::WidgetType::Button, "Keyboard shortcuts");
        egui::Popup::menu(&button)
            .id(egui::Id::new(SHORTCUT_SHEET))
            .align(egui::RectAlign::TOP_START)
            .align_alternatives(&[egui::RectAlign::TOP_END, egui::RectAlign::TOP])
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .gap(S2)
            .frame(
                egui::Frame::popup(ui.style())
                    .fill(ELEVATED)
                    .corner_radius(R_SURFACE)
                    .inner_margin(egui::Margin::same(S3 as i8)),
            )
            .show(|ui| {
                ui.set_min_width(260.0);
                ui.spacing_mut().item_spacing.y = S1 + 2.0;
                let groups: [(&str, &[Hint]); 3] = [
                    (self.step.title(), step_hints(self.step)),
                    ("Every step", STEPS),
                    ("Anywhere", ANYWHERE),
                ];
                for (i, (title, hints)) in groups.into_iter().enumerate() {
                    if i > 0 {
                        ui.add_space(S1);
                    }
                    ui.label(caption(title));
                    // Keys in one column, actions aligned in the next.
                    let column = hints.iter().map(|h| keys_width(ui, h)).fold(0.0, f32::max);
                    // Review and Copy are listed once, under Every step.
                    let own = i == 0;
                    let shared = |h: &&Hint| own && STEPS.iter().any(|s| s.keys == h.keys);
                    for hint in hints.iter().filter(|h| !shared(h)) {
                        let action = action_galley(ui, hint.action, INK);
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(column + S3 + action.size().x, CAP_HEIGHT + 2.0),
                            Sense::hover(),
                        );
                        let name = hint.text();
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &name)
                        });
                        let y = rect.center().y - 0.5;
                        paint_keys(ui, hint, rect.left(), y);
                        ui.painter().galley(
                            egui::pos2(rect.left() + column + S3, y - action.size().y / 2.0),
                            action,
                            INK,
                        );
                    }
                }
                ui.add_space(S1);
                ui.separator();
                if ui
                    .add(
                        egui::Button::new(RichText::new("Full keyboard guide…").color(MUTED))
                            .frame(false),
                    )
                    .clicked()
                {
                    self.help = true;
                    ui.close();
                }
            });
    }
}
fn shortcuts_label(labelled: bool) -> String {
    use egui_phosphor::regular as icon;
    if labelled {
        format!("{}  Shortcuts", icon::KEYBOARD)
    } else {
        icon::KEYBOARD.to_owned()
    }
}
/// Width of the Shortcuts button, so hints know the room left beside it.
pub(super) fn shortcuts_button_width(ui: &egui::Ui, labelled: bool) -> f32 {
    let text = ui
        .painter()
        .layout_no_wrap(shortcuts_label(labelled), FontId::proportional(11.5), MUTED)
        .size()
        .x;
    (text + 2.0 * ui.spacing().button_padding.x).max(26.0)
}
/// The shortcut sheet's popup id.
pub(super) const SHORTCUT_SHEET: &str = "shortcut_sheet";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_read_as_plain_text_for_the_guide_and_assistive_technology() {
        assert_eq!(REVIEW.text(), "Shift+Enter · Review");
        assert_eq!(EASY_DELETE[0].text(), "1/Left · Keep");
        assert_eq!(ORGANIZE[4].text(), "Shift+arrows · Compare");
        assert!(workflow_hint(Step::EasyDelete).contains("Enter · Next"));
        for step in Step::ALL {
            assert!(
                step_hints(step).iter().any(|h| h.action == "Review"),
                "{step:?}"
            );
            assert!(
                workflow_hint(step).contains("Ctrl/Cmd+C · Copy"),
                "{step:?}"
            );
        }
    }
}
