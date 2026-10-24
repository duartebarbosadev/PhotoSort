use super::*;
use crate::core::playback;
fn time(seconds: f64) -> String {
    let ticks = if seconds.is_finite() {
        (seconds.max(0.0) * 10.0).floor() as u64
    } else {
        0
    };
    let seconds = ticks / 10;
    let tenth = ticks % 10;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}.{tenth}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}.{tenth}", seconds / 60, seconds % 60)
    }
}
impl PhotoSortApp {
    fn video_target(&self) -> Option<&Media> {
        if !self.review || self.busy || self.scanning || !playback::supported() {
            return None;
        }
        self.active
            .and_then(|id| self.index.get(&id))
            .map(|i| &self.media[*i])
            .filter(|m| m.kind == MediaKind::Video)
    }
    pub(super) fn sync_playback(&mut self, ctx: &egui::Context) {
        let target = self.video_target();
        let context = target.map(|m| (self.step, m.id));
        self.runtime.playback.select(target);
        if context != self.video_context
            || !ctx.input(|i| i.raw.focused)
            || self.modal_active()
            || self.error.is_some()
        {
            self.runtime.playback.pause();
        }
        if context != self.video_context {
            self.video_texture = None;
        }
        self.video_context = context;
    }
    pub(super) fn video_keyboard(&mut self, ctx: &egui::Context) {
        let Some(media) = self.video_target() else {
            return;
        };
        if !ctx.input(|i| i.raw.focused) {
            return;
        }
        self.runtime.playback.select(Some(media));
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
            self.video_context = Some((self.step, media.id));
            self.runtime.playback.toggle();
        }
        for (key, delta) in [(egui::Key::ArrowLeft, -10.0), (egui::Key::ArrowRight, 10.0)] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key)) {
                self.runtime.playback.seek_relative(delta);
            }
        }
    }
    pub(super) fn video_canvas(&mut self, ui: &mut egui::Ui, id: MediaId, slot: u64, size: Vec2) {
        let media = self.media[self.index[&id]].clone();
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        response.context_menu(|ui| self.review_context_menu(ui, id));
        let active = self.active == Some(id);
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, R_SURFACE, BG);
        if active {
            painter.rect_stroke(
                rect,
                R_SURFACE,
                Stroke::new(1.5, ACCENT),
                StrokeKind::Inside,
            );
        }
        // Full transport controls reserve 80 points plus the title. Collapse
        // them before they consume most of a short review slot's picture area.
        let compact = size.y < 220.0 || size.x < 280.0;
        let title_height = if size.y < 80.0 { 0.0 } else { 26.0 };
        if title_height > 0.0 {
            painter.text(
                rect.left_top() + egui::vec2(8.0, 8.0),
                Align2::LEFT_TOP,
                format!("VIDEO · {}", media.name()),
                FontId::proportional(11.0),
                MUTED,
            );
        }
        let state = self.runtime.playback.snapshot();
        let owns = active && state.id == Some(id);
        let frame = state.frame.as_ref().filter(|_| owns);
        if let Some(frame) = frame
            && self
                .video_texture
                .as_ref()
                .is_none_or(|(old, serial, _)| *old != id || *serial != frame.serial)
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [
                    frame.pixels.width() as usize,
                    frame.pixels.height() as usize,
                ],
                frame.pixels.as_raw(),
            );
            if let Some((old, serial, texture)) = &mut self.video_texture
                && *old == id
            {
                texture.set(image, egui::TextureOptions::LINEAR);
                *serial = frame.serial;
            } else {
                self.video_texture = Some((
                    id,
                    frame.serial,
                    ui.ctx()
                        .load_texture("playing-video", image, egui::TextureOptions::LINEAR),
                ));
            }
        }
        let texture = if frame.is_some() {
            self.video_texture.as_ref().map(|(_, _, t)| t.clone())
        } else if !active {
            self.texture(id, false)
        } else {
            None
        };
        let controls = if active && !compact { 80.0 } else { 34.0 };
        let image_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(8.0, title_height),
            egui::pos2(
                rect.right() - 8.0,
                (rect.bottom() - controls).max(rect.top() + title_height + 1.0),
            ),
        );
        if let Some(texture) = texture {
            paint_image(ui, &texture, image_rect, 1.0, Vec2::ZERO, 0);
        } else {
            painter.text(
                image_rect.center(),
                Align2::CENTER_CENTER,
                if owns && state.loading {
                    "Opening video…"
                } else {
                    "VIDEO"
                },
                FontId::proportional(13.0),
                MUTED,
            );
        }
        let controls_rect = egui::Rect::from_min_max(
            egui::pos2(
                rect.left() + 8.0,
                (rect.bottom() - controls).max(rect.top() + title_height),
            ),
            rect.max - egui::vec2(8.0, 4.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(controls_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(5.0, 3.0);
                if !active {
                    if ui.button("Focus to watch").clicked() {
                        self.set_active(id);
                        self.focused_slot = slot;
                    }
                } else if !playback::supported() {
                    if ui
                        .add_enabled(
                            self.can_open_desktop(id),
                            egui::Button::new("Open in external player"),
                        )
                        .clicked()
                    {
                        self.open_desktop(id, crate::core::desktop::Action::OpenVideo);
                    }
                } else if let Some(error) = state.error.as_ref().filter(|_| owns) {
                    let mut recovery = |ui: &mut egui::Ui| {
                        ui.label(RichText::new("Playback unavailable").color(RED))
                            .on_hover_text(error);
                        ui.horizontal(|ui| {
                            if ui.button("Retry").clicked() {
                                self.runtime.playback.select(None);
                                self.runtime.playback.select(Some(&media));
                            }
                            if ui
                                .add_enabled(
                                    self.can_open_desktop(id),
                                    egui::Button::new("Open externally"),
                                )
                                .clicked()
                            {
                                self.open_desktop(id, crate::core::desktop::Action::OpenVideo);
                            }
                        });
                    };
                    if compact {
                        ui.menu_button("Playback error", recovery);
                    } else {
                        recovery(ui);
                    }
                } else {
                    if compact {
                        ui.horizontal(|ui| {
                            if ui
                                .button(if state.play_requested {
                                    "Pause"
                                } else {
                                    "Play"
                                })
                                .clicked()
                            {
                                self.runtime.playback.toggle();
                            }
                            ui.menu_button("Controls", |ui| {
                                playback_controls(ui, &self.runtime.playback, &state)
                            });
                        });
                    } else {
                        playback_controls(ui, &self.runtime.playback, &state);
                    }
                }
            },
        );
        if response.clicked() {
            self.set_active(id);
            self.focused_slot = slot;
        }
        if response.double_clicked()
            && active
            && let Some(pos) = response.interact_pointer_pos()
        {
            self.runtime
                .playback
                .seek_relative(if pos.x < rect.center().x { -10.0 } else { 10.0 });
        }
    }
}

