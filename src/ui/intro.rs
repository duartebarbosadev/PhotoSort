//! The first-run intro: a half-minute film of the workflow, played once over
//! the window on first launch and again from Menu → Watch the intro.
use super::*;
use crate::core::{playback, preferences::Preferences};

/// Builds whose first launch plays the intro. Tests and examples construct
/// the real app and must never start a film over it.
pub(super) const AUTOPLAY: bool = !cfg!(any(test, feature = "test-isolation"));
/// The card's corner radius, as in the Python app's intro.
const RADIUS: u8 = 18;
/// Seconds for the card to fade in once its first frame is ready.
const FADE: f64 = 0.32;
/// Seconds to wait for that frame before showing the card without it.
const FIRST_FRAME_WAIT: f64 = 0.5;
/// The film's shape until its first frame arrives.
const ASPECT: f32 = 16.0 / 9.0;
/// Keys that skip the intro, as in the Python app.
const SKIP_KEYS: [egui::Key; 3] = [egui::Key::Escape, egui::Key::Space, egui::Key::Enter];

pub(super) enum Intro {
    Off,
    /// Plays at the next frame with no other dialog open.
    Due(PathBuf),
    Playing(Box<Film>),
}
/// The intro's own player: the review player follows the focused library
/// video and would stop the film on its next frame.
pub(super) struct Film {
    player: playback::Player,
    /// Context time when the film was opened, and when its card appeared.
    opened: f64,
    shown: Option<f64>,
    texture: Option<(u64, TextureHandle)>,
}
impl Intro {
    /// Due on first launch only, where this build can play the bundled film.
    pub(super) fn at_startup(preferences: &Preferences, autoplay: bool) -> Self {
        if !autoplay || preferences.intro_shown || !playback::supported() {
            return Self::Off;
        }
        film_path().map_or(Self::Off, Self::Due)
    }
    pub(super) fn playing(&self) -> bool {
        matches!(self, Self::Playing(_))
    }
}
impl Film {
    /// A frame of the film is on the card.
    pub(super) fn ready(&self) -> bool {
        self.texture.is_some()
    }
}
/// The bundled film: in the app bundle's resources, or the checkout's.
fn film_path() -> Option<PathBuf> {
    crate::core::models::resource_roots()
        .into_iter()
        .map(|root| root.join("intro/intro.mp4"))
        .find(|path| path.is_file())
}
/// Card opacity `elapsed` seconds after it appeared: an ease-out fade, or
/// opaque at once when motion is reduced.
fn fade(elapsed: f64, reduce_motion: bool) -> f32 {
    if reduce_motion {
        return 1.0;
    }
    let t = (elapsed / FADE).clamp(0.0, 1.0);
    (1.0 - (1.0 - t).powi(3)) as f32
}
/// About three quarters of the window's width, at most 1000 points wide,
/// and always clear of the window's edges.
fn card_size(window: Vec2, aspect: f32) -> Vec2 {
    let room = (window - Vec2::splat(2.0 * S6)).max(Vec2::splat(1.0));
    let width = (window.x * 0.72)
        .clamp(480.0, 1000.0)
        .min(room.x)
        .min(room.y * aspect);
    egui::vec2(width, width / aspect)
}
fn player(ctx: &egui::Context) -> playback::Player {
    // Unit tests script playback; a real decoder makes them timing-dependent.
    #[cfg(all(test, any(target_os = "macos", feature = "portable-video")))]
    {
        let _ = ctx;
        playback::test_player()
    }
    #[cfg(not(all(test, any(target_os = "macos", feature = "portable-video"))))]
    {
        let ctx = ctx.clone();
        playback::Player::new(Arc::new(move || ctx.request_repaint()))
    }
}

