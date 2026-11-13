//! Arranging the board: sorting the photos into their destination
//! folders and the folder tree.
use super::*;

impl PhotoSortApp {
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
    /// Rearranges Organize when what it shows has changed. More photos
    /// found or read is background work: while it runs, the board catches
    /// up a few times a second, and once more when it ends.
    pub(in crate::ui::workspace) fn check_board(&mut self) {
        if self.step != Step::Organize {
            return;
        }
        let key = self.board_key();
        match &self.board.key {
            Some(shown) if *shown == key => {}
            Some(shown) if shown.grown_to(&key) => self.board.refresh.owed = true,
            _ => self.filter_dirty = true,
        }
        if !self.filter_dirty && self.board_refresh_wait() == Some(Duration::ZERO) {
            self.board.refresh.last = Some(Instant::now());
            self.filter_dirty = true;
        }
    }
    /// How long until the board next catches up with background work, when
    /// it has some to catch up with. At once when no such work is running;
    /// otherwise spaced out, and further apart the longer an arrangement
    /// takes, so a large library stays responsive while it is read.
    pub(in crate::ui::workspace) fn board_refresh_wait(&self) -> Option<Duration> {
        let refresh = &self.board.refresh;
        if self.step != Step::Organize || !refresh.owed {
            return None;
        }
        let working = self.scanning || self.metadata_run.is_some() || self.analysis_run.is_some();
        Some(match refresh.last {
            Some(last) if working => REFRESH_EVERY
                .max(refresh.cost * 4)
                .saturating_sub(Instant::now().saturating_duration_since(last)),
            _ => Duration::ZERO,
        })
    }
    /// Background work changed which photos the board shows or where they
    /// go: it catches up when [`Self::check_board`] says so.
    pub(in crate::ui::workspace) fn board_data_changed(&mut self) {
        self.board.refresh.owed = true;
    }
    fn board_library(&self) -> Library {
        Library {
            opened: self.runtime.epoch(),
            catalog: self.catalog.as_ref().map(Arc::downgrade),
        }
    }
    fn proposal_key(&self) -> ProposalKey {
        ProposalKey {
            mode: self.group_mode,
            library: self.board_library(),
            media: self.media.len(),
            metadata: if self.metadata_organize_mode() {
                (self.metadata.len(), self.metadata_revision)
            } else {
                (0, 0)
            },
            layout: self.preferences.location_layout,
            year: crate::core::dates::current_year(),
        }
    }
    /// [`Self::organize_labels`], kept until the library or the metadata
    /// the folders are named from changes. A grouping from an analysis is
    /// worked out each time: its result is replaced in place.
    pub(in crate::ui::workspace) fn proposed_labels(&mut self) -> Arc<HashMap<MediaId, String>> {
        if self.automatic_organize_mode() {
            self.board.proposal = None;
            return Arc::new(self.organize_labels());
        }
        let key = self.proposal_key();
        if let Some((kept, labels)) = &self.board.proposal
            && *kept == key
        {
            return labels.clone();
        }
        let labels = Arc::new(self.organize_labels());
        self.board.proposal = Some((key, labels.clone()));
        labels
    }
    /// Every photo's destination folder for the arrangement `key`: the ones
    /// kept from the last arrangement, or worked out again when the plan,
    /// the library or the metadata the folders are named from changed.
    fn board_inputs(&mut self, key: &BoardKey) -> Inputs {
        let drafted = !key.original && self.organize_drafts.contains_key(&self.group_mode);
        let kept =
            (drafted || key.original || !self.automatic_organize_mode()).then(|| InputsKey {
                board: key.clone(),
                proposal: self.proposal_key(),
            });
        if let Some(inputs) = self
            .board
            .inputs
            .take_if(|inputs| kept.is_some() && inputs.key == kept)
        {
            return inputs;
        }
        let generated = match (drafted, key.original) {
            (true, _) => None,
            (false, true) => Some(Arc::new(self.original_labels())),
            (false, false) => Some(self.proposed_labels()),
        };
        let labels = match &generated {
            Some(labels) => &**labels,
            None => self.organize_drafts[&self.group_mode].labels(),
        };
        // Many photos share a folder: each is named and ordered once.
        let mut rank: HashMap<&str, u32> = HashMap::new();
        for label in labels.values() {
            rank.entry(label).or_default();
        }
        let mut folders: Vec<&str> = rank.keys().copied().collect();
        folders.sort_unstable();
        for (place, folder) in folders.iter().enumerate() {
            rank.insert(folder, place as u32);
        }
        let folder = self
            .media
            .iter()
            .map(|m| {
                labels
                    .get(&m.id)
                    .map_or(UNPLACED, |label| rank[label.as_str()])
            })
            .collect();
        Inputs {
            key: kept,
            folders: folders.into_iter().map(str::to_owned).collect(),
            folder,
            labels: generated,
            changes: None,
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
        self.board.refresh.owed = false;
        let key = self.board_key();
        let original = self.board.original;
        let mut inputs = self.board_inputs(&key);
        let draft = self
            .organize_drafts
            .get(&self.group_mode)
            .filter(|_| !original);
        // A photo without a capture time is ordered by the day in its name.
        let mut named = std::mem::take(&mut self.board.name_days);
        let (library, year) = (self.board_library(), crate::core::dates::current_year());
        if named.library != library || named.year != year {
            named = NameDays {
                library,
                year,
                days: Vec::new(),
            };
        }
        named.days.resize(self.media.len(), None);
        // Folder (photos staying in place last), then capture time with
        // undated photos last, then name: `dates::capture_order`'s order.
        let mut keyed: Vec<(u32, bool, i64, usize)> = self
            .filtered
            .iter()
            .map(|&i| {
                let media = &self.media[i];
                let day = &mut named.days[i];
                let millis = crate::core::dates::capture_millis_or(
                    media,
                    self.metadata.get(&media.id),
                    || {
                        *day.get_or_insert_with(|| {
                            crate::core::dates::name_millis(&media.name(), year)
                        })
                    },
                );
                (
                    inputs.folder[i],
                    millis.is_none(),
                    millis.unwrap_or_default(),
                    i,
                )
            })
            .collect();
        let name = |i: usize| {
            self.media[i]
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        };
        keyed.sort_unstable_by(|a, b| {
            (a.0, a.1, a.2)
                .cmp(&(b.0, b.1, b.2))
                .then_with(|| name(a.3).cmp(&name(b.3)))
                .then(a.3.cmp(&b.3))
        });
        self.filtered = keyed.iter().map(|k| k.3).collect();
        self.board.name_days = named;
        if !original {
            // Counted again only when the plan, the destination or a new
            // name changed, not for a search or a filter.
            let names = self.file_names_stamp();
            let counted = inputs
                .changes
                .as_ref()
                .filter(|(output, stamp, together, _)| {
                    *output == self.output && *stamp == names && *together == self.companions
                })
                .map(|(.., count)| *count);
            let labels = inputs.labels.as_deref().or(draft.map(|d| d.labels()));
            if let Some(count) = counted {
                self.board.changes = count;
            } else if let Some(labels) = labels {
                let none = std::collections::BTreeSet::new();
                let count =
                    self.organize_changes_in(labels, draft.map_or(&none, |d| d.created_folders()));
                inputs.changes = Some((self.output.clone(), names, self.companions, count));
                self.board.changes = count;
            }
        }
        let mut sections: Vec<Section> = Vec::new();
        let mut last = None;
        for (position, &(folder, _, _, index)) in keyed.iter().enumerate() {
            let video = usize::from(self.media[index].kind == MediaKind::Video);
            match sections.last_mut() {
                Some(section) if last == Some(folder) => {
                    section.range.end = position + 1;
                    section.videos += video;
                }
                _ => sections.push(Section {
                    path: (folder != UNPLACED).then(|| inputs.folders[folder as usize].clone()),
                    range: position..position + 1,
                    videos: video,
                }),
            }
            last = Some(folder);
        }
        // The tree: the draft's folders (which keep empty ones), or every
        // proposed folder and its parents.
        let mut paths: std::collections::BTreeSet<String> = match (draft, &self.catalog) {
            (Some(draft), _) => draft.folders().clone(),
            // Every folder on disk, empty ones included.
            (None, Some(catalog)) if original => Draft::from_catalog(catalog, HashMap::new())
                .folders()
                .clone(),
            (None, _) => inputs.folders.iter().cloned().collect(),
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
        self.board.inputs = Some(inputs);
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
