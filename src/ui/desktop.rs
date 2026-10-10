//! Shared path menus and desktop dispatch for scanned files and folders.
use super::*;
use crate::core::desktop::{Action, Request};
/// Owned only while a menu is open. Original and proposed paths remain distinct.
#[derive(Default)]
pub(super) struct Locations {
    pub source: Option<PathBuf>,
    pub relative: Option<PathBuf>,
    pub proposed: Option<PathBuf>,
}
pub(super) fn locations_menu(
    ui: &mut egui::Ui,
    paths: &Locations,
    enabled: bool,
) -> Option<Request> {
    let mut request = None;
    for action in [Action::Open, Action::Reveal, Action::Terminal] {
        let target = paths
            .source
            .as_deref()
            .and_then(|p| Request::for_path(p, action).ok());
        if ui
            .add_enabled(
                enabled && target.is_some(),
                egui::Button::new(action.label()),
            )
            .clicked()
        {
            request = target;
            ui.close();
        }
    }
    ui.separator();
    for (label, path) in [
        ("Copy full path", paths.source.as_deref()),
        ("Copy relative path", paths.relative.as_deref()),
        ("Copy proposed destination", paths.proposed.as_deref()),
    ] {
        if label == "Copy proposed destination" && path.is_none() {
            continue;
        }
        let text = path.and_then(std::path::Path::to_str);
        let response = ui.add_enabled(text.is_some(), egui::Button::new(label));
        if label == "Copy proposed destination" {
            response.clone().on_hover_text(
                "Preview moves confirms the final path, including any filename conflicts.",
            );
        }
        if response.clicked() {
            ui.ctx().copy_text(text.unwrap().to_owned());
            ui.close();
        }
    }
    request
}
impl PhotoSortApp {
    pub(super) fn desktop_available(&self) -> bool {
        !self.scanning
            && (!self.busy || self.analysis_run.is_some() || self.rotation_run.is_some())
            && !self.runtime.desktop.running()
    }
    pub(super) fn can_open_desktop(&self, id: MediaId) -> bool {
        self.desktop_available() && self.index.contains_key(&id)
    }
    pub(super) fn library_path_menu(&mut self, ui: &mut egui::Ui, id: MediaId) {
        let Some(media) = self.index.get(&id).and_then(|i| self.media.get(*i)) else {
            return;
        };
        let paths = Locations {
            source: Some(media.path.clone()),
            relative: self
                .root
                .as_ref()
                .and_then(|root| media.path.strip_prefix(root).ok())
                .map(PathBuf::from),
            proposed: None,
        };
        if let Some(request) = locations_menu(ui, &paths, self.can_open_desktop(id)) {
            self.dispatch_desktop(request);
        }
    }
    pub(super) fn desktop_request(&self, id: MediaId, action: Action) -> Option<Request> {
        if !self.can_open_desktop(id) {
            return None;
        }
        self.media
            .get(*self.index.get(&id)?)
            .and_then(|media| Request::for_media(media, action).ok())
    }
    pub(super) fn open_desktop(&mut self, id: MediaId, action: Action) {
        let Some(request) = self.desktop_request(id, action) else {
            return;
        };
        self.dispatch_desktop(request);
    }
    pub(super) fn dispatch_desktop(&mut self, request: Request) {
        if !self.desktop_available() {
            return;
        }
        let action = request.action;
        match self.runtime.desktop.start(self.runtime.epoch(), request) {
            Ok(()) if matches!(action, Action::Open | Action::OpenVideo) => {
                self.runtime.playback.pause()
            }
            Ok(()) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    pub(super) fn reveal_button(&mut self, ui: &mut egui::Ui, id: MediaId) -> bool {
        if ui
            .add_enabled(
                self.can_open_desktop(id),
                egui::Button::new(Action::Reveal.label()),
            )
            .clicked()
        {
            self.open_desktop(id, Action::Reveal);
            return true;
        }
        false
    }
    pub(super) fn poll_desktop(&mut self) {
        if let Some(completion) = self.runtime.desktop.take()
            && let Err(error) = completion.result
        {
            let message = format!(
                "{} failed for {}: {}",
                completion.request.action.label(),
                completion.request.path.display(),
                error
            );
            if let Some(editor) = &mut self.organize_editor {
                editor.error = Some(message.clone());
            }
            self.error = Some(message);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_path_menu_copies_exact_text_without_dispatching_or_mutating_files() {
        let paths = Locations {
            source: Some(PathBuf::from("/library/a ' café.jpg")),
            relative: Some(PathBuf::from("a ' café.jpg")),
            proposed: Some(PathBuf::from("/output/Chosen/Keeper.jpg")),
        };
        for (label, expected) in [
            ("Copy full path", "/library/a ' café.jpg"),
            ("Copy relative path", "a ' café.jpg"),
            ("Copy proposed destination", "/output/Chosen/Keeper.jpg"),
            ("Open terminal here", "/library/a ' café.jpg"),
        ] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 300.0),
                )),
                ..Default::default()
            };
            let selected = std::cell::RefCell::new(None);
            let draw = |ctx: &egui::Context| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(request) = locations_menu(ui, &paths, label == "Open terminal here")
                    {
                        *selected.borrow_mut() = Some(request);
                    }
                });
            };
            let output = ctx.run(input(), draw);
            let tree = output.platform_output.accesskit_update.unwrap();
            let node = tree
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .unwrap();
            let bounds = node.1.bounds().unwrap();
            assert!(bounds.x1 <= 320.0 && bounds.y1 <= 300.0);
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            let mut raw = input();
            raw.events = vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
            let output = ctx.run(raw, draw);
            if label == "Open terminal here" {
                assert_eq!(
                    *selected.borrow(),
                    Some(
                        Request::for_path(std::path::Path::new(expected), Action::Terminal)
                            .unwrap()
                    )
                );
                assert!(output.platform_output.commands.is_empty());
            } else {
                assert!(selected.borrow().is_none());
                assert!(
                    output.platform_output.commands.iter().any(
                        |c| matches!(c, egui::OutputCommand::CopyText(text) if text == expected)
                    )
                );
            }
        }
    }

    #[test]
    fn every_workflow_resolves_the_clicked_photo_without_mutating_reviews_or_requesting_images() {
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.media = (1..=2)
            .map(|n| Media {
                id: MediaId(n),
                path: dir.path().join(format!("photo{n}.jpg")),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.active = Some(MediaId(1));
        std::fs::write(&app.media[1].path, b"not decoded").unwrap();
        let (sent, received) = std::sync::mpsc::channel();
        app.runtime.desktop = crate::core::desktop::Worker::with_launcher(
            Arc::new(std::sync::atomic::AtomicU64::new(app.runtime.epoch())),
            Arc::new(|| {}),
            Arc::new(move |request| {
                sent.send(request.clone()).unwrap();
                Ok(())
            }),
        );
        app.decisions.insert(
            MediaId(1),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        for step in Step::ALL {
            app.step = step;
            for action in [Action::Reveal, Action::Open, Action::Terminal] {
                let request = app.desktop_request(MediaId(2), action).unwrap();
                assert_eq!(request.path, app.media[1].path);
                app.open_desktop(MediaId(2), action);
                let expected = if action == Action::Terminal {
                    Request::for_path(dir.path(), action).unwrap()
                } else {
                    request
                };
                assert_eq!(
                    received.recv_timeout(Duration::from_secs(5)).unwrap(),
                    expected
                );
                let deadline = Instant::now() + Duration::from_secs(5);
                while app.runtime.desktop.running() {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(2));
                }
                app.poll_desktop();
                assert!(app.error.is_none());
                assert_eq!(app.active, Some(MediaId(1)));
                assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
                assert!(app.requested.is_empty());
                assert_eq!(app.stats.decodes, 0);
                assert!(app.desktop_request(MediaId(2), Action::OpenVideo).is_none());
            }
        }
        app.busy = true;
        assert!(app.desktop_request(MediaId(2), Action::Reveal).is_none());
        app.analysis_run = Some(1);
        assert!(app.desktop_request(MediaId(2), Action::Reveal).is_some());
        app.scanning = true;
        assert!(app.desktop_request(MediaId(2), Action::Reveal).is_none());
        app.runtime.shutdown();
    }
    #[test]
    fn reveal_control_is_rendered_in_the_shared_inspector_for_every_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        let id = MediaId(1);
        app.media.push(Media {
            id,
            path: dir.path().join("photo.jpg"),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        });
        app.index.insert(id, 0);
        app.active = Some(id);
        for step in Step::ALL {
            app.step = step;
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320., 500.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.inspector(ui));
                },
            );
            assert!(
                output
                    .platform_output
                    .accesskit_update
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, n)| n.label() == Some(Action::Reveal.label()))
            );
            assert_eq!(app.stats.decodes, 0);
            assert!(app.requested.is_empty());
        }
        app.runtime.shutdown();
    }
}
