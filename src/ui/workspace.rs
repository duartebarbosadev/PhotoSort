#[path = "browsing.rs"]
mod browsing;
#[path = "collection_controls.rs"]
mod collection_controls;
#[path = "delete_review.rs"]
mod delete_review;
#[path = "desktop.rs"]
mod desktop;
#[path = "dialogs.rs"]
mod dialogs;
#[path = "grouping.rs"]
mod grouping;
#[path = "inspector.rs"]
mod inspector;
#[path = "library.rs"]
mod library;
#[path = "models.rs"]
mod model_setup;
#[path = "navigation.rs"]
mod navigation;
#[path = "navigator.rs"]
mod navigator;
#[path = "organize.rs"]
mod organize;
#[path = "pick_review.rs"]
mod pick_review;
#[path = "plan_review.rs"]
mod plan_review;
#[path = "pointer.rs"]
mod pointer;
#[path = "review.rs"]
mod review;
#[path = "rotation.rs"]
mod rotation;
#[path = "selection.rs"]
mod selection;
#[path = "shortcuts.rs"]
mod shortcuts;
#[path = "status.rs"]
mod status;
#[path = "tree_controls.rs"]
mod tree_controls;
#[path = "video.rs"]
mod video;
use super::theme::*;
use crate::core::{
    analysis::Analysis,
    catalog::Catalog,
    inspection::{self, Request as InspectionRequest, View as InspectionView},
    jobs::{Event, ImageWork, IoWork, Runtime},
    model::*,
    operations::Plan,
    pipeline::Stats,
    session::Session,
};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, RichText, Sense, Stroke, StrokeKind, TextureHandle, Vec2,
};
#[path = "transitions.rs"]
mod transitions;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use transitions::Destination;

use crate::core::browsing::Filter;
use crate::core::views::GroupMode;
#[path = "bookmarks.rs"]
mod bookmarks;
#[path = "preferences.rs"]
mod preferences;
#[path = "updates.rs"]
mod updates;
impl GroupMode {
    const ALL: [Self; 7] = [
        Self::Current,
        Self::Date,
        Self::Camera,
        Self::Location,
        Self::Visual,
        Self::Faces,
        Self::Mixed,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Current => "Current folders",
            Self::Date => "Capture date",
            Self::Camera => "Camera",
            Self::Location => "Location",
            Self::Visual => "Visual similarity",
            Self::Faces => "Faces",
            Self::Mixed => "Date + similarity",
        }
    }
}
struct InspectionFrame {
    request: InspectionRequest,
    texture: Option<TextureHandle>,
    error: Option<String>,
}
struct Texture {
    handle: TextureHandle,
    bytes: usize,
    used: u64,
}

pub struct PhotoSortApp {
    native_quit: Option<super::platform::QuitHandler>,
    smoke_exit: Option<transitions::SmokeExit>,
    video_texture: Option<(MediaId, u64, TextureHandle)>,
    video_context: Option<(Step, MediaId)>,
    preferences: crate::core::preferences::Preferences,
    preferences_path: Option<PathBuf>,
    preferences_draft: Option<crate::core::preferences::Preferences>,
    preferences_section: Option<preferences::Section>,
    browse_filters_open: bool,
    about: bool,
    update_dialog: bool,
    update_startup_checked: bool,
    model_dialog: bool,
    model_removal: Option<crate::core::model_assets::ManagedCopy>,
    model_issue: Option<crate::core::model_assets::Issue>,
    model_retry: Option<model_setup::Retry>,
    saved_views: crate::core::views::Saved,
    pending_bookmark: Option<crate::core::views::Bookmark>,
    view_data_ready: HashSet<Step>,
    resume_started: bool,
    resume_suspended: bool,
    view_checkpoint: Instant,
    library_offset: f32,
    library_restore_offset: Option<f32>,
    organize_view: Option<crate::core::views::EditorBookmark>,
    runtime: Runtime,
    catalog: Option<Arc<Catalog>>,
    media: Vec<Media>,
    index: HashMap<MediaId, usize>,
    filtered: Vec<usize>,
    filter_dirty: bool,
    step: Step,
    root: Option<PathBuf>,
    decisions: HashMap<MediaId, Decision>,
    metadata: HashMap<MediaId, Metadata>,
    analysis: HashMap<MediaId, Analysis>,
    groups: HashMap<MediaId, usize>, // Provisional technical groups only.
    visual_groups: Option<HashMap<MediaId, usize>>,
    automatic_scenes: Arc<HashMap<MediaId, usize>>,
    scene_choices: Vec<(usize, usize, MediaId)>,
    scene_undo: Option<Arc<crate::core::manual_groups::ManualGroups>>,
    cull_groups: Arc<HashMap<MediaId, usize>>,
    automatic_groups: Arc<HashMap<MediaId, usize>>,
    manual_groups: Arc<crate::core::manual_groups::ManualGroups>,
    scene_groups: Arc<crate::core::manual_groups::ManualGroups>,
    group_undo: Option<Arc<crate::core::manual_groups::ManualGroups>>,
    group_task: Option<grouping::Task>,
    group_serial: u64,
    group_choices: Arc<HashMap<usize, crate::core::browsing::GroupChoice>>,
    cull_group_run: Option<u64>,
    duplicates: HashMap<MediaId, MediaId>,
    near_duplicates: HashMap<MediaId, MediaId>,
    delete_pairs: Vec<crate::core::near_duplicates::batch::Pair>,
    rotations: HashMap<MediaId, crate::core::models::RotationSuggestion>,
    rotation_review: crate::core::rotation::Review,
    rotation_run: Option<u64>,
    rotation_queue_only: bool,
    review_notice: Option<Step>,
    delete_review: crate::core::delete_review::Review,
    delete_current: Option<MediaId>,
    delete_categories: HashSet<crate::core::delete_review::Category>,
    delete_run: Option<u64>,
    pick_review: crate::core::pick_review::Review,
    pick_run: Option<u64>,
    pick_focus: bool,
    embeddings: HashMap<MediaId, Arc<Vec<f32>>>,
    rotation_plan: bool,
    rotation_reload: Option<crate::core::rotation::Review>,
    departure: Option<Destination>,
    organize_departure: Option<Destination>,
    after_apply: Option<Destination>,
    close_authorized: bool,
    close_save_pending: bool,
    close_checkpointed: bool,
    active: Option<MediaId>,
    selection: crate::core::selection::Selection,
    comparison: Vec<MediaId>,
    textures: HashMap<(MediaId, bool), Texture>,
    requested: HashSet<(MediaId, bool)>,
    failures: HashMap<MediaId, String>,
    image_warnings: HashMap<MediaId, String>,
    tick: u64,
    query: String,
    filter: Filter,
    browse: crate::core::browsing::Options,
    browse_groups: Vec<usize>,
    navigator: crate::core::browsing::tree::State,
    browse_tree: Option<Arc<crate::core::browsing::tree::View>>,
    navigation_index: Option<Arc<crate::core::navigation::Index>>,
    navigation_notice: Option<String>,
    browse_folders: Arc<Vec<crate::core::browsing::Folder>>,
    library_layout: library::Layout,
    library_reveal: bool,
    browse_ready: bool,
    browse_run: Option<u64>,
    metadata_run: Option<u64>,
    metadata_progress: Option<(usize, usize)>,
    metadata_failures: HashMap<MediaId, String>,
    review: bool,
    inspector: bool,
    was_wide: bool,
    companions: bool,
    group_mode: GroupMode,
    pending_location_layout: Option<crate::core::geography::Layout>,
    output: String,
    organize_drafts: HashMap<GroupMode, crate::core::organize::Draft>,
    organize_editor: Option<organize::Editor>,
    face_groups: Option<crate::core::regional::grouping::Output>,
    mixed_labels: Option<HashMap<MediaId, String>>,
    organize_run: Option<(u64, GroupMode)>,
    organize_preview_requested: bool,
    file_names: HashMap<MediaId, String>,
    status: String,
    status_scope: Option<(Step, Option<GroupMode>)>,
    error: Option<String>,
    scanning: bool,
    busy: bool,
    analysis_run: Option<u64>,
    progress: Option<(usize, usize)>,
    stats: Stats,
    plan: Option<Plan>,
    plan_rows: plan_review::Rows,
    retry_plan: Option<Plan>,
    retry_edit: Option<plan_review::Edit>,
    recovery_ticket: Option<crate::core::operations::recovery::Ticket>,
    applying_plan: Option<Plan>,
    plan_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    apply_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    plan_open: bool,
    help: bool,
    dirty: Option<Instant>,
    inspection_view: InspectionView,
    canvas_drag: pointer::DragTracker,
    independent_views: HashMap<u64, InspectionView>,
    sync_views: bool,
    focused_slot: u64,
    inspection_wanted: Vec<InspectionRequest>,
    inspection_frames: HashMap<u64, InspectionFrame>,
    view_started: Instant,
}
impl PhotoSortApp {
    fn report_scan_warnings(&mut self, warnings: &[String]) {
        if let Some(first) = warnings.first() {
            let warning = format!("{} scan warning(s). {first}", warnings.len());
            // Recovery is published before the catalog; discovery must not hide it.
            match &mut self.error {
                Some(error) => {
                    error.push('\n');
                    error.push_str(&warning);
                }
                None => self.error = Some(warning),
            }
        }
    }

