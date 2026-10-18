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
#[path = "events.rs"]
mod events;
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
#[path = "organize_board.rs"]
mod organize_board;
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
#[path = "states.rs"]
mod states;
#[path = "status.rs"]
mod status;
#[path = "tree_controls.rs"]
mod tree_controls;
#[path = "video.rs"]
mod video;
#[cfg(test)]
use super::test_frame::TestFrame;
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
    /// Recent folders that were missing at startup (e.g. an unplugged drive):
    /// hidden from the menu but kept, so they return once available again.
    missing_recent: HashSet<PathBuf>,
    /// A saved "Reduce motion" choice to apply to the context next frame.
    reduce_motion_pending: Option<bool>,
    /// A crash report to tell the user about, and whether it is from a
    /// previous session (true) or a background panic in this one (false).
    crash_notice: Option<(PathBuf, bool)>,
    crash_notice_shown: bool,
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
    /// The highest-scoring photo of each Cull group with two or more photos.
    group_best: HashSet<MediaId>,
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
    /// The comparison set aside by F1, restored by pressing F1 again.
    single_view_stash: Vec<MediaId>,
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
    /// Photos the current or last analysis could not process, with the cause.
    analysis_failures: HashMap<MediaId, String>,
    analysis_successes: usize,
    review: bool,
    inspector: bool,
    was_wide: bool,
    companions: bool,
    group_mode: GroupMode,
    pending_location_layout: Option<crate::core::geography::Layout>,
    output: String,
    organize_drafts: HashMap<GroupMode, crate::core::organize::Draft>,
    /// Organize's folder board and a counter bumped by its own edits.
    board: organize_board::Board,
    board_epoch: u64,
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
    progress_started: Option<Instant>,
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
    /// The last image each canvas slot showed, kept on screen briefly while
    /// the next photo's pixels load so stepping through photos never flashes
    /// an empty canvas.
    held: HashMap<u64, review::Held>,
    /// The photo whose neighbours' previews were last requested ahead.
    prefetched_for: Option<MediaId>,
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
            Ok(preferences) => {
                app.companions = preferences.keep_companions;
                app.preferences = preferences;
            }
            Err(e) => app.error = Some(format!("Preferences could not be loaded: {e}")),
        }
        app.crash_notice = crate::core::crash::unseen_report(&crate::core::crash::directory())
            .map(|path| (path, true));
        app.crash_notice_shown = app.crash_notice.is_some();
        app.missing_recent = app
            .preferences
            .recent_folders
            .iter()
            .filter(|p| !p.is_dir())
            .cloned()
            .collect();
        app.runtime.cache.configure(app.preferences.cache);
        app.runtime.processing.configure(app.preferences.processing);
        app.runtime.raw_look.set(app.preferences.raw_look);
        set_reduce_motion(&cc.egui_ctx, app.preferences.reduce_motion);
        app.runtime.cache.request(false);
        app.preferences_path = Some(path);
        if let Some(folder) = folder {
            app.open(folder);
        }
        // Without the native hook, Quit still closes through the window and the
        // shared departure guard; nothing here is actionable for the user.
        match super::platform::QuitHandler::install(&cc.egui_ctx) {
            Ok(handler) => app.native_quit = handler,
            Err(error) => tracing::warn!(%error, "native quit handler unavailable"),
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
            missing_recent: HashSet::new(),
            reduce_motion_pending: None,
            crash_notice: None,
            crash_notice_shown: false,
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
            group_best: HashSet::new(),
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
            single_view_stash: Vec::new(),
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
            analysis_failures: HashMap::new(),
            analysis_successes: 0,
            review: false,
            inspector: true,
            was_wide: true,
            companions: true,
            group_mode: GroupMode::Current,
            pending_location_layout: None,
            output: String::new(),
            organize_drafts: HashMap::new(),
            board: Default::default(),
            board_epoch: 0,
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
            progress_started: None,
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
            held: HashMap::new(),
            prefetched_for: None,
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
        self.group_best.clear();
        self.analysis_failures.clear();
        self.single_view_stash.clear();
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
        self.held.clear();
        self.prefetched_for = None;
        self.board.editing = None;
        self.board.focus = None;
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
            if self.handle_event(ctx, envelope.event).is_break() {
                return;
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
        // Keep few thumbnails queued: tiles ask again every frame while they
        // are visible, so after a fast scroll the photos on screen are next
        // in line instead of waiting behind ones already scrolled past.
        if !preview && self.requested.iter().filter(|(_, p)| !p).count() >= THUMBNAILS_IN_FLIGHT {
            return None;
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
        if self.step == Step::Organize {
            self.arrange_board();
        }
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
                self.analysis_failures.clear();
                self.analysis_successes = 0;
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
        if self.library_selection_mode() && (self.selection_ready() || self.scene_selection_ready())
        {
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
}
/// Thumbnail decodes queued at once; see [`PhotoSortApp::texture`].
pub(super) const THUMBNAILS_IN_FLIGHT: usize = 8;
fn theme_install(ctx: &egui::Context) {
    super::theme::install(ctx);
    // Headless frames carry no real time; finish every animation at once so
    // tests never measure a layout mid-slide.
    ctx.global_style_mut(|style| style.animation_time = 0.0);
}
fn paint_image(
    ui: &egui::Ui,
    texture: &TextureHandle,
    rect: egui::Rect,
    zoom: f32,
    pan: Vec2,
    turns: u8,
) {
    paint_image_tinted(ui, texture, rect, zoom, pan, turns, Color32::WHITE);
}

/// [`paint_image`] multiplied by `tint`; a translucent white fades it in.
/// Returns the area the image covers inside `rect`.
fn paint_image_tinted(
    ui: &egui::Ui,
    texture: &TextureHandle,
    rect: egui::Rect,
    zoom: f32,
    pan: Vec2,
    turns: u8,
    tint: Color32,
) -> egui::Rect {
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
            color: tint,
        });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    ui.painter()
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
    egui::Rect::from_center_size(center, oriented * scale).intersect(rect)
}

impl PhotoSortApp {
    // Shared frame boundary lets input and native-close ordering be verified
    // without constructing a GPU window or replacing the native delegate.
    fn update_workspace(&mut self, root: &mut egui::Ui, native_requested: bool) {
        let ctx = &root.ctx().clone();
        if let Some(reduce) = self.reduce_motion_pending.take() {
            set_reduce_motion(ctx, reduce);
        }
        self.check_updates_at_startup();
        self.tick += 1;
        self.poll(ctx);
        self.check_board();
        self.refilter();
        self.restore_bookmark();
        self.keyboard(ctx);
        // Pick Best is always a side-by-side review: it has no grid view, and
        // its choices and Confirm live under the photos. Older saved views
        // may still ask for the grid.
        if self.step == Step::PickBest {
            self.review = true;
        }
        // Organize has no separate Review: its board shows the clicked
        // photo in a preview panel beside the folders and thumbnails.
        if self.step == Step::Organize {
            self.review = false;
        }
        self.sync_playback(ctx);
        self.inspection_wanted.clear();
        let wide = ctx.content_rect().width() > 1180.0;
        if self.was_wide && !wide {
            self.inspector = false;
        }
        self.was_wide = wide;
        let compact = ctx.content_rect().width() < 1000.0;
        self.track_progress_clock();
        self.header(root);
        self.footer(root);
        // Details slides in and out beside the photos in wide windows;
        // narrow windows use the drawer instead.
        let wide = ctx.content_rect().width() > 1180.0;
        let shown = self.inspector && self.catalog.is_some() && wide;
        let mut expanded = shown;
        egui::Panel::right("inspector")
            .show_separator_line(false)
            .resizable(true)
            .drag_to_open(false)
            .default_size(300.0)
            .size_range(260.0..=440.0)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(S4 as i8))
            .show_collapsible(root, &mut expanded, |ui| {
                // While sliding out after the folder closed or the window
                // narrowed (the drawer then shows Details), the panel stays empty.
                if self.catalog.is_some() && wide {
                    egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
                }
            });
        // Dragging the edge past its minimum closes Details.
        if shown && !expanded {
            self.inspector = false;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG))
            .show(root, |ui| {
                if self.root.is_none() {
                    egui::Frame::new()
                        .inner_margin(if compact { 12 } else { 16 })
                        .show(ui, |ui| self.welcome(ui));
                } else {
                    // Reduce control padding in short windows so every workflow
                    // leaves useful vertical space for the photographs.
                    if ctx.content_rect().height() < 760.0 {
                        ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 5.0);
                    }
                    if self.busy { ui.disable(); }
                    // The step's controls sit on a panel strip above the photos.
                    egui::Frame::new()
                        .fill(PANEL)
                        .inner_margin(egui::Margin { left: 12, right: 12, top: 6, bottom: 2 })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            if self.group_task.is_some() {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Updating groups…");
                                    if ui.button("Cancel group update").clicked() { self.cancel_group_edit(); }
                                });
                            }
                            self.toolbar(ui);
                        });
                    egui::Frame::new()
                        .inner_margin(if compact { 8 } else { 12 })
                        .show(ui, |ui| {
                            ui.set_min_size(ui.available_size());
                            self.navigator_panel(ui);
                            if self.step == Step::PickBest && self.pick_run.is_some() {
                                self.working_state(ui, "Finding similar photos", "Grouping bursts and near-identical frames, then scoring each photo.");
                            } else if self.step == Step::EasyDelete && self.delete_run.is_some() {
                                self.working_state(ui, "Looking for photos to clean up", "Checking for duplicates, near-duplicates, and blurry or badly exposed shots.");
                            } else if self.step == Step::Rotation && self.rotation_run.is_some() {
                                self.working_state(ui, "Checking orientation", "Looking at each photo to find the ones that are sideways or upside down.");
                            } else if self.media.is_empty() && self.scanning {
                                self.working_state(ui, "Finding your photos", "Reading the folder and its subfolders.");
                            } else if self.filtered.is_empty() && self.browse_run.is_some() && self.step == Step::Cull {
                                self.working_state(ui, "Updating the view", "Applying the search and filters.");
                            } else if self.media.is_empty() || self.filtered.is_empty() {
                                self.empty_state(ui);
                            } else if self.organize_editor.is_some() {
                                // The editor owns the visible content. Do not render or
                                // submit hidden image/detail requests behind its modal.
                            } else if self.review && self.step != Step::Organize {
                                self.review_canvas(ui);
                            } else {
                                self.library(ui);
                            }
                        });
                }
            });
        if self.inspector && self.catalog.is_some() && ctx.content_rect().width() <= 1180.0 {
            self.inspector_drawer(ctx);
        }
        self.photo_drag_overlay(ctx);
        let dropped = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .find(|path| path.is_absolute())
        });
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
/// Hooks for the headless screenshot gallery (`examples/ui_gallery.rs`); not
/// part of the application API.
#[doc(hidden)]
impl PhotoSortApp {
    pub fn gallery_step(&mut self, step: Step) {
        self.switch_step(step);
    }
    /// Seeds Easy Delete, Fix Rotation and Pick Best with synthetic
    /// suggestions from the scanned photos, so screenshots need no models.
    pub fn gallery_seed_reviews(&mut self) {
        use crate::core::delete_review::{Category, Entry};
        let ids: Vec<MediaId> = self.media.iter().map(|m| m.id).collect();
        if ids.len() < 10 {
            return;
        }
        let entries = vec![
            Entry {
                subject: ids[1],
                companion: Some(ids[2]),
                category: Category::Near,
                reason: "Near-duplicate of the photo beside it · slightly softer".into(),
            },
            Entry {
                subject: ids[4],
                companion: None,
                category: Category::Blur,
                reason: "Blurry · motion blur across the frame".into(),
            },
        ];
        self.delete_review.replace(entries);
        self.view_data_ready.insert(Step::EasyDelete);
        self.delete_categories = Category::ALL.into_iter().collect();
        for (n, id) in ids.iter().take(3).enumerate() {
            self.rotations.insert(
                *id,
                crate::core::models::RotationSuggestion {
                    turns: 1,
                    confidence: 0.9,
                    uncertain: n == 2,
                    views: 3,
                },
            );
        }
        self.finish_rotation_detection();
        let cluster = |key: usize, members: &[MediaId]| crate::core::pick_review::Cluster {
            key,
            ordered: members.to_vec(),
            scores: members
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, 5.0 + ((i * 7) % 5) as f64 * 0.6))
                .collect(),
        };
        self.pick_review.replace(
            vec![cluster(1, &ids[0..5]), cluster(2, &ids[7..10])],
            &mut self.decisions,
        );
        self.view_data_ready.insert(Step::PickBest);
        self.filter_dirty = true;
    }
    /// Closes the Local AI models dialog. Builds without the native runtime
    /// open it whenever a step would analyze.
    pub fn gallery_close_model_dialog(&mut self) {
        self.model_dialog = false;
    }
    /// Shows or hides Cull's file names (preference, not saved).
    pub fn gallery_show_filenames(&mut self, show: bool) {
        self.preferences.show_filenames = show;
    }
    /// Groups Organize by capture date.
    pub fn gallery_organize_by_date(&mut self) {
        self.set_group_mode(GroupMode::Date);
    }
    /// Shows Cull's photo strip on the left (preference, not saved).
    pub fn gallery_left_strip(&mut self, left: bool) {
        self.preferences.cull_strip = if left {
            crate::core::preferences::StripPosition::Left
        } else {
            crate::core::preferences::StripPosition::Bottom
        };
    }
    pub fn gallery_review(&mut self, review: bool) {
        self.review = review;
    }
    pub fn gallery_organize_editor(&mut self) {
        self.open_organize_editor();
    }
    /// Compares the first `count` shown photos.
    pub fn gallery_compare(&mut self, count: usize) {
        self.comparison = self
            .filtered
            .iter()
            .take(count)
            .map(|i| self.media[*i].id)
            .collect();
        if let Some(first) = self.comparison.first().copied() {
            self.set_active(first);
        }
        self.review = true;
    }
    pub fn gallery_about(&mut self) {
        self.about = true;
    }
    /// Opens Menu → AI models.
    pub fn gallery_models(&mut self) {
        self.open_model_setup();
    }
    pub fn gallery_settings(&mut self) {
        self.preferences_draft = Some(self.preferences.clone());
    }
    /// Marks the first `count` photos for Trash and opens the plan review.
    /// Planning is a dry run; nothing is applied without the Apply button.
    pub fn gallery_plan_review(&mut self, count: usize) {
        let ids: Vec<_> = self.media.iter().take(count).map(|m| m.id).collect();
        for id in ids {
            self.decisions.entry(id).or_default().rejected = true;
        }
        self.changed();
        self.review_changes();
    }
    /// Thumbnails and previews currently uploaded as textures.
    pub fn gallery_textures(&self) -> usize {
        self.textures.len()
    }
    /// True while discovery, metadata, thumbnails or analysis are still arriving.
    pub fn gallery_busy(&self) -> bool {
        self.scanning
            || self.busy
            || !self.requested.is_empty()
            || self.metadata_run.is_some()
            || self.analysis_run.is_some()
    }
}

impl eframe::App for PhotoSortApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let native_requested = self
            .native_quit
            .as_ref()
            .is_some_and(|handler| handler.take_requested());
        self.update_workspace(ui, native_requested);
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
            theme_install(&ctx);
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            let _ = ctx.run_test(input, |ui| {
                let ctx = &ui.ctx().clone();
                {
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
                }
            });
        }
    }
    #[test]
    fn quick_modified_keys_keep_their_own_modifiers_and_order() {
        let ctx = egui::Context::default();
        theme_install(&ctx);
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
        let _ = ctx.run_test(input, |ui| {
            let ctx = &ui.ctx().clone();
            {
                captured = pressed_keys(ctx, false);
                assert!(ctx.input(|i| {
                    i.events
                        .iter()
                        .all(|event| !matches!(event, egui::Event::Key { .. }))
                }));
            }
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