fn playback_controls(ui: &mut egui::Ui, player: &playback::Player, state: &playback::Snapshot) {
    ui.horizontal(|ui| {
        if ui
            .button(if state.play_requested {
                "Pause"
            } else if state.ended {
                "Replay"
            } else {
                "Play"
            })
            .on_hover_text("Space")
            .clicked()
        {
            player.toggle();
        }
        if ui
            .add_enabled(state.ready, egui::Button::new("−10"))
            .on_hover_text("Back 10 seconds · Alt+Left")
            .clicked()
        {
            player.seek_relative(-10.0);
        }
        if ui
            .add_enabled(state.ready, egui::Button::new("+10"))
            .on_hover_text("Forward 10 seconds · Alt+Right")
            .clicked()
        {
            player.seek_relative(10.0);
        }
        ui.menu_button("Audio", |ui| {
            let mut volume = state.volume;
            if ui
                .add(egui::Slider::new(&mut volume, 0.0..=1.0).text("Volume"))
                .changed()
            {
                player.volume(volume);
            }
            if ui
                .button(if volume > 0.0 { "Mute" } else { "Unmute" })
                .clicked()
            {
                player.volume(if volume > 0.0 { 0.0 } else { 0.5 });
            }
        });
    });
    let mut position = state.seek_target.unwrap_or(state.position);
    ui.spacing_mut().slider_width = ui.available_width().max(24.0);
    if ui
        .add_enabled(
            state.ready,
            egui::Slider::new(&mut position, 0.0..=state.duration.max(0.001)).show_value(false),
        )
        .changed()
    {
        player.seek(position);
    }
    ui.label(
        RichText::new(format!("{} / {}", time(position), time(state.duration)))
            .small()
            .color(MUTED),
    );
}

