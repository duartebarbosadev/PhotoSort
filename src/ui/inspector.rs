use super::*;

impl PhotoSortApp {
    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.label(RichText::new("PHOTO DETAILS").small().color(MUTED));
        ui.add_space(16.0);
        let Some(id) = self.active else {
            ui.label("Select a photo");
            return;
        };
        let Some(&index) = self.index.get(&id) else {
            return;
        };
        let media = self.media[index].clone();
        ui.label(RichText::new(media.name()).size(17.0).strong());
        ui.label(
            RichText::new(format!(
                "{:.1} MB · {:?}",
                media.fingerprint.bytes as f64 / 1048576.0,
                media.kind
            ))
            .small()
            .color(MUTED),
        );
        self.reveal_button(ui, id);
        ui.add_space(10.0);
        ui.separator();
        if let Some(meta) = self.metadata.get(&id) {
            for (label, value) in [
                (
                    "Dimensions",
                    if meta.width > 0 && meta.height > 0 {
                        format!("{} × {}", meta.width, meta.height)
                    } else {
                        String::new()
                    },
                ),
                ("Camera", meta.camera.clone()),
                ("Lens", meta.lens.clone()),
                ("Captured", meta.captured.clone()),
                ("Exposure", meta.exposure.clone()),
                ("Aperture", meta.aperture.clone()),
                ("ISO", meta.iso.clone()),
            ] {
                if !value.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(label).small().color(MUTED));
                    ui.label(value);
                }
            }
        }
        if let Some(analysis) = self.analysis.get(&id) {
            ui.add_space(15.0);
            ui.separator();
            ui.label(RichText::new("PHOTO QUALITY").small().color(MUTED));
            ui.label(
                RichText::new(format!("{:.0} / 100", analysis.score))
                    .size(24.0)
                    .color(ACCENT),
            );
            ui.label(format!("Sharpness  {:.1}", analysis.sharpness));
            ui.label(format!("Brightness  {:.0} / 255", analysis.brightness));
            if let Some(faces) = &analysis.faces {
                ui.label(format!(
                    "{} faces · {} possibly closed eyes",
                    faces.count, faces.closed_eyes
                ));
            }
            for issue in &analysis.issues {
                ui.colored_label(RED, issue);
            }
            ui.label(
                RichText::new(if analysis.aesthetic.is_some() {
                    "Aesthetic quality, sharpness and open-eye suggestions."
                } else {
                    "Sharpness and exposure. Run Pick Best for aesthetic scoring."
                })
                .small()
                .color(MUTED),
            );
        }
        if let Some(original) = self.duplicates.get(&id)
            && let Some(&index) = self.index.get(original)
        {
            ui.colored_label(RED, "Exact duplicate");
            ui.label(self.media[index].name());
        }
        if let Some(other) = self.near_duplicates.get(&id).copied() {
            ui.label(RichText::new("Similar frame").color(ACCENT));
            if ui.button("Compare before marking").clicked() {
                self.compare_with(other);
                self.review = true;
            }
        }
        if self.step == Step::Organize {
            ui.add_space(12.0);
            ui.separator();
            ui.label(RichText::new("DESTINATION FOLDER").small().color(MUTED));
            let label = self
                .organize_drafts
                .get(&self.group_mode)
                .and_then(|draft| draft.labels().get(&id))
                .cloned()
                .unwrap_or_else(|| self.label_for(&media));
            ui.label(if label.is_empty() {
                "Library root"
            } else {
                &label
            });
            if ui
                .add_enabled(self.can_edit_folders(), egui::Button::new("Edit folders…"))
                .clicked()
            {
                self.open_organize_editor();
            }

            if let Some(name) = self.file_names.get(&id) {
                let after = crate::core::organize::renamed_file_name(
                    &media.path,
                    std::ffi::OsStr::new(name),
                );
                ui.label(format!("Staged name: {}", after.to_string_lossy()));
            }
            if ui
                .add_enabled(self.can_edit_folders(), egui::Button::new("Rename file…"))
                .clicked()
            {
                self.open_organize_editor();
                if let Some(editor) = self.organize_editor.as_mut() {
                    editor.start_rename(id);
                }
            }
        }
        if let Some(warning) = self.image_warnings.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(egui::Color32::from_rgb(233, 191, 116), "Preview warning");
            ui.label(warning);
        }
        if let Some(error) = self.failures.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(RED, error);
            if ui.button("Retry preview").clicked() {
                self.failures.remove(&id);
            }
        }
        if media.kind == MediaKind::Video
            && crate::core::playback::supported()
            && ui.button("Watch here").clicked()
        {
            self.review = true;
            self.runtime.playback.select(Some(&media));
            self.video_context = Some((self.step, media.id));
            self.runtime.playback.toggle();
        }
        if media.kind == MediaKind::Video
            && ui
                .add_enabled(
                    self.can_open_desktop(id),
                    egui::Button::new("Open in external player"),
                )
                .clicked()
        {
            self.open_desktop(id, crate::core::desktop::Action::OpenVideo);
        }
        ui.add_space(15.0);
        ui.separator();
        ui.label(
            RichText::new("Changes stay staged until you review and apply them.")
                .small()
                .color(MUTED),
        );
    }
}
