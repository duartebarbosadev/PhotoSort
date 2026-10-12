use super::*;
use crate::core::browsing::{GroupSort, RatingFilter, Sort};

impl PhotoSortApp {
    pub(super) fn clear_browse_filters(&mut self) {
        self.browse.rating = RatingFilter::All;
        self.browse.group = None;
        self.browse.date = None;
        self.browse.folder = Default::default();
        self.filter = Filter::All;
        self.query.clear();
        self.filter_dirty = true;
    }
    fn browse_item(&self, id: MediaId) -> crate::core::browsing::Update {
        crate::core::browsing::Update {
            embedding: self.embeddings.get(&id).cloned(),
            metadata: self.metadata.get(&id).cloned(),
            group: self.cull_groups.get(&id).copied(),
            issues: self.analysis.get(&id).is_some_and(|a| !a.issues.is_empty()),
            duplicate: self.duplicates.contains_key(&id) || self.near_duplicates.contains_key(&id),
        }
    }
    pub(super) fn update_browse_item(&self, id: MediaId) {
        if self.browse_ready {
            self.runtime.browsing.update(id, self.browse_item(id));
        }
    }
    pub(super) fn initialize_browse(&mut self) {
        if !self.browse_ready {
            let Some(catalog) = self.catalog.clone() else {
                return;
            };
            // One initial snapshot of already-known state. Thereafter only changed
            // records cross the worker boundary; queries do not clone the library.
            let ids: HashSet<_> = self
                .metadata
                .keys()
                .chain(self.embeddings.keys())
                .chain(self.cull_groups.keys())
                .chain(self.analysis.keys())
                .chain(self.duplicates.keys())
                .chain(self.near_duplicates.keys())
                .copied()
                .collect();
            let updates = ids
                .into_iter()
                .map(|id| (id, self.browse_item(id)))
                .collect();
            self.runtime
                .browsing
                .reset(Some(crate::core::browsing::Seed {
                    catalog,
                    updates,
                    decisions: self.decisions.clone(),
                }));
            self.browse_ready = true;
            self.filter_dirty = true;
        }
    }
    pub(super) fn refilter_browse(&mut self) {
        if self.scanning {
            return;
        }
        self.initialize_browse();
        if !self.browse_ready {
            return;
        }
        if self.filter_dirty {
            self.browse_run = Some(self.runtime.browsing.request(crate::core::browsing::Query {
                text: self.query.clone(),
                filter: self.filter,
                options: self.browse.clone(),
                navigator: self.navigator.clone(),
            }));
            self.filter_dirty = false;
        }
        if let Some(result) = self.runtime.browsing.take_result()
            && self.browse_run == Some(result.run)
        {
            self.browse_run = None;
            self.navigation_index = Some(result.navigation);
            self.navigation_notice = None;
            self.filtered = result.indices;
            self.browse_groups = result.groups;
            self.group_choices = result.group_choices;
            self.browse_folders = result.folders;
            self.browse_tree = result.tree.map(Arc::new);
            if self.pending_bookmark.is_some() {
                return;
            }
            self.selection.reconcile(&self.media, &self.filtered);
            self.comparison.retain(|id| {
                self.index
                    .get(id)
                    .is_some_and(|i| self.filtered.contains(i))
            });
            if self
                .active
                .is_none_or(|id| !self.filtered.iter().any(|i| self.media[*i].id == id))
            {
                self.active = self.filtered.first().map(|i| self.media[*i].id);
                self.library_reveal = true;
            }
            if self.selection.is_empty()
                && let Some(id) = self.active
            {
                self.selection.single(id);
            }
        }
    }

