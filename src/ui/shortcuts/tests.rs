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
fn organize_mode_keys_share_toolbar_state_without_repeating_expensive_work() {
    for width in [640.0, 1440.0] {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        app.step = Step::Organize;
        app.navigator.enabled = false;
        let catalog = app.catalog.clone().unwrap();
        let active = app.media[0].id;
        app.active = Some(active);
        app.decisions.insert(
            active,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let draft = crate::core::organize::Draft::new(HashMap::from([(active, "Chosen".into())]));
        app.organize_drafts
            .insert(GroupMode::Current, draft.clone());
        app.analysis_run = Some(77);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        for (key, expected) in [
            (egui::Key::Num2, GroupMode::Mixed),
            (egui::Key::Num3, GroupMode::Visual),
            (egui::Key::Num4, GroupMode::Faces),
            (egui::Key::Num5, GroupMode::Location),
            (egui::Key::Num1, GroupMode::Current),
        ] {
            app.pending_bookmark = Some(Default::default());
            let mut events = input(&[(key, egui::Modifiers::ALT)]);
            events.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 900.0),
            ));
            let out = ctx.run_test(events, |ui| {
                let ctx = &ui.ctx().clone();
                {
                    app.keyboard(ctx);
                    egui::CentralPanel::default().show(ui, |ui| app.toolbar(ui));
                }
            });
            assert_eq!(app.group_mode, expected);
            assert!(app.pending_bookmark.is_none());
            assert!(
                out.platform_output
                    .accesskit_update
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some(expected.title())
                        || n.value() == Some(expected.title()))
            );
            assert_eq!(app.organize_drafts[&GroupMode::Current], draft);
            assert_eq!(app.active, Some(active));
            assert_eq!(app.decisions[&active].rating, Some(4));
            assert_eq!(app.analysis_run, Some(77));
            assert!(app.plan.is_none() && app.requested.is_empty() && app.metadata_run.is_none());
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.stats.metadata_reads, 0);
            assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
        }
        app.runtime.shutdown();
    }
}

#[test]
fn organize_mode_location_reuses_one_metadata_run_and_cached_results() {
    let (_temp, mut app) = super::super::compact_cull_tests::fixture();
    app.step = Step::Organize;
    app.metadata.clear();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    for _ in 0..2 {
        let _ = ctx.run_test(input(&[(egui::Key::Num5, egui::Modifiers::ALT)]), |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        });
        assert_eq!(app.group_mode, GroupMode::Location);
        let run = app.metadata_run.unwrap();
        assert!(app.view_shortcut(egui::Key::Num5, egui::Modifiers::ALT));
        assert_eq!(app.metadata_run, Some(run));
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.metadata_run.is_some() {
        app.poll(&ctx);
        assert!(Instant::now() < deadline, "Metadata did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(app.stats.metadata_reads, 9);
    app.view_shortcut(egui::Key::Num1, egui::Modifiers::ALT);
    app.view_shortcut(egui::Key::Num5, egui::Modifiers::ALT);
    assert!(app.metadata_run.is_none());
    assert_eq!(app.stats.metadata_reads, 9);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.analysis_run.is_none() && app.analysis.is_empty() && app.plan.is_none());
    app.runtime.shutdown();
}

#[test]
fn organize_mode_keys_preserve_text_modal_and_workflow_ownership() {
    for blocker in ["text", "editor", "help", "busy", "plan", "welcome"] {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        app.step = Step::Organize;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let field = egui::Id::new("mode_shortcut_text");
        let mut text = String::new();
        match blocker {
            "text" => {
                let _ = ctx.run_test(input(&[]), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut text).id(field))
                            .request_focus();
                    });
                });
            }
            "editor" => {
                app.open_organize_editor();
                assert!(app.organize_editor.is_some());
            }
            "help" => app.help = true,
            "busy" => app.busy = true,
            "plan" => app.plan_open = true,
            "welcome" => app.root = None,
            _ => unreachable!(),
        }
        let _ = ctx.run_test(input(&[(egui::Key::Num3, egui::Modifiers::ALT)]), |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        });
        assert_eq!(app.group_mode, GroupMode::Current, "{blocker}");
        assert!(app.analysis_run.is_none() && app.metadata_run.is_none());
        app.runtime.shutdown();
    }
    for step in Step::ALL {
        for modifiers in [
            egui::Modifiers::ALT,
            egui::Modifiers {
                shift: true,
                alt: true,
                ..Default::default()
            },
        ] {
            let mut app = app(step);
            app.group_mode = GroupMode::Camera;
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let _ = ctx.run_test(input(&[(egui::Key::Num4, modifiers)]), |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            });
            assert_eq!(
                app.group_mode,
                if step == Step::Organize && !modifiers.shift {
                    GroupMode::Faces
                } else {
                    GroupMode::Camera
                }
            );
            assert!(app.decisions.is_empty());
            app.runtime.shutdown();
        }
    }
    for command in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
        let mut app = app(Step::Organize);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let _ = ctx.run_test(
            input(&[(
                egui::Key::Num5,
                egui::Modifiers {
                    alt: true,
                    ..command
                },
            )]),
            |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            },
        );
        assert_eq!(app.step, Step::Cull);
        assert_eq!(app.group_mode, GroupMode::Current);
        assert!(app.decisions.is_empty());
        app.runtime.shutdown();
    }
}

