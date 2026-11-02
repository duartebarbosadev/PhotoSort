//! Presentation only: tree construction/projection belongs to the browse worker.
use super::*;
use crate::core::browsing::tree::{Key, Mode};
/// The photos area must be this wide to keep the navigator beside it.
const INLINE_WIDTH: f32 = 780.;
/// Context data: the frame in which the navigator was drawn as a sidebar.
const SIDEBAR_FRAME: &str = "navigator_sidebar_frame";
/// A navigator row: as tall as one in Organize's folders.
const ROW: f32 = 26.0;
/// The sidebar's width and the range it can be dragged to, as Organize's.
const SIDEBAR_WIDTH: f32 = 240.;
const SIDEBAR_WIDTHS: std::ops::RangeInclusive<f32> = 180.0..=380.0;
/// The sidebar's grey and margins, as Organize's folders.
fn sidebar_frame() -> egui::Frame {
    egui::Frame::new().fill(SIDEBAR).inner_margin(egui::Margin {
        left: S2 as i8,
        right: S2 as i8,
        top: S3 as i8,
        bottom: S2 as i8,
    })
}

/// Where Cull's photos are, for the navigator, like Organize's folder
/// sidebar: the folder or day you are looking at is highlighted, and one
/// clicked is scrolled to rather than shown alone.
#[derive(Default)]
pub(super) struct Spy {
    /// The folder or date you are looking at: the active photo's while it
    /// is on screen, otherwise the topmost photo's.
    pub(super) current: Option<Key>,
    /// One just clicked: highlighted until the photos scroll or the active
    /// photo changes from where they were (recorded on the first frame
    /// after the click).
    hold: Option<Option<(f32, Option<MediaId>)>>,
    /// The photos' scroll offset and active photo when last followed.
    seen: Option<(f32, Option<MediaId>)>,
    /// The photos were last scrolled by hand rather than by a new active photo.
    scrolled: bool,
    /// The navigator scrolls `current` into view once after it changes.
    pub(super) reveal: bool,
    /// A folder or date the photos scroll to once they are up to date.
    pub(super) scroll_to: Option<Key>,
}