    pub(super) fn record_analysis_group(&mut self, run: u64, id: MediaId, group: usize) {
        if self.cull_group_run == Some(run) {
            Arc::make_mut(&mut self.cull_groups).insert(id, group);
        } else if self.organize_run != Some((run, GroupMode::Mixed))
            && self.organize_run != Some((run, GroupMode::Faces))
        {
            self.groups.insert(id, group);
        }
    }
    pub(super) fn cancel_metadata_index(&mut self) {
        self.resume_suspended = self.pending_bookmark.is_some();
        self.runtime.cancel_metadata();
        self.metadata_run = None;
    }
    pub(super) fn start_metadata_index(&mut self) {
        if self.scanning || self.busy || self.metadata_run.is_some() {
            return;
        }
        let missing: Vec<_> = self
            .media
            .iter()
            .filter(|m| !self.metadata.contains_key(&m.id))
            .cloned()
            .collect();
        if missing.is_empty() {
            return;
        }
        let total = missing.len();
        match self.runtime.index_metadata(Arc::new(missing)) {
            Ok(run) => {
                self.metadata_run = Some(run);
                self.metadata_progress = Some((0, total));
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn browse_controls(&mut self, ui: &mut egui::Ui) {
        // One row at every size: filters live in a popover that shows how many
        // are active, so photos keep the vertical space.
        let overlay = true;
        ui.horizontal_wrapped(|ui| {
            // Buttons keep their text on one line, so a control that does not
            // fit moves to the next row instead of being squeezed.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            if overlay {
                let active = [
                    self.browse.rating != RatingFilter::All,
                    self.browse.group.is_some(),
                    self.browse.date.is_some(),
                    self.browse.folder.path.is_some(),
                ]
                .into_iter()
                .filter(|active| *active)
                .count();
                let label = if active == 0 {
                    "Filters & sort".to_owned()
                } else {
                    format!("Filters & sort · {active}")
                };
                let response = icon_text_button(
                    ui,
                    true,
                    active > 0 || self.browse_filters_open,
                    egui_phosphor::regular::FUNNEL,
                    &label,
                );
                let mut open = self.browse_filters_open;
                if response.clicked() {
                    open = !open;
                }
                let width = (ui.ctx().content_rect().width() - 64.).clamp(240., 460.);
                let height = (ui.ctx().content_rect().height() - 180.).clamp(160., 440.);
                // This parent owns its transient state so nested rating/folder
                // ComboBoxes can use egui's normal popup state independently.
                egui::Popup::from_response(&response)
                    .id(egui::Id::new("compact_browse_filters"))
                    .open_bool(&mut open)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .width(width)
                    .show(|ui| {
                        ui.set_width(width);
                        ui.heading("Filters & sort");
                        egui::ScrollArea::vertical()
                            .max_height(height)
                            .show(ui, |ui| self.browse_filter_controls(ui));
                    });
                self.browse_filters_open = open;
            } else {
                self.browse_filters_open = false;
            }
            self.navigator_controls(ui);
            if !self.review {
                ui.add_space(S3);
                self.selection_controls(ui);
            }
        });
        if !overlay {
            self.browse_filter_controls(ui);
        }
        // Progress and cancellation remain visible even while filters are closed.
        self.metadata_status(ui);
    }
    fn browse_filter_controls(&mut self, ui: &mut egui::Ui) {
        let columns = ((ui.available_width() / 220.0).floor() as usize).clamp(1, 4);
        egui::Grid::new(("browse_filters", columns))
            .num_columns(columns)
            .show(ui, |ui| {
                egui::ComboBox::from_id_salt("browse_rating")
                    .width(200.0)
                    .truncate()
                    .selected_text(self.browse.rating.title())
                    .show_ui(ui, |ui| {
                        for filter in RatingFilter::ALL {
                            if ui
                                .selectable_value(&mut self.browse.rating, filter, filter.title())
                                .clicked()
                            {
                                self.filter_dirty = true;
                                ui.close();
                            }
                        }
                    });
                if columns == 1 {
                    ui.end_row();
                }
                egui::ComboBox::from_id_salt("browse_group")
                    .width(200.0)
                    .truncate()
                    .selected_text(
                        self.browse.group.map_or_else(
                            || "All similarity groups".into(),
                            |g| format!("Group {g}"),
                        ),
                    )
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_value(&mut self.browse.group, None, "All similarity groups")
                            .clicked()
                        {
                            self.filter_dirty = true;
                            ui.close();
                        }
                        egui::ScrollArea::vertical().max_height(220.0).show_rows(
                            ui,
                            button_row_height(ui),
                            self.browse_groups.len(),
                            |ui, range| {
                                for i in range {
                                    let group = self.browse_groups[i];
                                    if ui
                                        .selectable_value(
                                            &mut self.browse.group,
                                            Some(group),
                                            format!("Group {group}"),
                                        )
                                        .clicked()
                                    {
                                        self.filter_dirty = true;
                                        ui.close();
                                    }
                                }
                            },
                        );
                    });
                if 2 % columns == 0 {
                    ui.end_row();
                }
                egui::ComboBox::from_id_salt("browse_sort")
                    .width(200.0)
                    .truncate()
                    .selected_text(self.browse.sort.title())
                    .show_ui(ui, |ui| {
                        for sort in Sort::ALL {
                            if ui
                                .selectable_value(&mut self.browse.sort, sort, sort.title())
                                .clicked()
                            {
                                self.filter_dirty = true;
                                ui.close();
                            }
                        }
                    });
                if 3 % columns == 0 {
                    ui.end_row();
                }
                if ui.button("Clear browse filters").clicked() {
                    self.clear_browse_filters();
                }
            });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("browse_group_sort")
                .width(240.0)
                .truncate()
                .selected_text(self.browse.group_sort.title())
                .show_ui(ui, |ui| {
                    for mode in GroupSort::ALL {
                        if ui
                            .selectable_value(&mut self.browse.group_sort, mode, mode.title())
                            .clicked()
                        {
                            self.filter_dirty = true;
                            ui.close();
                        }
                    }
                });
            if self.browse.group_sort != GroupSort::Flat && self.cull_groups.is_empty() {
                ui.label("Analyze the library to order similarity groups.");
            }
        });
        ui.horizontal_wrapped(|ui| {
            let selected = self.browse.folder.path.as_ref().map_or_else(
                || "All folders".into(),
                |path| {
                    self.root
                        .as_ref()
                        .and_then(|root| path.strip_prefix(root).ok())
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Library root".into())
                },
            );
            egui::ComboBox::from_id_salt("browse_folder")
                .width(200.0)
                .truncate()
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(&mut self.browse.folder.path, None, "All folders")
                        .clicked()
                    {
                        self.filter_dirty = true;
                        ui.close();
                    }
                    ui.spacing_mut().button_padding.y = 2.0;
                    let row_height = button_row_height(ui).max(22.0);
                    egui::ScrollArea::vertical().max_height(240.0).show_rows(
                        ui,
                        row_height,
                        self.browse_folders.len(),
                        |ui, range| {
                            for i in range {
                                let folder = &self.browse_folders[i];
                                if ui
                                    .add_sized(
                                        [200.0, row_height],
                                        egui::Button::selectable(
                                            self.browse.folder.path.as_ref() == Some(&folder.path),
                                            &folder.label,
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(folder.path.display().to_string())
                                    .clicked()
                                {
                                    self.browse.folder.path = Some(folder.path.clone());
                                    self.filter_dirty = true;
                                    ui.close();
                                }
                            }
                        },
                    );
                });
            if ui
                .add_enabled(
                    self.browse.folder.path.is_some(),
                    Switch::new(&mut self.browse.folder.recursive, "Include subfolders"),
                )
                .changed()
            {
                self.filter_dirty = true;
            }
            if let Some(path) = self.browse.folder.path.as_ref()
                && self.root.as_ref().is_some_and(|root| path != root)
                && ui.button("Parent folder").clicked()
            {
                self.browse.folder.path = path.parent().map(|p| p.to_path_buf());
                self.filter_dirty = true;
            }
        });
    }
    pub(super) fn metadata_status(&mut self, ui: &mut egui::Ui) {
        let reading = self.metadata_progress.is_some() && self.metadata_run.is_some();
        if !reading
            && self.metadata.len() >= self.media.len()
            && self.metadata_failures.is_empty()
        {
            return; // An empty row would still take a full line of height.
        }
        ui.horizontal_wrapped(|ui| {
            if let Some((done, total)) = self.metadata_progress && self.metadata_run.is_some() {
                ui.spinner();
                ui.label(format!("Reading photo metadata · {done} / {total}"));
                if ui.button("Cancel metadata reading").clicked() { self.cancel_metadata_index(); }
            } else if self.metadata.len() < self.media.len() && ui.button("Read remaining metadata").clicked() {
                self.resume_suspended = false;
                self.start_metadata_index();
            }
            if !self.metadata_failures.is_empty() {
                ui.colored_label(RED, format!("{} files have unavailable metadata", self.metadata_failures.len()))
                    .on_hover_text("Restore access and retry metadata reading. Folder proposals wait for complete metadata; photos remain browsable.");
            }
        });
    }
}

