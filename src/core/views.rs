//! Durable presentation state. Contains no file decisions or model results.
use super::{browsing, delete_review::Category, inspection, model::*};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    List,
    Icons,
    #[default]
    Grid,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroupMode {
    #[default]
    Current,
    Date,
    Camera,
    Location,
    Visual,
    Faces,
    Mixed,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmark {
    #[serde(skip)]
    pub missing_focus: bool,
    pub editor: Option<EditorBookmark>,
    pub active: Option<MediaId>,
    pub selected: Vec<MediaId>,
    pub anchor: Option<MediaId>,
    pub comparison: Vec<MediaId>,
    pub review: bool,
    pub inspector: bool,
    pub layout: Layout,
    pub query: String,
    pub filter: browsing::Filter,
    pub browse: browsing::Options,
    pub navigator: browsing::tree::State,
    pub group_anchor: Option<MediaId>,
    pub group_mode: GroupMode,
    pub delete_current: Option<MediaId>,
    pub delete_categories: Vec<Category>,
    pub rotation_queue_only: bool,
    pub pick_focus: bool,
    pub view: inspection::View,
    pub independent_views: HashMap<u64, inspection::View>,
    pub sync_views: bool,
    pub scroll: f32,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EditorBookmark {
    pub selected: Vec<MediaId>,
    pub folders: Vec<String>,
    pub folder: String,
    pub photo_scroll: f32,
    pub folder_scroll: f32,
    pub compact_folders: bool,
}
impl Default for Bookmark {
    fn default() -> Self {
        Self {
            missing_focus: false,
            editor: None,
            active: None,
            selected: vec![],
            anchor: None,
            comparison: vec![],
            review: false,
            inspector: true,
            layout: Layout::Grid,
            query: String::new(),
            filter: Default::default(),
            browse: Default::default(),
            navigator: Default::default(),
            group_anchor: None,
            group_mode: GroupMode::Current,
            delete_current: None,
            delete_categories: Category::ALL.to_vec(),
            rotation_queue_only: false,
            pick_focus: false,
            view: Default::default(),
            independent_views: HashMap::new(),
            sync_views: true,
            scroll: 0.0,
        }
    }
}
impl Bookmark {
    pub fn ids(&self) -> impl Iterator<Item = MediaId> + '_ {
        self.active
            .into_iter()
            .chain(self.selected.iter().copied())
            .chain(self.comparison.iter().copied())
            .chain(self.delete_current)
            .chain(self.group_anchor)
            .chain(self.anchor)
            .chain(self.editor.iter().flat_map(|e| e.selected.iter().copied()))
    }
    fn reconcile(
        &mut self,
        valid: &impl Fn(MediaId) -> bool,
        valid_file: &impl Fn(MediaId) -> bool,
    ) {
        self.missing_focus = self.active.is_some_and(|id| !valid(id));
        self.active = self.active.filter(|id| valid(*id));
        if let Some(editor) = self.editor.as_mut() {
            editor.selected.retain(|id| valid_file(*id));
            for offset in [&mut editor.photo_scroll, &mut editor.folder_scroll] {
                if !offset.is_finite() || *offset < 0.0 {
                    *offset = 0.0;
                }
            }
        }
        self.selected.retain(|id| valid(*id));
        self.anchor = self.anchor.filter(|id| valid(*id));
        if self.comparison.len() > 9 || self.comparison.iter().any(|id| !valid(*id)) {
            self.comparison.clear();
        }
        self.delete_current = self.delete_current.filter(|id| valid(*id));
        self.group_anchor = self.group_anchor.filter(|id| valid(*id));
        if self.group_anchor.is_none() {
            self.browse.group = None;
        }
        if !self.scroll.is_finite() {
            self.scroll = 0.0;
        }
        self.scroll = self.scroll.max(0.0);
        fn clean(v: &mut inspection::View) {
            if !v.zoom.is_finite() || v.pan.iter().any(|n| !n.is_finite()) {
                *v = Default::default();
            }
            v.zoom = v
                .zoom
                .clamp(inspection::View::MIN_ZOOM, inspection::View::MAX_ZOOM);
        }
        clean(&mut self.view);
        self.independent_views.retain(|slot, v| {
            clean(v);
            *slot < 9
        });
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub step: Option<Step>,
    pub bookmarks: HashMap<Step, Bookmark>,
    pub fingerprints: HashMap<MediaId, Fingerprint>,
}
impl Saved {
    pub fn reconcile(&mut self, media: &[Media]) {
        self.reconcile_identities(
            &media.iter().map(|m| (m.id, m.fingerprint)).collect(),
            &HashMap::new(),
        );
    }
    pub fn reconcile_catalog(&mut self, catalog: &super::catalog::Catalog) {
        self.reconcile_identities(
            &catalog
                .media
                .iter()
                .map(|m| (m.id, m.fingerprint))
                .collect(),
            &catalog.supporting_fingerprints,
        );
    }
    fn reconcile_identities(
        &mut self,
        media: &HashMap<MediaId, Fingerprint>,
        supporting: &HashMap<MediaId, Fingerprint>,
    ) {
        let valid = |id| {
            self.fingerprints
                .get(&id)
                .is_some_and(|f| media.get(&id) == Some(f))
        };
        let valid_file = |id| {
            valid(id)
                || self
                    .fingerprints
                    .get(&id)
                    .is_some_and(|f| supporting.get(&id) == Some(f))
        };
        for view in self.bookmarks.values_mut() {
            view.reconcile(&valid, &valid_file);
        }
        self.fingerprints
            .retain(|id, f| media.get(id).or_else(|| supporting.get(id)) == Some(f));
    }
    /// Checkpoints reuse the scan's supporting-file identities. No UI-thread IO.
    pub fn capture_catalog_fingerprints(
        &mut self,
        catalog: &super::catalog::Catalog,
        index: &HashMap<MediaId, usize>,
    ) {
        self.capture_fingerprints(&catalog.media, index);
        for id in self
            .bookmarks
            .values()
            .flat_map(|view| view.editor.iter())
            .flat_map(|editor| editor.selected.iter())
        {
            if let Some(fingerprint) = catalog.supporting_fingerprints.get(id) {
                self.fingerprints.insert(*id, *fingerprint);
            }
        }
    }
    pub fn capture_fingerprints(&mut self, media: &[Media], index: &HashMap<MediaId, usize>) {
        self.fingerprints = self
            .bookmarks
            .values()
            .flat_map(Bookmark::ids)
            .filter_map(|id| {
                index
                    .get(&id)
                    .and_then(|i| media.get(*i))
                    .map(|m| (id, m.fingerprint))
            })
            .collect();
    }
    pub fn relocate(&mut self, moves: &[(MediaId, MediaId, Fingerprint)]) {
        let mapping: HashMap<_, _> = moves.iter().map(|(a, b, f)| (*a, (*b, *f))).collect();
        for view in self.bookmarks.values_mut() {
            for id in view
                .active
                .iter_mut()
                .chain(view.selected.iter_mut())
                .chain(view.comparison.iter_mut())
                .chain(view.delete_current.iter_mut())
                .chain(view.group_anchor.iter_mut())
                .chain(view.anchor.iter_mut())
                .chain(view.editor.iter_mut().flat_map(|e| e.selected.iter_mut()))
            {
                if let Some((new, _)) = mapping.get(id) {
                    *id = *new;
                }
            }
            // Folder paths may have moved; media focus remains the stable navigation anchor.
            if moves.iter().any(|(old, new, _)| old != new) {
                view.browse.folder = Default::default();
            }
        }
        for (old, (new, fingerprint)) in mapping {
            if self.fingerprints.remove(&old).is_some() {
                self.fingerprints.insert(new, fingerprint);
            }
        }
    }
    /// Merge only bookmarks that refer to verified arrivals. Unrelated destination
    /// workflows retain their own state. No catalog scan or image work is needed.
    pub fn import_relocated(
        &mut self,
        source: &Self,
        moves: &[(MediaId, MediaId, Fingerprint)],
    ) -> bool {
        let arrivals: HashMap<_, _> = moves
            .iter()
            .filter(|(old, new, _)| old != new && source.fingerprints.contains_key(old))
            .map(|(old, new, fingerprint)| (*old, (*new, *fingerprint)))
            .collect();
        let mut incoming = Self {
            step: source.step,
            bookmarks: source
                .bookmarks
                .iter()
                .filter(|(_, view)| view.ids().any(|id| arrivals.contains_key(&id)))
                .map(|(step, view)| (*step, view.clone()))
                .collect(),
            fingerprints: arrivals
                .keys()
                .filter_map(|id| source.fingerprints.get(id).map(|f| (*id, *f)))
                .collect(),
        };
        if incoming.bookmarks.is_empty() {
            return false;
        }
        incoming.relocate(moves);
        for view in incoming.bookmarks.values_mut() {
            view.reconcile(&|id| incoming.fingerprints.contains_key(&id), &|id| {
                incoming.fingerprints.contains_key(&id)
            });
            view.active = view.active.or_else(|| view.selected.first().copied());
            view.missing_focus = false;
            // Folder paths, generated group numbers and scroll offsets belong to
            // the source inventory. Keep layout/inspection preferences and use
            // media identities to restore focus in the destination inventory.
            view.browse.folder = Default::default();
            view.browse.group = None;
            view.navigator.collapsed.clear();
            view.scroll = 0.0;
            if let Some(editor) = &mut view.editor {
                editor.folders.clear();
                editor.folder.clear();
                editor.photo_scroll = 0.0;
                editor.folder_scroll = 0.0;
            }
        }
        if incoming
            .step
            .is_some_and(|step| incoming.bookmarks.contains_key(&step))
        {
            self.step = incoming.step;
        }
        self.bookmarks.extend(incoming.bookmarks);
        self.fingerprints.extend(incoming.fingerprints);
        // Drop identities owned only by overwritten bookmarks, without touching
        // fingerprints used by the destination's unrelated workflow bookmarks.
        let referenced: std::collections::HashSet<_> =
            self.bookmarks.values().flat_map(Bookmark::ids).collect();
        self.fingerprints.retain(|id, _| referenced.contains(id));
        true
    }
    pub fn path(root: &Path) -> PathBuf {
        let root: PathBuf = root.components().collect();
        super::session::data_dir().join("views").join(format!(
            "{}.json",
            blake3::hash(root.as_os_str().as_encoded_bytes()).to_hex()
        ))
    }
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = Self::path(root);
        if !path.exists() {
            return Ok(Self::default());
        }
        let (version, saved): (u32, Self) = serde_json::from_slice(&std::fs::read(path)?)?;
        if version != 1 {
            return Ok(Self::default());
        }
        Ok(saved)
    }
    pub fn save(&self, root: &Path) -> anyhow::Result<()> {
        super::metadata::atomic_write(&Self::path(root), &serde_json::to_vec(&(1, self))?)
    }
}
