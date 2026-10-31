#[path = "browsing.rs"]
mod browsing;
#[path = "clipboard.rs"]
mod clipboard;
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
#[path = "intro.rs"]
mod intro;
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
#[path = "people.rs"]
mod people;
#[path = "photo_rows.rs"]
mod photo_rows;
#[path = "pick_review.rs"]
mod pick_review;
#[path = "plan_review.rs"]
mod plan_review;
#[path = "pointer.rs"]
mod pointer;
#[path = "review.rs"]
mod review;
#[path = "review_card.rs"]
mod review_card;
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
    /// Whether eframe last reported the window occluded or minimized.
    hidden: bool,
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
    /// The first-run intro, due at startup or replayed from Menu.
    intro: intro::Intro,
    preferences_path: Option<PathBuf>,
    /// The preferences file exists but could not be read; defaults stand in.
    preferences_unreadable: bool,
    preferences_draft: Option<crate::core::preferences::Preferences>,
    preferences_section: Option<preferences::Section>,
    /// Settings is asking before it forgets every remembered folder.
    forget_confirm: bool,
    /// What the last "Forget remembered folders" or "Clear recent folders" did.
    forget_message: Option<String>,
    browse_filters_open: bool,
    about: bool,
    update_dialog: bool,
    update_startup_checked: bool,
    model_dialog: bool,
    model_removal: Option<crate::core::model_assets::ManagedCopy>,
    model_issue: Option<crate::core::model_assets::Issue>,
    model_retry: Option<model_setup::Retry>,
    saved_views: crate::core::views::Saved,
    /// Remembered folders were forgotten while this folder was open. It keeps
    /// its views for this visit but saves none, so it also opens fresh next time.
    views_forgotten: bool,
    /// The library open before the user chose another folder. Its views
    /// must not carry over: the new folder opens with clean views.
    switched_from: Option<PathBuf>,
    /// Every scroll position returns to the top on the next frame.
    forget_scroll: bool,
    pending_bookmark: Option<crate::core::views::Bookmark>,
    view_data_ready: HashSet<Step>,
    resume_started: bool,
    resume_suspended: bool,
    view_checkpoint: Instant,
    library_offset: f32,
    library_restore_offset: Option<f32>,
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
    /// Photos with closed eyes and the other shots of their moment.
    delete_blinks: Vec<crate::core::eyes::Blink>,
    rotations: HashMap<MediaId, crate::core::models::RotationSuggestion>,
    rotation_review: crate::core::rotation::Review,
    rotation_run: Option<u64>,
    rotation_queue_only: bool,
    /// The clicked card's photos, large, in a panel beside Fix Rotation's grid.
    rotation_panel: bool,
    /// The panel's width once the user has seen or resized it.
    rotation_panel_width: Option<f32>,
    delete_review: crate::core::delete_review::Review,
    delete_current: Option<MediaId>,
    delete_categories: HashSet<crate::core::delete_review::Category>,
    delete_run: Option<u64>,
    /// The clicked card's photos, large, in a panel beside Easy Delete's grid.
    delete_panel: bool,
    /// The panel's width once the user has seen or resized it.
    delete_panel_width: Option<f32>,
    pick_review: crate::core::pick_review::Review,
    pick_run: Option<u64>,
    /// A photo of the selected Pick Best group, the one the keyboard acts on.
    pick_current: Option<MediaId>,
    /// The selected group's photos, large, in a panel beside Pick Best's grid.
    pick_panel: bool,
    /// The panel's width once the user has seen or resized it.
    pick_panel_width: Option<f32>,
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
    /// A "photos without people" choice waiting to replace Faces folder edits.
    pending_faceless: Option<crate::core::preferences::Faceless>,
    output: String,
    organize_drafts: HashMap<GroupMode, crate::core::organize::Draft>,
    /// Organize's folder board and a counter bumped by its own edits.
    board: organize_board::Board,
    board_epoch: u64,
    face_groups: Option<crate::core::regional::grouping::Output>,
    mixed_labels: Option<HashMap<MediaId, String>>,
    organize_run: Option<(u64, GroupMode)>,
    /// The automatic grouping whose last run was cancelled or failed; it is not
    /// restarted on its own until the mode is chosen again or the changes
    /// are reviewed.
    organize_halted: Option<GroupMode>,
    /// The halted grouping was cancelled, rather than failed.
    organize_cancelled: bool,
    organize_preview_requested: bool,
    /// Review changes in Organize shows the moves first; once they are
    /// applied, the staged Trash marks, ratings and rotations come next.
    review_after_moves: bool,
    /// The next planned file plan applies without a review: the staged
    /// choices of a review that also covered moves.
    apply_planned: bool,
    file_names: HashMap<MediaId, String>,
    status: String,
    status_scope: Option<(Step, Option<GroupMode>)>,
    error: Option<String>,
    scanning: bool,
    busy: bool,
    analysis_run: Option<u64>,
    progress: Option<(usize, usize)>,
    progress_started: Option<Instant>,
    /// Time this progress run spent preparing models, and when the current
    /// preparation began; excluded from the photo estimate.
    progress_prep: (Duration, Option<Instant>),
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
    /// The 1:1 zoom at which each canvas slot's photo last fitted its view,
    /// so zooming out from the keyboard or slider stops at Fit.
    fit_zooms: HashMap<u64, f32>,
    sync_views: bool,
    focused_slot: u64,
    /// A photo clicked inside a comparison of selected photos: D and Trash
    /// act on it alone. Without one they act on every selected photo.
    comparison_focus: Option<MediaId>,
    copier: crate::core::clipboard::Copier,
    /// A short message over the photos, such as what Cmd/Ctrl+C copied.
    toast: Option<clipboard::Toast>,
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
        app.load_preferences(crate::core::preferences::Preferences::path());
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
            hidden: false,
            smoke_exit: None,
            video_texture: None,
            video_context: None,
            preferences: Default::default(),
            missing_recent: HashSet::new(),
            reduce_motion_pending: None,
            crash_notice: None,
            crash_notice_shown: false,
            intro: intro::Intro::Off,
            preferences_path: None,
            preferences_unreadable: false,
            preferences_draft: None,
            preferences_section: None,
            forget_confirm: false,
            forget_message: None,
            views_forgotten: false,
            browse_filters_open: false,
            about: false,
            update_dialog: false,
            update_startup_checked: false,
            model_dialog: false,
            model_removal: None,
            model_issue: None,
            model_retry: None,
            saved_views: Default::default(),
            switched_from: None,
            forget_scroll: false,
            pending_bookmark: None,
            view_data_ready: [Step::Organize, Step::Cull, Step::Rotation]
                .into_iter()
                .collect(),
            resume_started: false,
            resume_suspended: false,
            view_checkpoint: Instant::now(),
            library_offset: 0.0,
            library_restore_offset: None,
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
            delete_blinks: Vec::new(),
            rotations: HashMap::new(),
            rotation_review: Default::default(),
            rotation_run: None,
            rotation_queue_only: false,
            rotation_panel: false,
            rotation_panel_width: None,
            delete_review: Default::default(),
            delete_current: None,
            delete_categories: crate::core::delete_review::Category::ALL
                .into_iter()
                .collect(),
            delete_run: None,
            delete_panel: false,
            delete_panel_width: None,
            pick_review: Default::default(),
            pick_run: None,
            pick_current: None,
            pick_panel: false,
            pick_panel_width: None,
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
            pending_faceless: None,
            output: String::new(),
            organize_drafts: HashMap::new(),
            board: Default::default(),
            board_epoch: 0,
            face_groups: None,
            mixed_labels: None,
            organize_run: None,
            organize_halted: None,
            organize_cancelled: false,
            organize_preview_requested: false,
            review_after_moves: false,
            apply_planned: false,
            file_names: HashMap::new(),
            status_scope: None,
            status: "Ready when you are".into(),
            error: None,
            scanning: false,
            busy: false,
            analysis_run: None,
            progress: None,
            progress_started: None,
            progress_prep: Default::default(),
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
            fit_zooms: HashMap::new(),
            sync_views: true,
            focused_slot: 0,
            comparison_focus: None,
            copier: Default::default(),
            toast: None,
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
            suggested: self.delete_review.suggested().clone(),
            pick_suggested: self.pick_review.suggested().clone(),
            rotations_staged: self.rotation_review.staged().clone(),
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
        let previous = self
            .catalog
            .as_ref()
            .map(|catalog| catalog.root.clone())
            .or_else(|| self.root.clone());
        let switching = previous
            .as_ref()
            .is_some_and(|root| !root.components().eq(folder.components()));
        self.scan_folder(folder);
        // Another spelling of the same folder (a link, say) is only known to
        // be the same once the scan resolves it; it then gets its views back.
        if switching {
            self.reset_folder_views();
            self.switched_from = previous;
        }
    }
    // Reload after an apply has already queued its reconciled session save.
    // Saving the old catalog again would overwrite migrated fingerprints.
    fn scan_folder(&mut self, folder: PathBuf) {
        self.switched_from = None;
        self.browse_filters_open = false;
        self.clear_model_retry();
        self.runtime.next_epoch();
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
        self.delete_blinks.clear();
        self.rotations.clear();
        self.delete_review = Default::default();
        self.delete_current = None;
        self.delete_run = None;
        self.delete_panel = false;
        self.pick_review = Default::default();
        self.pick_run = None;
        self.pick_current = None;
        self.pick_panel = false;
        self.embeddings.clear();
        self.rotation_review = Default::default();
        self.rotation_run = None;
        self.rotation_queue_only = false;
        self.rotation_panel = false;
        self.active = None;
        self.comparison.clear();
        self.inspection_wanted.clear();
        self.inspection_frames.clear();
        self.textures.clear();
        self.held.clear();
        self.prefetched_for = None;
        self.board.editing = None;
        self.board.focus.clear();
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
        self.pending_faceless = None;
        self.organize_drafts.clear();
        self.organize_departure = None;
        self.mixed_labels = None;
        self.organize_run = None;
        self.organize_halted = None;
        self.organize_preview_requested = false;
        // `review_after_moves` survives: the rescan after applying the moves
        // is what opens the next review, and it consumes the flag.
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
        if self.browse_ready {
            self.runtime
                .browsing
                .decision_batch(ids.into_iter().map(|id| (id, self.decision(id))));
        }
        self.dirty = Some(Instant::now());
        self.filter_dirty = true;
    }
    fn changed(&mut self) {
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
        self.decision(id).turns
    }
    fn reject(&mut self, id: MediaId) {
        if self.step == Step::PickBest {
            if self.pick_review.group_of(id).is_some() {
                self.toggle_pick(id);
            }
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
            } else if self.delete_review.entry(id).is_some() {
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
                // Back to how it is on disk, nothing stays staged.
                let staged = crate::core::rotation::turns(&self.decisions, id);
                crate::core::rotation::set_turns(&mut self.decisions, id, staged + turns);
                self.pending_bookmark = None;
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
            && let Some(cluster) = self.pick_review.cluster_of(id)
        {
            let slot = cluster.ordered.iter().position(|p| *p == id).unwrap_or(0);
            let group = self.pick_group();
            self.library_reveal |= self.active != Some(id);
            self.pick_current = Some(id);
            self.active = Some(id);
            self.focused_slot = slot as u64;
            // Another group starts at fit, like a new photo.
            if group != self.pick_review.group_of(id) {
                self.inspection_view = InspectionView::default();
                self.independent_views.clear();
            }
            self.view_started = Instant::now();
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
            .or_else(|| {
                self.board_selection_comparison()?
                    .iter()
                    .position(|other| *other == id)
            })
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
                .filter(|(k, t)| {
                    !(k.1 && (Some(k.0) == self.active || self.comparison.contains(&k.0)))
                        // On screen in the last frame: evicting it would only
                        // load it again, over and over, when a page of large
                        // previews outgrows the budget.
                        && t.used + 1 < self.tick
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
            // One entry per group: its suggested photo, in capture order.
            self.filtered = self
                .pick_review
                .clusters()
                .iter()
                .filter_map(|c| self.index.get(&c.best()).copied())
                .collect();
            if self
                .pick_current
                .is_some_and(|id| self.pick_review.group_of(id).is_none())
            {
                self.deselect_pick_group();
            }
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
                    // Easy Delete's tabs choose its cards; a Trash filter
                    // would hide each card the moment its suggestion is staged.
                    && (self.step == Step::EasyDelete || self.filter.matches(
                        self.decision(m.id).rejected,
                        crate::core::browsing::rating(m.id, &self.decisions, &self.metadata),
                        self.analysis
                            .get(&m.id)
                            .is_some_and(|a| !a.issues.is_empty()),
                        self.duplicates.contains_key(&m.id)
                            || self.near_duplicates.contains_key(&m.id),
                    ))
            })
            .map(|(i, _)| i)
            .collect();
        if self.step == Step::Organize {
            self.arrange_board();
        }
        if self.step == Step::Rotation {
            self.refilter_rotations();
            self.filter_dirty = false;
            return;
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
            // No card is selected until the user picks one; a card hidden by
            // the tab or search stops being selected.
            if self
                .delete_current
                .is_some_and(|id| !self.filtered.iter().any(|i| self.media[*i].id == id))
            {
                self.deselect_delete_card();
            }
            self.filter_dirty = false;
            return;
        }
        if self
            .active
            .is_none_or(|id| !self.filtered.iter().any(|&i| self.media[i].id == id))
        {
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
        if self.organize_run.is_some() {
            self.halt_organize_run();
            self.organize_cancelled = true;
        }
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
        // Organize's moves, renames and new folders are one plan; staged
        // Trash marks, ratings and rotations follow once it is applied.
        // Nothing changes on disk until each plan is reviewed and applied.
        if self.step == Step::Organize && self.organize_changes_pending() {
            self.review_after_moves = self.decisions.values().any(|d| !d.is_empty());
            self.set_review_plan(None);
            self.organize();
            // Metadata or a grouping comes first: no plan is on its way.
            if !self.busy {
                self.review_after_moves = false;
            }
            return;
        }
        // With nothing staged, a grouping that has not run yet runs first
        // and its plan opens after; staged marks never wait for a grouping.
        if self.step == Step::Organize
            && !self.organize_grouping_ready()
            && !self.organize_drafts.contains_key(&self.group_mode)
            && self.decisions.values().all(|d| d.is_empty())
        {
            self.organize();
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
            GroupMode::Current => self.current_label(media).unwrap_or_else(|| {
                self.relative_parent(media)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default()
            }),
            GroupMode::Date => {
                crate::core::dates::label(media, meta.unwrap_or(&Metadata::default()))
                    .unwrap_or_else(|| "undated".into())
            }
            GroupMode::Camera => meta
                .filter(|m| !m.camera.trim().is_empty())
                .map(|m| crate::core::operations::safe_component(&m.camera))
                .unwrap_or_else(|| "Unknown camera".into()),
            GroupMode::Location => self
                .preferences
                .location_layout
                .label(meta.unwrap_or(&Metadata::default())),
            GroupMode::Faces => self
                .face_groups
                .as_ref()
                .filter(|groups| groups.groups.get(&media.id).is_some_and(|&g| g != 0))
                .and_then(|groups| groups.folders.get(&media.id))
                .cloned()
                .unwrap_or_else(|| {
                    if media.kind == MediaKind::Video {
                        "Video · kept in place".into()
                    } else if self.face_groups.is_some() {
                        "No person found · kept in place".into()
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
    fn relative_parent<'a>(&self, media: &'a Media) -> Option<&'a std::path::Path> {
        let root = &self.catalog.as_ref()?.root;
        media.path.parent()?.strip_prefix(root).ok()
    }
    /// The exact folder label for Current folders, or `None` when the folder
    /// name is not valid Unicode and the file must stay where it is.
    pub(super) fn current_label(&self, media: &Media) -> Option<String> {
        match self.relative_parent(media) {
            Some(parent) => crate::core::names::relative_label(parent),
            None => Some(String::new()),
        }
    }
    pub(super) fn review_trash(&mut self, request: crate::core::operations::TrashRequest) {
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
        self.organize_halted = None;
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
        if self.step == Step::Organize {
            self.board_follow_active();
        }
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
            self.navigate_pick(delta);
            return;
        }
        if self.filtered.is_empty() {
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
            let shown = self.board_shown_order();
            crate::core::navigation::next_visible(
                &self.media,
                shown.as_deref().unwrap_or(&self.filtered),
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
            // A clicked photo hands its focus to the one replacing it.
            if self.comparison_focus == Some(self.comparison[slot]) {
                self.comparison_focus = Some(id);
            }
            self.comparison[slot] = id;
            if self.step == Step::Cull {
                self.selection.replace(self.comparison.iter().copied());
            }
        }
        self.set_active(id);
    }
    fn modal_active(&self) -> bool {
        self.other_modal_active()
    }
    fn other_modal_active(&self) -> bool {
        self.plan_open
            || self.browse_filters_open
            || self.pending_location_layout.is_some()
            || self.pending_faceless.is_some()
            || self.board.confirm_reset
            || self.organize_departure.is_some()
            || self.help
            || self.update_dialog
            || self.about
            || self.model_dialog
            || self.preferences_draft.is_some()
            || self.busy
            || self.departure.is_some()
            || self.intro.playing()
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
        if std::mem::take(&mut self.forget_scroll) {
            ctx.data_mut(|d| d.remove_by_type::<egui::scroll_area::State>());
        }
        self.tick += 1;
        self.poll(ctx);
        self.check_board();
        self.refilter();
        self.restore_bookmark();
        self.intro_keys(ctx);
        self.keyboard(ctx);
        // Organize, Easy Delete, Fix Rotation and Pick Best have no separate
        // Review: they show the clicked photo or card in a panel beside their
        // board or grid.
        if matches!(
            self.step,
            Step::Organize | Step::EasyDelete | Step::Rotation | Step::PickBest
        ) {
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
            .size_range(260.0..=560.0)
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
                        // Organize's, Easy Delete's, Fix Rotation's and Pick Best's
                        // bars have no context line: as much room below as above.
                        .inner_margin(egui::Margin { left: 12, right: 12, top: 6, bottom: if matches!(self.step, Step::Organize | Step::EasyDelete | Step::Rotation | Step::PickBest) { 6 } else { 2 } })
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
                    let margin: i8 = if compact { 8 } else { 12 };
                    // Organize's panels sit as far below the bar as they are
                    // from each other.
                    let top = if self.step == Step::Organize {
                        (organize_board::BOARD_GAP - ui.spacing().item_spacing.y).max(0.0) as i8
                    } else {
                        margin
                    };
                    egui::Frame::new()
                        .inner_margin(egui::Margin { left: margin, right: margin, top, bottom: margin })
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
                            } else {
                                // Organize shows it above its board, between the panels.
                                if self.step != Step::Organize && self.analysis_run.is_some() {
                                    self.analysis_banner(ui);
                                    ui.add_space(S2);
                                }
                                if self.review && self.step != Step::Organize {
                                    self.review_canvas(ui);
                                } else {
                                    self.library(ui);
                                }
                            }
                        });
                }
            });
        if self.inspector && self.catalog.is_some() && ctx.content_rect().width() <= 1180.0 {
            self.inspector_drawer(ctx);
        }
        self.show_toast(ctx);
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
        self.intro_overlay(ctx);
        // Include detail requested by the preview canvases drawn this frame,
        // then a few of Organize's sharp face crops.
        self.request_sharp_faces();
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
    /// Opens a step at once: the seeded suggestions are staged, and
    /// screenshots never stop at the prompt to resolve them.
    pub fn gallery_step(&mut self, step: Step) {
        if self.step != step && self.preferences.visible(step) {
            self.set_step(step);
        }
    }
    /// Seeds Easy Delete, Fix Rotation and Pick Best with synthetic
    /// suggestions from the scanned photos, so screenshots need no models.
    pub fn gallery_seed_reviews(&mut self) {
        use crate::core::delete_review::{Category, Entry};
        // Scenes use the demo photos the ui_gallery example derives from
        // tests/samples, so each shows what it claims: a byte copy is the exact
        // duplicate, a softened copy the near duplicate, a blurred copy the
        // blurry photo, darkened and brightened copies the badly exposed ones,
        // a sideways copy needs rotating, and three frames of one scene form
        // the Pick Best burst.
        // Folders without these files get no scenes.
        let names: HashMap<String, MediaId> = self.media.iter().map(|m| (m.name(), m.id)).collect();
        let id = |name: &str| names.get(name).copied();
        let mut entries = Vec::new();
        if let (Some(copy), Some(original)) = (
            id("20230428_035435_IMG_4666 copy.JPG"),
            id("20230428_035435_IMG_4666.JPG"),
        ) {
            entries.push(Entry {
                subject: copy,
                companion: Some(original),
                category: Category::Exact,
                tag: "Identical copy".into(),
                keep_why: "The two files are identical, so either can go. This one keeps its place in the library.".into(),
                eyes: Vec::new(),
                extra: Vec::new(),
                reason: "Byte-for-byte identical to the photo beside it.".into(),
            });
        }
        if let (Some(copy), Some(original)) = (id("GAB00915 (2).jpg"), id("GAB00915.jpg")) {
            entries.push(Entry {
                subject: copy,
                companion: Some(original),
                category: Category::Near,
                tag: "Sharper kept".into(),
                keep_why: "It is the sharper photo of the two.".into(),
                eyes: Vec::new(),
                extra: Vec::new(),
                reason: "No visible subject change detected. PhotoSort suggests keeping the sharper photo.".into(),
            });
        }
        if let Some(blurry) = id("GAB02000-2.jpg") {
            entries.push(Entry {
                subject: blurry,
                companion: None,
                category: Category::Blur,
                tag: Category::Blur.title().into(),
                keep_why: String::new(),
                eyes: Vec::new(),
                extra: Vec::new(),
                reason: "Out of focus across the frame. PhotoSort suggests moving it to Trash."
                    .into(),
            });
        }
        for (name, category, reason) in [
            (
                "DSC01310.jpg",
                Category::Dark,
                "Very dark: sharpness 3.2, brightness 21.4/255. PhotoSort suggests moving it to Trash.",
            ),
            (
                "DSC01320.jpg",
                Category::Bright,
                "Overexposed: sharpness 6.8, brightness 238.9/255. PhotoSort suggests moving it to Trash.",
            ),
        ] {
            if let Some(subject) = id(name) {
                entries.push(Entry {
                    subject,
                    companion: None,
                    category,
                    tag: category.title().into(),
                    keep_why: String::new(),
                    eyes: Vec::new(),
                    extra: Vec::new(),
                    reason: reason.into(),
                });
            }
        }
        if !entries.is_empty() {
            self.delete_review.replace(entries);
            self.delete_review
                .stage_new_suggestions(&mut self.decisions);
            self.view_data_ready.insert(Step::EasyDelete);
            self.delete_categories = Category::ALL.into_iter().collect();
        }
        if let Some(sideways) = id("DSC01180.jpg") {
            self.rotations.insert(
                sideways,
                crate::core::models::RotationSuggestion {
                    turns: 1,
                    confidence: 0.94,
                    uncertain: false,
                    views: 3,
                },
            );
            self.finish_rotation_detection();
        }
        // DSC01231 is the sharp frame, suggested over the slightly soft
        // DSC01232 and DSC01233.
        let burst: Vec<(MediaId, f64)> = [
            ("DSC01232.jpg", 6.1),
            ("DSC01231.jpg", 8.4),
            ("DSC01233.jpg", 5.2),
        ]
        .into_iter()
        .filter_map(|(name, score)| id(name).map(|id| (id, score)))
        .collect();
        if burst.len() >= 2 {
            let cluster = crate::core::pick_review::Cluster::new(
                1,
                burst.iter().map(|(id, _)| *id).collect(),
                burst.into_iter().collect(),
            );
            self.pick_review.replace(vec![cluster]);
            self.pick_review.stage_new_suggestions(&mut self.decisions);
            self.view_data_ready.insert(Step::PickBest);
        }
        self.filter_dirty = true;
    }
    /// Closes the Local AI models dialog. Builds without the native runtime
    /// open it whenever a step would analyze.
    pub fn gallery_close_model_dialog(&mut self) {
        self.model_dialog = false;
    }
    /// Shows or hides file names under thumbnails (preference, not saved).
    pub fn gallery_show_filenames(&mut self, show: bool) {
        self.preferences.show_filenames = show;
    }
    /// Shows an analysis about a third done, as if started 40 seconds ago:
    /// a visual-similarity grouping in Organize, Analyze library elsewhere.
    /// No worker runs, so the scene must not wait for it to settle.
    pub fn gallery_running_analysis(&mut self) {
        let run = u64::MAX;
        if self.step == Step::Organize {
            self.group_mode = GroupMode::Visual;
            self.visual_groups = None;
            self.organize_run = Some((run, GroupMode::Visual));
        } else {
            self.cull_group_run = Some(run);
        }
        let total = self.media.len().max(1);
        self.analysis_run = Some(run);
        self.progress = Some((total / 3, total));
        self.progress_started = Instant::now().checked_sub(Duration::from_secs(40));
        self.set_status("Analyzing locally · previews stay available".into());
        self.filter_dirty = true;
    }
    /// Shows Pick Best's analysis while the aesthetic model compiles, 72 s
    /// into a step that took 115 s last time, with the similarity model ready.
    pub fn gallery_preparing_model(&mut self) {
        use crate::core::processing::{Backend, ModelStatus, Prep, PrepStep};
        self.gallery_running_analysis();
        self.cull_group_run = None;
        self.pick_run = self.analysis_run;
        self.progress = Some((0, self.media.len().max(1)));
        let now = Instant::now();
        let ago = |seconds| now.checked_sub(Duration::from_secs(seconds)).unwrap_or(now);
        self.progress_started = Some(ago(80));
        self.progress_prep = (Duration::ZERO, Some(ago(74)));
        for (model, prep) in [
            ("dinov2", None),
            (
                "aesthetic",
                Some(Prep {
                    step: PrepStep::Compiling,
                    started: ago(74),
                    step_started: ago(72),
                    usual: Some(Duration::from_secs(115)),
                }),
            ),
        ] {
            self.runtime.processing.show_status(ModelStatus {
                model,
                backend: Backend::CoreMl,
                detail: String::new(),
                preparing: prep.is_some(),
                prep,
            });
        }
    }
    /// Groups Organize by faces with a synthetic result, so screenshots need
    /// no models: every fourth photo shows one person, the next another, and
    /// the rest have no recognized person.
    pub fn gallery_organize_by_faces(&mut self) {
        let mut output = crate::core::regional::grouping::Output {
            groups: HashMap::new(),
            folders: HashMap::new(),
        };
        let photos = self.media.iter().filter(|m| m.kind == MediaKind::Photo);
        for (n, media) in photos.enumerate() {
            let (group, folder) = match n % 4 {
                0 => (1, "Person 001"),
                1 => (2, "Person 002"),
                _ => (0, "Unassigned"),
            };
            output.groups.insert(media.id, group);
            output.folders.insert(media.id, folder.into());
        }
        self.face_groups = Some(output);
        self.set_group_mode(GroupMode::Faces);
        self.filter_dirty = true;
    }
    /// Gives each person in the synthetic faces grouping a face: the middle
    /// of one of their photos, so screenshots need no face models.
    pub fn gallery_face_portraits(&mut self) {
        let Some(groups) = &self.face_groups else {
            return;
        };
        for (id, group) in &groups.groups {
            if let Some(label) = groups.folders.get(id).filter(|_| *group != 0)
                && let Some([person]) = people::people_in(label).as_deref()
            {
                self.board
                    .people
                    .portraits
                    .entry(person.clone())
                    .or_insert_with(|| {
                        vec![people::Portrait {
                            id: *id,
                            bbox: [0.35, 0.25, 0.65, 0.55],
                        }]
                    });
            }
        }
    }
    /// Groups Organize by the current folders.
    pub fn gallery_organize_by_folders(&mut self) {
        self.set_group_mode(GroupMode::Current);
    }
    /// Makes a new folder holding the first two photos and starts naming it.
    pub fn gallery_organize_new_folder(&mut self) {
        let ids: Vec<_> = self
            .filtered
            .iter()
            .take(2)
            .map(|&i| self.media[i].id)
            .collect();
        self.selection.replace(ids.iter().copied());
        self.board_new_folder("", &ids);
    }
    /// Closes the review, then gathers one photo from each of the first
    /// `count` folders of the board into a new folder and reviews again.
    pub fn gallery_organize_gather(&mut self, count: usize) {
        self.plan_open = false;
        let mut seen = HashSet::new();
        let ids: Vec<_> = self
            .filtered
            .iter()
            .map(|&i| &self.media[i])
            .filter(|m| !self.decision(m.id).rejected && seen.insert(self.label_for(m)))
            .take(count)
            .map(|m| m.id)
            .collect();
        self.board_new_folder("", &ids);
        self.board.editing = None;
        self.review_changes();
    }
    /// Groups Organize by capture date.
    pub fn gallery_organize_by_date(&mut self) {
        self.set_group_mode(GroupMode::Date);
    }
    /// Shows Review's photo strip on the left (preference, not saved).
    pub fn gallery_left_strip(&mut self, left: bool) {
        self.preferences.strip_position = if left {
            crate::core::preferences::StripPosition::Left
        } else {
            crate::core::preferences::StripPosition::Bottom
        };
    }
    /// Review, or in Easy Delete the comparison panel beside the grid.
    pub fn gallery_review(&mut self, review: bool) {
        if self.step == Step::EasyDelete {
            // Easy Delete opens with no card selected: show the first one.
            if review && self.delete_current.is_none() {
                self.refilter();
                if let Some(first) = self.filtered.first().map(|i| self.media[*i].id) {
                    self.set_active(first);
                }
            }
            self.delete_panel = review;
        } else if self.step == Step::Rotation {
            if review && self.active.is_none() {
                self.refilter();
                if let Some(first) = self.filtered.first().map(|i| self.media[*i].id) {
                    self.set_active(first);
                }
            }
            self.rotation_panel = review;
        } else if self.step == Step::PickBest {
            // So does Pick Best.
            if review && self.pick_current.is_none() {
                self.refilter();
                if let Some(first) = self.filtered.first().map(|i| self.media[*i].id) {
                    self.set_active(first);
                }
            }
            self.pick_panel = review;
        } else {
            self.review = review;
        }
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
    /// Selects the first `count` shown photos, as Shift+arrows would.
    pub fn gallery_select(&mut self, count: usize) {
        let ids: Vec<_> = self
            .filtered
            .iter()
            .take(count)
            .map(|i| self.media[*i].id)
            .collect();
        for (n, id) in ids.into_iter().enumerate() {
            self.select_library(id, false, n > 0);
        }
    }
    pub fn gallery_about(&mut self) {
        self.about = true;
    }
    /// Plays the intro, as Menu → Watch the intro does.
    pub fn gallery_intro(&mut self) {
        self.watch_intro();
    }
    /// True once the intro's card shows a frame of the film.
    pub fn gallery_intro_ready(&self) -> bool {
        matches!(&self.intro, intro::Intro::Playing(film) if film.ready())
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

impl PhotoSortApp {
    fn take_native_quit(&self) -> bool {
        self.native_quit
            .as_ref()
            .is_some_and(|handler| handler.take_requested())
    }
    /// The frame work that must continue while eframe skips `ui` because the
    /// window is occluded or minimized: worker results, the smoke-test
    /// deadline and Quit. Without it, Quit waits until the window is shown.
    fn update_hidden(&mut self, ctx: &egui::Context, native_requested: bool) {
        self.poll(ctx);
        let requested = self.smoke_close_requested(ctx) || native_requested;
        // A minimized window can still be closed, e.g. from the Windows taskbar.
        let window_requested = ctx.input(|i| i.viewport().close_requested());
        self.handle_close_request(ctx, requested);
        if (requested || window_requested) && !self.close_authorized {
            // A departure prompt or busy state cannot be seen in a hidden window.
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

/// eframe runs no egui pass for an occluded or minimized window; it calls
/// only [`eframe::App::logic`], so `ui` work stops until the window is shown.
fn window_hidden(ctx: &egui::Context) -> bool {
    ctx.input(|input| input.viewport().visible()) == Some(false)
}

impl eframe::App for PhotoSortApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let hidden = window_hidden(ctx);
        if hidden != self.hidden {
            self.hidden = hidden;
            tracing::debug!(hidden, "window visibility changed");
        }
        // A shown window does all of this in `ui`, after the frame's input.
        if hidden {
            let native_requested = self.take_native_quit();
            self.update_hidden(ctx, native_requested);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let native_requested = self.take_native_quit();
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

#[cfg(test)]
#[path = "folder_switch_tests.rs"]
mod folder_switch_tests;

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
