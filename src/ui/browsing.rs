use super::*;
use crate::core::browsing::{GroupSort, RatingFilter, Sort};

impl PhotoSortApp {
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
        let compact = ui.available_width() < 850. || ui.ctx().content_rect().height() < 760.;
        ui.horizontal_wrapped(|ui| {
            if compact {
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
                    "Filters & sort".into()
                } else {
                    format!("Filters & sort · {active}")
                };
                let response = ui.button(label);
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
        });
        if !compact {
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
                    self.browse.rating = RatingFilter::All;
                    self.browse.group = None;
                    self.browse.date = None;
                    self.browse.folder = Default::default();
                    self.filter = Filter::All;
                    self.query.clear();
                    self.filter_dirty = true;
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
                    egui::Checkbox::new(&mut self.browse.folder.recursive, "Include subfolders"),
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
        ui.horizontal_wrapped(|ui| {
            if self.browse_run.is_some() { ui.spinner(); ui.label("Updating library view…"); }
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
mod tests {
    use super::*;
    fn settle(app: &mut PhotoSortApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.refilter();
            if app.browse_run.is_none() {
                return;
            }
            assert!(Instant::now() < deadline, "Browse worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
        let temp = tempfile::tempdir().unwrap();
        for n in 0..65 {
            std::fs::write(
                temp.path().join(format!("{n:03}.jpg")),
                b"metadata-only fixture",
            )
            .unwrap();
        }
        let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.media = catalog.media.clone();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.root = Some(catalog.root.clone());
        app.catalog = Some(Arc::new(catalog));
        (temp, app)
    }
    #[test]
    fn folder_filter_preserves_focus_and_stays_in_cull_without_more_metadata_work() {
        let (_temp, mut app) = fixture();
        app.step = Step::Cull;
        app.active = Some(app.media[12].id);
        settle(&mut app);
        let folders = app.browse_folders.clone();
        app.browse.folder.path = app.root.clone();
        app.browse.folder.recursive = false;
        app.filter_dirty = true;
        settle(&mut app);
        assert_eq!(app.filtered.len(), 65);
        assert_eq!(app.active, Some(app.media[12].id));
        assert!(Arc::ptr_eq(&folders, &app.browse_folders));
        app.browse.folder.path = Some(app.root.as_ref().unwrap().join("missing"));
        app.filter_dirty = true;
        settle(&mut app);
        assert!(app.filtered.is_empty());
        assert_eq!(app.active, None);
        app.set_step(Step::Organize);
        settle(&mut app);
        assert_eq!(app.filtered.len(), 65);
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 0);
    }
    #[test]
    fn cull_indexes_once_without_decoding_and_keeps_richer_metadata_and_staged_ratings() {
        let (_temp, mut app) = fixture();
        let id = app.media[0].id;
        app.set_step(Step::Cull);
        let run = app.metadata_run;
        assert!(run.is_some());
        app.start_metadata_index();
        assert_eq!(app.metadata_run, run);
        app.metadata.insert(
            id,
            Metadata {
                width: 6000,
                height: 4000,
                rating: 4,
                ..Default::default()
            },
        );
        app.decisions.insert(
            id,
            Decision {
                rating: Some(2),
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.metadata_run.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.metadata_run.is_none());
        assert_eq!(app.metadata.len(), 65);
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 65);
        assert_eq!(app.metadata[&id].width, 6000);
        app.browse.rating = RatingFilter::Exactly(2);
        settle(&mut app);
        assert_eq!(app.filtered, vec![0]);
        assert_eq!(app.selected_rating(id), 2);
        app.start_metadata_index();
        assert!(app.metadata_run.is_none());
        assert_eq!(app.stats.metadata_reads, 65);
        app.set_step(Step::Organize);
        settle(&mut app);
        assert_eq!(
            app.filtered.len(),
            65,
            "Cull filters must not leak into guided workflows"
        );
    }
    #[test]
    fn cancelled_metadata_batches_do_not_change_a_different_workflow() {
        let (_temp, mut app) = fixture();
        app.set_step(Step::Cull);
        assert!(app.metadata_run.is_some());
        app.set_step(Step::Organize);
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(app.metadata.is_empty());
        assert!(app.metadata_run.is_none());
        assert_eq!(app.stats.decodes, 0);
        app.set_step(Step::Cull);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.metadata_run.is_some() && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(app.metadata.len(), 65);
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn browse_controls_wrap_at_compact_width_and_filters_keep_visible_focus() {
        let (_temp, mut app) = fixture();
        app.step = Step::Cull;
        for m in &app.media {
            app.metadata.insert(
                m.id,
                Metadata {
                    rating: 3,
                    ..Default::default()
                },
            );
        }
        let active = app.media[20].id;
        app.active = Some(active);
        app.browse.rating = RatingFilter::Exactly(3);
        app.browse.sort = Sort::Newest;
        settle(&mut app);
        assert_eq!(app.active, Some(active));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for width in [360.0, 576.0, 960.0] {
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            app.browse_controls(ui);
                            assert!(
                                ui.min_rect().right() <= width,
                                "Browse controls overflow at {width}: {:?}",
                                ui.min_rect()
                            );
                        });
                    },
                );
            }
        }
        app.browse.rating = RatingFilter::Exactly(5);
        app.filter_dirty = true;
        settle(&mut app);
        assert!(app.filtered.is_empty());
        assert_eq!(app.active, None);
    }
    #[test]
    fn cull_group_filter_and_comparison_do_not_use_other_workflows_cluster_ids() {
        let (_temp, mut app) = fixture();
        app.step = Step::Cull;
        let a = app.media[0].id;
        let b = app.media[1].id;
        let c = app.media[2].id;
        app.cull_group_run = Some(1);
        app.record_analysis_group(1, a, 7);
        app.record_analysis_group(1, b, 7);
        app.groups.clear();
        app.record_analysis_group(2, a, 7);
        app.record_analysis_group(2, c, 7);
        app.active = Some(a);
        app.browse.group = Some(7);
        settle(&mut app);
        assert_eq!(app.filtered, vec![0, 1]);
        assert_eq!(app.browse_groups, vec![7]);
        app.compare_similar();
        assert!(app.comparison.contains(&b));
        assert!(!app.comparison.contains(&c));
        app.set_step(Step::Organize);
        settle(&mut app);
        assert_eq!(app.filtered.len(), 65);
        assert_eq!(app.cull_groups.len(), 2);
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn group_sort_reuses_application_embeddings_and_preserves_selection_and_bookmarks() {
        let (_temp, mut app) = fixture();
        app.step = Step::Cull;
        for (i, m) in app.media.iter().enumerate() {
            app.metadata.insert(
                m.id,
                Metadata {
                    captured_ms: Some(i as i64),
                    ..Default::default()
                },
            );
            if i < 3 {
                Arc::make_mut(&mut app.cull_groups).insert(m.id, i + 1);
                app.embeddings
                    .insert(m.id, Arc::new(vec![3.0 - i as f32, 0.0]));
            }
        }
        let focus = app.media[1].id;
        app.active = Some(focus);
        app.selection.single(focus);
        let original = app.embeddings[&focus].clone();
        assert!(Arc::ptr_eq(
            &original,
            app.browse_item(focus).embedding.as_ref().unwrap()
        ));
        app.browse.group_sort = GroupSort::Similarity;
        settle(&mut app);
        assert_eq!(&app.filtered[..3], &[2, 1, 0]);
        assert_eq!(app.active, Some(focus));
        assert!(app.selection.contains(focus));
        app.checkpoint_bookmarks();
        let saved = &app.saved_views.bookmarks[&Step::Cull];
        assert_eq!(saved.browse.group_sort, GroupSort::Similarity);
        app.browse.group_sort = GroupSort::Time;
        app.filter_dirty = true;
        settle(&mut app);
        assert_eq!(&app.filtered[..3], &[0, 1, 2]);
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(
            app.requested.is_empty() && app.analysis_run.is_none() && app.metadata_run.is_none()
        );
    }
}

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
