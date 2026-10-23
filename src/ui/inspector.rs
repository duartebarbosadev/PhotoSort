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
            self.organize_destination(ui, &media);
        }
        if let Some(warning) = self.image_warnings.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(egui::Color32::from_rgb(233, 191, 116), "Preview warning");
            ui.label(warning);
        }
        if let Some(error) = self.analysis_failures.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(RED, "This photo could not be analyzed");
            ui.add(egui::Label::new(RichText::new(error).small().color(MUTED)).wrap());
            ui.label(
                RichText::new("It gets no suggestions and is never marked for Trash automatically. Analyze again to retry.")
                    .small()
                    .color(MUTED),
            );
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
    /// Details in windows too narrow for a side panel: a full-height drawer
    /// over the right edge, so the photos keep their width when it closes.
    pub(super) fn inspector_drawer(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let width = (screen.width() * 0.85).min(320.0);
        let top = screen.top() + navigation::HEADER_HEIGHT;
        let height = screen.bottom() - navigation::STATUS_HEIGHT - top;
        egui::Area::new(egui::Id::new("inspector-drawer"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(screen.right() - width, top))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, EDGE))
                    .shadow(egui::Shadow {
                        offset: [-6, 0],
                        blur: 18,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .inner_margin(S4 as i8)
                    .show(ui, |ui| {
                        ui.set_width(width - 2.0 * S4);
                        ui.set_height(height - 2.0 * S4);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Details").font(semibold(14.0)));
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if icon_button(ui, egui_phosphor::regular::X, "Close details · I")
                                    .clicked()
                                {
                                    self.inspector = false;
                                }
                            });
                        });
                        egui::ScrollArea::vertical()
                            .id_salt("inspector-drawer-scroll")
                            .show(ui, |ui| self.inspector(ui));
                    });
            });
    }

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
                for row in &section.rows {
                    detail_row(ui, row.label, |ui| {
                        ui.add(egui::Label::new(&row.values[0]).wrap());
                    });
                }
            }
            return;
        }
        // Stacked rather than tabular so any number of photos fits the panel:
        // shared values take one line, differing values get one line per photo.
        ui.add_space(S3);
        section_title(ui, "Compare details");
        for (n, (media, _)) in items.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("{}", n + 1))
                        .font(semibold(12.0))
                        .color(ACCENT),
                );
                ui.add(egui::Label::new(media.name()).truncate())
                    .on_hover_text(media.path.display().to_string());
            });
        }
        for section in &sections {
            ui.add_space(S3);
            section_title(ui, section.title);
            for row in &section.rows {
                let differs = row.values.iter().any(|v| *v != row.values[0]);
                detail_row(ui, row.label, |ui| {
                    if !differs {
                        ui.add(egui::Label::new(&row.values[0]).wrap());
                        return;
                    }
                    for (n, value) in row.values.iter().enumerate() {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = S2;
                            ui.label(RichText::new(format!("{}", n + 1)).small().color(FAINT));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(if value.is_empty() { "—" } else { value })
                                        .color(ACCENT),
                                )
                                .wrap(),
                            );
                        });
                    }
                });
            }
        }
    }
}

/// One label/value line with a fixed label column; the value wraps in the
/// remaining width instead of being clipped by a grid column.
fn detail_row(ui: &mut egui::Ui, label: &str, value: impl FnOnce(&mut egui::Ui)) {
    const LABEL: f32 = 96.0;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL, 0.0),
            egui::Layout::top_down(Align::Min),
            |ui| {
                ui.set_width(LABEL);
                ui.add(egui::Label::new(RichText::new(label).color(MUTED)).wrap());
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 0.0),
            egui::Layout::top_down(Align::Min),
            value,
        );
    });
}

