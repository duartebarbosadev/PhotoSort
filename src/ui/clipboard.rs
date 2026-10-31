//! Cmd/Ctrl+C copies the selected photos to the system clipboard, with a toast
//! saying what was copied and where it can be pasted.
use super::*;
use crate::core::clipboard::Payload;

const SHOWN: f64 = 2.6;
const SHOWN_ERROR: f64 = 4.5;

pub(super) struct Toast {
    text: String,
    error: bool,
    /// `egui` input time when it appeared.
    since: Option<f64>,
}

fn file_manager() -> &'static str {
    if cfg!(target_os = "macos") {
        "Finder"
    } else if cfg!(target_os = "windows") {
        "File Explorer"
    } else {
        "your file manager"
    }
}

/// "Photo copied - paste it in Finder or any app", "3 photos copied - ...".
fn copied_text(media: &[Media]) -> String {
    let videos = media.iter().filter(|m| m.kind == MediaKind::Video).count();
    let noun = match (videos, media.len()) {
        (0, _) => "photo",
        (v, n) if v == n => "video",
        _ => "file",
    };
    let place = file_manager();
    if let [_] = media {
        let mut noun = noun.to_owned();
        noun[..1].make_ascii_uppercase();
        format!("{noun} copied - paste it in {place} or any app")
    } else {
        format!(
            "{} {noun}s copied - paste them in {place} or any app",
            media.len()
        )
    }
}

impl PhotoSortApp {
    /// What Cmd/Ctrl+C copies: the selected photos in the order shown, or the
    /// photo the keyboard acts on when nothing is selected.
    pub(super) fn copy_media(&self) -> Vec<Media> {
        let selected = if self.library_selection_mode() && self.comparison_focus.is_none() {
            self.selection.ordered(&self.media, &self.filtered)
        } else {
            Vec::new()
        };
        let ids = if selected.is_empty() {
            let focused = match self.step {
                Step::PickBest => self.pick_current.or(self.active),
                Step::EasyDelete => self.active.or(self.delete_current),
                _ => self.comparison_focus.or(self.active),
            };
            focused.into_iter().collect()
        } else {
            selected
        };
        ids.into_iter()
            .filter_map(|id| self.index.get(&id).and_then(|i| self.media.get(*i)))
            .cloned()
            .collect()
    }

    /// Takes the frame's copy request, which egui sends for Cmd/Ctrl+C instead
    /// of a key press. The caller has already let a focused text field have it.
    pub(super) fn copy_shortcut(&mut self, ctx: &egui::Context) {
        let mut requested = false;
        ctx.input_mut(|input| {
            input.events.retain(|event| {
                let copy = matches!(event, egui::Event::Copy);
                requested |= copy;
                !copy
            })
        });
        if requested && self.root.is_some() {
            self.copy_photos(ctx);
        }
    }

    fn copy_photos(&mut self, ctx: &egui::Context) {
        let media = self.copy_media();
        let (text, error) = match Payload::new(media.clone()) {
            None => ("Select a photo to copy".to_owned(), true),
            Some(payload) => match self.copier.copy(payload, crate::core::session::cache_dir()) {
                Ok(()) => (copied_text(&media), false),
                Err(problem) => (format!("Couldn't copy: {problem:#}"), true),
            },
        };
        self.toast = Some(Toast {
            text,
            error,
            since: None,
        });
        ctx.request_repaint();
    }

    pub(super) fn show_toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &mut self.toast else { return };
        let now = ctx.input(|i| i.time);
        let since = *toast.since.get_or_insert(now);
        let life = if toast.error { SHOWN_ERROR } else { SHOWN };
        let age = now - since;
        if age >= life {
            self.toast = None;
            return;
        }
        // Fades in and out over the interface animation time: none with
        // Reduce motion, so the toast simply appears and goes.
        let fade = f64::from(ctx.global_style().animation_time);
        let opacity = if fade > 0.0 {
            (age / fade).min((life - age) / fade).clamp(0.0, 1.0) as f32
        } else {
            1.0
        };
        if opacity < 1.0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(life - age - fade));
        }
        let (icon, color) = if toast.error {
            (egui_phosphor::regular::WARNING_CIRCLE, RED)
        } else {
            (egui_phosphor::regular::COPY, GREEN)
        };
        let text = toast.text.clone();
        egui::Area::new(egui::Id::new("toast"))
            .order(egui::Order::Foreground)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                egui::vec2(0.0, -(navigation::STATUS_HEIGHT + S4)),
            )
            .interactable(false)
            .show(ctx, |ui| {
                ui.multiply_opacity(opacity);
                egui::Frame::new()
                    .fill(ELEVATED)
                    .stroke(egui::Stroke::new(1.0, LINE))
                    .corner_radius(R_SURFACE)
                    .inner_margin(egui::Margin::symmetric(S4 as i8, S2 as i8 + 2))
                    .shadow(egui::Shadow {
                        offset: [0, 4],
                        blur: 16,
                        spread: 0,
                        color: Color32::from_black_alpha(90),
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = S2;
                            ui.label(egui::RichText::new(icon).color(color).size(16.0));
                            ui.add(egui::Label::new(egui::RichText::new(text).color(INK)).extend());
                        });
                    });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn photo(id: u64, name: &str) -> Media {
        Media {
            id: MediaId(id),
            path: PathBuf::from("/absent/library").join(name),
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        }
    }
    fn copy_event(app: &mut PhotoSortApp, ctx: &egui::Context) {
        let _ = ctx.run_test(
            egui::RawInput {
                events: vec![egui::Event::Copy],
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            },
        );
    }
    #[test]
    fn copy_takes_the_selection_in_shown_order_or_the_active_photo_and_leaves_text_fields_alone() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.root = Some("/absent/library".into());
        app.step = Step::Cull;
        app.media = (1..=4).map(|n| photo(n, &format!("{n}.jpg"))).collect();
        for (i, media) in app.media.iter().enumerate() {
            app.index.insert(media.id, i);
        }
        app.filtered = vec![3, 2, 1, 0];
        app.active = Some(MediaId(2));
        let paths = |app: &PhotoSortApp| -> Vec<PathBuf> {
            app.copy_media().into_iter().map(|m| m.path).collect()
        };
        assert_eq!(paths(&app), [PathBuf::from("/absent/library/2.jpg")]);
        app.selection.replace([MediaId(1), MediaId(3)]);
        assert_eq!(
            paths(&app),
            [
                PathBuf::from("/absent/library/3.jpg"),
                PathBuf::from("/absent/library/1.jpg"),
            ]
        );
        // Guided steps copy the photo they show, not a Cull selection.
        app.step = Step::Rotation;
        assert_eq!(paths(&app), [PathBuf::from("/absent/library/2.jpg")]);

        // A focused text field copies its own text; nothing reaches the clipboard.
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let field = egui::Id::new("copy_text_field");
        let mut text = "query".to_owned();
        let _ = ctx.run_test(egui::RawInput::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(field))
                .request_focus();
        });
        copy_event(&mut app, &ctx);
        assert!(app.toast.is_none());
        // Without one, the missing file is reported instead of copied.
        ctx.memory_mut(|memory| memory.surrender_focus(field));
        copy_event(&mut app, &ctx);
        let toast = app.toast.as_ref().unwrap();
        assert!(toast.error && toast.text.contains("2.jpg is no longer on disk"));
        app.runtime.shutdown();
    }
}
