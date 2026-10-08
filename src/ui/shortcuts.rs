//! Application shortcuts share the existing navigation and departure actions.
use super::*;
use crate::core::browsing::tree::Mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApplicationKey {
    Open,
    Find,
    About,
}
fn application_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<ApplicationKey> {
    if modifiers.alt || modifiers.shift {
        return None;
    }
    let command = modifiers.command || modifiers.ctrl;
    match key {
        egui::Key::O if command => Some(ApplicationKey::Open),
        egui::Key::F if command => Some(ApplicationKey::Find),
        egui::Key::F12 if !command => Some(ApplicationKey::About),
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
                    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("library_search")));
                }
            }
            ApplicationKey::About => self.about = true,
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
        if self.step != Step::Cull || modifiers.command || modifiers.ctrl || modifiers.shift {
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
mod tests {
    use super::*;
    fn app(step: Step) -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.root = Some("/absent/library".into());
        app.step = step;
        app.active = Some(MediaId(7));
        app.media = vec![Media {
            id: MediaId(7),
            path: "/absent/library/a.jpg".into(),
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        }];
        app.index.insert(MediaId(7), 0);
        app.filtered = vec![0];
        app
    }
    fn input(keys: &[(egui::Key, egui::Modifiers)]) -> egui::RawInput {
        egui::RawInput {
            events: keys
                .iter()
                .map(|(key, modifiers)| egui::Event::Key {
                    key: *key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: *modifiers,
                })
                .collect(),
            ..Default::default()
        }
    }
    #[test]
    fn global_key_mapping_is_exact_and_platform_independent() {
        for command in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
            assert_eq!(
                application_key(egui::Key::O, command),
                Some(ApplicationKey::Open)
            );
            assert_eq!(
                application_key(egui::Key::F, command),
                Some(ApplicationKey::Find)
            );
            assert_eq!(application_key(egui::Key::F12, command), None);
            assert_eq!(
                application_key(
                    egui::Key::O,
                    egui::Modifiers {
                        shift: true,
                        ..command
                    }
                ),
                None
            );
            assert_eq!(
                application_key(
                    egui::Key::F,
                    egui::Modifiers {
                        alt: true,
                        ..command
                    }
                ),
                None
            );
        }
        assert_eq!(
            application_key(egui::Key::F12, egui::Modifiers::NONE),
            Some(ApplicationKey::About)
        );
        assert_eq!(application_key(egui::Key::O, egui::Modifiers::NONE), None);
    }
    #[test]
    fn view_keys_share_actions_and_never_become_ratings_or_guided_choices() {
        for step in Step::ALL {
            let mut app = app(step);
            let ctx = egui::Context::default();
            app.library_layout = library::Layout::Grid;
            let _ = ctx.run(input(&[(egui::Key::Num1, egui::Modifiers::ALT)]), |ctx| {
                app.keyboard(ctx)
            });
            assert_eq!(app.active, Some(MediaId(7)));
            assert!(app.decisions.is_empty());
            assert_eq!(
                app.library_layout,
                if step == Step::Cull {
                    library::Layout::List
                } else {
                    library::Layout::Grid
                }
            );
            assert_eq!(app.stats.decodes, 0);
        }
        let mut app = app(Step::Cull);
        let ctx = egui::Context::default();
        for (key, mode) in [(egui::Key::F, Mode::Folders), (egui::Key::S, Mode::Groups)] {
            app.navigator.enabled = false;
            let _ = ctx.run(input(&[(key, egui::Modifiers::NONE)]), |ctx| {
                app.keyboard(ctx)
            });
            assert!(app.navigator.enabled && !app.review);
            assert_eq!(app.navigator.mode, mode);
            assert!(app.filter_dirty);
            let _ = ctx.run(input(&[(key, egui::Modifiers::NONE)]), |ctx| {
                app.keyboard(ctx)
            });
            assert!(!app.navigator.enabled);
        }
        assert!(app.analysis_run.is_none() && app.requested.is_empty());
    }
    #[test]
    fn navigator_activation_shares_review_transition_without_image_or_model_work() {
        for mode in Mode::ALL {
            let mut app = app(Step::Cull);
            app.review = true;
            app.comparison = vec![MediaId(7)];
            app.decisions.insert(
                MediaId(7),
                Decision {
                    rating: Some(4),
                    ..Default::default()
                },
            );
            app.set_navigator(mode, true);
            assert!(!app.review && app.navigator.enabled);
            assert_eq!(app.navigator.mode, mode);
            assert_eq!(app.active, Some(MediaId(7)));
            assert_eq!(app.comparison, vec![MediaId(7)]);
            assert_eq!(app.decisions[&MediaId(7)].rating, Some(4));
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.stats.metadata_reads, 0);
            assert!(
                app.requested.is_empty()
                    && app.analysis_run.is_none()
                    && app.metadata_run.is_none()
            );
            app.runtime.shutdown();
        }
    }
    #[test]
    fn modified_unbound_keys_do_not_change_photos_in_any_workflow() {
        for step in Step::ALL {
            for command in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
                let mut app = app(step);
                let before = app.inspector;
                let ctx = egui::Context::default();
                let _ = ctx.run(
                    input(&[
                        (egui::Key::R, command),
                        (egui::Key::D, command),
                        (egui::Key::I, command),
                        (egui::Key::Delete, command),
                    ]),
                    |ctx| app.keyboard(ctx),
                );
                assert!(app.decisions.is_empty() && !app.plan_open);
                assert_eq!(app.inspector, before);
                let _ = ctx.run(input(&[(egui::Key::Plus, command)]), |ctx| {
                    app.keyboard(ctx)
                });
                assert_eq!(app.inspection_view.zoom, 1.25);
                let _ = ctx.run(input(&[(egui::Key::Minus, command)]), |ctx| {
                    app.keyboard(ctx)
                });
                assert_eq!(app.inspection_view.zoom, 1.0);
            }
        }
    }
    #[test]
    fn search_focus_handles_text_input_but_dialogs_and_busy_state_keep_ownership() {
        let mut app = app(Step::Cull);
        let ctx = egui::Context::default();
        let search = egui::Id::new("library_search");
        let mut text = String::new();
        let _ = ctx.run(input(&[(egui::Key::F, egui::Modifiers::COMMAND)]), |ctx| {
            app.keyboard(ctx);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(search));
            });
        });
        assert_eq!(ctx.memory(|m| m.focused()), Some(search));
        let _ = ctx.run(
            input(&[
                (egui::Key::R, egui::Modifiers::NONE),
                (egui::Key::Num1, egui::Modifiers::ALT),
            ]),
            |ctx| {
                app.keyboard(ctx);
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut text).id(search));
                });
            },
        );
        assert!(app.decisions.is_empty());
        assert_eq!(app.library_layout, library::Layout::Grid);
        let _ = ctx.run(input(&[(egui::Key::F12, egui::Modifiers::NONE)]), |ctx| {
            app.keyboard(ctx)
        });
        assert!(app.about);
        for busy in [true, false] {
            app.about = false;
            app.busy = busy;
            app.help = !busy;
            let _ = ctx.run(
                input(&[
                    (egui::Key::F12, egui::Modifiers::NONE),
                    (egui::Key::O, egui::Modifiers::COMMAND),
                ]),
                |ctx| app.keyboard(ctx),
            );
            assert!(!app.about); // Open must also be left untouched; no native picker.
        }
    }
    #[test]
    fn repeated_global_keys_and_search_without_a_search_field_are_harmless() {
        let mut app = app(Step::PickBest);
        let ctx = egui::Context::default();
        let _ = ctx.run(input(&[(egui::Key::F, egui::Modifiers::COMMAND)]), |ctx| {
            app.keyboard(ctx)
        });
        assert!(ctx.memory(|m| m.focused()).is_none());
        let _ = ctx.run(input(&[(egui::Key::F12, egui::Modifiers::NONE)]), |ctx| {
            app.keyboard(ctx)
        });
        assert!(app.about);
        app.about = false;
        // egui derives repeat state from held keys, rather than trusting a raw
        // repeat flag on a key it has never seen pressed.
        let mut repeated = input(&[(egui::Key::F12, egui::Modifiers::NONE)]);
        if let egui::Event::Key { repeat, .. } = &mut repeated.events[0] {
            *repeat = true;
        }
        let _ = ctx.run(repeated, |ctx| app.keyboard(ctx));
        assert!(!app.about);
    }
    #[test]
    fn opening_a_departure_guard_stops_later_photo_keys_in_the_same_frame() {
        let mut app = app(Step::Cull);
        app.decisions.insert(
            MediaId(7),
            Decision {
                rejected: true,
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        let _ = ctx.run(
            input(&[
                (
                    egui::Key::Num1,
                    egui::Modifiers {
                        command: true,
                        alt: true,
                        ..Default::default()
                    },
                ),
                (egui::Key::R, egui::Modifiers::NONE),
            ]),
            |ctx| app.keyboard(ctx),
        );
        assert!(app.departure.is_some());
        assert_eq!(app.decisions[&MediaId(7)].turns, 0);
    }
}
