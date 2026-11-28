//! The intros: a 54-second film of the workflow, played once over the window
//! on first launch, and a shorter film for each step, played the first time
//! that step is opened with a folder in it. Menu plays either again. The
//! film has a player's controls along its bottom edge: play and pause, a
//! bar to move through it, and its sound.
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
/// Keys that close the intro. Space pauses it, as in any player.
const SKIP_KEYS: [egui::Key; 2] = [egui::Key::Escape, egui::Key::Enter];
/// Seconds the arrow keys move through the film.
const STEP_SECONDS: f64 = 5.0;
/// How loud the film plays with its sound on: the player's own level.
const VOLUME: f32 = 0.5;
/// How far from the film's end the bar can be dragged, in seconds: the end
/// itself closes the film, which a drag must not do by accident.
const END_MARGIN: f64 = 0.4;
/// The height of the shade the controls sit on, over the film's bottom edge.
const SHADE: f32 = 96.0;

/// Which film: the tour of the whole workflow, or one step's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Clip {
    Tour,
    Step(Step),
}
impl Clip {
    /// The film's file under the resources' `intro` folder.
    fn file(self) -> &'static str {
        match self {
            Self::Tour => "intro.mp4",
            Self::Step(Step::Organize) => "organize.mp4",
            Self::Step(Step::EasyDelete) => "easy-delete.mp4",
            Self::Step(Step::Rotation) => "fix-rotation.mp4",
            Self::Step(Step::PickBest) => "pick-best.mp4",
            Self::Step(Step::Cull) => "cull.mp4",
            Self::Step(Step::Review) => "review.mp4",
        }
    }
}
pub(super) enum Intro {
    Off,
    /// Plays at the next frame with no other dialog open.
    Due(Clip, PathBuf),
    Playing(Box<Film>),
}
/// The intro's own player: the review player follows the focused library
/// video and would stop the film on its next frame.
pub(super) struct Film {
    pub(super) clip: Clip,
    player: playback::Player,
    /// Context time when the film was opened, and when its card appeared.
    opened: f64,
    shown: Option<f64>,
    texture: Option<(u64, TextureHandle)>,
}
impl Intro {
    /// Due on first launch only.
    pub(super) fn at_startup(preferences: &Preferences, autoplay: bool) -> Self {
        if !autoplay || preferences.intro_shown {
            return Self::Off;
        }
        film_path(Clip::Tour).map_or(Self::Off, |path| Self::Due(Clip::Tour, path))
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
/// A bundled film: in the app bundle's resources, or the checkout's.
fn film_path(clip: Clip) -> Option<PathBuf> {
    crate::core::models::resource_roots()
        .into_iter()
        .map(|root| root.join("intro").join(clip.file()))
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
/// Seconds as a clock: "0:07", "1:12".
fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
/// A round button of the controls, drawn as its icon alone.
fn control(
    ui: &egui::Ui,
    id: egui::Id,
    center: egui::Pos2,
    icon: &str,
    name: &str,
    key: &str,
) -> bool {
    let rect = egui::Rect::from_center_size(center, Vec2::splat(32.0));
    let response = named(
        ui.interact(rect, id, Sense::click()),
        egui::WidgetType::Button,
        name,
    );
    if response.hovered() {
        ui.painter()
            .circle_filled(center, 16.0, Color32::from_white_alpha(34));
    }
    ui.painter().text(
        center,
        egui::Align2::CENTER_CENTER,
        icon,
        medium(18.0),
        Color32::WHITE,
    );
    response.on_hover_text(format!("{name} · {key}")).clicked()
}
/// The controls along the film's bottom edge, on a shade that fades up into
/// the picture: play or pause, where the film is, a bar to move through it,
/// and the sound. Returns the sound wanted when its button is pressed.
fn controls(
    ui: &egui::Ui,
    card: egui::Rect,
    player: &playback::Player,
    state: &playback::Snapshot,
    muted: bool,
) -> Option<bool> {
    use egui_phosphor::regular as icon;
    let painter = ui.painter();
    let radius = f32::from(RADIUS);
    let (clear, dark) = (Color32::TRANSPARENT, Color32::from_black_alpha(200));
    // The shade ends above the card's round corners; a strip with the same
    // corners finishes it.
    let shade = egui::Rect::from_min_max(
        egui::pos2(card.left(), card.bottom() - SHADE),
        egui::pos2(card.right(), card.bottom() - radius),
    );
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(shade.left_top(), clear);
    mesh.colored_vertex(shade.right_top(), clear);
    mesh.colored_vertex(shade.left_bottom(), dark);
    mesh.colored_vertex(shade.right_bottom(), dark);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_filled(
        egui::Rect::from_min_max(shade.left_bottom(), card.right_bottom()),
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: RADIUS,
            se: RADIUS,
        },
        dark,
    );
    let id = egui::Id::new("intro-controls");
    let y = card.bottom() - 28.0;
    let (left, right) = (card.left() + 22.0, card.right() - 22.0);
    let (glyph, name) = if state.play_requested {
        (icon::PAUSE, "Pause")
    } else {
        (icon::PLAY, "Play")
    };
    if control(
        ui,
        id.with("play"),
        egui::pos2(left + 12.0, y),
        glyph,
        name,
        "Space",
    ) {
        player.toggle();
    }
    // Where the film is, and how long it runs.
    let at = state.seek_target.unwrap_or(state.position);
    let time = painter.text(
        egui::pos2(left + 40.0, y),
        egui::Align2::LEFT_CENTER,
        format!("{} / {}", clock(at), clock(state.duration)),
        medium(12.0),
        Color32::from_white_alpha(220),
    );
    let (glyph, name) = if muted {
        (icon::SPEAKER_X, "Unmute")
    } else {
        (icon::SPEAKER_HIGH, "Mute")
    };
    let sound = control(
        ui,
        id.with("sound"),
        egui::pos2(right - 12.0, y),
        glyph,
        name,
        "M",
    )
    .then_some(muted);
    // The bar: a click or a drag anywhere on it moves the film there.
    let track = egui::Rect::from_min_max(
        egui::pos2(time.right().max(left + 112.0) + 16.0, y - 2.0),
        egui::pos2(right - 44.0, y + 2.0),
    );
    if track.width() > 24.0 {
        let response = named(
            ui.interact(
                track.expand2(egui::vec2(6.0, 12.0)),
                id.with("seek"),
                Sense::click_and_drag(),
            ),
            egui::WidgetType::Slider,
            "Position",
        );
        let length = state.duration.max(0.001);
        if state.ready
            && (response.is_pointer_button_down_on() || response.clicked())
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let wanted =
                f64::from(((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0)) * length;
            let wanted = wanted.min((length - END_MARGIN).max(0.0));
            if (wanted - at).abs() > 0.05 {
                player.seek(wanted);
            }
        }
        let held = response.hovered() || response.is_pointer_button_down_on();
        let played = track.left() + track.width() * (at / length).clamp(0.0, 1.0) as f32;
        painter.rect_filled(track, 2.0, Color32::from_white_alpha(56));
        painter.rect_filled(
            egui::Rect::from_min_max(track.min, egui::pos2(played, track.bottom())),
            2.0,
            ACCENT,
        );
        painter.circle_filled(
            egui::pos2(played, y),
            if held { 7.0 } else { 5.0 },
            Color32::WHITE,
        );
    }
    sound
}
fn player(ctx: &egui::Context) -> playback::Player {
    // Unit tests script playback; a real decoder makes them timing-dependent.
    #[cfg(test)]
    {
        let _ = ctx;
        playback::test_player()
    }
    #[cfg(not(test))]
    {
        let ctx = ctx.clone();
        playback::Player::new(Arc::new(move || ctx.request_repaint()))
    }
}

impl PhotoSortApp {
    /// Menu → Watch the intro.
    pub(super) fn watch_intro(&mut self) {
        self.watch(Clip::Tour);
    }
    /// Menu → Watch the intro of the step on screen.
    pub(super) fn watch_step_intro(&mut self) {
        self.watch(Clip::Step(self.step));
    }
    fn watch(&mut self, clip: Clip) {
        match film_path(clip) {
            Some(path) => self.intro = Intro::Due(clip, path),
            None => self.set_status("The intro film is missing from this installation".into()),
        }
    }
    /// The step on screen is due its own film the first time it is opened
    /// with a folder in it. Without a folder there is nothing to explain yet.
    fn step_intro_due(&mut self) {
        let step = self.step;
        match &self.intro {
            // The step was left before its film could start.
            Intro::Due(Clip::Step(due), _) if *due != step => self.intro = Intro::Off,
            Intro::Off
                if self.step_intros
                    && self.root.is_some()
                    && !self.preferences_unreadable
                    && !self.preferences.step_intros_shown.contains(&step) =>
            {
                match film_path(Clip::Step(step)) {
                    Some(path) => self.intro = Intro::Due(Clip::Step(step), path),
                    // Nothing to play: not looked for again on every frame.
                    None => {
                        self.preferences.step_intros_shown.insert(step);
                    }
                }
            }
            _ => {}
        }
    }
    /// The film's keys, taken before any other keyboard handling can see
    /// them: Esc or Enter closes it, Space pauses, the arrows move through
    /// it and M turns its sound off and on.
    pub(super) fn intro_keys(&mut self, ctx: &egui::Context) {
        let Intro::Playing(film) = &self.intro else {
            return;
        };
        let pressed = |key| ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
        // Consume every skip key, so none reaches the window once it closes.
        if SKIP_KEYS.into_iter().filter(|key| pressed(*key)).count() > 0 {
            tracing::debug!("intro skipped");
            self.intro = Intro::Off;
            return;
        }
        if pressed(egui::Key::Space) {
            film.player.toggle();
        }
        if pressed(egui::Key::ArrowLeft) {
            film.player.seek_relative(-STEP_SECONDS);
        }
        if pressed(egui::Key::ArrowRight) {
            film.player.seek_relative(STEP_SECONDS);
        }
        if pressed(egui::Key::M) {
            self.set_intro_sound(self.preferences.intro_muted);
        }
    }
    /// Turns the films' sound on or off, for this film and the next ones.
    fn set_intro_sound(&mut self, on: bool) {
        if let Intro::Playing(film) = &self.intro {
            film.player.volume(if on { VOLUME } else { 0.0 });
        }
        if self.preferences.intro_muted == on {
            self.preferences.intro_muted = !on;
            self.save_preferences();
        }
    }
    fn start_intro(&mut self, ctx: &egui::Context, clip: Clip, path: PathBuf) {
        self.intro = Intro::Off;
        // Remembered as it opens: quitting part-way through, or a film that
        // cannot play, must not bring it back on every launch.
        let first = match clip {
            Clip::Tour => !std::mem::replace(&mut self.preferences.intro_shown, true),
            Clip::Step(step) => self.preferences.step_intros_shown.insert(step),
        };
        if first {
            self.save_preferences();
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
        if self.preferences.intro_muted {
            player.volume(0.0);
        }
        player.toggle();
        self.intro = Intro::Playing(Box::new(Film {
            clip,
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
        self.step_intro_due();
        if matches!(self.intro, Intro::Due(..))
            && !self.other_modal_active()
            && !self.departure_pending()
            && self.crash_notice.is_none()
            && self.error.is_none()
            && let Intro::Due(clip, path) = std::mem::replace(&mut self.intro, Intro::Off)
        {
            self.start_intro(ctx, clip, path);
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
        // What a screen reader calls the film.
        let name = match film.clip {
            Clip::Tour => "PhotoSort intro".to_owned(),
            Clip::Step(step) => format!("{} intro", step.title()),
        };
        let id = egui::Id::new("intro");
        let (player, muted) = (&film.player, self.preferences.intro_muted);
        let (skipped, sound) = egui::Modal::new(id)
            .area(egui::Modal::default_area(id).fade_in(false))
            .backdrop_color(Color32::from_black_alpha((170.0 * alpha) as u8))
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                let (card, response) = ui.allocate_exact_size(size, Sense::click());
                // A click on the picture pauses it and plays it again.
                if named(response, egui::WidgetType::Image, name.as_str()).clicked() {
                    player.toggle();
                }
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
                let sound = controls(ui, card, player, &state, muted);
                // Skip floats over the film's top-right corner: a pill drawn
                // by hand, so its words and its icon sit centred in it.
                let inset = f32::from(RADIUS);
                let words = painter.layout_no_wrap("Skip intro".to_owned(), medium(12.0), INK);
                let glyph = painter.layout_no_wrap(
                    egui_phosphor::regular::SKIP_FORWARD.to_owned(),
                    medium(14.0),
                    INK,
                );
                let (pad, gap) = (14.0, 7.0);
                let pill = egui::Rect::from_min_size(
                    egui::pos2(
                        card.right() - inset - 2.0 * pad - words.size().x - gap - glyph.size().x,
                        card.top() + inset,
                    ),
                    egui::vec2(2.0 * pad + words.size().x + gap + glyph.size().x, 30.0),
                );
                let skip = named(
                    ui.interact(pill, id.with("skip"), Sense::click()),
                    egui::WidgetType::Button,
                    "Skip intro",
                );
                painter.rect(
                    pill,
                    15.0,
                    Color32::from_black_alpha(if skip.hovered() { 200 } else { 150 }),
                    Stroke::new(1.0, Color32::from_white_alpha(30)),
                    StrokeKind::Inside,
                );
                let middle = pill.center().y;
                let after = pill.left() + pad + words.size().x + gap;
                painter.galley(
                    egui::pos2(pill.left() + pad, middle - words.size().y / 2.0),
                    words,
                    INK,
                );
                painter.galley(egui::pos2(after, middle - glyph.size().y / 2.0), glyph, INK);
                let skipped = skip.on_hover_text("Esc").clicked();
                (skipped, sound)
            })
            .inner;
        if let Some(on) = sound {
            self.set_intro_sound(on);
        }
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
        assert!(matches!(
            Intro::at_startup(&fresh, true),
            Intro::Due(Clip::Tour, _)
        ));
        // Every step has a film of its own beside it.
        for step in Step::ALL {
            assert!(film_path(Clip::Step(step)).is_some(), "{step:?}");
        }
    }
}

#[cfg(test)]
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
        assert!(matches!(app.intro, Intro::Due(..)));
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
        for label in ["Skip intro", "PhotoSort intro", "Pause", "Position", "Mute"] {
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
        assert!(matches!(app.intro, Intro::Due(..)));
        frame(&mut app, &ctx, vec![]);
        assert!(app.intro.playing());
        // Space pauses it and M turns its sound off, for the next film too;
        // neither closes it. Enter does.
        let film = |app: &PhotoSortApp| match &app.intro {
            Intro::Playing(film) => film.player.snapshot(),
            _ => panic!("the film closed"),
        };
        assert!(film(&app).play_requested && film(&app).volume > 0.0);
        frame(
            &mut app,
            &ctx,
            vec![key(egui::Key::Space), key(egui::Key::M)],
        );
        assert!(!film(&app).play_requested);
        assert_eq!(film(&app).volume, 0.0);
        assert!(app.preferences.intro_muted);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)]);
        assert!(matches!(app.intro, Intro::Off));
        app.watch_intro();
        frame(&mut app, &ctx, vec![]);
        assert_eq!(film(&app).volume, 0.0, "still muted");
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
                assert!(matches!(app.intro, Intro::Due(..)), "{name}");
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
    /// The film playing, if any.
    fn playing(app: &PhotoSortApp) -> Option<Clip> {
        match &app.intro {
            Intro::Playing(film) => Some(film.clip),
            _ => None,
        }
    }
    /// Escape, then a frame of the intro alone.
    fn skip(app: &mut PhotoSortApp, ctx: &egui::Context) {
        let _ = ctx.run_test(input(vec![key(egui::Key::Escape)]), |ui| {
            let ctx = ui.ctx().clone();
            app.intro_keys(&ctx);
            app.intro_overlay(&ctx);
        });
    }

    #[test]
    fn a_step_plays_its_own_film_the_first_time_it_is_opened_with_a_folder() {
        let root = tempfile::tempdir().unwrap();
        let mut app = first_launch(root.path());
        app.step_intros = true;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        // The tour comes first. With no folder open, no step's film follows.
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), Some(Clip::Tour));
        skip(&mut app, &ctx);
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), None);
        assert!(app.preferences.step_intros_shown.is_empty());
        // A folder is open: the step on screen plays its film, once.
        app.root = Some(root.path().to_path_buf());
        let first = app.step;
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), Some(Clip::Step(first)));
        skip(&mut app, &ctx);
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), None, "already seen");
        // Another step has its own, and going back does not replay the first.
        app.step = Step::Cull;
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), Some(Clip::Step(Step::Cull)));
        skip(&mut app, &ctx);
        app.step = first;
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), None);
        // Menu plays one again.
        app.watch_step_intro();
        overlay(&mut app, &ctx);
        assert_eq!(playing(&app), Some(Clip::Step(first)));
        // Shutdown finishes every queued write: both are remembered for
        // the next launch.
        app.runtime.shutdown();
        assert_eq!(
            saved(root.path()).step_intros_shown,
            HashSet::from([first, Step::Cull])
        );
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
                |app, root| {
                    // Staged choices count for photos of the folder.
                    app.media.push(Media {
                        id: MediaId(1),
                        path: root.join("a.jpg"),
                        kind: MediaKind::Photo,
                        fingerprint: Default::default(),
                    });
                    app.index.insert(MediaId(1), 0);
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