/// Sentence-case section heading for the Details panel.
impl PhotoSortApp {
    /// Where this photo goes in the current folder proposal, in one line,
    /// and the two things you can do about it: move it elsewhere or rename it.
    fn organize_destination(&mut self, ui: &mut egui::Ui, media: &Media) {
        use egui_phosphor::regular as icon;
        let id = media.id;
        let Placement {
            label,
            moves,
            folder,
        } = self.placement(media);
        let name = self.file_names.get(&id).map(|stem| {
            crate::core::organize::renamed_file_name(&media.path, std::ffi::OsStr::new(stem))
                .to_string_lossy()
                .into_owned()
        });
        let heading = if moves { "Will move to" } else { "Stays in" };
        section_title(ui, heading);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(if moves {
                    icon::ARROW_RIGHT
                } else {
                    icon::FOLDER_SIMPLE
                })
                .color(if moves { ACCENT } else { MUTED }),
            );
            ui.add(egui::Label::new(RichText::new(&folder).color(INK)).wrap());
        });
        if label.is_none() {
            ui.label(
                RichText::new(format!("Not moving · {}", self.unplaced_reason(media)))
                    .small()
                    .color(MUTED),
            );
        }
        if let Some(name) = &name {
            ui.label(
                RichText::new(format!("Renamed to {name}"))
                    .small()
                    .color(MUTED),
            );
        }
        ui.add_space(S1);
        let editable = self.can_edit_folders() && self.organize_editor.is_none();
        ui.horizontal_wrapped(|ui| {
            if label.is_some() {
                ui.add_enabled_ui(editable, |ui| {
                    ui.menu_button(format!("{}  Move to…", icon::FOLDER_SIMPLE), |ui| {
                        self.board_folder_list(ui, &[id]);
                        ui.separator();
                        if ui.button("New folder…").clicked() {
                            self.board_new_folder_with(&[id]);
                            ui.close();
                        }
                    })
                    .response
                    .on_hover_text("Choose another folder for this photo")
                });
            }
            if ui
                .add_enabled(
                    editable,
                    egui::Button::new(format!("{}  Rename…", icon::PENCIL_SIMPLE)),
                )
                .on_hover_text("Rename this photo · F2")
                .clicked()
            {
                self.open_organize_editor();
                if let Some(editor) = self.organize_editor.as_mut() {
                    editor.start_rename(id);
                }
            }
        });
    }
    /// Where an item goes in the current folder proposal, edits included.
    pub(super) fn placement(&self, media: &Media) -> Placement {
        let label = match self.organize_drafts.get(&self.group_mode) {
            Some(draft) => draft.labels().get(&media.id).cloned(),
            None => self.organize_label(media),
        };
        let parent = media.path.parent().unwrap_or(std::path::Path::new(""));
        // Compared as folders on disk: with another destination, an item
        // whose folder name stays the same still moves.
        let target = label.as_deref().map(|label| {
            crate::core::operations::folder_destination(std::path::Path::new(&self.output), label)
        });
        match target.filter(|target| target != parent) {
            Some(target) => Placement {
                label,
                moves: true,
                folder: self.readable_folder(&target),
            },
            None => Placement {
                label,
                moves: false,
                folder: self.readable_folder(parent),
            },
        }
    }
    /// A folder as "Photos / 2025 / Trip": from the opened folder's name when
    /// it is inside it, otherwise the path with the home folder as "~".
    pub(super) fn readable_folder(&self, path: &std::path::Path) -> String {
        if let Some(root) = self.root.as_deref()
            && let Ok(relative) = path.strip_prefix(root)
        {
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            return std::iter::once(name)
                .chain(
                    relative
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned()),
                )
                .collect::<Vec<_>>()
                .join(" / ");
        }
        readable_path(path)
    }
}
/// Where an item goes: its proposed folder label (`None` keeps it where it
/// is), whether that moves it, and the folder it ends up in, readably.
pub(super) struct Placement {
    pub(super) label: Option<String>,
    pub(super) moves: bool,
    pub(super) folder: String,
}
/// A path with the home folder shown as "~".
pub(super) fn readable_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}
fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).font(medium(12.5)).color(INK));
    ui.add_space(2.0);
}
