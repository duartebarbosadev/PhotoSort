use super::*;

impl PhotoSortApp {
    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(S2);
        let Some(id) = self.active else {
            ui.label(RichText::new("Select a photo to see its details.").color(MUTED));
            return;
        };
        let Some(&index) = self.index.get(&id) else {
            return;
        };
        let media = self.media[index].clone();
        ui.add(egui::Label::new(RichText::new(media.name()).font(semibold(15.0))).truncate())
            .on_hover_text(media.path.display().to_string());
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
        self.details_tables(ui, id);
        if let Some(analysis) = self.analysis.get(&id) {
            ui.add_space(15.0);
            ui.separator();
            section_title(ui, "Photo quality");
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
            section_title(ui, "Destination folder");
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

impl PhotoSortApp {
    /// Metadata tables for the active photo, or one column per photo while a
    /// comparison is open. Values come from the shared metadata index.
    fn details_tables(&self, ui: &mut egui::Ui, active: MediaId) {
        let mut ids = self.comparison_ids();
        if ids.len() < 2 || !ids.contains(&active) {
            ids = vec![active];
        }
        let empty = Metadata::default();
        let items: Vec<(&Media, &Metadata)> = ids
            .iter()
            .filter_map(|id| {
                let media = self.media.get(*self.index.get(id)?)?;
                Some((media, self.metadata.get(id).unwrap_or(&empty)))
            })
            .collect();
        let sections = crate::core::details::sections(&items);
        if items.len() < 2 {
            for section in &sections {
                ui.add_space(S3);
                section_title(ui, section.title);
                egui::Grid::new(("details", section.title))
                    .num_columns(2)
                    .spacing([S3, 5.0])
                    .min_col_width(84.0)
                    .show(ui, |ui| {
                        for row in &section.rows {
                            ui.label(RichText::new(row.label).color(MUTED));
                            ui.add(egui::Label::new(&row.values[0]).wrap());
                            ui.end_row();
                        }
                    });
            }
            return;
        }
        ui.add_space(10.0);
        section_title(ui, "Compare details");
        egui::ScrollArea::horizontal()
            .id_salt("compare-details")
            .show(ui, |ui| {
                for section in &sections {
                    ui.add_space(8.0);
                    ui.label(RichText::new(section.title).small().color(MUTED));
                    egui::Grid::new(("compare-details", section.title))
                        .num_columns(items.len() + 1)
                        .spacing([14.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("");
                            for (media, _) in &items {
                                let name = media.name();
                                let short = if name.chars().count() > 15 {
                                    format!("{}…", name.chars().take(12).collect::<String>())
                                } else {
                                    name.clone()
                                };
                                ui.label(RichText::new(short).small().strong())
                                    .on_hover_text(name);
                            }
                            ui.end_row();
                            for row in &section.rows {
                                let differs = row.values.iter().any(|v| *v != row.values[0]);
                                ui.label(RichText::new(row.label).small().color(MUTED));
                                for value in &row.values {
                                    let shown = if value.is_empty() {
                                        "—"
                                    } else {
                                        value.as_str()
                                    };
                                    ui.label(if differs {
                                        RichText::new(shown).color(ACCENT)
                                    } else {
                                        RichText::new(shown)
                                    });
                                }
                                ui.end_row();
                            }
                        });
                }
            });
    }
}

/// Sentence-case section heading for the Details panel.
fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).font(medium(12.5)).color(INK));
    ui.add_space(2.0);
}
