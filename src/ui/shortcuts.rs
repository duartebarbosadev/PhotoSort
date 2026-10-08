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
                    ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("library_search")));
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
    #[test]
    fn analysis_shortcut_uses_workflow_owner_and_preserves_active_work() {
        for step in Step::ALL {
            for modifiers in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
                let mut app = app(step);
                app.group_mode = GroupMode::Current;
                app.decisions.insert(
                    MediaId(7),
                    Decision {
                        rating: Some(4),
                        ..Default::default()
                    },
                );
                let ctx = egui::Context::default();
                let _ = ctx.run(
                    input(&[
                        (egui::Key::S, modifiers),
                        (egui::Key::R, egui::Modifiers::NONE),
                    ]),
                    |ctx| app.keyboard(ctx),
                );
                if step == Step::Rotation {
                    // Minimal builds have no model; the same action opens setup.
                    assert!(app.model_dialog || app.rotation_run.is_some());
                    assert_eq!(app.analysis_run, app.rotation_run);
                } else {
                    assert!(app.analysis_run.is_some(), "{step:?}");
                    assert!(app.rotation_run.is_none());
                }
                assert_eq!(app.step, step);
                assert_eq!(app.active, Some(MediaId(7)));
                assert_eq!(app.decision(MediaId(7)).rating, Some(4));
                assert_eq!(app.decision(MediaId(7)).turns, 0);
                if let Some(run) = app.analysis_run {
                    // Both a second UI request and a direct retry leave the
                    // admitted generation intact: no duplicate decode/model pass.
                    app.analyze_current_workflow();
                    app.analyze();
                    assert_eq!(app.analysis_run, Some(run));
                    let _ = ctx.run(input(&[(egui::Key::S, modifiers)]), |ctx| app.keyboard(ctx));
                    assert_eq!(app.analysis_run, Some(run));
                    app.cancel_analysis();
                    assert!(app.analysis_run.is_none());
                }
                app.runtime.shutdown();
            }
        }
    }
    #[test]
    fn analysis_shortcut_respects_availability_modal_and_modifier_guards() {
        for modifiers in [
            egui::Modifiers::NONE,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Modifiers::CTRL | egui::Modifiers::ALT,
        ] {
            assert!(application_key(egui::Key::S, modifiers).is_none());
        }
        for step in Step::ALL {
            for guard in 0..7 {
                let mut app = app(step);
                let ctx = egui::Context::default();
                match guard {
                    0 => app.media.clear(),
                    1 => app.scanning = true,
                    2 => app.busy = true,
                    3 => app.help = true,
                    4 => egui::Popup::open_id(&ctx, egui::Id::new("analysis-guard")),
                    5 => ctx.input_mut(|i| {
                        i.keys_down.insert(egui::Key::S);
                    }),
                    _ => app.analysis_run = Some(42),
                }
                let before = app.analysis_run;
                let _ = ctx.run(input(&[(egui::Key::S, egui::Modifiers::COMMAND)]), |ctx| {
                    app.keyboard(ctx)
                });
                assert_eq!(app.analysis_run, before, "{step:?}, guard={guard}");
                assert!(!app.model_dialog && app.rotation_run.is_none());
                assert!(!app.navigator.enabled);
                app.runtime.shutdown();
            }
        }
    }
    #[test]
    fn settings_access_keys_open_the_existing_draft_without_photo_work() {
        for step in Step::ALL {
            for (key, modifiers) in [
                (egui::Key::F9, egui::Modifiers::NONE),
                (egui::Key::F10, egui::Modifiers::NONE),
                (egui::Key::Comma, egui::Modifiers::COMMAND),
                (egui::Key::Comma, egui::Modifiers::CTRL),
            ] {
                let mut app = app(step);
                app.decisions.insert(
                    MediaId(7),
                    Decision {
                        rating: Some(4),
                        rejected: true,
                        ..Default::default()
                    },
                );
                let before = serde_json::to_vec(&app.preferences).unwrap();
                let ctx = egui::Context::default();
                let _ = ctx.run(
                    input(&[(key, modifiers), (egui::Key::R, egui::Modifiers::NONE)]),
                    |ctx| app.keyboard(ctx),
                );
                assert!(app.preferences_draft.is_some(), "{step:?}: {key:?}");
                assert_eq!(
                    serde_json::to_vec(app.preferences_draft.as_ref().unwrap()).unwrap(),
                    before
                );
                assert_eq!(app.step, step);
                assert_eq!(app.active, Some(MediaId(7)));
                assert_eq!(app.decision(MediaId(7)).rating, Some(4));
                assert!(app.decision(MediaId(7)).rejected);
                assert_eq!(app.decision(MediaId(7)).turns, 0);
                assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
                assert!(
                    app.analysis_run.is_none()
                        && app.metadata_run.is_none()
                        && app.requested.is_empty()
                );
                app.runtime.shutdown();
            }
        }
    }

    #[test]
    fn settings_access_respects_existing_modal_popup_repeat_and_modifier_guards() {
        for key in [egui::Key::F9, egui::Key::F10] {
            for modifiers in [
                egui::Modifiers::ALT,
                egui::Modifiers::SHIFT,
                egui::Modifiers::CTRL,
                egui::Modifiers::COMMAND,
            ] {
                assert!(application_key(key, modifiers).is_none());
            }
            for guard in 0..4 {
                let mut app = app(Step::Cull);
                let ctx = egui::Context::default();
                match guard {
                    0 => app.busy = true,
                    1 => app.help = true,
                    2 => egui::Popup::open_id(&ctx, egui::Id::new("settings-guard")),
                    _ => ctx.input_mut(|i| {
                        i.keys_down.insert(key);
                    }),
                }
                let _ = ctx.run(input(&[(key, egui::Modifiers::NONE)]), |ctx| {
                    app.keyboard(ctx)
                });
                assert!(app.preferences_draft.is_none(), "guard={guard}");
                assert!(app.decisions.is_empty());
                app.runtime.shutdown();
            }
        }
        assert!(application_key(egui::Key::Comma, egui::Modifiers::NONE).is_none());
        assert!(
            application_key(
                egui::Key::Comma,
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
            )
            .is_none()
        );
    }
}
