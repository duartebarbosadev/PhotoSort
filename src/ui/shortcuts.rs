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
}

#[cfg(test)]
#[path = "shortcuts/tests.rs"]
mod tests;
