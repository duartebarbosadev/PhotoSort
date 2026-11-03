//! Arranging the board: sorting the photos into their destination
//! folders, the folder tree, and the counts and notes about them.
use super::*;

impl PhotoSortApp {
    /// The note under the folder tree about photos outside every folder.
    pub(in crate::ui::workspace) fn board_note(&self) -> Option<String> {
        let unplaced = self.board.unplaced;
        if self.board.original {
            return Some("The folders as they are on disk now · release to see the plan".into());
        }
        let planned = (self.board.changes > 0).then(|| {
            format!(
                "{} planned {} · nothing changes on disk until you review and apply {}",
                self.board.changes,
                if self.board.changes == 1 {
                    "change"
                } else {
                    "changes"
                },
                if self.board.changes == 1 {
                    "it"
                } else {
                    "them"
                }
            )
        });
        Some(match self.board_pending() {
            None if unplaced > 0 => match planned {
                Some(planned) => format!("{planned}\n\n{}", self.unplaced_sentence()),
                None => self.unplaced_sentence(),
            },
            None if planned.is_some() => return planned,
            Some(Pending::Running) => "Folders appear when grouping finishes".into(),
            Some(Pending::Waiting) => {
                "Not grouped yet · nothing moves until you review the changes".into()
            }
            Some(Pending::Cancelled) => "Grouping cancelled · no folders proposed".into(),
            Some(Pending::Failed) => "Grouping stopped · no folders proposed".into(),
            None => return None,
        })
    }
    /// "58 photos without a recognized person stay in their current folders".
    pub(in crate::ui::workspace) fn unplaced_sentence(&self) -> String {
        let (all, videos) = (self.board.unplaced, self.board.unplaced_videos);
        let what = self.unplaced_phrase(all - videos, videos);
        format!(
            "{what} {} in {} current {}",
            if all == 1 { "stays" } else { "stay" },
            if all == 1 { "its" } else { "their" },
            if all == 1 { "folder" } else { "folders" }
        )
    }
    /// The counts beside the board's title.
    pub(in crate::ui::workspace) fn board_counts(
        &self,
        folders: usize,
        items: usize,
        all: bool,
    ) -> String {
        let items = format!("{items} {}", if items == 1 { "item" } else { "items" });
        match self.board_pending() {
            Some(Pending::Running) => format!("{items} · grouping…"),
            Some(_) => format!("{items} · not grouped yet"),
            None => {
                let mut summary = format!(
                    "{folders} {} · {items}",
                    if folders == 1 { "folder" } else { "folders" }
                );
                if all && self.board.unplaced > 0 {
                    summary.push_str(&format!(" · {} not moving", self.board.unplaced));
                }
                summary
            }
        }
    }
    pub(in crate::ui::workspace) fn board_key(&self) -> BoardKey {
        BoardKey {
            mode: self.group_mode,
            epoch: self.board_epoch,
            media: self.media.len(),
            // Only metadata-based modes regroup as metadata arrives; elsewhere
            // arriving thumbnails and metadata must not reshuffle the tiles.
            metadata: if matches!(
                self.group_mode,
                GroupMode::Date | GroupMode::Camera | GroupMode::Location
            ) {
                self.metadata.len()
            } else {
                0
            },
            draft: self
                .organize_drafts
                .get(&self.group_mode)
                .map(|d| (d.labels().len(), d.folders().len())),
            grouped: [
                self.visual_groups.is_some(),
                self.mixed_labels.is_some(),
                self.face_groups.is_some(),
            ],
            layout: self.preferences.location_layout,
            faceless: self.preferences.faceless,
            original: self.board.original,
        }
    }
    /// Rearranges Organize when what it shows has changed.
    pub(in crate::ui::workspace) fn check_board(&mut self) {
        if self.step == Step::Organize && self.board.key.as_ref() != Some(&self.board_key()) {
            self.filter_dirty = true;
        }
    }
    /// Why the grouping has no result yet; never while the original
    /// folders are shown, since they need no grouping.
    pub(in crate::ui::workspace) fn board_pending(&self) -> Option<Pending> {
        self.organize_pending().filter(|_| !self.board.original)
    }
    /// Shows the folders as they are on disk now (`true`) or the plan.
    pub(in crate::ui::workspace) fn show_original(&mut self, on: bool) {
        if self.board.original == on {
            return;
        }
        if on {
            self.board.focus_before = Some(std::mem::take(&mut self.board.focus));
            self.board.editing = None;
        } else {
            self.board.focus = self.board.focus_before.take().unwrap_or_default();
        }
        self.board.original = on;
        self.board.current = None;
        self.filter_dirty = true;
    }
    /// Every item's folder on disk now, relative to the library folder.
    fn original_labels(&self) -> HashMap<MediaId, String> {
        self.media
            .iter()
            .filter_map(|m| Some((m.id, self.current_label(m)?)))
            .collect()
    }
    /// The output folder's own name, for the top of the tree (the library
    /// folder's while the original folders are shown).
    pub(super) fn board_root_name(&self) -> String {
        let root = match (&self.catalog, self.board.original) {
            (Some(catalog), true) => catalog.root.clone(),
            _ => PathBuf::from(&self.output),
        };
        root.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Destination".into())
    }
    /// Sorts `filtered` by destination folder, then capture time, and
    /// records the folder sections and tree.
    pub(in crate::ui::workspace) fn arrange_board(&mut self) {
        self.board.arranged += 1;
        let key = self.board_key();
        let original = self.board.original;
        let draft = self
            .organize_drafts
            .get(&self.group_mode)
            .filter(|_| !original);
        let generated;
        let labels = match draft {
            Some(draft) => draft.labels(),
            None => {
                generated = if original {
                    self.original_labels()
                } else {
                    self.organize_labels()
                };
                &generated
            }
        };
        let mut keyed: Vec<_> = self
            .filtered
            .iter()
            .map(|&i| {
                let media = &self.media[i];
                let path = labels.get(&media.id).map(String::as_str);
                let order = crate::core::dates::capture_order(media, self.metadata.get(&media.id));
                (path.is_none(), path, order, i)
            })
            .collect();
        keyed.sort();
        self.filtered = keyed.iter().map(|k| k.3).collect();
        if !original {
            let output = std::path::Path::new(&self.output);
            // Many photos share a folder: each destination is worked out once.
            let mut destinations: HashMap<&str, PathBuf> = HashMap::new();
            for label in labels.values() {
                destinations
                    .entry(label)
                    .or_insert_with(|| crate::core::operations::folder_destination(output, label));
            }
            let moves = |id: &MediaId, label: &str| {
                self.index.get(id).is_some_and(|&i| {
                    self.media[i].path.parent() != destinations.get(label).map(PathBuf::as_path)
                })
            };
            // Items that move, and items renamed where they are.
            let moved = labels.iter().filter(|(id, label)| moves(id, label)).count();
            let renamed = self
                .file_names
                .keys()
                .filter(|id| !labels.get(id).is_some_and(|label| moves(id, label)))
                .count();
            self.board.changes = moved + renamed + draft.map_or(0, |d| d.created_folders().len());
        }
        let mut sections: Vec<Section> = Vec::new();
        for (position, (_, path, _, index)) in keyed.iter().enumerate() {
            let video = usize::from(self.media[*index].kind == MediaKind::Video);
            match sections.last_mut() {
                Some(section) if section.path.as_deref() == *path => {
                    section.range.end = position + 1;
                    section.videos += video;
                }
                _ => sections.push(Section {
                    path: path.map(str::to_owned),
                    range: position..position + 1,
                    videos: video,
                }),
            }
        }
        // The tree: the draft's folders (which keep empty ones), or every
        // proposed folder and its parents.
        let mut paths: std::collections::BTreeSet<String> = match (draft, &self.catalog) {
            (Some(draft), _) => draft.folders().clone(),
            // Every folder on disk, empty ones included.
            (None, Some(catalog)) if original => Draft::from_catalog(catalog, HashMap::new())
                .folders()
                .clone(),
            (None, _) => {
                let distinct: std::collections::BTreeSet<&str> =
                    labels.values().map(String::as_str).collect();
                distinct.into_iter().map(str::to_owned).collect()
            }
        };
        for path in paths.clone() {
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                paths.insert(prefix.to_owned());
                parent = prefix;
            }
        }
        paths.insert(String::new());
        let direct: HashMap<&str, usize> = sections
            .iter()
            .filter_map(|s| Some((s.path.as_deref()?, s.range.len())))
            .collect();
        // Each folder's items count toward it and every folder above it.
        let mut totals: HashMap<&str, usize> = HashMap::new();
        for (&path, &count) in &direct {
            let mut folder = path;
            loop {
                *totals.entry(folder).or_default() += count;
                if folder.is_empty() {
                    break;
                }
                folder = folder.rsplit_once('/').map_or("", |(parent, _)| parent);
            }
        }
        let folders: Vec<FolderRow> = paths
            .iter()
            .map(|path| FolderRow {
                path: path.clone(),
                depth: if path.is_empty() {
                    0
                } else {
                    path.matches('/').count() + 1
                },
                direct: direct.get(path.as_str()).copied().unwrap_or(0),
                total: totals.get(path.as_str()).copied().unwrap_or(0),
            })
            .collect();
        // A generated grouping lists the folders it fills, not the library's
        // existing folders it empties; folders made on the board stay.
        let folders = if self.group_mode == GroupMode::Current || original {
            folders
        } else {
            let created = draft.map(|d| d.created_folders());
            folders
                .into_iter()
                .filter(|f| {
                    f.path.is_empty()
                        || f.total > 0
                        || created.is_some_and(|c| c.iter().any(|p| within(p, &f.path)))
                })
                .collect()
        };
        self.board.unplaced = sections
            .iter()
            .filter(|s| s.path.is_none())
            .map(|s| s.range.len())
            .sum();
        self.board.unplaced_videos = sections
            .iter()
            .filter(|s| s.path.is_none())
            .map(|s| s.videos)
            .sum();
        self.board.sections = sections;
        self.board.folders = folders;
        self.board.key = Some(key);
        self.arrange_people();
        let folders = &self.board.folders;
        self.board
            .focus
            .retain(|focus| folders.iter().any(|f| &f.path == focus));
        if let Some(anchor) = &self.board.folder_anchor
            && !folders.iter().any(|f| &f.path == anchor)
        {
            self.board.folder_anchor = None;
        }
    }
    /// The board section holding the active photo.
    pub(super) fn active_section(&self) -> Option<&Section> {
        let index = self.index.get(&self.active?)?;
        let position = self.filtered.iter().position(|i| i == index)?;
        self.board
            .sections
            .iter()
            .find(|s| s.range.contains(&position))
    }
    /// Follows the folder you are looking at: the active photo's while its
    /// tile is on screen (`active`), or the folder at the top of the view
    /// (`top`) once you scroll by hand. Right after a folder is clicked in
    /// the sidebar, that folder stays highlighted while the board scrolls to
    /// it, and then until you scroll or move to another photo.
    pub(super) fn board_look_at(
        &mut self,
        active: Option<Option<String>>,
        top: Option<Option<String>>,
        offset: f32,
    ) {
        let now = (offset, self.active);
        let by_hand = match self.board.spy.replace(now) {
            Some((_, was)) if was != self.active => false,
            Some((then, _)) if (then - offset).abs() > 0.5 => true,
            _ => self.board.scrolled,
        };
        self.board.scrolled = by_hand;
        if self.board.glide.is_some() {
            return;
        }
        match self.board.hold {
            Some(None) => {
                self.board.hold = Some(Some(now));
                return;
            }
            Some(Some(then)) if then == now => return,
            Some(Some(_)) => self.board.hold = None,
            None => {}
        }
        let current = if by_hand { top } else { active.or(top) };
        if current.is_some() && current != self.board.current {
            self.board.current = current;
            self.board.reveal_current = true;
        }
    }
    /// The sidebar row to highlight: the folder you are looking at, or the
    /// folder shown (the library row when every item is). Several folders
    /// shown are all highlighted instead, see [`Self::board_highlighted`].
    pub(in crate::ui::workspace) fn board_highlight(&self) -> Option<String> {
        match (&self.board.current, self.board.focus.as_slice()) {
            (Some(Some(path)), _) => Some(path.clone()),
            (_, [focus]) => Some(focus.clone()),
            _ => Some(String::new()),
        }
    }
    /// Whether the sidebar highlights folder `path`: every folder shown
    /// when there are several, otherwise [`Self::board_highlight`]'s.
    pub(in crate::ui::workspace) fn board_highlighted(
        &self,
        path: &str,
        highlight: Option<&str>,
    ) -> bool {
        if self.board.focus.len() > 1 {
            self.board.focus.iter().any(|f| f == path)
        } else {
            highlight == Some(path)
        }
    }
}