impl PhotoSortApp {
    pub(super) fn set_navigator(&mut self, enabled: bool) {
        self.navigator.enabled = enabled;
        self.navigator_changed();
    }
    /// Lists folders or capture dates. Nothing chosen in one carries over
    /// to the other: both only scroll the photos.
    pub(super) fn set_navigator_mode(&mut self, mode: Mode) {
        if self.navigator.mode != mode {
            self.navigator.mode = mode;
            self.spy = Default::default();
            self.spy.reveal = true;
            self.navigator_changed();
        }
    }
    fn navigator_changed(&mut self) {
        self.pending_bookmark = None;
        self.filter_dirty = true;
        self.library_reveal = true;
    }
    /// What the navigator highlights: the folder or date you are looking
    /// at, or else the library folder.
    fn navigator_key(&self) -> Option<Key> {
        let current = self.spy.current.clone().filter(|key| {
            matches!(
                (key, self.navigator.mode),
                (Key::Folder(_), Mode::Folders) | (Key::Date(_), Mode::Dates)
            )
        });
        match self.navigator.mode {
            Mode::Folders => current.or_else(|| self.root.clone().map(Key::Folder)),
            _ => current,
        }
    }
    /// The folder or calendar day a shown photo belongs to, in the
    /// navigator's current list.
    fn navigator_place(&self, index: usize) -> Option<Key> {
        let media = &self.media[index];
        match self.navigator.mode {
            Mode::Dates => {
                let metadata = self.metadata.get(&media.id);
                let captured = crate::core::dates::capture_millis(media, metadata);
                let date = crate::core::dates::calendar(captured, metadata);
                Some(Key::Date(date.unwrap_or_default()))
            }
            _ => media.path.parent().map(|p| Key::Folder(p.to_path_buf())),
        }
    }
    /// Whether a shown photo is in folder or date `key`, subfolders and the
    /// days of a month or year included.
    fn navigator_holds(&self, key: &Key, index: usize) -> bool {
        match key {
            Key::Folder(path) => self.media[index].path.starts_with(path),
            Key::Date(scope) => match self.navigator_place(index) {
                Some(Key::Date(date)) => crate::core::browsing::tree::date_matches(
                    Some(scope),
                    (!date.is_empty()).then_some(date.as_str()),
                ),
                _ => false,
            },
            Key::Group(_) => false,
        }
    }
    /// Scrolls the photos to a folder or date, the first of its photos in
    /// the order shown, and highlights it. A folder hidden by "Show only"
    /// brings every folder back first; one with no photos leaves the
    /// photos where they are.
    pub(super) fn go_to(&mut self, key: Key) {
        if let Key::Folder(path) = key {
            if self.browse.folder.path.is_some() && !self.browse.folder.shows(&path) {
                self.browse.folder.show_only(None);
                self.navigator_changed();
            }
            return self.go_to_place(Key::Folder(path));
        }
        self.go_to_place(key);
    }
    fn go_to_place(&mut self, key: Key) {
        self.pending_bookmark = None;
        self.spy.current = Some(key.clone());
        self.spy.hold = Some(None);
        self.spy.scroll_to = Some(key);
    }
    /// The position in the photos shown to scroll to for a folder or date
    /// clicked in the navigator, once the photos are up to date.
    pub(super) fn take_folder_scroll(&mut self) -> Option<usize> {
        if self.filter_dirty || self.browse_run.is_some() {
            return None;
        }
        let key = self.spy.scroll_to.take()?;
        self.filtered
            .iter()
            .position(|i| self.navigator_holds(&key, *i))
    }
    /// Follows the folder or date you are looking at, as Organize's sidebar
    /// does: the active photo's while it is on screen (`active`), or the
    /// topmost photo's (`top`) once you scroll by hand. One just clicked
    /// stays highlighted until you scroll or move to another photo.
    pub(super) fn follow_photos(&mut self, active: bool, top: Option<usize>, offset: f32) {
        let now = (offset, self.active);
        let by_hand = match self.spy.seen.replace(now) {
            Some((_, was)) if was != self.active => false,
            Some((then, _)) if (then - offset).abs() > 0.5 => true,
            _ => self.spy.scrolled,
        };
        self.spy.scrolled = by_hand;
        if self.spy.scroll_to.is_some() {
            return;
        }
        match self.spy.hold {
            Some(None) => {
                self.spy.hold = Some(Some(now));
                return;
            }
            Some(Some(then)) if then == now => return,
            Some(Some(_)) => self.spy.hold = None,
            None => {}
        }
        let active = active
            .then(|| self.active.and_then(|id| self.index.get(&id)).copied())
            .flatten()
            .and_then(|index| self.navigator_place(index));
        let top = top
            .and_then(|position| self.filtered.get(position).copied())
            .and_then(|index| self.navigator_place(index));
        let current = if by_hand { top } else { active.or(top) };
        if current.is_some() && current != self.spy.current {
            self.spy.current = current;
            self.spy.reveal = true;
        }
    }
    /// Whether the navigator highlights `path`: every folder shown when
    /// "Show only" picked several, otherwise the one you are looking at.
    fn navigator_highlighted(&self, path: &std::path::Path) -> bool {
        if self.browse.folder.folders().count() > 1 {
            return self.browse.folder.shows(path);
        }
        self.navigator_key() == Some(Key::Folder(path.to_path_buf()))
    }
    /// A Ctrl/Cmd or Shift click on a folder in the navigator, by the
    /// Organize sidebar's rules: Ctrl/Cmd shows only the folders chosen,
    /// adding this one or taking it away again; Shift shows the folders
    /// listed from the last one clicked through this one, added to those
    /// shown with Ctrl/Cmd.
    fn choose_navigator_folder(
        &mut self,
        view: &crate::core::browsing::tree::View,
        path: std::path::PathBuf,
        modifiers: egui::Modifiers,
    ) {
        let toggle = modifiers.command || modifiers.ctrl;
        let row = |target: &std::path::Path| {
            view.visible.iter().position(
                |i| matches!(&view.tree.nodes[*i].key, Key::Folder(folder) if folder == target),
            )
        };
        let span = self
            .browse
            .folder
            .path
            .as_deref()
            .filter(|_| modifiers.shift)
            .and_then(row)
            .zip(row(&path));
        let folder = &mut self.browse.folder;
        if let Some((from, to)) = span {
            let rows = view.visible[from.min(to)..=from.max(to)].iter();
            let span = rows.filter_map(|i| match &view.tree.nodes[*i].key {
                Key::Folder(folder) => Some(folder.clone()),
                _ => None,
            });
            folder.show_span(span.collect(), toggle);
        } else {
            folder.toggle(path);
        }
        self.navigator_changed();
    }
    pub(super) fn navigator_controls(&mut self, ui: &mut egui::Ui) {
        // Every Cull view, Review included, shows the same navigator.
        let mut visible = self.navigator.enabled;
        let button = if self.bar_fit >= super::top_bar::Fit::Icons {
            super::top_bar::bar_icon(
                ui,
                true,
                visible,
                egui_phosphor::regular::TREE_VIEW,
                "Library navigator",
            )
        } else {
            icon_text_button(
                ui,
                true,
                visible,
                egui_phosphor::regular::TREE_VIEW,
                "Library navigator",
            )
        };
        if button
            .on_hover_text("Folders and capture dates, to jump between them · F")
            .clicked()
        {
            visible = !visible;
            self.set_navigator(visible);
        }
    }
    /// The width Review's left photo strip takes from the photos, so the
    /// navigator keeps to its sidebar only while the photo itself has room.
    fn review_strip_width(&self, ctx: &egui::Context) -> f32 {
        if !self.review
            || self.preferences.strip_position != crate::core::preferences::StripPosition::Left
        {
            return 0.0;
        }
        let (id, default) = super::review::side_strip_panel(self.preferences.compact_strip);
        egui::PanelState::load(ctx, egui::Id::new(id)).map_or(default, |p| p.size().x)
    }
    fn navigator_popup_id(&self) -> egui::Id {
        egui::Id::new(("navigator_popup_open", &self.root))
    }
    pub(super) fn navigator_popup_open(&self, ctx: &egui::Context) -> bool {
        ctx.data(|data| data.get_temp::<(bool, u64)>(self.navigator_popup_id()))
            .is_some_and(|(open, frame)| {
                open && frame.saturating_add(1) >= ctx.cumulative_frame_nr()
            })
    }
    /// Cull's library navigator as a sidebar flush with the window's left
    /// edge, from the top of the window to the page bar, like Organize's
    /// folders.
    /// Drawn before the middle of the window when the photos area (`room`
    /// wide) has space for it; [`Self::navigator_panel`] shows the popover
    /// otherwise.
    pub(super) fn navigator_sidebar(&mut self, root: &mut egui::Ui, room: f32) {
        if self.step != Step::Cull
            || !self.navigator.enabled
            || room - self.review_strip_width(root.ctx()) < INLINE_WIDTH
        {
            return;
        }
        let frame = root.ctx().cumulative_frame_nr();
        let popup_state = self.navigator_popup_id();
        root.ctx().data_mut(|data| {
            data.remove::<(bool, u64)>(popup_state);
            data.insert_temp(egui::Id::new(SIDEBAR_FRAME), frame);
        });
        egui::Panel::left("library_tree_panel")
            .resizable(true)
            .show_separator_line(false)
            .default_size(SIDEBAR_WIDTH)
            .size_range(SIDEBAR_WIDTHS)
            .frame(sidebar_frame())
            .show(root, |ui| {
                self.sidebar_title_band(ui, S3);
                if self.busy {
                    ui.disable();
                }
                self.navigator_contents(ui, ui.available_height());
            });
    }
    pub(super) fn navigator_panel(&mut self, ui: &mut egui::Ui) {
        let popup_state = self.navigator_popup_id();
        if self.step != Step::Cull || !self.navigator.enabled {
            ui.ctx()
                .data_mut(|data| data.remove::<(bool, u64)>(popup_state));
            return;
        }
        let frame = ui.ctx().cumulative_frame_nr();
        if ui
            .ctx()
            .data(|d| d.get_temp::<u64>(egui::Id::new(SIDEBAR_FRAME)))
            == Some(frame)
        {
            return;
        }
        if ui.available_width() - self.review_strip_width(ui.ctx()) >= INLINE_WIDTH {
            ui.ctx()
                .data_mut(|data| data.remove::<(bool, u64)>(popup_state));
            egui::Panel::left("library_tree_panel")
                .resizable(true)
                .show_separator_line(false)
                .default_size(SIDEBAR_WIDTH)
                .size_range(SIDEBAR_WIDTHS)
                .frame(sidebar_frame())
                .show(ui, |ui| self.navigator_contents(ui, ui.available_height()));
        } else {
            // A stacked tree can consume the entire photo viewport in a short
            // window. Reuse the same virtualized navigator in a bounded overlay.
            // Names what "Show only" keeps, if anything; otherwise stays put
            // while the photos scroll.
            let several = self.browse.folder.folders().count();
            let label = match &self.browse.folder.path {
                _ if several > 1 => format!("Folders: {several} folders"),
                Some(path) => format!(
                    "Folders: {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ),
                None if self.navigator.mode == Mode::Dates => "Browse dates".to_owned(),
                None => "Browse folders".to_owned(),
            };
            let width = ui.available_width().clamp(240., 420.);
            let height = (ui.ctx().content_rect().height() - 180.).clamp(160., 420.);
            let response = ui
                .add(egui::Button::new(label).truncate())
                .on_hover_text("The library's folders. Counts cover the whole library.");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("test_navigator_toggle"), response.rect));
            // Context menus use egui's single memory-owned popup slot. Keep
            // this enclosing browser independently open so they cannot evict it.
            let mut open = self.navigator_popup_open(ui.ctx());
            if response.clicked() {
                open = !open;
            }
            egui::Popup::from_response(&response)
                .open_bool(&mut open)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(width)
                .frame(egui::Frame::popup(ui.style()).fill(SIDEBAR))
                .show(|ui| {
                    ui.set_width(width);
                    self.navigator_contents(ui, height);
                });
            let frame = ui.ctx().cumulative_frame_nr();
            ui.ctx()
                .data_mut(|data| data.insert_temp(popup_state, (open, frame)));
            ui.add_space(4.);
        }
    }
    fn navigator_contents(&mut self, ui: &mut egui::Ui, height: f32) {
        // Only one presentation is rendered: inline panel or compact popup.
        ui.scope_builder(
            egui::UiBuilder::new().id(egui::Id::new(("library_navigator", &self.root))),
            |ui| self.navigator_body(ui, height),
        );
    }
    fn navigator_body(&mut self, ui: &mut egui::Ui, height: f32) {
        use egui_phosphor::regular as icon;
        let selected = self.navigator_key();
        let mode = self.navigator.mode;
        let view = self.browse_tree.clone().filter(|v| v.mode == mode);
        // A header like Organize's folders: what it lists, then steps to the
        // neighbouring entry at the same level.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = S3;
            for choice in [Mode::Folders, Mode::Dates] {
                let current = mode == choice;
                let text = caption(choice.title()).color(if current { INK } else { FAINT });
                let response =
                    ui.add(egui::Button::new(text).frame(false))
                        .on_hover_text(match choice {
                            Mode::Dates => "Capture dates. Counts cover the whole library.",
                            _ => "The library's folders. Counts cover the whole library.",
                        });
                let response = named(response, egui::WidgetType::Button, choice.title());
                if response.clicked() {
                    self.set_navigator_mode(choice);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let node = selected.as_ref().and_then(|key| {
                    let view = view.as_ref()?;
                    view.tree.lookup.get(key).map(|i| &view.tree.nodes[*i])
                });
                let steps = [
                    ("Next", icon::CARET_DOWN, node.and_then(|n| n.next)),
                    ("Previous", icon::CARET_UP, node.and_then(|n| n.previous)),
                ];
                for (name, glyph, index) in steps {
                    let response = ui.add_enabled(
                        index.is_some(),
                        egui::Button::new(RichText::new(glyph).size(13.0))
                            .frame(false)
                            .min_size(egui::vec2(22.0, 20.0)),
                    );
                    if named(response, egui::WidgetType::Button, name)
                        .on_hover_text(format!("{name} {} at the same level", mode.entry()))
                        .clicked()
                        && let Some((index, view)) = index.zip(view.as_ref())
                    {
                        self.go_to(view.tree.nodes[index].key.clone());
                    }
                }
            });
        });
        ui.add_space(S1);
        let Some(view) = view else {
            ui.spinner();
            return;
        };
        if view.tree.nodes.is_empty() {
            ui.label(if mode == Mode::Dates {
                "No capture dates yet."
            } else {
                "No folders in this library."
            });
            return;
        }
        // Navigation only changes the query against this cached tree. Keep it
        // responsive while the worker replaces an older projection/filter result.
        ui.scope(|ui| {
            let row_height = ROW;
            let max_height = (height - 70.).max(24.);
            let salt = ("library_tree", mode);
            let keyboard = tree_controls::Keyboard::new(ui, salt, max_height, row_height);
            if let Some(result) = keyboard.navigate(
                ui,
                &view.tree,
                selected.as_ref(),
                &mut self.navigator.collapsed,
            ) {
                if selected.as_ref() != Some(&result.key) {
                    self.go_to(result.key);
                } else if result.expansion_changed {
                    self.navigator_changed();
                }
            }
            if std::mem::take(&mut self.spy.reveal)
                && let Some(key) = selected.clone()
            {
                keyboard.reveal(ui, key);
            }
            let scroll = keyboard.scroll(
                ui,
                egui::ScrollArea::vertical().id_salt(salt),
                &view.tree,
                &view.visible,
                !self.filter_dirty && self.browse_run.is_none(),
            );
            let output = scroll
                .max_height(max_height)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, view.visible.len(), |ui, range| {
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new("test_navigator_first_row"), range.start)
                    });
                    for row in range {
                        let i = view.visible[row];
                        let node = &view.tree.nodes[i];
                        ui.scope(|ui| {
                            let folder = match &node.key {
                                Key::Folder(path) => Some(path),
                                _ => None,
                            };
                            let top = folder.is_some_and(|path| self.root.as_ref() == Some(path));
                            let highlighted = match folder {
                                Some(path) => self.navigator_highlighted(path),
                                None => selected.as_ref() == Some(&node.key),
                            };
                            // The library folder by its name, as Organize's
                            // top row; dates as words.
                            let label = match &node.key {
                                Key::Folder(path) if top => path
                                    .file_name()
                                    .map_or(node.label.clone(), |n| n.to_string_lossy().into_owned()),
                                Key::Date(date) => date_label(date),
                                _ => node.label.clone(),
                            };
                            // Organize's folder row, one step in for the
                            // caret that opens and closes the folder.
                            let depth = node.depth.min(12);
                            let response = super::organize_board::folder_row(
                                ui,
                                ROW,
                                &label,
                                if top {
                                    icon::HARD_DRIVES
                                } else if folder.is_none() {
                                    icon::CALENDAR_BLANK
                                } else if node.count == 0 {
                                    icon::FOLDER_DASHED
                                } else {
                                    icon::FOLDER_SIMPLE
                                },
                                depth + 1,
                                node.count,
                                highlighted,
                                false,
                                None,
                                None,
                            );
                            let caret = egui::Rect::from_center_size(
                                egui::pos2(
                                    response.rect.left() + 13.0 + depth as f32 * 14.0,
                                    response.rect.center().y,
                                ),
                                egui::vec2(16.0, ROW),
                            );
                            if tree_controls::caret(
                                ui,
                                caret,
                                &view.tree,
                                i,
                                &mut self.navigator.collapsed,
                                &keyboard,
                            ) {
                                self.navigator_changed();
                            }
                            keyboard.focus_on_click(&response);
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(
                                    egui::Id::new(("test_navigator_node", &node.key)),
                                    response.rect,
                                )
                            });
                            response.context_menu(|ui| {
                                if tree_controls::menu(
                                    ui,
                                    &view.tree,
                                    i,
                                    &mut self.navigator.collapsed,
                                ) {
                                    self.navigator_changed();
                                }
                                let Some(path) = folder else {
                                    return;
                                };
                                ui.separator();
                                if ui.button("Show only this folder").clicked() {
                                    self.browse.folder.show_only(Some(path.clone()));
                                    self.navigator_changed();
                                    ui.close();
                                }
                                if self.browse.folder.path.is_some()
                                    && ui.button("Show every folder").clicked()
                                {
                                    self.browse.folder.show_only(None);
                                    self.navigator_changed();
                                    ui.close();
                                }
                            });
                            let hover = match folder {
                                Some(path) => format!(
                                    "{} · click to scroll to it · Ctrl/Cmd-click to show only the folders you choose",
                                    path.display()
                                ),
                                None => format!("{label} · click to scroll to its photos"),
                            };
                            if response.on_hover_text(hover).clicked() {
                                let modifiers = ui.input(|i| i.modifiers);
                                match folder {
                                    Some(path)
                                        if modifiers.shift || modifiers.command || modifiers.ctrl =>
                                    {
                                        self.choose_navigator_folder(&view, path.clone(), modifiers);
                                    }
                                    _ => self.go_to(node.key.clone()),
                                }
                            }
                        });
                    }
                });
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("test_navigator_scroll"),
                    (
                        output.id,
                        output.inner_rect,
                        output.state.offset.y,
                        row_height,
                    ),
                )
            });
            keyboard.finish(ui, output.inner_rect);
        });
    }
}

/// A date in the navigator as words: the year, a month by name, a day with
/// its weekday ("2025", "October", "Fri 24").
fn date_label(date: &str) -> String {
    use chrono::{Datelike, NaiveDate};
    if date.is_empty() {
        return "Unknown date".into();
    }
    if let Ok(day) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        return format!("{} {}", day.format("%a"), day.day());
    }
    if let Ok(month) = NaiveDate::parse_from_str(&format!("{date}-01"), "%Y-%m-%d")
        && date.len() == 7
    {
        return month.format("%B").to_string();
    }
    date.into()
}

#[cfg(test)]
#[path = "navigator/tests.rs"]
mod tests;