    fn restore_operations(&mut self, restored: crate::core::operations::recovery::Restored) {
        self.rotation_plan = !restored.plan.actions.is_empty() && restored.rotation_only;
        self.retry_plan = (!restored.plan.actions.is_empty()).then_some(restored.plan);
        self.recovery_ticket = self.retry_plan.as_ref().map(|_| restored.ticket);
        if let Some(plan) = &self.retry_plan {
            self.set_status(format!(
                "{} saved operations await review",
                plan.actions.len()
            ));
            let mut details = restored.errors;
            if restored.uncertain > 0 {
                details.insert(0, format!(
                    "{} operation(s) were interrupted before a completion record. They may already be partly applied. Check their source and destination files before retrying; the original source and overwrite guards remain active.",
                    restored.uncertain
                ));
            }
            if !details.is_empty() {
                self.error = Some(details.join("\n"));
            }
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>, folder: Option<PathBuf>) -> Self {
        theme_install(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let runtime = Runtime::new(move || ctx.request_repaint());
        let mut app = Self::with_runtime(runtime, None);
        let path = crate::core::preferences::Preferences::path();
        match crate::core::preferences::Preferences::load(&path) {
            Ok(preferences) => app.preferences = preferences,
            Err(e) => app.error = Some(format!("Preferences could not be loaded: {e}")),
        }
        app.runtime.cache.configure(app.preferences.cache);
        app.runtime.processing.configure(app.preferences.processing);
        app.runtime.cache.request(false);
        app.preferences_path = Some(path);
        if let Some(folder) = folder {
            app.open(folder);
        }
        match super::platform::QuitHandler::install(&cc.egui_ctx) {
            Ok(handler) => app.native_quit = handler,
            Err(error) => app.error = Some(error),
        }
        app
    }
    fn with_runtime(runtime: Runtime, folder: Option<PathBuf>) -> Self {
        let mut app = Self {
            native_quit: None,
            smoke_exit: None,
            video_texture: None,
            video_context: None,
            preferences: Default::default(),
            preferences_path: None,
            preferences_draft: None,
            preferences_section: None,
            browse_filters_open: false,
            about: false,
            update_dialog: false,
            update_startup_checked: false,
            model_dialog: false,
            model_removal: None,
            model_issue: None,
            model_retry: None,
            saved_views: Default::default(),
            pending_bookmark: None,
            view_data_ready: [Step::Organize, Step::Cull, Step::Rotation]
                .into_iter()
                .collect(),
            resume_started: false,
            resume_suspended: false,
            view_checkpoint: Instant::now(),
            library_offset: 0.0,
            library_restore_offset: None,
            organize_view: None,
            runtime,
            catalog: None,
            media: Vec::new(),
            index: HashMap::new(),
            filtered: Vec::new(),
            filter_dirty: true,
            step: Step::Organize,
            root: None,
            decisions: HashMap::new(),
            metadata: HashMap::new(),
            analysis: HashMap::new(),
            groups: HashMap::new(),
            visual_groups: None,
            automatic_scenes: Arc::default(),
            scene_choices: Vec::new(),
            scene_undo: None,
            cull_groups: Arc::default(),
            automatic_groups: Arc::default(),
            manual_groups: Arc::default(),
            scene_groups: Arc::default(),
            group_undo: None,
            group_task: None,
            group_serial: 0,
            group_choices: Arc::default(),
            cull_group_run: None,
            duplicates: HashMap::new(),
            near_duplicates: HashMap::new(),
            delete_pairs: Vec::new(),
            rotations: HashMap::new(),
            rotation_review: Default::default(),
            rotation_run: None,
            rotation_queue_only: false,
            review_notice: None,
            delete_review: Default::default(),
            delete_current: None,
            delete_categories: crate::core::delete_review::Category::ALL
                .into_iter()
                .collect(),
            delete_run: None,
            pick_review: Default::default(),
            pick_run: None,
            pick_focus: false,
            embeddings: HashMap::new(),
            rotation_plan: false,
            rotation_reload: None,
            departure: None,
            organize_departure: None,
            after_apply: None,
            close_authorized: false,
            close_save_pending: false,
            close_checkpointed: false,
            active: None,
            selection: Default::default(),
            comparison: Vec::new(),
            textures: HashMap::new(),
            requested: HashSet::new(),
            failures: HashMap::new(),
            image_warnings: HashMap::new(),
            tick: 0,
            query: String::new(),
            filter: Filter::All,
            browse: Default::default(),
            browse_groups: Vec::new(),
            navigator: Default::default(),
            browse_tree: None,
            navigation_index: None,
            navigation_notice: None,
            browse_folders: Default::default(),
            library_layout: Default::default(),
            library_reveal: true,
            browse_ready: false,
            browse_run: None,
            metadata_run: None,
            metadata_progress: None,
            metadata_failures: HashMap::new(),
            review: false,
            inspector: true,
            was_wide: true,
            companions: true,
            group_mode: GroupMode::Current,
            pending_location_layout: None,
            output: String::new(),
            organize_drafts: HashMap::new(),
            organize_editor: None,
            face_groups: None,
            mixed_labels: None,
            organize_run: None,
            organize_preview_requested: false,
            file_names: HashMap::new(),
            status_scope: None,
            status: "Ready when you are".into(),
            error: None,
            scanning: false,
            busy: false,
            analysis_run: None,
            progress: None,
            stats: Stats::default(),
            plan: None,
            plan_rows: plan_review::Rows::default(),
            retry_plan: None,
            retry_edit: None,
            recovery_ticket: None,
            applying_plan: None,
            plan_stop: None,
            apply_stop: None,
            plan_open: false,
            help: false,
            dirty: None,
            inspection_view: InspectionView::default(),
            canvas_drag: Default::default(),
            independent_views: HashMap::new(),
            sync_views: true,
            focused_slot: 0,
            inspection_wanted: Vec::new(),
            inspection_frames: HashMap::new(),
            view_started: Instant::now(),
        };
        if let Some(folder) = folder {
            app.open(folder);
        }
        app
    }
    fn session_intent(&self) -> Session {
        Session {
            manual_groups: self.manual_groups.clone(),
            scene_groups: self.scene_groups.clone(),
            decisions: self.decisions.clone(),
            organize: self.catalog.as_ref().and_then(|catalog| {
                crate::core::organize::Intent::capture(
                    catalog,
                    self.organize_drafts.clone(),
                    self.file_names.clone(),
                    PathBuf::from(&self.output),
                )
            }),
            ..Default::default()
        }
    }
    fn session_snapshot(&self) -> Session {
        let mut session = self.session_intent();
        session.fingerprints = self.media.iter().map(|m| (m.id, m.fingerprint)).collect();
        if let Some(catalog) = &self.catalog {
            session.fingerprints.extend(
                session
                    .decisions
                    .keys()
                    .filter_map(|id| catalog.supporting_fingerprints.get(id).map(|f| (*id, *f))),
            );
        }
        session
    }
    fn save_session(&mut self) {
        // The accepted operation context owns persistence until its receipt is
        // reconciled. Never queue an old pre-move snapshot behind the executor.
        if self.applying_plan.is_some() {
            return;
        }
        if let (Some(root), Some(_)) = (&self.root, &self.catalog) {
            let session = self.session_snapshot();
            if let Err(e) = self.runtime.io(IoWork::Save {
                root: root.clone(),
                session,
            }) {
                self.error = Some(e);
                return;
            }
        }
        self.dirty = None;
    }
    fn open(&mut self, folder: PathBuf) {
        self.request_departure(Destination::Folder(folder));
    }
    fn open_now(&mut self, folder: PathBuf) {
        if self.busy {
            return;
        }
        self.checkpoint_bookmarks();
        self.save_session();
        self.scan_folder(folder);
    }
    // Reload after an apply has already queued its reconciled session save.
    // Saving the old catalog again would overwrite migrated fingerprints.
    fn scan_folder(&mut self, folder: PathBuf) {
        self.browse_filters_open = false;
        self.clear_model_retry();
        self.runtime.next_epoch();
        self.organize_view = None;
        self.saved_views = Default::default();
        self.pending_bookmark = None;
        self.view_data_ready = [Step::Organize, Step::Cull, Step::Rotation]
            .into_iter()
            .collect();
        self.library_restore_offset = None;
        self.library_offset = 0.0;
        self.catalog = None;
        self.media.clear();
        self.index.clear();
        self.filtered.clear();
        self.selection.clear();
        self.decisions.clear();
        self.metadata.clear();
        self.browse = Default::default();
        self.browse_groups.clear();
        self.navigator = Default::default();
        self.browse_tree = None;
        self.navigation_index = None;
        self.navigation_notice = None;
        self.browse_folders = Default::default();
        self.library_reveal = true;
        self.browse_ready = false;
        self.browse_run = None;
        self.metadata_run = None;
        self.metadata_progress = None;
        self.metadata_failures.clear();
        self.analysis.clear();
        self.groups.clear();
        self.visual_groups = None;
        self.automatic_scenes = Arc::default();
        self.scene_choices.clear();
        self.scene_undo = None;
        self.face_groups = None;
        self.cull_groups = Arc::default();
        self.automatic_groups = Arc::default();
        self.manual_groups = Arc::default();
        self.scene_groups = Arc::default();
        self.group_choices = Arc::default();
        self.group_undo = None;
        self.group_task = None;
        self.cull_group_run = None;
        self.duplicates.clear();
        self.near_duplicates.clear();
        self.delete_pairs.clear();
        self.rotations.clear();
        self.delete_review = Default::default();
        self.delete_current = None;
        self.delete_run = None;
        self.pick_review = Default::default();
        self.pick_run = None;
        self.pick_focus = false;
        self.embeddings.clear();
        self.rotation_review = Default::default();
        self.rotation_run = None;
        self.rotation_queue_only = false;
        self.active = None;
        self.comparison.clear();
        self.inspection_wanted.clear();
        self.inspection_frames.clear();
        self.textures.clear();
        self.requested.clear();
        self.failures.clear();
        self.image_warnings.clear();
        self.query.clear();
        self.filter = Filter::All;
        self.set_review_plan(None);
        self.retry_plan = None;
        self.recovery_ticket = None;
        self.applying_plan = None;
        self.apply_stop = None;
        self.plan_open = false;
        self.pending_location_layout = None;
        self.organize_drafts.clear();
        self.organize_editor = None;
        self.organize_departure = None;
        self.mixed_labels = None;
        self.organize_run = None;
        self.organize_preview_requested = false;
        self.file_names.clear();
        self.analysis_run = None;
        self.progress = None;
        self.error = None;
        self.scanning = true;
        self.root = Some(folder.clone());
        self.output = folder.display().to_string();
        self.set_status("Discovering photos…".into());
        if let Err(e) = self.runtime.io(IoWork::Scan(folder)) {
            self.error = Some(e);
            self.scanning = false;
        }
    }
    fn select_folder(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Open a photo library")
            .pick_folder()
        {
            self.open(path);
        }
    }
    fn changed_item(&mut self, id: MediaId) {
        self.changed_items([id]);
    }
    fn changed_items(&mut self, ids: impl IntoIterator<Item = MediaId>) {
        self.pick_review.reconcile_external(&self.decisions);
        self.delete_review.reconcile_external(&self.decisions);
        if self.browse_ready {
            self.runtime
                .browsing
                .decision_batch(ids.into_iter().map(|id| (id, self.decision(id))));
        }
        self.dirty = Some(Instant::now());
        self.filter_dirty = true;
    }
    fn changed(&mut self) {
        self.pick_review.reconcile_external(&self.decisions);
        self.delete_review.reconcile_external(&self.decisions);
        self.review_changed();
    }
    fn review_changed(&mut self) {
        if self.browse_ready {
            self.runtime.browsing.decisions(self.decisions.clone());
        }
        self.dirty = Some(Instant::now());
        self.filter_dirty = true;
    }
    fn decision(&self, id: MediaId) -> Decision {
        self.decisions.get(&id).cloned().unwrap_or_default()
    }
    // Decoded textures stay canonical; every surface applies this display-only transform.
    fn preview_turns(&self, id: MediaId) -> u8 {
        if self.step == Step::Rotation {
            self.rotation_review.choice(id).map_or(0, |c| c.turns())
        } else {
            self.decision(id).turns
        }
    }
    fn reject(&mut self, id: MediaId) {
        if self.step == Step::PickBest {
            self.toggle_pick(id);
            return;
        }
        if self.step == Step::EasyDelete {
            if let Some(row) = self.delete_current
                && let Some(side) = self.delete_review.entry(row).and_then(|entry| {
                    entry
                        .candidates()
                        .iter()
                        .position(|candidate| *candidate == id)
                })
            {
                self.toggle_delete_candidate(side);
            } else if self.delete_review.entry(id).is_some() && self.allow_review_departure() {
                self.set_active(id);
                self.toggle_delete_candidate(0);
            }
            return;
        }
        self.stage_manual([id], crate::core::decisions::Change::ToggleTrash);
    }
    fn rating(&mut self, id: MediaId, rating: u8) {
        self.stage_manual([id], crate::core::decisions::Change::Rating(rating));
    }
    fn rotate(&mut self, id: MediaId, turns: u8) {
        if self.busy || (self.step == Step::Rotation && self.rotation_run.is_some()) {
            return;
        }
        if self
            .media
            .get(*self.index.get(&id).unwrap_or(&usize::MAX))
            .is_some_and(|m| m.kind != MediaKind::Video)
        {
            if self.step == Step::Rotation {
                self.ensure_rotation_choice(id);
                self.rotation_review.rotate(id, turns);
            } else {
                self.stage_manual([id], crate::core::decisions::Change::Rotate(turns));
                return;
            }
            self.changed_item(id);
        }
    }
    fn set_active(&mut self, id: MediaId) {
        self.navigation_notice = None;
        if self.step == Step::PickBest
            && let Some(slot) = self
                .pick_review
                .comparison()
                .and_then(|c| c.paths.iter().position(|p| *p == id))
        {
            self.library_reveal |= self.active != Some(id);
            self.active = Some(id);
            self.focused_slot = if self.pick_focus { 0 } else { slot as u64 };
            self.view_started = Instant::now();
            return;
        }
        if self.active != Some(id) && !self.allow_review_departure() {
            return;
        }
        if self.step == Step::EasyDelete && self.delete_review.entry(id).is_some() {
            self.delete_current = Some(id);
        }
        if self.step == Step::Cull && self.selection.len() <= 1 {
            self.selection.single(id);
        }
        self.library_reveal |= self.active != Some(id);
        self.active = Some(id);
        self.inspection_view = InspectionView::default();
        self.independent_views.clear();
        self.focused_slot = self
            .comparison
            .iter()
            .position(|other| *other == id)
            .unwrap_or(0) as u64;
        self.view_started = Instant::now();
    }
    fn selected_rating(&self, id: MediaId) -> u8 {
        self.decisions
            .get(&id)
            .and_then(|d| d.rating)
            .unwrap_or_else(|| self.metadata.get(&id).map_or(0, |m| m.rating))
    }
    fn poll(&mut self, ctx: &egui::Context) {
        self.poll_desktop();
        if self.runtime.desktop.poll_terminals() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        for _ in 0..12 {
            let Ok(envelope) = self.runtime.events.try_recv() else {
                break;
            };
            if envelope.epoch != self.runtime.epoch() {
                continue;
            }
            match envelope.event {
                Event::MetadataBatch {
                    run,
                    items,
                    done,
                    total,
                    stats,
                } => {
                    if self.metadata_run == Some(run) {
                        for (id, result) in items {
                            if !self.index.contains_key(&id) {
                                continue;
                            }
                            match result {
                                Ok(metadata) => {
                                    self.metadata.entry(id).or_insert(metadata);
                                    self.update_browse_item(id);
                                    self.metadata_failures.remove(&id);
                                }
                                Err(error) => {
                                    self.metadata_failures.insert(id, error);
                                }
                            }
                        }
                        self.stats = stats;
                        self.metadata_progress = Some((done, total));
                        if done == total {
                            self.metadata_run = None;
                            if self.metadata_organize_mode()
                                && self.organize_departure.is_some()
                                && self.plan.is_none()
                                && self.organize_metadata_ready()
                            {
                                self.organize();
                            }
                        }
                        self.filter_dirty = true;
                    }
                }
                Event::Batch(batch) => {
                    for item in batch {
                        self.index.insert(item.id, self.media.len());
                        self.media.push(item);
                    }
                    self.filter_dirty = true;
                    self.set_status(format!("Discovered {} items…", self.media.len()));
                }
                Event::Opened(catalog, session, views) => {
                    // The scan worker already resolved aliases and relative paths.
                    // Follow that identity only while destination still has its
                    // initial value; preserve edits made during discovery.
                    if self
                        .root
                        .as_ref()
                        .is_some_and(|root| self.output == root.to_string_lossy())
                    {
                        self.output = catalog.root.display().to_string();
                    }
                    self.media = catalog.media.clone();
                    self.index = self
                        .media
                        .iter()
                        .enumerate()
                        .map(|(i, m)| (m.id, i))
                        .collect();
                    self.manual_groups = session.manual_groups;
                    self.scene_groups = session.scene_groups;
                    self.decisions = session.decisions;
                    if let Some(intent) = session.organize {
                        self.organize_drafts = intent.drafts;
                        self.file_names = intent.names;
                        self.output = intent.output.to_string_lossy().into_owned();
                    }
                    self.active = self.media.first().map(|m| m.id);
                    self.root = Some(catalog.root.clone());
                    self.set_status(format!("{} items · library ready", self.media.len()));
                    self.report_scan_warnings(&catalog.warnings);
                    self.preferences.remember(&catalog.root);
                    self.save_preferences();
                    self.catalog = Some(catalog);
                    self.initialize_browse();
                    self.scanning = false;
                    self.load_bookmarks(views);
                    if let Some(mut review) = self.rotation_reload.take() {
                        review.retain(|id| self.index.contains_key(&id));
                        self.rotation_review = review;
                        self.rotation_queue_only = true;
                    }
                    self.filter_dirty = true;
                    if let Some(destination) = self.after_apply.take() {
                        self.continue_departure(destination);
                    }
                    if let Some(plan) = &self.retry_plan {
                        self.set_status(format!(
                            "{} unfinished operations · Review changes to resolve",
                            plan.actions.len()
                        ));
                    }
                    if !self.manual_groups.is_empty() {
                        self.resolve_groups(
                            self.automatic_groups.clone(),
                            crate::core::manual_groups::Edit::Resolve,
                            None,
                            false,
                        );
                    }
                    self.start_ready_pick_analysis();
                    if self.step == Step::Cull || self.metadata_organize_mode() {
                        self.start_metadata_index();
                    }
                }
                Event::Asset {
                    id,
                    preview,
                    pixels,
                    decoder_warning,
                    metadata,
                    stats,
                } => {
                    self.requested.remove(&(id, preview));
                    self.accept_image_warning(id, decoder_warning);
                    self.failures.remove(&id);
                    self.metadata.insert(id, metadata);
                    self.update_browse_item(id);
                    self.metadata_failures.remove(&id);
                    self.filter_dirty |= self.filter == Filter::Rated || self.step == Step::Cull;
                    self.stats = stats;
                    let size = [pixels.width() as usize, pixels.height() as usize];
                    let bytes = pixels.as_raw().len();
                    self.evict(bytes);
                    let handle = ctx.load_texture(
                        format!("{}-{preview}-{}", id.0, self.tick),
                        egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw()),
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(
                        (id, preview),
                        Texture {
                            handle,
                            bytes,
                            used: self.tick,
                        },
                    );
                }
                Event::Inspected {
                    request,
                    result,
                    decoder_warning,
                    stats,
                } => {
                    if self.inspection_wanted.contains(&request) {
                        if result.is_ok() {
                            self.accept_image_warning(request.media.id, decoder_warning);
                        }
                        self.stats = stats;
                        let (texture, error) = match result {
                            Ok(pixels) => (
                                Some(ctx.load_texture(
                                    format!("detail-{}-{}", request.slot, self.tick),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [pixels.width() as usize, pixels.height() as usize],
                                        pixels.as_raw(),
                                    ),
                                    egui::TextureOptions::LINEAR,
                                )),
                                None,
                            ),
                            Err(error) => (None, Some(error)),
                        };
                        self.inspection_frames.insert(
                            request.slot,
                            InspectionFrame {
                                request,
                                texture,
                                error,
                            },
                        );
                    }
                }
                Event::AnalysisError { run, id, error } => {
                    if self.analysis_run == Some(run) {
                        self.set_status(format!(
                            "Analysis skipped {}: {error}",
                            self.index
                                .get(&id)
                                .map(|&i| self.media[i].name())
                                .unwrap_or_default()
                        ));
                        self.error = Some(error);
                        if self.pick_run == Some(run) || self.delete_run == Some(run) {
                            let deleting = self.delete_run == Some(run);
                            self.runtime.cancel_analysis();
                            self.delete_run = None;
                            self.analysis_run = None;
                            self.pick_run = None;
                            self.progress = None;
                            self.set_workflow_status(if deleting { Step::EasyDelete } else { Step::PickBest }, if deleting {"Easy Delete stopped: a photo could not be analyzed. Existing reviews retained."}else{"Pick Best stopped: every photo must be scored. Existing reviews retained."}.into());
                        }
                    }
                }
                Event::AssetError { id, preview, error } => {
                    self.requested.remove(&(id, preview));
                    self.failures.insert(id, error);
                }
                Event::Analyzed {
                    run,
                    id,
                    analysis,
                    embedding,
                    group,
                    duplicate,
                    metadata,
                } => {
                    if self.analysis_run == Some(run) {
                        if let Some(embedding) = embedding {
                            self.embeddings.insert(id, embedding);
                        }
                        self.analysis.insert(id, analysis);
                        if group > 0 && self.cull_group_run != Some(run) {
                            self.record_analysis_group(run, id, group);
                        }
                        self.metadata.insert(id, metadata);
                        self.metadata_failures.remove(&id);
                        if let Some(original) = duplicate {
                            self.duplicates.insert(id, original);
                        }
                        self.update_browse_item(id);
                        self.filter_dirty |= self.step == Step::Cull
                            || matches!(
                                self.filter,
                                Filter::Issues | Filter::Duplicates | Filter::Rated
                            );
                    }
                }
                Event::Rotation {
                    run,
                    id,
                    suggestion,
                } => {
                    if self.analysis_run == Some(run) {
                        self.rotations.insert(id, suggestion);
                    }
                }
                Event::Progress { run, done, total } => {
                    if run == 0 || self.analysis_run == Some(run) {
                        self.progress = Some((done, total));
                    }
                }
                Event::GeneralStarted { run } => {
                    if self.analysis_run == Some(run) {
                        self.set_status(if self.organize_run == Some((run, GroupMode::Faces)) {
                            "Grouping people · previews remain available"
                        } else {
                            "Grouping photos by matching image regions · previews remain available"
                        }
                        .into());
                        self.progress = None;
                    }
                }
                Event::GeneralGroups {
                    run,
                    automatic,
                    choices,
                    groups,
                } => {
                    if self.analysis_run == Some(run)
                        && self.organize_run != Some((run, GroupMode::Mixed))
                        && self.cull_group_run != Some(run)
                        && self.organize_run != Some((run, GroupMode::Faces))
                    {
                        self.automatic_scenes = automatic;
                        self.scene_choices = choices;
                        self.visual_groups = Some(groups);
                        self.filter_dirty = true;
                    }
                }
                Event::DuplicateStarted { run } => {
                    if self.analysis_run == Some(run) && self.delete_run == Some(run) {
                        self.set_status("Checking duplicate pairs for visible subject changes · previews remain available".into());
                        self.progress = None;
                    }
                }
                Event::DuplicatePairs { run, pairs } => {
                    if self.analysis_run == Some(run) && self.delete_run == Some(run) {
                        self.delete_pairs = pairs;
                    }
                }
                Event::SubjectStarted { run } => {
                    if self.analysis_run == Some(run) && self.cull_group_run == Some(run) {
                        self.set_status(
                            "Comparing same-subject candidates · previews remain available".into(),
                        );
                        self.progress = None;
                    }
                }
                Event::FaceGroups { run, groups } => {
                    if self.analysis_run == Some(run)
                        && self.organize_run == Some((run, GroupMode::Faces))
                    {
                        self.face_groups = Some(groups);
                        self.filter_dirty = true;
                    }
                }
                Event::SubjectGroups { run, groups } => {
                    if self.analysis_run == Some(run) && self.cull_group_run == Some(run) {
                        let automatic = Arc::new(groups);
                        if self.manual_groups.is_empty() {
                            self.automatic_groups = automatic.clone();
                            self.publish_groups(automatic);
                        } else {
                            self.resolve_groups(
                                automatic,
                                crate::core::manual_groups::Edit::Resolve,
                                Some(run),
                                false,
                            );
                        }
                    }
                }
                Event::MixedGroups { run, labels } => {
                    if self.analysis_run == Some(run)
                        && self.organize_run == Some((run, GroupMode::Mixed))
                    {
                        self.mixed_labels = Some(labels);
                    }
                }
                Event::GroupsEdited(result) => self.finish_group_edit(result),
                Event::ModelsNeeded { run, issue } => self.models_needed(run, issue),
                Event::AnalysisDone { run, .. } => {
                    if self.analysis_run == Some(run) {
                        self.analysis_run = None;
                        self.progress = None;
                        if let Some((_, completed)) =
                            self.organize_run.filter(|(owner, _)| *owner == run)
                        {
                            self.organize_run = None;
                            self.set_organize_status(completed, match completed {
                                GroupMode::Faces => "People groups ready · group portraits have one shared destination",
                                GroupMode::Mixed => "Date and similarity groups ready",
                                _ => "Visual groups ready · review destinations before applying",
                            }.into());
                            let preview = std::mem::take(&mut self.organize_preview_requested);
                            if preview
                                && self.step == Step::Organize
                                && self.group_mode == completed
                                && self.organize_grouping_ready()
                            {
                                self.organize();
                            }
                        } else if self.rotation_run == Some(run) {
                            self.finish_rotation_detection();
                        } else if self.pick_run == Some(run) {
                            if self.group_task.is_none() {
                                self.finish_pick_analysis();
                            }
                        } else if self.delete_run == Some(run) {
                            self.finish_delete_analysis();
                        } else {
                            self.set_status(format!(
                                "Analysis ready · {} photos · {} exact duplicates",
                                self.analysis.len(),
                                self.duplicates.len()
                            ));
                        }
                    }
                }
                Event::PlanningCancelled => {
                    self.plan_stop = None;
                    self.busy = false;
                    self.organize_departure = None;
                    self.set_status("File planning cancelled · edits retained".into());
                }
                Event::Planned(mut plan) => {
                    if let Some(stop) = self.plan_stop.take() {
                        if stop.load(std::sync::atomic::Ordering::Acquire) {
                            self.busy = false;
                            self.organize_departure = None;
                            self.set_status("File planning cancelled · edits retained".into());
                            continue;
                        }
                        self.set_status("File plan ready · review your changes".into());
                    }

                    if self.rotation_plan {
                        plan.description = "Apply confirmed rotation corrections".into();
                    }
                    self.busy = false;
                    if self.organize_departure.is_some() && plan.actions.is_empty() {
                        let destination = self.organize_departure.take().unwrap();
                        self.set_review_plan(None);
                        self.continue_departure(destination);
                    } else {
                        self.set_review_plan(Some(plan));
                        self.plan_open = self.organize_departure.is_none();
                    }
                }
                Event::OperationsRestored(restored) => self.restore_operations(restored),
                Event::ApplyFailed(error) => {
                    self.applying_plan = None;
                    self.apply_stop = None;
                    self.rotation_plan = false;
                    self.busy = false;
                    self.after_apply = None;
                    self.dirty = None;
                    if let Some(root) = self.root.clone() {
                        self.scan_folder(root);
                    }
                    self.error = Some(error);
                }
                Event::OperationsAmended(result) => {
                    self.busy = false;
                    match result {
                        Ok(restored) => {
                            self.retry_edit = None;
                            self.recovery_ticket = Some(restored.ticket);
                            self.retry_plan = Some(restored.plan.clone());
                            self.set_review_plan(Some(restored.plan));
                            self.error = None;
                            self.set_status(
                                "Destination saved · review the revised plan before retrying"
                                    .into(),
                            );
                        }
                        Err(error) => {
                            if let Some(edit) = &mut self.retry_edit {
                                if edit.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                                    self.retry_edit = None;
                                    self.set_status(
                                        "Destination edit cancelled · saved plan unchanged".into(),
                                    );
                                } else {
                                    edit.saving = false;
                                    edit.error = Some(error);
                                }
                            }
                        }
                    }
                }
                Event::OperationsDiscarded => {
                    self.busy = false;
                    self.recovery_ticket = None;
                    self.discard_remaining_operations();
                }
                Event::Applied(report) => {
                    let recovery_ticket = report.recovery.clone();
                    self.busy = false;
                    // The pre-apply bookmark is already in the durable operation
                    // context. Do not enqueue that old identity snapshot here.
                    self.saved_views.relocate(&report.relocated);
                    if let Some(root) = self.root.clone() {
                        let _ = self.runtime.io(IoWork::SaveViews {
                            root,
                            views: self.saved_views.clone(),
                        });
                    }
                    self.progress = None;
                    self.applying_plan = None;
                    self.apply_stop = None;
                    let mut session = self.session_snapshot();
                    if !self.rotation_plan {
                        session.apply_report(&report, &self.media);
                    } else {
                        session.apply_verified_identities(&report.relocated);
                    }
                    self.decisions = session.decisions.clone();
                    self.manual_groups = session.manual_groups.clone();
                    self.scene_groups = session.scene_groups.clone();
                    self.rotation_review
                        .complete(&report.applied_decisions(&self.media));
                    self.rotation_review.relocate(&report.relocated);
                    let remaining_rotations = self.rotation_review.clone();
                    let retry_rotation = self.rotation_plan;
                    self.rotation_plan = false;
                    let incomplete = report.cancelled
                        || !report.errors.is_empty()
                        || !report.remaining.actions.is_empty();
                    if incomplete {
                        self.after_apply = None;
                    }
                    let output = report.remaining.output_root.clone();
                    let retry = (!report.remaining.actions.is_empty()).then_some(report.remaining);
                    let message = format!(
                        "{} changes applied{}",
                        report.completed.len(),
                        if incomplete {
                            " · unfinished operations retained"
                        } else {
                            ""
                        }
                    );
                    let _ = self.runtime.image(ImageWork::Invalidate);
                    if let Some(root) = self.root.clone() {
                        // Save before the epoch changes. These explicit-root jobs
                        // survive navigation and execute before the following scan.
                        if let Err(error) = self.runtime.io(IoWork::Save {
                            root: root.clone(),
                            session: session.clone(),
                        }) {
                            self.error = Some(error);
                            self.retry_plan = retry;
                            return;
                        }
                        if let Some(output) = output.as_ref().filter(|output| **output != root)
                            && let Err(error) = self.runtime.io(IoWork::MergeSession {
                                root: output.clone(),
                                session,
                            })
                        {
                            self.error = Some(error);
                            self.retry_plan = retry;
                            return;
                        }
                        self.scan_folder(if incomplete {
                            root
                        } else {
                            output.unwrap_or(root)
                        });
                        self.rotation_reload = Some(remaining_rotations);
                    }
                    self.retry_plan = retry;
                    self.recovery_ticket = recovery_ticket;
                    if self.retry_plan.is_some() {
                        self.rotation_plan = retry_rotation;
                    }
                    self.set_status(message);
                    if !report.errors.is_empty() {
                        self.error = Some(report.errors.join("\n"));
                    }
                }
                Event::CacheStats(stats) => self.stats = stats,
                Event::Saved => {}
                Event::Checkpointed(result) => {
                    if self.close_save_pending {
                        self.close_save_pending = false;
                        self.busy = false;
                        match result {
                            Ok(()) => {
                                self.dirty = None;
                                self.close_checkpointed = true;
                                self.close_authorized = true;
                            }
                            Err(error) => {
                                self.error = Some(error);
                                self.set_status(
                                    "Save failed · your unfinished plan is still available".into(),
                                );
                            }
                        }
                    }
                }
                Event::ViewError(error) | Event::PreferencesError(error) => {
                    self.error = Some(error);
                }
                Event::Error(e) => {
                    self.plan_stop = None;
                    self.apply_stop = None;
                    self.group_task = None;
                    self.resume_suspended = self.pending_bookmark.is_some();
                    if let Some(plan) = self.applying_plan.take() {
                        self.retry_plan = Some(plan);
                    }
                    self.runtime.cancel_analysis();
                    self.analysis_run = None;
                    self.error = Some(e);
                    self.after_apply = None;
                    if self.retry_plan.is_none() {
                        self.rotation_plan = false;
                    }
                    self.rotation_run = None;
                    self.delete_run = None;
                    self.pick_run = None;
                    self.organize_run = None;
                    self.organize_preview_requested = false;
                    self.busy = false;
                    self.scanning = false;
                    self.progress = None;
                }
            }
        }
        if !self.requested.is_empty() || self.scanning || self.analysis_run.is_some() || self.busy {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self
            .dirty
            .is_some_and(|t| t.elapsed() > Duration::from_millis(700))
        {
            self.save_session();
        } else if self.dirty.is_some() {
            ctx.request_repaint_after(Duration::from_millis(700));
        }
    }
    fn evict(&mut self, incoming: usize) {
        let mut total: usize = self.textures.values().map(|t| t.bytes).sum();
        while total + incoming > 96 * 1024 * 1024 || self.textures.len() >= 240 {
            let key = self
                .textures
                .iter()
                .filter(|(k, _)| {
                    !(k.1 && (Some(k.0) == self.active || self.comparison.contains(&k.0)))
                })
                .min_by_key(|(_, t)| t.used)
                .map(|(k, _)| *k);
            let Some(key) = key else { break };
            if let Some(texture) = self.textures.remove(&key) {
                total = total.saturating_sub(texture.bytes);
            }
        }
    }
    fn texture(&mut self, id: MediaId, preview: bool) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get_mut(&(id, preview)) {
            t.used = self.tick;
            return Some(t.handle.clone());
        }
        if !self.requested.contains(&(id, preview))
            && !self.failures.contains_key(&id)
            && let Some(&i) = self.index.get(&id)
            && self
                .runtime
                .image(ImageWork::Asset(self.media[i].clone(), preview))
                .is_ok()
        {
            self.requested.insert((id, preview));
        }
        None
    }
    fn refilter(&mut self) {
        if self.step == Step::Cull {
            self.refilter_browse();
            return;
        }
        if !self.filter_dirty {
            return;
        }
        if self.step == Step::PickBest {
            self.filtered = self
                .pick_review
                .comparison()
                .map(|c| {
                    c.paths
                        .iter()
                        .filter_map(|id| self.index.get(id).copied())
                        .collect()
                })
                .unwrap_or_default();
            self.filter_dirty = false;
            return;
        }
        let query = self.query.to_lowercase();
        self.filtered = self
            .media
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                (query.is_empty() || m.path.to_string_lossy().to_lowercase().contains(&query))
                    && self.filter.matches(
                        self.decision(m.id).rejected,
                        crate::core::browsing::rating(m.id, &self.decisions, &self.metadata),
                        self.analysis
                            .get(&m.id)
                            .is_some_and(|a| !a.issues.is_empty()),
                        self.duplicates.contains_key(&m.id)
                            || self.near_duplicates.contains_key(&m.id),
                    )
            })
            .map(|(i, _)| i)
            .collect();
        if self.step == Step::Rotation {
            self.filtered.retain(|i| {
                self.media[*i].kind != MediaKind::Video
                    && (!self.rotation_queue_only
                        || self.rotation_review.choice(self.media[*i].id).is_some())
            });
            if self.rotation_queue_only {
                let order: HashMap<_, _> = self
                    .rotation_review
                    .ids()
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (*id, i))
                    .collect();
                self.filtered
                    .sort_by_key(|i| order.get(&self.media[*i].id).copied().unwrap_or(usize::MAX));
            }
        }
        if self.step == Step::EasyDelete {
            self.filtered.retain(|i| {
                self.delete_review
                    .entry(self.media[*i].id)
                    .is_some_and(|e| self.delete_categories.contains(&e.category))
            });
            let order: HashMap<_, _> = self
                .delete_review
                .entries()
                .iter()
                .enumerate()
                .map(|(i, e)| (e.subject, i))
                .collect();
            self.filtered
                .sort_by_key(|i| order.get(&self.media[*i].id).copied().unwrap_or(usize::MAX));
            if self
                .delete_current
                .is_none_or(|id| !self.filtered.iter().any(|i| self.media[*i].id == id))
            {
                if !self.allow_review_departure() {
                    return;
                }
                self.delete_current = self.filtered.first().map(|i| self.media[*i].id);
                self.active = self.delete_current;
            }
            self.filter_dirty = false;
            return;
        }
        if self
            .active
            .is_none_or(|id| !self.filtered.iter().any(|&i| self.media[i].id == id))
        {
            if !self.allow_review_departure() {
                return;
            }
            self.active = self.filtered.first().map(|&i| self.media[i].id);
        }
        self.filter_dirty = false;
    }
    fn cancel_analysis(&mut self) {
        if self
            .group_task
            .as_ref()
            .is_some_and(|task| task.source.is_some())
        {
            if let Some(task) = self.group_task.take() {
                task.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            }
            self.busy = false;
        }
        self.resume_suspended = self.pending_bookmark.is_some();
        self.runtime.cancel_analysis();
        if self.rotation_run.is_some() {
            self.finish_rotation_detection();
        }
        self.delete_run = None; // A cancelled pass never publishes partial pair choices.
        self.pick_run = None;
        self.organize_run = None;
        self.organize_preview_requested = false;
        self.analysis_run = None;
        self.progress = None;
        self.set_status("Analysis cancelled · completed results retained".into());
    }
    pub(super) fn can_start_analysis(&self) -> bool {
        !self.media.is_empty() && !self.scanning && !self.busy && self.analysis_run.is_none()
    }
    pub(super) fn analyze_current_workflow(&mut self) {
        if self.step == Step::Rotation {
            self.detect_rotations();
        } else {
            self.analyze();
        }
    }
    fn analyze(&mut self) {
        if !self.can_start_analysis() || !self.model_install_ready() {
            return;
        }
        if !self.allow_review_departure() {
            return;
        }
        self.resume_suspended = false;
        match self.runtime.analyze_with_scene_groups(
            Arc::new(self.media.clone()),
            match self.step {
                Step::PickBest => crate::core::jobs::AnalysisProfile::Quality,
                Step::EasyDelete => crate::core::jobs::AnalysisProfile::Duplicates,
                Step::Cull => crate::core::jobs::AnalysisProfile::Subjects,
                Step::Organize if self.group_mode == GroupMode::Faces => {
                    crate::core::jobs::AnalysisProfile::Faces
                }
                Step::Organize if self.group_mode == GroupMode::Mixed => {
                    crate::core::jobs::AnalysisProfile::Mixed
                }
                Step::Organize if self.group_mode == GroupMode::Visual => {
                    crate::core::jobs::AnalysisProfile::Visual
                }
                _ => crate::core::jobs::AnalysisProfile::Technical,
            },
            if self.step == Step::EasyDelete {
                self.preferences.easy_delete
            } else {
                Default::default()
            },
            self.preferences.subject_strictness,
            self.preferences.similarity_distance,
            self.scene_groups.clone(),
        ) {
            Ok(run) => {
                self.clear_model_retry();
                if matches!(self.step, Step::Cull | Step::PickBest) {
                    self.cull_group_run = Some(run);
                    // Keep the completed snapshot until the new pass publishes atomically.
                    self.filter_dirty = true;
                }
                self.organize_preview_requested = false;
                self.organize_run = None;
                if self.step == Step::Organize && self.automatic_organize_mode() {
                    self.organize_run = Some((run, self.group_mode));
                }
                if self.step == Step::PickBest {
                    self.pick_run = Some(run);
                    self.analysis.clear();
                    self.embeddings.clear();
                }
                if self.step == Step::EasyDelete {
                    self.delete_run = Some(run);
                    self.analysis.clear();
                }
                self.runtime.browsing.clear_analysis(
                    matches!(self.step, Step::PickBest | Step::EasyDelete),
                    false, // Existing duplicate reviews remain valid until atomic replacement.
                    false, // Keep completed group memberships during reanalysis.
                );
                self.analysis_run = Some(run);
                self.progress = Some((0, self.media.len()));
                self.set_status("Analyzing locally · previews stay available".into());
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn review_changes(&mut self) {
        if self.review_remaining_operations() {
            return;
        }
        if !self.allow_review_departure() {
            return;
        }
        if self.step == Step::Rotation {
            self.review_rotations();
            return;
        }
        if self.step == Step::PickBest && !self.pick_review.any_completed() {
            self.set_workflow_status(
                Step::PickBest,
                "Complete every comparison in at least one cluster before applying.".into(),
            );
            return;
        }
        self.review_decisions();
    }
    fn review_decisions(&mut self) {
        if self.review_remaining_operations() {
            return;
        }
        self.rotation_plan = false;
        if let Some(catalog) = self.catalog.clone() {
            self.save_session();
            match self
                .runtime
                .image(crate::core::jobs::ImageWork::PlanDecisions {
                    catalog,
                    decisions: self.decisions.clone(),
                    companions: self.companions,
                    policy: self.preferences.metadata_writes,
                }) {
                Ok(()) => self.busy = true,
                Err(e) => {
                    self.after_apply = None;
                    self.error = Some(e);
                }
            }
        }
    }
    fn label_for(&self, media: &Media) -> String {
        let meta = self.metadata.get(&media.id);
        match self.group_mode {
            GroupMode::Current => media
                .path
                .parent()
                .and_then(|p| {
                    self.catalog
                        .as_ref()
                        .and_then(|c| p.strip_prefix(&c.root).ok())
                })
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default(),
            GroupMode::Date => {
                crate::core::dates::label(media, meta.unwrap_or(&Metadata::default()))
                    .unwrap_or_else(|| "undated".into())
            }
            GroupMode::Camera => meta
                .filter(|m| !m.camera.is_empty())
                .map(|m| m.camera.clone())
                .unwrap_or_else(|| "Unknown camera".into()),
            GroupMode::Location => self
                .preferences
                .location_layout
                .label(meta.unwrap_or(&Metadata::default())),
            GroupMode::Faces => self
                .face_groups
                .as_ref()
                .and_then(|groups| groups.folders.get(&media.id))
                .cloned()
                .unwrap_or_else(|| {
                    if media.kind == MediaKind::Video {
                        "Video · kept in place".into()
                    } else {
                        "Analyze to group faces".into()
                    }
                }),
            GroupMode::Mixed => self
                .mixed_labels
                .as_ref()
                .and_then(|labels| labels.get(&media.id))
                .cloned()
                .unwrap_or_else(|| {
                    if media.kind == MediaKind::Video {
                        "Video · kept in place".into()
                    } else {
                        "Analyze to group by date".into()
                    }
                }),
            GroupMode::Visual => self
                .visual_groups
                .as_ref()
                .and_then(|groups| groups.get(&media.id))
                .map(|g| format!("Scene {g:03}"))
                .unwrap_or_else(|| {
                    if media.kind == MediaKind::Video {
                        "Video · kept in place".into()
                    } else if self.visual_groups.is_some() {
                        "Not grouped · kept in place".into()
                    } else {
                        "Analyze to group scenes".into()
                    }
                }),
        }
    }
    fn review_trash(&mut self, request: crate::core::operations::TrashRequest) {
        if self.scanning || self.busy || self.review_remaining_operations() {
            return;
        }
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        self.save_session();
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match self.runtime.io(IoWork::PlanTrash {
            catalog,
            request,
            companions: self.companions,
            cancelled: cancelled.clone(),
        }) {
            Ok(()) => {
                self.set_review_plan(None);
                self.rotation_plan = false;
                self.busy = true;
                self.plan_stop = Some(cancelled);
                self.set_status("Checking files and folders for Trash review…".into());
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn organize(&mut self) {
        if self.review_remaining_operations() {
            return;
        }
        if self.scanning || self.busy {
            return;
        }
        if !self.organize_metadata_ready() {
            self.start_metadata_index();
            return;
        }
        if self.automatic_organize_mode()
            && (!self.organize_grouping_ready() || self.analysis_run.is_some())
            && !self.organize_drafts.contains_key(&self.group_mode)
        {
            let current = self
                .analysis_run
                .is_some_and(|run| self.organize_run == Some((run, self.group_mode)));
            if !current {
                self.analyze();
            }
            self.organize_preview_requested = self
                .analysis_run
                .is_some_and(|run| self.organize_run == Some((run, self.group_mode)));
            return;
        }
        self.rotation_plan = false;
        if let Some(catalog) = self.catalog.clone() {
            let labels = self
                .organize_drafts
                .get(&self.group_mode)
                .map(|draft| draft.labels().clone())
                .unwrap_or_else(|| self.organize_labels());
            let directories = self
                .organize_drafts
                .get(&self.group_mode)
                .map(|draft| draft.directory_edits())
                .unwrap_or_else(|| {
                    crate::core::organize::Draft::from_catalog(&catalog, labels.clone())
                        .directory_edits()
                });
            let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
            match self.runtime.io(IoWork::PlanOrganize {
                cancelled: cancelled.clone(),
                directories,
                catalog,
                labels,
                names: self.file_names.clone(),
                output: PathBuf::from(&self.output),
                companions: self.companions,
            }) {
                Ok(()) => {
                    self.busy = true;
                    self.plan_stop = Some(cancelled);
                }
                Err(e) => self.error = Some(e),
            }
        }
    }
    fn compare_similar(&mut self) {
        let empty = HashMap::new();
        let people = self.step == Step::Organize && self.group_mode == GroupMode::Faces;
        let groups = if matches!(self.step, Step::Cull | Step::PickBest) {
            &self.cull_groups
        } else if people {
            self.face_groups
                .as_ref()
                .map(|output| &output.groups)
                .unwrap_or(&empty)
        } else if self.step == Step::Organize && self.group_mode == GroupMode::Visual {
            self.visual_groups.as_ref().unwrap_or(&empty)
        } else {
            self.visual_groups.as_ref().unwrap_or(&self.groups)
        };
        if let Some(active) = self.active {
            if let Some(group) = groups.get(&active).filter(|g| !people || **g != 0) {
                let mut candidates: Vec<_> = self
                    .media
                    .iter()
                    .filter(|m| m.id != active && groups.get(&m.id) == Some(group))
                    .map(|m| m.id)
                    .collect();
                candidates.sort_by(|a, b| {
                    self.analysis
                        .get(b)
                        .map_or(0.0, |m| m.score)
                        .total_cmp(&self.analysis.get(a).map_or(0.0, |m| m.score))
                });
                self.comparison.clear();
                if let Some(other) = candidates.first() {
                    self.compare_with(*other);
                }
                self.review = true;
                if self.comparison.is_empty() {
                    self.set_workflow_status(
                        self.step,
                        "No similar frame found in this visual group".into(),
                    );
                }
            } else {
                self.set_workflow_status(
                    self.step,
                    "Analyze the library to find visual groups".into(),
                );
            }
        }
    }
    fn navigate_selection(&mut self, delta: isize, extend: bool, include_marked: bool) {
        self.navigate_axis(delta, extend, include_marked, false);
    }
    fn navigate_axis(
        &mut self,
        delta: isize,
        extend: bool,
        include_marked: bool,
        horizontal: bool,
    ) {
        self.navigation_notice = None;
        if self.step == Step::Cull
            && (self.busy
                || self.scanning
                || self.browse_run.is_some()
                || (self.navigation_index.is_some() && self.filter_dirty))
        {
            self.navigation_notice =
                Some("Updating library view… Navigation will resume when ready.".into());
            return;
        }
        if self.step == Step::PickBest {
            self.navigate_pick(delta, false);
            return;
        }
        if self.filtered.is_empty() || !self.allow_review_departure() {
            return;
        }
        self.pending_bookmark = None;
        let current = if self.step == Step::EasyDelete {
            self.delete_current
        } else {
            self.active
        };
        let skip_marked =
            self.step == Step::Cull && self.filter != Filter::Rejected && !include_marked;
        let target = if self.step == Step::Cull
            && let Some(index) = &self.navigation_index
        {
            index.target(
                current,
                delta,
                horizontal,
                self.preferences.navigation,
                include_marked,
                self.filter == Filter::Rejected,
            )
        } else {
            crate::core::navigation::next_visible(
                &self.media,
                &self.filtered,
                current,
                delta,
                &self.decisions,
                skip_marked,
            )
        };
        let Some(id) = target else {
            if self.step == Step::Cull
                && !horizontal
                && !include_marked
                && let Some(rating) = self.preferences.navigation.rating
            {
                self.navigation_notice = Some(format!(
                    "No more photos rated {rating} stars in this direction."
                ));
            }
            return;
        };
        if self.step == Step::Cull && current == Some(id) && !extend {
            return;
        }
        if self.library_selection_mode() {
            self.select_library(id, false, extend);
            return;
        }
        if extend {
            self.compare_with(id);
        } else if !self.comparison.is_empty() && !self.comparison.contains(&id) {
            let slot = (self.focused_slot as usize).min(self.comparison.len() - 1);
            self.comparison[slot] = id;
            if self.step == Step::Cull {
                self.selection.replace(self.comparison.iter().copied());
            }
        }
        self.set_active(id);
    }
    fn modal_active(&self) -> bool {
        self.organize_editor.is_some() || self.other_modal_active()
    }
    fn other_modal_active(&self) -> bool {
        self.plan_open
            || self.browse_filters_open
            || self.pending_location_layout.is_some()
            || self.organize_departure.is_some()
            || self.help
            || self.update_dialog
            || self.about
            || self.model_dialog
            || self.preferences_draft.is_some()
            || self.busy
            || self.review_notice.is_some()
            || self.departure.is_some()
    }
    fn keyboard(&mut self, ctx: &egui::Context) {
        if egui::Popup::is_any_open(ctx) || self.navigator_popup_open(ctx) || self.modal_active() {
            return;
        }
        if self.application_shortcuts(ctx) || ctx.wants_keyboard_input() {
            return;
        }
        self.video_keyboard(ctx);
        for (pressed, modifiers) in pressed_keys(
            ctx,
            matches!(
                self.step,
                Step::Rotation | Step::EasyDelete | Step::PickBest
            ),
        ) {
            if self.modal_active() {
                break;
            }
            if self.workflow_shortcut(pressed, modifiers) {
                continue;
            }
            if self.view_shortcut(pressed, modifiers) {
                continue;
            }
            let command = modifiers.command || modifiers.ctrl;
            if selection::clear_trash_shortcut(pressed, modifiers) {
                self.clear_all_trash_marks();
                continue;
            }
            if command
                && (modifiers.alt
                    || !matches!(
                        pressed,
                        egui::Key::A
                            | egui::Key::Plus
                            | egui::Key::Equals
                            | egui::Key::Minus
                            | egui::Key::ArrowLeft
                            | egui::Key::ArrowRight
                            | egui::Key::ArrowUp
                            | egui::Key::ArrowDown
                            | egui::Key::Num0
                            | egui::Key::Num1
                            | egui::Key::Num2
                            | egui::Key::Num3
                            | egui::Key::Num4
                            | egui::Key::Num5
                    ))
            {
                continue;
            }
            if modifiers.alt && pressed != egui::Key::R {
                continue;
            }
            self.pending_bookmark = None;
            if self.cull_selection_key(pressed, modifiers) {
                continue;
            }
            if command && pressed == egui::Key::A {
                continue; // Select All belongs to the Cull browser or a text field.
            }
            if self.step == Step::PickBest
                && !modifiers.command
                && !modifiers.ctrl
                && !modifiers.alt
            {
                match pressed {
                    egui::Key::Num1 | egui::Key::Num2 | egui::Key::Num3 => {
                        let slot = match pressed {
                            egui::Key::Num1 => 0,
                            egui::Key::Num2 => 1,
                            _ => 2,
                        };
                        if let Some(id) = self
                            .pick_review
                            .comparison()
                            .and_then(|c| c.paths.get(slot).copied())
                        {
                            self.toggle_pick(id);
                        }
                        continue;
                    }
                    egui::Key::ArrowLeft | egui::Key::ArrowRight => {
                        self.navigate_pick(
                            if pressed == egui::Key::ArrowLeft {
                                -1
                            } else {
                                1
                            },
                            true,
                        );
                        continue;
                    }
                    egui::Key::ArrowUp | egui::Key::ArrowDown => {
                        self.navigate_pick(
                            if pressed == egui::Key::ArrowUp { -1 } else { 1 },
                            false,
                        );
                        continue;
                    }
                    egui::Key::C => {
                        self.toggle_pick_focus();
                        continue;
                    }
                    egui::Key::K => {
                        self.confirm_pick(true);
                        continue;
                    }
                    egui::Key::R => {
                        self.reset_pick(modifiers.shift);
                        continue;
                    }
                    egui::Key::Enter => {
                        if modifiers.shift {
                            self.review_changes();
                        } else {
                            self.confirm_pick(false);
                        }
                        continue;
                    }
                    _ => {}
                }
            }
            if self.step == Step::EasyDelete
                && !modifiers.command
                && !modifiers.ctrl
                && !modifiers.alt
            {
                match pressed {
                    egui::Key::ArrowLeft | egui::Key::Num1 => {
                        self.toggle_delete_candidate(0);
                        continue;
                    }
                    egui::Key::ArrowRight | egui::Key::Num2 => {
                        self.toggle_delete_candidate(1);
                        continue;
                    }
                    egui::Key::R => {
                        self.reset_delete(modifiers.shift);
                        continue;
                    }
                    egui::Key::Enter => {
                        if modifiers.shift {
                            self.review_changes();
                        } else {
                            self.confirm_delete();
                        }
                        continue;
                    }
                    egui::Key::A => {
                        self.accept_delete_defaults();
                        continue;
                    }
                    _ => {}
                }
            }
            if self.step == Step::Rotation
                && !modifiers.command
                && !modifiers.ctrl
                && !modifiers.alt
            {
                match pressed {
                    egui::Key::Q | egui::Key::E => {
                        if let Some(id) = self.active {
                            self.rotate(id, if pressed == egui::Key::Q { 3 } else { 1 });
                        }
                        continue;
                    }
                    egui::Key::R => {
                        self.reset_rotation(modifiers.shift);
                        continue;
                    }
                    egui::Key::Enter => {
                        if modifiers.shift {
                            self.review_rotations();
                        } else {
                            self.confirm_rotation();
                        }
                        continue;
                    }
                    egui::Key::Num1 | egui::Key::Num2 if !modifiers.command && !modifiers.ctrl => {
                        self.select_rotation(pressed == egui::Key::Num2);
                        continue;
                    }
                    _ => {}
                }
            }
            if pressed == egui::Key::ArrowRight || pressed == egui::Key::ArrowDown {
                self.navigate_axis(
                    1,
                    modifiers.shift,
                    modifiers.command || modifiers.ctrl,
                    pressed == egui::Key::ArrowRight,
                );
            }
            if pressed == egui::Key::ArrowLeft || pressed == egui::Key::ArrowUp {
                self.navigate_axis(
                    -1,
                    modifiers.shift,
                    modifiers.command || modifiers.ctrl,
                    pressed == egui::Key::ArrowLeft,
                );
            }
            if pressed == egui::Key::D
                && !matches!(self.step, Step::EasyDelete | Step::PickBest)
                && let Some(id) = self.active
            {
                self.reject(id);
            }
            if pressed == egui::Key::R
                && let Some(id) = self.active
            {
                self.rotate(
                    id,
                    if modifiers.alt {
                        2
                    } else if modifiers.shift {
                        3
                    } else {
                        1
                    },
                );
            }
            if pressed == egui::Key::I {
                self.inspector ^= true;
            }
            if pressed == egui::Key::F1 {
                self.comparison.clear();
                self.review = true;
            }
            if pressed == egui::Key::F3 {
                self.toggle_sync();
            }
            if pressed == egui::Key::A {
                self.actual_size();
            }
            if pressed == egui::Key::Plus || pressed == egui::Key::Equals {
                self.zoom_view(1.25);
            }
            if pressed == egui::Key::Minus {
                self.zoom_view(0.8);
            }
            for (slot, key) in [
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
                egui::Key::Num6,
                egui::Key::Num7,
                egui::Key::Num8,
                egui::Key::Num9,
            ]
            .into_iter()
            .enumerate()
            {
                if pressed == key && !modifiers.command && !modifiers.ctrl {
                    let ids = self.comparison_ids();
                    if let Some(id) = ids.get(slot) {
                        self.active = Some(*id);
                        self.focused_slot = slot as u64;
                    }
                }
            }
            if pressed == egui::Key::F2 {
                self.compare_similar();
            }
            if pressed == egui::Key::Escape {
                self.comparison.clear();
                self.fit_view();
            }
            if pressed == egui::Key::Enter && modifiers.shift {
                self.review_changes();
            }
            if !matches!(self.step, Step::EasyDelete | Step::PickBest)
                && (pressed == egui::Key::Delete || pressed == egui::Key::Backspace)
                && let Some(id) = self.active
            {
                self.decisions.entry(id).or_default().rejected = true;
                self.changed();
                self.review_changes();
            }
            for (rating, key) in [
                egui::Key::Num0,
                egui::Key::Num1,
                egui::Key::Num2,
                egui::Key::Num3,
                egui::Key::Num4,
                egui::Key::Num5,
            ]
            .into_iter()
            .enumerate()
            {
                if pressed == key {
                    if modifiers.command || modifiers.ctrl {
                        if let Some(id) = self.active {
                            self.rating(id, rating as u8);
                        }
                    } else if rating == 0 {
                        self.fit_view();
                    }
                }
            }
        }
    }
}
fn theme_install(ctx: &egui::Context) {
    super::theme::install(ctx);
}
fn paint_image(
    ui: &egui::Ui,
    texture: &TextureHandle,
    rect: egui::Rect,
    zoom: f32,
    pan: Vec2,
    turns: u8,
) {
    let dimensions = texture.size_vec2();
    let oriented = if turns % 2 == 1 {
        egui::vec2(dimensions.y, dimensions.x)
    } else {
        dimensions
    };
    let scale = (rect.width() / oriented.x).min(rect.height() / oriented.y) * zoom;
    let size = dimensions * scale;
    let center = rect.center() + pan;
    let angle = f32::from(turns) * std::f32::consts::FRAC_PI_2;
    let rot = egui::emath::Rot2::from_angle(angle);
    let positions = [
        egui::vec2(-0.5, -0.5),
        egui::vec2(0.5, -0.5),
        egui::vec2(0.5, 0.5),
        egui::vec2(-0.5, 0.5),
    ];
    let uvs = [
        egui::pos2(0.0, 0.0),
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
    ];
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (p, uv) in positions.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + rot * (p * size),
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    ui.painter()
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
}

impl PhotoSortApp {
    // Shared frame boundary lets input and native-close ordering be verified
    // without constructing a GPU window or replacing the native delegate.
    fn update_workspace(&mut self, ctx: &egui::Context, native_requested: bool) {
        self.check_updates_at_startup();
        self.tick += 1;
        self.poll(ctx);
        self.refilter();
        self.restore_bookmark();
        self.keyboard(ctx);
        self.sync_playback(ctx);
        self.inspection_wanted.clear();
        let wide = ctx.content_rect().width() > 1180.0;
        if self.was_wide && !wide {
            self.inspector = false;
        }
        self.was_wide = wide;
        let compact = ctx.content_rect().width() < 1000.0;
        self.header(ctx);
        if compact {
            self.compact_steps(ctx);
        } else {
            self.sidebar(ctx);
        }
        self.footer(ctx);
        self.workflow_footer(ctx);
        if self.inspector && self.catalog.is_some() && ctx.content_rect().width() > 1180.0 {
            egui::SidePanel::right("inspector")
                .resizable(true)
                .default_width(245.0)
                .width_range(220.0..=350.0)
                .frame(egui::Frame::new().fill(PANEL).inner_margin(18))
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
                });
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(if compact { 16 } else { 26 }),
            )
            .show(ctx, |ui| {
                if self.root.is_none() {
                    self.welcome(ui);
                } else {
                    // Reduce control padding in short windows so every workflow
                    // leaves useful vertical space for the photographs.
                    if ctx.content_rect().height() < 760.0 {
                        ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);
                    }
                    if self.group_task.is_some() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Updating groups…");
                            if ui.button("Cancel group update").clicked() { self.cancel_group_edit(); }
                        });
                    }
                    if self.busy { ui.disable(); }
                    self.toolbar(ui);
                    self.navigator_panel(ui);
                    if self.step == Step::PickBest && self.pick_run.is_some() {
                        ui.centered_and_justified(|ui| {ui.label("Preparing Pick Best clusters… Use Cancel analysis to stop.");});
                    } else if self.step == Step::EasyDelete && self.delete_run.is_some() {
                        ui.centered_and_justified(|ui| { ui.label("Preparing Easy Delete reviews… Use Cancel analysis to stop."); });
                    } else if self.step == Step::Rotation && self.rotation_run.is_some() {
                        ui.centered_and_justified(|ui| { ui.label("Preparing rotation suggestions… Use Cancel analysis to stop."); });
                    } else if self.media.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.label(if self.scanning {
                                "Finding your photos…"
                            } else {
                                "No supported media found in this folder."
                            });
                        });
                    } else if self.filtered.is_empty() && self.browse_run.is_some() && self.step == Step::Cull {
                        ui.spinner(); ui.label("Updating library view…");
                    } else if self.filtered.is_empty() {
                        ui.label(if self.step == Step::Rotation && self.rotation_queue_only {
                            "No rotation suggestions remain. Turn off Suggestions only to review any photo manually."
                        } else if self.step == Step::PickBest { "No comparable clusters. Analyze the library to find similar photos." } else if self.step == Step::EasyDelete { "No reviews match. Analyze the library or enable more categories." } else { "No photos match these filters." });
                    } else if self.organize_editor.is_some() {
                        // The editor owns the visible content. Do not render or
                        // submit hidden image/detail requests behind its modal.
                    } else if self.review {
                        self.review_canvas(ui);
                    } else {
                        self.library(ui);
                    }
                }
            });
        if self.inspector && self.catalog.is_some() && ctx.content_rect().width() <= 1180.0 {
            let mut open = self.inspector;
            egui::Window::new("Photo details")
                .open(&mut open)
                .default_width(260.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(500.0)
                        .show(ui, |ui| self.inspector(ui));
                });
            self.inspector = open;
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()));
        if let Some(path) = dropped {
            self.open(if path.is_dir() {
                path
            } else {
                path.parent().unwrap_or(&path).to_path_buf()
            });
        }
        // A confirm/rating/edit can arrive with Quit before the next repaint.
        // Evaluate departure only after this frame's keyboard and widget actions.
        let smoke_requested = self.smoke_close_requested(ctx);
        self.handle_close_request(ctx, native_requested || smoke_requested);
        self.dialogs(ctx);
        // Include detail requested by the folder editor's shared preview canvas.
        self.runtime.inspect(self.inspection_wanted.clone());
        self.inspection_frames.retain(|slot, _| {
            self.inspection_wanted
                .iter()
                .any(|request| request.slot == *slot)
        });
        self.sync_playback(ctx);
        if self.view_checkpoint.elapsed() >= Duration::from_secs(1) {
            self.checkpoint_bookmarks();
            self.view_checkpoint = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.close_authorized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
impl eframe::App for PhotoSortApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let native_requested = self
            .native_quit
            .as_ref()
            .is_some_and(|handler| handler.take_requested());
        self.update_workspace(ctx, native_requested);
    }
    fn on_exit(&mut self) {
        self.checkpoint_bookmarks();
        self.runtime.shutdown();
        if !self.close_checkpointed
            && let (Some(root), Some(_)) = (&self.root, &self.catalog)
        {
            let session = self.session_snapshot();
            if let Err(error) = session.save(root) {
                eprintln!("Could not save session: {error}");
            }
        }
    }
}

