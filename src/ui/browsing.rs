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
    /// What is narrowing Cull's view, in words; empty when nothing is.
    pub(super) fn active_cull_filters(&self) -> Vec<String> {
        let mut active = Vec::new();
        if !self.query.trim().is_empty() {
            active.push(format!("Search “{}”", self.query.trim()));
        }
        if self.filter != Filter::All {
            active.push(self.filter.title().to_owned());
        }
        if self.browse.rating != RatingFilter::All {
            active.push(self.browse.rating.title().to_string());
        }
        if let Some(group) = self.browse.group {
            active.push(format!("Group {group}"));
        }
        if let Some(date) = &self.browse.date {
            active.push(if date.is_empty() {
                "Unknown date".to_owned()
            } else {
                date.clone()
            });
        }
        let folders = self.browse.folder.folders().count();
        if folders > 1 {
            active.push(format!("{folders} folders"));
        } else if let Some(path) = &self.browse.folder.path {
            active.push(format!("Folder {}", self.folder_label(path)));
        }
        active
    }
    /// A folder's path inside the library, or "Library root".
    fn folder_label(&self, path: &std::path::Path) -> String {
        self.root
            .as_ref()
            .and_then(|root| path.strip_prefix(root).ok())
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Library root".into())
    }
    /// A line above the photos whenever Cull shows only some of them, so a
    /// filter restored with the folder's saved view is never a surprise.
    pub(super) fn cull_filter_notice(&mut self, ui: &mut egui::Ui) {
        let active = self.active_cull_filters();
        if active.is_empty() {
            return;
        }
        let total = self.media.len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(egui_phosphor::regular::FUNNEL).color(ACCENT));
            ui.label(
                RichText::new(format!(
                    "Showing {} of {total} {}",
                    self.filtered.len(),
                    if total == 1 { "item" } else { "items" }
                ))
                .color(INK),
            );
            if ui
                .small_button("Clear filters")
                .on_hover_text("Show every photo and video in this folder")
                .clicked()
            {
                self.clear_browse_filters();
            }
            let reasons = active.join(" · ");
            ui.add(egui::Label::new(RichText::new(&reasons).small().color(MUTED)).truncate())
                .on_hover_text(reasons);
        });
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
    /// The line above Cull's photos, in the grid and over Review's strip:
    /// how many photos are shown, then small toggles for how they are
    /// shown. All are app-wide preferences, also in Settings, except the
    /// similar-set sections, which are part of the folder's view.
    pub(super) fn photos_header(&mut self, ui: &mut egui::Ui, strip: bool) {
        use crate::core::preferences::StripPosition;
        use egui_phosphor::regular as icon;
        ui.horizontal(|ui| {
            ui.set_height(22.0);
            let count = self.filtered.len();
            let mut text = format!("{count} {}", if count == 1 { "photo" } else { "photos" });
            if self.grouped_view() {
                let sets = self
                    .group_best
                    .keys()
                    .filter(|group| !self.group_members(**group).is_empty())
                    .count();
                text.push_str(&format!(
                    " · {sets} similar {}",
                    if sets == 1 { "set" } else { "sets" }
                ));
            }
            ui.add(egui::Label::new(RichText::new(text).small().color(MUTED)).truncate());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(4.0);
                let left = self.preferences.strip_position == StripPosition::Left;
                if strip {
                    if left
                        && header_toggle(
                            ui,
                            icon::ROWS,
                            self.preferences.compact_strip,
                            if self.preferences.compact_strip {
                                "Compact rows · click for large thumbnails"
                            } else {
                                "Large thumbnails · click for compact rows"
                            },
                        )
                        .clicked()
                    {
                        self.set_compact_strip(!self.preferences.compact_strip);
                    }
                    let (glyph, hover) = if left {
                        (
                            icon::SQUARE_HALF,
                            "Strip on the left · click to move it to the bottom",
                        )
                    } else {
                        (
                            icon::SQUARE_HALF_BOTTOM,
                            "Strip at the bottom · click to move it to the left",
                        )
                    };
                    if header_toggle(ui, glyph, false, hover).clicked() {
                        self.preferences.strip_position = if left {
                            StripPosition::Bottom
                        } else {
                            StripPosition::Left
                        };
                        self.save_preferences();
                    }
                }
                let names = self.preferences.show_filenames;
                if header_toggle(
                    ui,
                    icon::TEXT_AA,
                    names,
                    if names {
                        "File names shown · click to hide them"
                    } else {
                        "File names hidden · click to show them"
                    },
                )
                .clicked()
                {
                    self.preferences.show_filenames = !names;
                    self.save_preferences();
                }
                let grouped = self.browse.group_sort != GroupSort::Flat;
                let ready = !self.group_best.is_empty();
                let toggle = ui.add_enabled_ui(ready, |ui| {
                    header_toggle(
                        ui,
                        icon::STACK,
                        grouped && ready,
                        if grouped {
                            "Grouped into similar sets · click to show the photos in one run"
                        } else {
                            "Group similar photos into sets"
                        },
                    )
                });
                let toggle = toggle
                    .inner
                    .on_disabled_hover_text("Analyze the library to find similar photos");
                if toggle.clicked() {
                    self.browse.group_sort = if grouped {
                        GroupSort::Flat
                    } else {
                        GroupSort::Time
                    };
                    self.pending_bookmark = None;
                    self.filter_dirty = true;
                    self.library_reveal = true;
                }
            });
        });
    }
    pub(super) fn browse_controls(&mut self, ui: &mut egui::Ui) {
        // One row at every size: filters live in a popover that shows how many
        // are active, so photos keep the vertical space.
        let overlay = true;
        let fit = self.bar_fit;
        self.search_field(ui);
        // An overflowing bar moves the photo filter into Filters & sort.
        if fit < super::top_bar::Fit::Overflow {
            self.filter_combo(ui);
        }
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
            let selected = active > 0 || self.browse_filters_open;
            let response = if fit >= super::top_bar::Fit::Icons {
                let glyph = if active == 0 {
                    egui_phosphor::regular::FUNNEL.to_owned()
                } else {
                    format!("{} {active}", egui_phosphor::regular::FUNNEL)
                };
                super::top_bar::bar_icon(ui, true, selected, &glyph, &label).on_hover_text(&label)
            } else {
                icon_text_button(ui, true, selected, egui_phosphor::regular::FUNNEL, &label)
            };
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
                    if fit == super::top_bar::Fit::Overflow {
                        ui.horizontal(|ui| {
                            ui.label("Show");
                            self.filter_combo(ui);
                        });
                    }
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
            self.selection_controls(ui);
        }
        if !overlay {
            self.browse_filter_controls(ui);
        }
        // Metadata progress and cancellation live in the status bar
        // (`footer_metadata`): a row here would push the grid down as soon
        // as a folder opens and back up when reading ends.
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
            let several = self.browse.folder.folders().count();
            let selected = match &self.browse.folder.path {
                _ if several > 1 => format!("{several} folders"),
                Some(path) => self.folder_label(path),
                None => "All folders".into(),
            };
            egui::ComboBox::from_id_salt("browse_folder")
                .width(200.0)
                .truncate()
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.browse.folder.path.is_none(), "All folders")
                        .clicked()
                    {
                        self.browse.folder.show_only(None);
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
                                            self.browse.folder.shows(&folder.path),
                                            &folder.label,
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(folder.path.display().to_string())
                                    .clicked()
                                {
                                    self.browse.folder.show_only(Some(folder.path.clone()));
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
                && several == 1
                && self.root.as_ref().is_some_and(|root| path != root)
                && ui.button("Parent folder").clicked()
            {
                let parent = path.parent().map(|p| p.to_path_buf());
                self.browse.folder.show_only(parent);
                self.filter_dirty = true;
            }
        });
    }
    /// The status bar's metadata controls, for Cull and metadata-based
    /// Organize. Called inside the footer's right-to-left layout: Cancel is
    /// added first so the changing count to its left never moves it.
    pub(super) fn footer_metadata(&mut self, ui: &mut egui::Ui, narrow: bool) {
        use egui_phosphor::regular as icon;
        if self.root.is_none()
            || !(self.step == Step::Cull
                || (self.step == Step::Organize && self.metadata_organize_mode()))
        {
            return;
        }
        if let Some((done, total)) = self.metadata_progress
            && self.metadata_run.is_some()
        {
            let progress = format!("Reading metadata · {done} / {total}");
            if narrow {
                if icon_button(ui, icon::X_CIRCLE, "Cancel metadata reading")
                    .on_hover_text(&progress)
                    .clicked()
                {
                    self.cancel_metadata_index();
                }
            } else {
                if ui.small_button("Cancel metadata reading").clicked() {
                    self.cancel_metadata_index();
                }
                ui.label(RichText::new(progress).small().color(MUTED));
            }
        } else if self.metadata.len() < self.media.len() {
            let clicked = if narrow {
                icon_button(ui, icon::ARROW_CLOCKWISE, "Read remaining metadata").clicked()
            } else {
                ui.small_button("Read remaining metadata").clicked()
            };
            if clicked {
                self.resume_suspended = false;
                self.start_metadata_index();
            }
        }
        if !self.metadata_failures.is_empty() {
            let text = format!(
                "{} files have unavailable metadata",
                self.metadata_failures.len()
            );
            ui.label(
                RichText::new(if narrow {
                    icon::WARNING.to_owned()
                } else {
                    text.clone()
                })
                .small()
                .color(RED),
            )
            .on_hover_text(format!("{text}. Restore access and retry metadata reading. Folder proposals wait for complete metadata; photos remain browsable."));
        }
    }
    pub(super) fn metadata_status(&mut self, ui: &mut egui::Ui) {
        let reading = self.metadata_progress.is_some() && self.metadata_run.is_some();
        if !reading && self.metadata.len() >= self.media.len() && self.metadata_failures.is_empty()
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

/// A small icon in a photos header that shows a setting and changes it.
/// Its tooltip is also its accessible name.
fn header_toggle(ui: &mut egui::Ui, glyph: &str, on: bool, hover: &str) -> egui::Response {
    let response = ui
        .add(
            egui::Button::new(
                RichText::new(glyph)
                    .size(14.0)
                    .color(if on { INK } else { MUTED }),
            )
            .frame(on)
            .selected(on)
            .min_size(egui::vec2(24.0, 22.0)),
        )
        .on_hover_text(hover);
    let enabled = response.enabled();
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, hover));
    response
}

#[cfg(test)]
#[path = "browsing/tests.rs"]
mod tests;

#[cfg(test)]
mod async_tests {
    use super::*;
    use crate::test_support::worker_patience;
    fn settle(app: &mut PhotoSortApp) {
        let deadline = Instant::now() + worker_patience();
        loop {
            let expired = Instant::now() >= deadline;
            app.refilter();
            if app.browse_run.is_none() {
                return;
            }
            assert!(!expired);
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