#[cfg(all(test, any(target_os = "macos", feature = "portable-video")))]
mod tests {
    use super::*;
    use crate::test_support::worker_patience;
    #[test]
    fn time_counters_show_short_clips_and_carry_without_invalid_values() {
        for (value, expected) in [
            (0.0, "0:00.0"),
            (0.4, "0:00.4"),
            (59.9, "0:59.9"),
            (60.0, "1:00.0"),
            (3599.9, "59:59.9"),
            (3600.0, "1:00:00.0"),
            (-1.0, "0:00.0"),
            (f64::NAN, "0:00.0"),
            (f64::INFINITY, "0:00.0"),
        ] {
            assert_eq!(time(value), expected);
        }
    }
    fn app() -> (PhotoSortApp, tempfile::TempDir) {
        let root = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::with_cache(|| {}, root.path().join("cache"));
        runtime.playback = playback::test_player();
        let mut app = PhotoSortApp::with_runtime(runtime, None);
        for (i, name, kind) in [
            (1, "a.mp4", MediaKind::Video),
            (2, "b.mp4", MediaKind::Video),
            (3, "c.jpg", MediaKind::Photo),
        ] {
            let path = root.path().join(name);
            std::fs::write(&path, "fixture").unwrap();
            app.index.insert(MediaId(i), app.media.len());
            app.media.push(Media {
                id: MediaId(i),
                fingerprint: Fingerprint::read(&path).unwrap(),
                path,
                kind,
            });
        }
        app.active = Some(MediaId(1));
        app.review = true;
        (app, root)
    }
    fn input(focused: bool, size: Vec2) -> egui::RawInput {
        egui::RawInput {
            focused,
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        }
    }
    #[test]
    fn shared_video_focus_navigation_dialogs_and_folder_epochs_stop_or_pause_playback() {
        let (mut app, _root) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        app.runtime.playback.toggle();
        assert!(app.runtime.playback.snapshot().play_requested);
        let _ = ctx.run_test(input(false, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        assert!(!app.runtime.playback.snapshot().play_requested);
        for step in Step::ALL {
            app.step = step;
            let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
                let ctx = &ui.ctx().clone();
                app.sync_playback(ctx)
            });
            app.runtime.playback.toggle();
            app.help = true;
            let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
                let ctx = &ui.ctx().clone();
                app.sync_playback(ctx)
            });
            assert!(!app.runtime.playback.snapshot().play_requested);
            app.help = false;
        }
        app.runtime.playback.toggle();
        app.active = Some(MediaId(2));
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        assert_eq!(app.runtime.playback.snapshot().id, Some(MediaId(2)));
        assert!(!app.runtime.playback.snapshot().play_requested);
        app.active = Some(MediaId(3));
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        assert!(app.runtime.playback.snapshot().id.is_none());
        app.active = Some(MediaId(1));
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        app.runtime.next_epoch();
        assert!(app.runtime.playback.snapshot().id.is_none());
        app.runtime.shutdown();
    }
    #[test]
    fn video_canvas_fits_compact_and_large_regions_and_reuses_the_same_texture() {
        let (mut app, _root) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        let until = Instant::now() + worker_patience();
        while app.runtime.playback.snapshot().frame.is_none() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut texture = None;
        for size in [
            egui::vec2(180.0, 40.0),
            egui::vec2(220.0, 100.0),
            egui::vec2(480.0, 240.0),
            egui::vec2(1200.0, 700.0),
        ] {
            for _ in 0..2 {
                let _ = ctx.run_test(input(true, size + egui::vec2(16.0, 16.0)), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let top = ui.cursor().top();
                        let left = ui.cursor().left();
                        app.video_canvas(ui, MediaId(1), 0, size);
                        assert!(
                            ui.min_rect().bottom() <= top + size.y + 2.0,
                            "vertical overflow: {:?}",
                            ui.min_rect()
                        );
                        assert!(
                            ui.min_rect().right() <= left + size.x + 2.0,
                            "horizontal overflow: {:?}",
                            ui.min_rect()
                        );
                    });
                });
                let current = app.video_texture.as_ref().unwrap().2.id();
                if let Some(old) = texture {
                    assert_eq!(old, current);
                }
                texture = Some(current);
            }
        }
        assert!(
            app.requested.is_empty(),
            "Video review should not request photo previews"
        );
        app.runtime.shutdown();
    }
    #[test]
    fn video_controls_stay_inside_horizontal_review_slots_with_readable_timeline() {
        let (mut app, _root) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        let until = Instant::now() + worker_patience();
        while !app.runtime.playback.snapshot().ready {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        for (size, compact) in [
            (egui::vec2(220.0, 100.0), true),
            (egui::vec2(260.0, 160.0), true),
            (egui::vec2(280.0, 160.0), true),
            (egui::vec2(640.0, 131.0), true),
            (egui::vec2(640.0, 220.0), false),
            (egui::vec2(640.0, 320.0), false),
            (egui::vec2(1000.0, 500.0), false),
        ] {
            let mut slot = egui::Rect::NOTHING;
            let mut output = None;
            for _ in 0..3 {
                output = Some(
                    ctx.run_test(input(true, size + egui::vec2(40.0, 40.0)), |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            // Review's photo slots have a horizontal parent layout.
                            ui.horizontal_top(|ui| {
                                slot = egui::Rect::from_min_size(ui.cursor().min, size);
                                app.video_canvas(ui, MediaId(1), 0, size);
                            });
                        });
                    }),
                );
            }
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            for label in if compact {
                vec!["Play", "Controls"]
            } else {
                vec!["Play", "−10", "+10", "Audio"]
            } {
                let (_, node) = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .expect(label);
                let b = node.bounds().unwrap();
                assert!(
                    b.x0 >= f64::from(slot.left())
                        && b.x1 <= f64::from(slot.right())
                        && b.y0 >= f64::from(slot.top())
                        && b.y1 <= f64::from(slot.bottom()),
                    "{label} {size:?}: {b:?} outside {slot:?}"
                );
            }
            if !compact {
                let (_, node) = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.value() == Some("0:00.0 / 0:30.0"))
                    .expect("full time counter");
                let b = node.bounds().unwrap();
                assert!(
                    b.x0 >= f64::from(slot.left())
                        && b.x1 <= f64::from(slot.right())
                        && b.y0 >= f64::from(slot.top())
                        && b.y1 <= f64::from(slot.bottom()),
                    "time {size:?}: {b:?} outside {slot:?}"
                );
                let (_, play) = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some("Play"))
                    .unwrap();
                assert!(
                    b.y0 > play.bounds().unwrap().y1,
                    "time belongs below the buttons, not off their right edge"
                );
            }
        }
        assert!(app.requested.is_empty());
        app.runtime.shutdown();
    }

    #[test]
    fn short_video_slots_reserve_picture_space_across_workflows_without_reuploading() {
        let (mut app, _root) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let _ = ctx.run_test(input(true, egui::vec2(800.0, 600.0)), |ui| {
            let ctx = &ui.ctx().clone();
            app.sync_playback(ctx)
        });
        let until = Instant::now() + worker_patience();
        while app.runtime.playback.snapshot().frame.is_none() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        for step in Step::ALL {
            app.step = step;
            // Workflow changes intentionally pause/reset presentation; resizing
            // within that workflow must retain its existing frame texture.
            let mut texture = None;
            for height in [120.0, 131.0, 160.0, 200.0, 220.0, 320.0] {
                let size = egui::vec2(640.0, height);
                let output = ctx.run_test(input(true, size + egui::vec2(40.0, 40.0)), |ui| {
                    let ctx = &ui.ctx().clone();
                    {
                        app.sync_playback(ctx);
                        egui::CentralPanel::default().show(ui, |ui| {
                            app.video_canvas(ui, MediaId(1), 0, size);
                        });
                    }
                });
                let current = app.video_texture.as_ref().unwrap().2.id();
                if let Some(previous) = texture {
                    assert_eq!(previous, current, "Resizing must reuse the video texture");
                }
                texture = Some(current);
                let picture = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::epaint::Shape::Mesh(mesh) if mesh.texture_id == current => {
                            Some(mesh.calc_bounds())
                        }
                        _ => None,
                    })
                    .expect("rendered video picture");
                assert!(
                    picture.height() >= height * 0.5,
                    "Video picture {:?} consumes less than half the {height}-point slot",
                    picture
                );
            }
        }
        assert!(
            app.requested.is_empty(),
            "Layout must not request photo processing"
        );
        app.runtime.shutdown();
    }

    #[test]
    fn video_space_and_seek_keys_are_owned_by_playback_without_navigating_the_library() {
        let (mut app, _root) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let mut raw = input(true, egui::vec2(800.0, 600.0));
        raw.events.push(egui::Event::Key {
            key: egui::Key::Space,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run_test(raw, |ui| {
            let ctx = &ui.ctx().clone();
            {
                app.keyboard(ctx);
                app.sync_playback(ctx);
            }
        });
        assert!(app.runtime.playback.snapshot().play_requested);
        assert_eq!(app.active, Some(MediaId(1)));
        let until = Instant::now() + worker_patience();
        while !app.runtime.playback.snapshot().ready {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut raw = input(true, egui::vec2(800.0, 600.0));
        raw.events.push(egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::ALT,
        });
        let _ = ctx.run_test(raw, |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        });
        assert_eq!(app.active, Some(MediaId(1)));
        app.runtime.shutdown();
    }
}