fn pressed_keys(ctx: &egui::Context, rotation: bool) -> Vec<(egui::Key, egui::Modifiers)> {
    // Preserve key-down modifiers even when key-up arrives before repaint. Consume
    // handled events so an arrow cannot also move a focused zoom slider or button.
    ctx.input_mut(|input| {
        let mut pressed = Vec::new();
        input.events.retain(|event| {
            if let egui::Event::Key {
                key,
                pressed: down,
                modifiers,
                ..
            } = event
            {
                let handled = matches!(
                    key,
                    egui::Key::ArrowLeft
                        | egui::Key::ArrowRight
                        | egui::Key::ArrowUp
                        | egui::Key::ArrowDown
                        | egui::Key::D
                        | egui::Key::C
                        | egui::Key::K
                        | egui::Key::R
                        | egui::Key::Q
                        | egui::Key::E
                        | egui::Key::I
                        | egui::Key::F
                        | egui::Key::S
                        | egui::Key::A
                        | egui::Key::F1
                        | egui::Key::F2
                        | egui::Key::F3
                        | egui::Key::Escape
                        | egui::Key::Delete
                        | egui::Key::Backspace
                        | egui::Key::Plus
                        | egui::Key::Equals
                        | egui::Key::Minus
                        | egui::Key::Num0
                        | egui::Key::Num1
                        | egui::Key::Num2
                        | egui::Key::Num3
                        | egui::Key::Num4
                        | egui::Key::Num5
                        | egui::Key::Num6
                        | egui::Key::Num7
                        | egui::Key::Num8
                        | egui::Key::Num9
                ) || (*key == egui::Key::Enter && (modifiers.shift || rotation));
                if handled {
                    if *down {
                        pressed.push((*key, *modifiers));
                    }
                    return false;
                }
            }
            true
        });
        pressed
    })
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    #[test]
    fn plain_enter_is_owned_by_guided_review_only() {
        for rotation in [false, true] {
            let ctx = egui::Context::default();
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            let _ = ctx.run(input, |ctx| {
                assert_eq!(pressed_keys(ctx, rotation).len(), usize::from(rotation));
                assert_eq!(
                    ctx.input(|i| i.events.iter().any(|e| matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            ..
                        }
                    ))),
                    !rotation
                );
            });
        }
    }
    #[test]
    fn quick_modified_keys_keep_their_own_modifiers_and_order() {
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        for _ in 0..3 {
            input.events.push(egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::SHIFT,
            });
            input.events.push(egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut captured = Vec::new();
        let _ = ctx.run(input, |ctx| {
            captured = pressed_keys(ctx, false);
            assert!(ctx.input(|i| {
                i.events
                    .iter()
                    .all(|event| !matches!(event, egui::Event::Key { .. }))
            }));
        });
        assert_eq!(
            captured,
            vec![(egui::Key::ArrowRight, egui::Modifiers::SHIFT); 3]
        );
    }
}

#[cfg(test)]
mod organize_tests;

#[cfg(test)]
#[path = "folder_open_tests.rs"]
mod folder_open_tests;

impl PhotoSortApp {
    fn accept_image_warning(&mut self, id: MediaId, warning: Option<String>) {
        if let Some(warning) = warning {
            self.image_warnings.insert(id, warning);
        } else {
            self.image_warnings.remove(&id);
        }
    }
}

#[cfg(test)]
#[path = "compact_cull_tests.rs"]
mod compact_cull_tests;

#[cfg(test)]
#[path = "stop_apply_tests.rs"]
mod stop_apply_tests;