impl PhotoSortApp {
    /// Menu → Watch the intro.
    pub(super) fn watch_intro(&mut self) {
        match film_path() {
            Some(path) => self.intro = Intro::Due(path),
            None => self.set_status("The intro film is missing from this installation".into()),
        }
    }
    /// Skip keys, taken before any other keyboard handling can see them.
    pub(super) fn intro_keys(&mut self, ctx: &egui::Context) {
        if self.intro.playing()
            && ctx.input_mut(|input| {
                // Consume every skip key, so none reaches the window once it closes.
                SKIP_KEYS
                    .into_iter()
                    .filter(|key| input.consume_key(egui::Modifiers::NONE, *key))
                    .count()
                    > 0
            })
        {
            tracing::debug!("intro skipped");
            self.intro = Intro::Off;
        }
    }
    fn start_intro(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.intro = Intro::Off;
        if !self.preferences.intro_shown {
            // Remembered as it opens: quitting part-way through, or a film
            // that cannot play, must not bring it back on every launch.
            self.preferences.intro_shown = true;
            // Never replace an unreadable file with defaults behind the
            // user's back; only an explicit Settings change does that.
            if !self.preferences_unreadable {
                self.save_preferences();
            }
        }
        let fingerprint = match Fingerprint::read(&path) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                tracing::warn!(%error, "intro unavailable");
                return;
            }
        };
        let media = Media {
            id: MediaId::for_path(&path),
            path,
            kind: MediaKind::Video,
            fingerprint,
        };
        let player = player(ctx);
        player.select(Some(&media));
        player.toggle();
        self.intro = Intro::Playing(Box::new(Film {
            player,
            opened: ctx.input(|i| i.time),
            shown: None,
            texture: None,
        }));
    }
    /// A quit, step or folder change waiting on an answer, in whichever
    /// prompt `request_departure` asked it.
    fn departure_pending(&self) -> bool {
        self.departure.is_some() || self.organize_departure.is_some() || self.after_apply.is_some()
    }
    /// Starts a due intro once nothing else asks for attention, then draws
    /// it over everything, including other dialogs.
    pub(super) fn intro_overlay(&mut self, ctx: &egui::Context) {
        if matches!(self.intro, Intro::Due(_))
            && !self.other_modal_active()
            && !self.departure_pending()
            && self.crash_notice.is_none()
            && self.error.is_none()
            && let Intro::Due(path) = std::mem::replace(&mut self.intro, Intro::Off)
        {
            self.start_intro(ctx, path);
        }
        // That answer must not wait under the film.
        if self.departure_pending() {
            self.intro = Intro::Off;
        }
        let Intro::Playing(film) = &mut self.intro else {
            return;
        };
        let state = film.player.snapshot();
        if let Some(error) = &state.error {
            tracing::warn!(%error, "intro could not play");
            self.intro = Intro::Off;
            return;
        }
        if state.ended {
            tracing::debug!("intro finished");
            self.intro = Intro::Off;
            return;
        }
        if let Some(frame) = &state.frame
            && film
                .texture
                .as_ref()
                .is_none_or(|(serial, _)| *serial != frame.serial)
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [
                    frame.pixels.width() as usize,
                    frame.pixels.height() as usize,
                ],
                frame.pixels.as_raw(),
            );
            match &mut film.texture {
                Some((serial, texture)) => {
                    texture.set(image, egui::TextureOptions::LINEAR);
                    *serial = frame.serial;
                }
                None => {
                    film.texture = Some((
                        frame.serial,
                        ctx.load_texture("intro-film", image, egui::TextureOptions::LINEAR),
                    ));
                }
            }
        }
        let now = ctx.input(|i| i.time);
        if film.shown.is_none() && (film.texture.is_some() || now - film.opened >= FIRST_FRAME_WAIT)
        {
            film.shown = Some(now);
        }
        // Invisible until the card appears, but already holding the input.
        let alpha = film.shown.map_or(0.0, |shown| {
            fade(now - shown, self.preferences.reduce_motion)
        });
        if alpha < 1.0 {
            ctx.request_repaint();
        }
        let aspect = film.texture.as_ref().map_or(ASPECT, |(_, texture)| {
            let size = texture.size_vec2();
            size.x / size.y.max(1.0)
        });
        let size = card_size(ctx.content_rect().size(), aspect);
        let texture = film.texture.as_ref().map(|(_, texture)| texture.id());
        let id = egui::Id::new("intro");
        let skipped = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).fade_in(false))
            .backdrop_color(Color32::from_black_alpha((170.0 * alpha) as u8))
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                let (card, response) = ui.allocate_exact_size(size, Sense::hover());
                let _ = named(response, egui::WidgetType::Image, "PhotoSort intro");
                let painter = ui.painter();
                painter.add(
                    egui::Shadow {
                        offset: [0, 12],
                        blur: 48,
                        spread: 0,
                        color: Color32::from_black_alpha(160),
                    }
                    .as_shape(card, RADIUS),
                );
                match texture {
                    Some(texture) => painter.add(
                        egui::epaint::RectShape::filled(card, RADIUS, Color32::WHITE).with_texture(
                            texture,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        ),
                    ),
                    None => painter.add(egui::epaint::RectShape::filled(card, RADIUS, BG)),
                };
                painter.rect_stroke(
                    card,
                    RADIUS,
                    Stroke::new(1.0, Color32::from_white_alpha(14)),
                    StrokeKind::Inside,
                );
                // Skip floats over the film's top-right corner.
                let mut corner = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(card.shrink(f32::from(RADIUS)))
                        .layout(egui::Layout::right_to_left(Align::Min)),
                );
                let skip = corner.add(
                    egui::Button::new(
                        RichText::new(format!(
                            "Skip intro  {}",
                            egui_phosphor::regular::SKIP_FORWARD
                        ))
                        .font(medium(12.0))
                        .color(INK),
                    )
                    .fill(Color32::from_black_alpha(150))
                    .stroke(Stroke::new(1.0, Color32::from_white_alpha(30)))
                    .corner_radius(u8::MAX)
                    .min_size(egui::vec2(0.0, 30.0)),
                );
                named(skip, egui::WidgetType::Button, "Skip intro")
                    .on_hover_text("Esc")
                    .clicked()
            })
            .inner;
        if skipped {
            tracing::debug!("intro skipped");
            self.intro = Intro::Off;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_fades_in_with_ease_out_and_appears_at_once_under_reduce_motion() {
        assert_eq!(fade(0.0, false), 0.0);
        assert!(fade(FADE / 2.0, false) > 0.5, "ease-out starts quickly");
        assert_eq!(fade(FADE, false), 1.0);
        assert_eq!(fade(10.0, false), 1.0);
        for elapsed in [0.0, 0.05, FADE] {
            assert_eq!(fade(elapsed, true), 1.0);
        }
    }

    #[test]
    fn the_card_keeps_the_film_shape_inside_any_window() {
        for window in [
            egui::vec2(1440.0, 960.0),
            egui::vec2(3000.0, 2000.0),
            egui::vec2(640.0, 600.0),
            egui::vec2(1600.0, 420.0),
        ] {
            let card = card_size(window, ASPECT);
            assert!(
                (card.x / card.y - ASPECT).abs() < 1e-3,
                "{window:?}: {card:?}"
            );
            assert!(card.x <= 1000.0, "{window:?}: {card:?}");
            assert!(
                card.x <= window.x - 2.0 * S6 && card.y <= window.y - 2.0 * S6,
                "{window:?}: {card:?}"
            );
        }
        assert_eq!(card_size(egui::vec2(1440.0, 960.0), ASPECT).x, 1000.0);
    }

    #[test]
    fn startup_plays_the_intro_on_first_launch_only_and_never_in_test_builds() {
        let fresh = Preferences::default();
        // This crate's own tests build with test isolation: never autoplay.
        const { assert!(!AUTOPLAY) };
        assert!(matches!(Intro::at_startup(&fresh, AUTOPLAY), Intro::Off));
        let seen = Preferences {
            intro_shown: true,
            ..Default::default()
        };
        assert!(matches!(Intro::at_startup(&seen, true), Intro::Off));
        // The checkout's film is found from the crate root, where tests run.
        assert_eq!(
            matches!(Intro::at_startup(&fresh, true), Intro::Due(_)),
            playback::supported()
        );
    }
}