#[cfg(test)]
#[path = "browsing/tests.rs"]
mod tests;

#[cfg(test)]
mod async_tests {
    use super::*;
    fn settle(app: &mut PhotoSortApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.refilter();
            if app.browse_run.is_none() {
                return;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn live_decisions_and_metadata_update_cached_view_across_departure() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["a.jpg", "b.jpg"] {
            std::fs::write(temp.path().join(name), b"metadata fixture").unwrap();
        }
        let catalog = Arc::new(Catalog::scan(temp.path(), || false, |_| {}).unwrap());
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.media = catalog.media.clone();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.catalog = Some(catalog);
        app.step = Step::Cull;
        let id = app.media[1].id;
        app.active = Some(id);
        settle(&mut app);
        app.browse.rating = RatingFilter::Exactly(5);
        app.filter_dirty = true;
        app.rating(id, 5);
        settle(&mut app);
        assert_eq!(app.filtered, vec![1]);
        assert_eq!(app.active, Some(id));
        app.runtime
            .browsing
            .request(crate::core::browsing::Query::default());
        app.set_step(Step::Organize);
        settle(&mut app);
        assert_eq!(app.filtered.len(), 2);
        app.rating(id, 0);
        // Simulate the same accepted metadata path as an asset/metadata event.
        let other = app.media[0].id;
        app.metadata.insert(
            other,
            Metadata {
                rating: 5,
                ..Default::default()
            },
        );
        app.update_browse_item(other);
        app.set_step(Step::Cull);
        settle(&mut app);
        assert_eq!(app.filtered, vec![0]);
        assert_eq!(app.active, Some(other));
        assert_eq!(app.stats.decodes, 0);
        app.decisions.clear();
        app.changed();
        app.browse.rating = RatingFilter::Exactly(0);
        app.filter_dirty = true;
        settle(&mut app);
        assert!(
            app.filtered.is_empty(),
            "unknown metadata must stay unknown after discarding staged zero"
        );
    }
}
