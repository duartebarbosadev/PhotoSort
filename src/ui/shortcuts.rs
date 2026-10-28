//! Application shortcuts share the existing navigation and departure actions.
use super::*;
use crate::core::browsing::tree::Mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplicationKey {
    Open,
    Find,
    About,
    Settings,
    Caches,
    Analyze,
}
fn application_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<ApplicationKey> {
    if modifiers.alt || modifiers.shift {
        return None;
    }
    let command = modifiers.command || modifiers.ctrl;
    match key {
        egui::Key::O if command => Some(ApplicationKey::Open),
        egui::Key::F if command => Some(ApplicationKey::Find),
        egui::Key::S if command => Some(ApplicationKey::Analyze),
        egui::Key::F12 if !command => Some(ApplicationKey::About),
        egui::Key::F10 if !command => Some(ApplicationKey::Settings),
        egui::Key::F9 if !command => Some(ApplicationKey::Caches),
        egui::Key::Comma if command => Some(ApplicationKey::Settings),
        _ => None,
    }
}
impl PhotoSortApp {
    /// Global controls work while search is focused. Dialog/popup/busy ownership
    /// is checked by the caller before collecting anything from the event queue.
    pub(super) fn application_shortcuts(&mut self, ctx: &egui::Context) -> bool {
        let mut requested = None;
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    modifiers,
                    pressed,
                    repeat,
                    ..
                } = event
                    && let Some(action) = application_key(*key, *modifiers)
                {
                    if *pressed && !*repeat && requested.is_none() {
                        requested = Some(action);
                    }
                    return false;
                }
                true
            })
        });
        let Some(action) = requested else {
            return false;
        };
        match action {
            ApplicationKey::Open => self.select_folder(),
            ApplicationKey::Find => {
                if self.root.is_some() && self.step != Step::PickBest {
                    let search = egui::Id::new("library_search");
                    let mut state =
                        egui::text_edit::TextEditState::load(ctx, search).unwrap_or_default();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(self.query.chars().count()),
                        )));
                    state.store(ctx, search);
                    ctx.memory_mut(|memory| memory.request_focus(search));
                }
            }
            ApplicationKey::About => self.about = true,
            ApplicationKey::Settings => self.open_settings(preferences::Section::General),
            ApplicationKey::Caches => self.open_settings(preferences::Section::Caches),
            ApplicationKey::Analyze => self.analyze_current_workflow(),
        }
        true
    }
    pub(super) fn set_library_layout(&mut self, layout: library::Layout) {
        self.library_layout = layout;
        self.review = false;
        self.library_reveal = true;
        self.pending_bookmark = None;
    }
    pub(super) fn view_shortcut(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        if modifiers.command || modifiers.ctrl || modifiers.shift {
            return false;
        }
        if self.step == Step::Organize && modifiers.alt {
            // Retain the workflow's established keys, independently of the
            // additional Date/Camera choices and their dropdown order.
            let mode = match key {
                egui::Key::Num1 => GroupMode::Current,
                egui::Key::Num2 => GroupMode::Mixed,
                egui::Key::Num3 => GroupMode::Visual,
                egui::Key::Num4 => GroupMode::Faces,
                egui::Key::Num5 => GroupMode::Location,
                _ => return false,
            };
            if self.root.is_some() {
                self.set_group_mode(mode);
            }
            return true;
        }
        if self.step != Step::Cull {
            return false;
        }
        if modifiers.alt {
            let layout = match key {
                egui::Key::Num1 => library::Layout::List,
                egui::Key::Num2 => library::Layout::Icons,
                egui::Key::Num3 => library::Layout::Grid,
                _ => return false,
            };
            self.set_library_layout(layout);
            return true;
        }
        let mode = match key {
            egui::Key::F => Mode::Folders,
            egui::Key::S => Mode::Groups,
            _ => return false,
        };
        let enabled = !self.navigator.enabled || self.navigator.mode != mode || self.review;
        self.set_navigator(mode, enabled);
        true
    }

    /// Keys that reach the photo canvas once dialogs, popups and text fields have
    /// declined them. Each stage returns whether it consumed the key.
    pub(super) fn keyboard(&mut self, ctx: &egui::Context) {
        if egui::Popup::is_any_open(ctx) || self.navigator_popup_open(ctx) || self.modal_active() {
            return;
        }
        // An inline name field in Organize keeps its keys: egui lets go of
        // its focus on Escape before this runs, and the field then cancels.
        let naming = self.step == Step::Organize && self.board.typing;
        if self.application_shortcuts(ctx) || ctx.egui_wants_keyboard_input() || naming {
            return;
        }
        self.video_keyboard(ctx);
        let guided = matches!(
            self.step,
            Step::Rotation | Step::EasyDelete | Step::PickBest
        );
        for (pressed, modifiers) in pressed_keys(ctx, guided) {
            if self.modal_active() {
                break;
            }
            if self.workflow_shortcut(pressed, modifiers)
                || self.organize_shortcut(pressed, modifiers)
                || self.view_shortcut(pressed, modifiers)
            {
                continue;
            }
            if selection::clear_trash_shortcut(pressed, modifiers) {
                self.clear_all_trash_marks();
                continue;
            }
            let command = modifiers.command || modifiers.ctrl;
            if command && (modifiers.alt || !command_chord_allowed(pressed))
                || modifiers.alt && pressed != egui::Key::R
            {
                continue;
            }
            self.pending_bookmark = None;
            if self.cull_selection_key(pressed, modifiers) {
                continue;
            }
            if command && pressed == egui::Key::A {
                continue; // Select All belongs to the Cull browser or a text field.
            }
            if !command && !modifiers.alt && self.guided_step_key(pressed, modifiers) {
                continue;
            }
            self.arrow_key(pressed, modifiers);
            self.photo_key(pressed, modifiers);
            self.numeric_key(pressed, modifiers);
        }
    }

    fn guided_step_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) -> bool {
        match self.step {
            Step::PickBest => self.pick_key(pressed, modifiers),
            Step::EasyDelete => self.delete_key(pressed, modifiers),
            Step::Rotation => self.rotation_key(pressed, modifiers),
            _ => false,
        }
    }

    fn pick_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) -> bool {
        match pressed {
            egui::Key::Num1 | egui::Key::Num2 | egui::Key::Num3 => {
                let slot = match pressed {
                    egui::Key::Num1 => 0,
                    egui::Key::Num2 => 1,
                    _ => 2,
                };
                if let Some(id) = self
                    .pick_review
                    .comparison()
                    .and_then(|c| c.paths.get(slot).copied())
                {
                    self.toggle_pick(id);
                }
            }
            egui::Key::ArrowLeft | egui::Key::ArrowRight => {
                self.navigate_pick(
                    if pressed == egui::Key::ArrowLeft {
                        -1
                    } else {
                        1
                    },
                    true,
                );
            }
            egui::Key::ArrowUp | egui::Key::ArrowDown => {
                self.navigate_pick(if pressed == egui::Key::ArrowUp { -1 } else { 1 }, false);
            }
            egui::Key::C => self.toggle_pick_focus(),
            egui::Key::K => self.confirm_pick(true),
            egui::Key::R => self.reset_pick(modifiers.shift),
            egui::Key::Enter if modifiers.shift => self.review_changes(),
            egui::Key::Enter => self.confirm_pick(false),
            _ => return false,
        }
        true
    }

    fn delete_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) -> bool {
        // Keys that change a card act on the one shown beside the grid.
        let shown = self.delete_panel;
        match pressed {
            egui::Key::ArrowLeft | egui::Key::Num1 if shown => self.toggle_delete_candidate(0),
            egui::Key::ArrowRight | egui::Key::Num2 if shown => self.toggle_delete_candidate(1),
            egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::Num1 | egui::Key::Num2 => {}
            egui::Key::R if modifiers.shift => self.accept_delete_defaults(),
            egui::Key::R if shown => self.undo_delete(),
            egui::Key::R => {}
            egui::Key::Enter if modifiers.shift => self.review_changes(),
            egui::Key::Enter => self.next_delete_card(),
            egui::Key::A => self.accept_delete_defaults(),
            egui::Key::Escape if self.delete_panel => self.delete_panel = false,
            _ => return false,
        }
        true
    }

    fn rotation_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) -> bool {
        match pressed {
            egui::Key::Q | egui::Key::E => {
                if let Some(id) = self.active {
                    self.rotate(id, if pressed == egui::Key::Q { 3 } else { 1 });
                }
            }
            egui::Key::R => self.reset_rotation(modifiers.shift),
            egui::Key::Enter if modifiers.shift => self.review_rotations(),
            egui::Key::Enter => self.confirm_rotation(),
            egui::Key::Num1 | egui::Key::Num2 => self.select_rotation(pressed == egui::Key::Num2),
            _ => return false,
        }
        true
    }

    fn arrow_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) {
        let command = modifiers.command || modifiers.ctrl;
        let (direction, horizontal) = match pressed {
            egui::Key::ArrowRight => (1, true),
            egui::Key::ArrowDown => (1, false),
            egui::Key::ArrowLeft => (-1, true),
            egui::Key::ArrowUp => (-1, false),
            _ => return,
        };
        self.navigate_axis(direction, modifiers.shift, command, horizontal);
        if self.step == Step::Organize {
            self.board_follow_active();
        }
        // Easy Delete shows the selected card beside its grid.
        if self.step == Step::EasyDelete {
            self.delete_panel = self.delete_current.is_some();
        }
    }

    /// Single-photo actions shared by every workflow that shows a canvas.
    fn photo_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) {
        let reviewing_candidates = matches!(self.step, Step::EasyDelete | Step::PickBest);
        match pressed {
            egui::Key::D if !reviewing_candidates => {
                if let Some(id) = self.active {
                    self.reject(id);
                }
            }
            egui::Key::R => {
                if let Some(id) = self.active {
                    let turns = if modifiers.alt {
                        2
                    } else if modifiers.shift {
                        3
                    } else {
                        1
                    };
                    self.rotate(id, turns);
                }
            }
            egui::Key::I => self.inspector ^= true,
            egui::Key::F1 => self.toggle_single_view(),
            egui::Key::F2 => self.compare_similar(),
            egui::Key::F3 => self.toggle_sync(),
            egui::Key::A => self.actual_size(),
            egui::Key::Plus | egui::Key::Equals => self.zoom_view(1.25),
            egui::Key::Minus => self.zoom_view(0.8),
            egui::Key::Escape => {
                self.comparison.clear();
                self.fit_view();
            }
            egui::Key::Enter if modifiers.shift => self.review_changes(),
            egui::Key::Delete | egui::Key::Backspace if !reviewing_candidates => {
                if let Some(id) = self.active {
                    self.decisions.entry(id).or_default().rejected = true;
                    self.changed();
                    self.review_changes();
                }
            }
            _ => {}
        }
    }

    /// Digits focus a comparison slot; with Ctrl/Cmd they set the star rating.
    pub(super) fn numeric_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) {
        let command = modifiers.command || modifiers.ctrl;
        let Some(digit) = digit(pressed) else { return };
        if command {
            if let Some(id) = self.active {
                self.rating(id, digit.min(5) as u8);
            }
            return;
        }
        let ids = self.comparison_ids();
        if digit == 0 {
            self.fit_view();
        } else if ids.len() > 1 {
            if let Some(&id) = ids.get(digit - 1) {
                self.active = Some(id);
                self.focused_slot = (digit - 1) as u64;
            }
        } else if self.step == Step::Cull
            && let Some(group) = self
                .active
                .and_then(|id| self.cull_groups.get(&id).copied())
            && let Some(&id) = self.group_members(group).get(digit - 1)
        {
            // Single view: digits pick the nth photo of the active photo's group.
            self.set_active(id);
        }
    }
}

/// With Ctrl/Cmd held, only select-all, zoom, arrows and digits reach the canvas.
fn command_chord_allowed(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::A
            | egui::Key::Plus
            | egui::Key::Equals
            | egui::Key::Minus
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::ArrowUp
            | egui::Key::ArrowDown
    ) || matches!(digit(key), Some(0..=5))
}

fn digit(key: egui::Key) -> Option<usize> {
    Some(match key {
        egui::Key::Num0 => 0,
        egui::Key::Num1 => 1,
        egui::Key::Num2 => 2,
        egui::Key::Num3 => 3,
        egui::Key::Num4 => 4,
        egui::Key::Num5 => 5,
        egui::Key::Num6 => 6,
        egui::Key::Num7 => 7,
        egui::Key::Num8 => 8,
        egui::Key::Num9 => 9,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "shortcuts/tests.rs"]
mod tests;