#[cfg(all(test, any(target_os = "macos", feature = "portable-video")))]
mod playback_tests {
    use super::*;
    use crate::test_support::worker_patience;

    fn input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            focused: true,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        }
    }
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }
    /// A first-launch app with its preferences in `root`.
    fn first_launch(root: &std::path::Path) -> PhotoSortApp {
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
        app.preferences_path = Some(root.join("preferences.json"));
        app.intro = Intro::at_startup(&app.preferences, true);
        assert!(matches!(app.intro, Intro::Due(_)));
        app
    }
    fn frame(app: &mut PhotoSortApp, ctx: &egui::Context, events: Vec<egui::Event>) {
        let _ = ctx.run_test(input(events), |ui| app.update_workspace(ui, false));
    }
    fn saved(root: &std::path::Path) -> Preferences {
        let until = Instant::now() + worker_patience();
        loop {
            if let Ok(preferences) = Preferences::load(&root.join("preferences.json"))
                && preferences.intro_shown
            {
                return preferences;
            }
            assert!(Instant::now() < until, "intro_shown was never saved");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_first_launch_plays_the_intro_once_and_remembers_it() {
        let root = tempfile::tempdir().unwrap();
        let mut app = first_launch(root.path());
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        frame(&mut app, &ctx, vec![]);
        assert!(app.intro.playing());
        assert!(app.other_modal_active(), "the film holds the keyboard");
        assert!(app.preferences.intro_shown);
        let preferences = saved(root.path());
        // The next launch reads that back and stays quiet.
        assert!(matches!(Intro::at_startup(&preferences, true), Intro::Off));
        let output = ctx.run_test(input(vec![]), |ui| app.update_workspace(ui, false));
        let tree = output.platform_output.accesskit_update.unwrap();
        for label in ["Skip intro", "PhotoSort intro"] {
            assert!(
                tree.nodes.iter().any(|(_, n)| n.label() == Some(label)),
                "{label}"
            );
        }
        // The scripted film ends; the card closes by itself.
        let Intro::Playing(film) = &app.intro else {
            unreachable!()
        };
        let until = Instant::now() + worker_patience();
        while !film.player.snapshot().ready {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        film.player.seek(29.95);
        while !film.player.snapshot().ended {
            assert!(Instant::now() < until, "scripted film never ended");
            std::thread::sleep(Duration::from_millis(2));
        }
        frame(&mut app, &ctx, vec![]);
        assert!(matches!(app.intro, Intro::Off));
        assert!(!app.other_modal_active());
        app.runtime.shutdown();
    }

    #[test]
    fn smoke_tests_never_play_the_intro_or_use_up_the_first_launch() {
        let root = tempfile::tempdir().unwrap();
        let mut app = first_launch(root.path()).with_smoke_test(Duration::from_secs(60));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for _ in 0..3 {
            frame(&mut app, &ctx, vec![]);
            assert!(matches!(app.intro, Intro::Off));
        }
        assert!(!app.preferences.intro_shown);
        app.runtime.shutdown();
        assert!(!root.path().join("preferences.json").exists());
    }

    #[test]
    fn escape_skips_the_intro_without_reaching_the_window_and_menu_replays_it() {
        let root = tempfile::tempdir().unwrap();
        let mut app = first_launch(root.path());
        let ctx = egui::Context::default();
        theme_install(&ctx);
        frame(&mut app, &ctx, vec![]);
        assert!(app.intro.playing());
        let _ = ctx.run_test(input(vec![key(egui::Key::Escape)]), |ui| {
            let ctx = ui.ctx().clone();
            app.intro_keys(&ctx);
            assert!(
                !ctx.input(|i| i.key_pressed(egui::Key::Escape)),
                "Escape belongs to the intro alone"
            );
        });
        assert!(matches!(app.intro, Intro::Off));
        let _ = saved(root.path());
        app.watch_intro();
        assert!(matches!(app.intro, Intro::Due(_)));
        frame(&mut app, &ctx, vec![]);
        assert!(app.intro.playing());
        frame(&mut app, &ctx, vec![key(egui::Key::Space)]);
        assert!(matches!(app.intro, Intro::Off));
        app.runtime.shutdown();
    }

    #[test]
    fn a_due_intro_waits_for_an_open_dialog_or_error_and_yields_to_a_departure() {
        type Toggle = fn(&mut PhotoSortApp, bool);
        type Shown = fn(&PhotoSortApp) -> bool;
        let cases: [(&str, Toggle, Shown); 2] = [
            (
                "about dialog",
                |app, open| app.about = open,
                |app| app.about,
            ),
            (
                "error window",
                |app, open| app.error = open.then(|| "Folder is no longer available".into()),
                |app| app.error.is_some(),
            ),
        ];
        for (name, show, shown) in cases {
            let root = tempfile::tempdir().unwrap();
            let mut app = first_launch(root.path());
            let ctx = egui::Context::default();
            theme_install(&ctx);
            show(&mut app, true);
            for _ in 0..2 {
                frame(&mut app, &ctx, vec![]);
                assert!(matches!(app.intro, Intro::Due(_)), "{name}");
                assert!(shown(&app), "{name}: the intro closed it");
                assert!(!app.preferences.intro_shown, "{name}");
            }
            show(&mut app, false);
            frame(&mut app, &ctx, vec![]);
            assert!(app.intro.playing(), "{name}");
            app.departure = Some(Destination::Close);
            app.intro_overlay(&ctx);
            assert!(matches!(app.intro, Intro::Off), "{name}");
            app.runtime.shutdown();
        }
    }

    /// One frame of an intro with nothing else drawn.
    fn overlay(app: &mut PhotoSortApp, ctx: &egui::Context) {
        let _ = ctx.run_test(input(vec![]), |ui| app.intro_overlay(&ui.ctx().clone()));
    }
    /// One frame of the native Quit path: the shared departure guard, then the intro.
    fn native_quit(app: &mut PhotoSortApp, ctx: &egui::Context) {
        let _ = ctx.run_test(input(vec![]), |ui| {
            let ctx = ui.ctx().clone();
            app.handle_close_request(&ctx, true);
            app.intro_overlay(&ctx);
        });
    }

    #[test]
    fn a_replayed_intro_never_saves_over_an_unreadable_preferences_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("preferences.json");
        let unreadable = b"[99, {\"show_shortcuts\": false}]";
        std::fs::write(&path, unreadable).unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.path().join("cache")), None);
        app.load_preferences(path.clone());
        assert!(app.preferences_unreadable && app.error.is_some());
        assert!(matches!(app.intro, Intro::Off), "no autoplay on defaults");
        let ctx = egui::Context::default();
        theme_install(&ctx);
        // The warning is dismissed, then Menu → Watch the intro.
        app.error = None;
        app.watch_intro();
        frame(&mut app, &ctx, vec![]);
        assert!(app.intro.playing());
        // Shutdown finishes every queued write.
        app.runtime.shutdown();
        assert_eq!(std::fs::read(&path).unwrap(), unreadable);
    }

    /// A folder renamed on the Organize board but not applied.
    fn unapplied_folder_edit(app: &mut PhotoSortApp, dir: &std::path::Path) {
        let root = dir.canonicalize().unwrap();
        std::fs::create_dir(root.join("Before")).unwrap();
        std::fs::write(root.join("Before/a.jpg"), b"original bytes").unwrap();
        let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
        app.root = Some(root.clone());
        app.output = root.display().to_string();
        app.media = catalog.media.clone();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.catalog = Some(Arc::new(catalog));
        let mut draft = crate::core::organize::Draft::from_catalog(
            app.catalog.as_ref().unwrap(),
            app.organize_labels(),
        );
        draft.rename_folder("Before", "After").unwrap();
        app.organize_drafts.insert(app.group_mode, draft);
    }

    #[test]
    fn every_quit_prompt_stops_the_intro_instead_of_waiting_under_it() {
        type Setup = fn(&mut PhotoSortApp, &std::path::Path);
        type Asked = fn(&PhotoSortApp) -> bool;
        let cases: [(&str, Setup, Asked); 3] = [
            (
                "staged choices",
                |app, _| {
                    app.decisions.insert(
                        MediaId(1),
                        Decision {
                            rejected: true,
                            ..Default::default()
                        },
                    );
                },
                |app| app.departure.is_some(),
            ),
            (
                "saved operations",
                |app, _| app.retry_plan = Some(Plan::default()),
                |app| app.after_apply.is_some() && app.plan_open,
            ),
            ("folder edits", unapplied_folder_edit, |app| {
                app.organize_departure.is_some()
            }),
        ];
        for (name, setup, asked) in cases {
            let root = tempfile::tempdir().unwrap();
            let mut app = first_launch(root.path());
            setup(&mut app, root.path());
            let ctx = egui::Context::default();
            theme_install(&ctx);
            overlay(&mut app, &ctx);
            assert!(app.intro.playing(), "{name}: plays until Quit");
            native_quit(&mut app, &ctx);
            assert!(asked(&app), "{name}: Quit asks in its own prompt");
            assert!(
                matches!(app.intro, Intro::Off),
                "{name}: not under the film"
            );
            assert!(!app.close_authorized, "{name}");
            // A prompt still waiting holds back a replay as well.
            app.watch_intro();
            overlay(&mut app, &ctx);
            assert!(!app.intro.playing(), "{name}: no replay over the prompt");
            app.runtime.shutdown();
        }
    }
}