#[test]
fn view_keys_share_actions_and_never_become_ratings_or_guided_choices() {
    for step in Step::ALL {
        let mut app = app(step);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        app.library_layout = library::Layout::Grid;
        let _ = ctx.run_test(input(&[(egui::Key::Num1, egui::Modifiers::ALT)]), |ui| {
            let ctx = &ui.ctx().clone();
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
    theme_install(&ctx);
    for (key, mode) in [(egui::Key::F, Mode::Folders), (egui::Key::S, Mode::Groups)] {
        app.navigator.enabled = false;
        let _ = ctx.run_test(input(&[(key, egui::Modifiers::NONE)]), |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        });
        assert!(app.navigator.enabled && !app.review);
        assert_eq!(app.navigator.mode, mode);
        assert!(app.filter_dirty);
        let _ = ctx.run_test(input(&[(key, egui::Modifiers::NONE)]), |ui| {
            let ctx = &ui.ctx().clone();
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
            app.requested.is_empty() && app.analysis_run.is_none() && app.metadata_run.is_none()
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
            theme_install(&ctx);
            let _ = ctx.run_test(
                input(&[
                    (egui::Key::R, command),
                    (egui::Key::D, command),
                    (egui::Key::I, command),
                    (egui::Key::Delete, command),
                ]),
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.keyboard(ctx)
                },
            );
            assert!(app.decisions.is_empty() && !app.plan_open);
            assert_eq!(app.inspector, before);
            let _ = ctx.run_test(input(&[(egui::Key::Plus, command)]), |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            });
            assert_eq!(app.inspection_view.zoom, 1.25);
            let _ = ctx.run_test(input(&[(egui::Key::Minus, command)]), |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            });
            assert_eq!(app.inspection_view.zoom, 1.0);
        }
    }
}
#[test]
fn find_shortcut_replaces_existing_query_in_the_real_toolbar() {
    for step in Step::ALL.into_iter().filter(|step| *step != Step::PickBest) {
        for width in [640.0, 1280.0] {
            for modifiers in [egui::Modifiers::CTRL, egui::Modifiers::COMMAND] {
                let mut app = app(step);
                app.query = "A café/東京📷 e\u{301}".into();
                let ctx = egui::Context::default();
                theme_install(&ctx);
                let mut frame = |mut events: egui::RawInput| {
                    events.screen_rect = Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 700.0),
                    ));
                    let _ = ctx.run_test(events, |ui| {
                        let ctx = &ui.ctx().clone();
                        {
                            app.keyboard(ctx);
                            egui::CentralPanel::default().show(ui, |ui| app.toolbar(ui));
                        }
                    });
                };
                frame(input(&[]));
                frame(input(&[(egui::Key::F, modifiers)]));
                let mut typing = input(&[]);
                typing.events.push(egui::Event::Key {
                    key: egui::Key::F,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers,
                });
                typing.events.push(egui::Event::Text("new search".into()));
                frame(typing);
                assert_eq!(app.query, "new search", "{step:?}, width={width}");
                assert!(app.filter_dirty);
                assert_eq!(app.active, Some(MediaId(7)));
                assert!(app.decisions.is_empty());
                assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
                assert!(app.analysis_run.is_none() && app.requested.is_empty());
                app.runtime.shutdown();
            }
        }
    }
}
#[test]
fn find_does_not_replace_selection_when_another_owner_blocks_it() {
    let search = egui::Id::new("library_search");
    for guard in 0..6 {
        let mut app = app(Step::Cull);
        app.query = "existing query".into();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let range = egui::text::CCursorRange::one(egui::text::CCursor::new(3));
        let mut state = egui::text_edit::TextEditState::default();
        state.cursor.set_char_range(Some(range));
        state.store(&ctx, search);
        match guard {
            0 => app.busy = true,
            1 => app.help = true,
            2 => egui::Popup::open_id(&ctx, egui::Id::new("find-guard")),
            3 => app.step = Step::PickBest,
            4 => app.root = None,
            _ => ctx.input_mut(|input| {
                input.keys_down.insert(egui::Key::F);
            }),
        }
        let _ = ctx.run_test(input(&[(egui::Key::F, egui::Modifiers::COMMAND)]), |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        });
        assert_eq!(
            egui::text_edit::TextEditState::load(&ctx, search)
                .unwrap()
                .cursor
                .char_range(),
            Some(range),
            "guard={guard}"
        );
        assert_eq!(app.query, "existing query");
        assert!(ctx.memory(|memory| memory.focused()).is_none());
        app.runtime.shutdown();
    }
}
#[test]
fn search_focus_handles_text_input_but_dialogs_and_busy_state_keep_ownership() {
    let mut app = app(Step::Cull);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let search = egui::Id::new("library_search");
    let mut text = String::new();
    let _ = ctx.run_test(input(&[(egui::Key::F, egui::Modifiers::COMMAND)]), |ui| {
        let ctx = &ui.ctx().clone();
        {
            app.keyboard(ctx);
            egui::CentralPanel::default().show(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut text).id(search));
            });
        }
    });
    assert_eq!(ctx.memory(|m| m.focused()), Some(search));
    let _ = ctx.run_test(
        input(&[
            (egui::Key::R, egui::Modifiers::NONE),
            (egui::Key::Num1, egui::Modifiers::ALT),
        ]),
        |ui| {
            let ctx = &ui.ctx().clone();
            {
                app.keyboard(ctx);
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut text).id(search));
                });
            }
        },
    );
    assert!(app.decisions.is_empty());
    assert_eq!(app.library_layout, library::Layout::Grid);
    let _ = ctx.run_test(input(&[(egui::Key::F12, egui::Modifiers::NONE)]), |ui| {
        let ctx = &ui.ctx().clone();
        app.keyboard(ctx)
    });
    assert!(app.about);
    for busy in [true, false] {
        app.about = false;
        app.busy = busy;
        app.help = !busy;
        let _ = ctx.run_test(
            input(&[
                (egui::Key::F12, egui::Modifiers::NONE),
                (egui::Key::O, egui::Modifiers::COMMAND),
            ]),
            |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            },
        );
        assert!(!app.about); // Open must also be left untouched; no native picker.
    }
}
#[test]
fn repeated_global_keys_and_search_without_a_search_field_are_harmless() {
    let mut app = app(Step::PickBest);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let _ = ctx.run_test(input(&[(egui::Key::F, egui::Modifiers::COMMAND)]), |ui| {
        let ctx = &ui.ctx().clone();
        app.keyboard(ctx)
    });
    assert!(ctx.memory(|m| m.focused()).is_none());
    let _ = ctx.run_test(input(&[(egui::Key::F12, egui::Modifiers::NONE)]), |ui| {
        let ctx = &ui.ctx().clone();
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
    let _ = ctx.run_test(repeated, |ui| {
        let ctx = &ui.ctx().clone();
        app.keyboard(ctx)
    });
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
    theme_install(&ctx);
    let _ = ctx.run_test(
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
        |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        },
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
            theme_install(&ctx);
            let _ = ctx.run_test(
                input(&[
                    (egui::Key::S, modifiers),
                    (egui::Key::R, egui::Modifiers::NONE),
                ]),
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.keyboard(ctx)
                },
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
                let _ = ctx.run_test(input(&[(egui::Key::S, modifiers)]), |ui| {
                    let ctx = &ui.ctx().clone();
                    app.keyboard(ctx)
                });
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
            theme_install(&ctx);
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
            let _ = ctx.run_test(input(&[(egui::Key::S, egui::Modifiers::COMMAND)]), |ui| {
                let ctx = &ui.ctx().clone();
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
            theme_install(&ctx);
            let _ = ctx.run_test(
                input(&[(key, modifiers), (egui::Key::R, egui::Modifiers::NONE)]),
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.keyboard(ctx)
                },
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
            theme_install(&ctx);
            match guard {
                0 => app.busy = true,
                1 => app.help = true,
                2 => egui::Popup::open_id(&ctx, egui::Id::new("settings-guard")),
                _ => ctx.input_mut(|i| {
                    i.keys_down.insert(key);
                }),
            }
            let _ = ctx.run_test(input(&[(key, egui::Modifiers::NONE)]), |ui| {
                let ctx = &ui.ctx().clone();
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
