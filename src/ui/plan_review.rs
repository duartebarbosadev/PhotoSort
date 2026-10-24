//! Shared presentation of immutable operation plans. Rendering never rescans files.
use super::*;
use crate::core::operations::Action;
use egui::text::{LayoutJob, TextFormat};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct Rows {
    // One cumulative offset per operation group, not a second copy of file paths.
    ends: Vec<usize>,
    movable: Vec<usize>,
    generation: u64,
    summary: Summary,
    /// The plain view: photos grouped by what happens to them, as plan indices.
    lines: Vec<Line>,
    /// Folders photos go into, in the order the plain view shows them.
    folders: Vec<Folder>,
}

/// One line of the plain view. Every line has the same height so the list
/// stays virtualized for any plan size; a heading line starts a card.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Line {
    /// The card for `folders[i]`: photos going there, or a new empty folder.
    Into(usize),
    /// A photo, with its companions, moved or renamed (plan group).
    Moved(usize),
    FoldersRenamed,
    FolderRenamed(usize),
    Updates,
    Updated(usize),
    Trash,
    Trashed(usize),
}
impl Line {
    fn heading(self) -> bool {
        matches!(
            self,
            Self::Into(_) | Self::FoldersRenamed | Self::Updates | Self::Trash
        )
    }
}

struct Folder {
    path: PathBuf,
    new: bool,
    photos: usize,
    /// False when every photo here keeps its folder and only gets a new name.
    arrivals: bool,
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
fn photos(n: usize) -> String {
    count(n, "photo", "photos")
}

/// Plain counts for the plan headline: photos, not file operations.
#[derive(Default)]
struct Summary {
    moved: usize,
    renamed: usize,
    destinations: usize,
    new_destinations: usize,
    empty_folders: usize,
    trashed: usize,
    trashed_folders: usize,
    updated: usize,
    rotated: usize,
    rated: usize,
    folder_renames: usize,
    /// Folders removed afterwards, only if nothing is left in them (plan groups).
    cleanup: Vec<usize>,
}
impl Summary {
    fn parts(&self) -> Vec<(String, Color32)> {
        let mut parts = Vec::new();
        if self.moved > 0 {
            let into = if self.new_destinations == self.destinations {
                count(self.destinations, "new folder", "new folders")
            } else if self.new_destinations > 0 {
                format!(
                    "{} ({} new)",
                    count(self.destinations, "folder", "folders"),
                    self.new_destinations
                )
            } else {
                count(self.destinations, "folder", "folders")
            };
            parts.push((format!("Move {} into {into}", photos(self.moved)), ACCENT));
        }
        if self.renamed > 0 {
            parts.push((format!("Rename {}", photos(self.renamed)), INK));
        }
        if self.folder_renames > 0 {
            parts.push((
                format!("Rename {}", count(self.folder_renames, "folder", "folders")),
                INK,
            ));
        }
        if self.empty_folders > 0 {
            parts.push((
                format!(
                    "Create {}",
                    count(self.empty_folders, "empty folder", "empty folders")
                ),
                INK,
            ));
        }
        if self.updated > 0 {
            parts.push((self.updates(), INK));
        }
        if self.trashed > 0 {
            parts.push((format!("Move {} to the Trash", photos(self.trashed)), RED));
        }
        if self.trashed_folders > 0 {
            parts.push((
                format!(
                    "Move {} to the Trash",
                    count(self.trashed_folders, "folder", "folders")
                ),
                RED,
            ));
        }
        parts
    }
    fn updates(&self) -> String {
        let n = photos(self.updated);
        if self.rotated == self.updated && self.rated == 0 {
            format!("Rotate {n}")
        } else if self.rated == self.updated && self.rotated == 0 {
            format!("Set ratings on {n}")
        } else {
            format!("Update {n}")
        }
    }
    /// What Apply applies, counted the way the headline counts.
    fn changes(&self) -> usize {
        self.moved
            + self.renamed
            + self.folder_renames
            + self.empty_folders
            + self.updated
            + self.trashed
            + self.trashed_folders
    }
}
impl Rows {
    fn replace(&mut self, plan: Option<&Plan>) {
        self.generation = self.generation.wrapping_add(1);
        self.ends.clear();
        self.movable.clear();
        self.lines.clear();
        self.folders.clear();
        self.summary = Summary::default();
        let Some(plan) = plan else {
            return;
        };
        let summary = &mut self.summary;
        // Destination folder -> (photos arrive, plan groups).
        let mut into = BTreeMap::<&Path, (bool, Vec<usize>)>::new();
        let (mut created, mut renamed, mut updates, mut trash) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut end = 0;
        for (group, action) in plan.actions.iter().enumerate() {
            if matches!(action, Action::Move { files } if !files.is_empty()) {
                self.movable.push(group);
            }
            let pairs: Vec<(&Path, &Path)> = match action {
                Action::Move { files } | Action::FinishMove { files } => files
                    .iter()
                    .map(|(s, to)| (s.path.as_path(), to.as_path()))
                    .collect(),
                Action::RenameFile { source, to } => vec![(source.path.as_path(), to.as_path())],
                _ => Vec::new(),
            };
            if let Some(folder) = pairs.first().and_then(|(_, to)| to.parent()) {
                let arrives = pairs.iter().any(|(s, to)| s.parent() != to.parent());
                let entry = into.entry(folder).or_default();
                entry.0 |= arrives;
                entry.1.push(group);
                if arrives {
                    summary.moved += 1;
                } else {
                    summary.renamed += 1;
                }
            }
            match action {
                Action::Trash { files } if !files.is_empty() => {
                    summary.trashed += 1;
                    trash.push(group);
                }
                Action::TrashDirectory(_) => {
                    summary.trashed_folders += 1;
                    trash.push(group);
                }
                Action::Metadata {
                    rating,
                    orientation,
                    ..
                } => {
                    summary.updated += 1;
                    summary.rated += usize::from(rating.is_some());
                    summary.rotated += usize::from(orientation.is_some());
                    updates.push(group);
                }
                Action::RenameDirectory(_) => {
                    summary.folder_renames += 1;
                    renamed.push(group);
                }
                Action::CreateDirectory { path } => created.push(path.as_path()),
                Action::RemoveEmptyDirectory { .. } => summary.cleanup.push(group),
                _ => {}
            }
            end += match action {
                Action::Move { files } | Action::FinishMove { files } => files.len().max(1),
                Action::Trash { files } => files.len().max(1),
                Action::TrashDirectory(folder) => 1 + folder.inventory.files.len(),
                _ => 1,
            };
            self.ends.push(end);
        }
        // A new folder that only holds other new folders is shown through them.
        let empty: Vec<&Path> = created
            .iter()
            .copied()
            .filter(|path| {
                !into.keys().any(|folder| folder.starts_with(path))
                    && !created.iter().any(|c| c != path && c.starts_with(path))
            })
            .collect();
        summary.empty_folders = empty.len();
        for path in empty {
            into.insert(path, (false, Vec::new()));
        }
        for (path, (arrivals, groups)) in into {
            let new = created.contains(&path);
            if arrivals {
                summary.destinations += 1;
                summary.new_destinations += usize::from(new);
            }
            self.lines.push(Line::Into(self.folders.len()));
            self.lines.extend(groups.iter().map(|&g| Line::Moved(g)));
            self.folders.push(Folder {
                path: path.to_path_buf(),
                new,
                photos: groups.len(),
                arrivals,
            });
        }
        for (heading, groups, line) in [
            (
                Line::FoldersRenamed,
                renamed,
                Line::FolderRenamed as fn(usize) -> Line,
            ),
            (Line::Updates, updates, Line::Updated),
            (Line::Trash, trash, Line::Trashed),
        ] {
            if !groups.is_empty() {
                self.lines.push(heading);
                self.lines.extend(groups.into_iter().map(line));
            }
        }
    }
    fn len(&self) -> usize {
        self.ends.last().copied().unwrap_or(0)
    }
    fn at(&self, row: usize) -> (usize, usize) {
        let group = self.ends.partition_point(|end| *end <= row);
        let start = group.checked_sub(1).map_or(0, |i| self.ends[i]);
        (group, row - start)
    }
}

/// A folder as a person reads it: `2025-10-25 / group-2 / h`, relative to
/// `base`, or the base folder's own name when it is the base.
fn folder_text(path: &Path, base: Option<&Path>) -> String {
    match base.and_then(|base| path.strip_prefix(base).ok()) {
        Some(relative) if relative.as_os_str().is_empty() => path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        ),
        Some(relative) => relative
            .iter()
            .map(|c| c.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" / "),
        None => path.display().to_string(),
    }
}
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}
/// Which of a photo's files a person thinks of as the photo, and a short
/// note for the rest: a RAW twin or XMP sidecar is "+ RAW, XMP".
fn lead_and_extras(paths: &[&Path]) -> (usize, String) {
    let kind = |p: &Path| MediaKind::from_path(p);
    let sidecar = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp"));
    let lead = paths
        .iter()
        .position(|p| matches!(kind(p), Some(MediaKind::Photo | MediaKind::Video)))
        .or_else(|| paths.iter().position(|p| !sidecar(p)))
        .unwrap_or(0);
    let mut extras: Vec<String> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let tag = if kind(path) == Some(MediaKind::Raw) {
            "RAW".to_string()
        } else {
            path.extension()
                .map_or_else(String::new, |e| e.to_string_lossy().to_uppercase())
        };
        if i != lead && !tag.is_empty() && !extras.contains(&tag) {
            extras.push(tag);
        }
    }
    let note = if extras.is_empty() {
        String::new()
    } else {
        format!("+ {}", extras.join(", "))
    };
    (lead, note)
}
/// A rating or rotation in everyday words.
fn update_text(rating: Option<u8>, orientation: Option<u8>, pixels: bool) -> String {
    let mut parts = Vec::new();
    if let Some(orientation) = orientation {
        parts.push(
            match orientation {
                1 => "set upright",
                2 => "mirror",
                3 => "turn upside down",
                4 => "flip",
                5 | 7 => "flip and turn",
                6 => "rotate right",
                8 => "rotate left",
                _ => "change orientation",
            }
            .to_string(),
        );
    }
    match rating {
        Some(0) => parts.push("clear rating".into()),
        Some(n) => parts.push(count(n.into(), "star", "stars")),
        None => {}
    }
    let mut text = parts.join(", ");
    if pixels {
        text.push_str(" (re-saves the image)");
    }
    text
}

const LINE_HEIGHT: f32 = 30.0;
/// Space above each card; part of its heading line so all lines stay equal.
const CARD_GAP: f32 = 8.0;

/// One plain line as text runs: (text, color, emphasised).
fn plain_line(
    plan: &Plan,
    index: &Rows,
    line: Line,
    root: Option<&Path>,
) -> Vec<(String, Color32, bool)> {
    let output = plan.output_root.as_deref().or(root);
    let mut runs = Vec::new();
    let mut add = |text: String, color: Color32, strong: bool| runs.push((text, color, strong));
    match line {
        Line::Into(i) => {
            let folder = &index.folders[i];
            let name = folder_text(&folder.path, output);
            if folder.photos == 0 {
                add(format!("New folder {name}"), INK, true);
                add("   stays empty for now".into(), MUTED, false);
            } else {
                add(
                    format!("{} {name}", if folder.arrivals { "Into" } else { "In" }),
                    INK,
                    true,
                );
                if folder.new {
                    add("   new folder".into(), GREEN, false);
                }
            }
        }
        Line::FoldersRenamed => add("Rename folders".into(), INK, true),
        Line::Updates => add(index.summary.updates(), INK, true),
        Line::Trash => add("To the Trash".into(), RED, true),
        Line::Moved(group) => {
            let (pairs, finishing): (Vec<(&Path, &Path)>, bool) = match &plan.actions[group] {
                Action::Move { files } => (
                    files
                        .iter()
                        .map(|(s, t)| (s.path.as_path(), t.as_path()))
                        .collect(),
                    false,
                ),
                Action::FinishMove { files } => (
                    files
                        .iter()
                        .map(|(s, t)| (s.path.as_path(), t.as_path()))
                        .collect(),
                    true,
                ),
                Action::RenameFile { source, to } => {
                    (vec![(source.path.as_path(), to.as_path())], false)
                }
                _ => (Vec::new(), false),
            };
            let sources: Vec<&Path> = pairs.iter().map(|(s, _)| *s).collect();
            let (lead, extras) = lead_and_extras(&sources);
            let Some(&(source, to)) = pairs.get(lead) else {
                return runs;
            };
            let (before, after) = (file_name(source), file_name(to));
            if before != after {
                add(before, INK, false);
                add("  →  ".into(), MUTED, false);
            }
            add(after, INK, false);
            if !extras.is_empty() {
                add(format!("  {extras}"), MUTED, false);
            }
            if source.parent() != to.parent() {
                let from = source.parent().unwrap_or(Path::new(""));
                add(format!("   from {}", folder_text(from, root)), MUTED, false);
            }
            if finishing {
                add("   finishing an earlier move".into(), MUTED, false);
            }
        }
        Line::Trashed(group) => match &plan.actions[group] {
            Action::Trash { files } if !files.is_empty() => {
                let sources: Vec<&Path> = files.iter().map(|s| s.path.as_path()).collect();
                let (lead, extras) = lead_and_extras(&sources);
                add(file_name(sources[lead]), INK, false);
                if !extras.is_empty() {
                    add(format!("  {extras}"), MUTED, false);
                }
                if let Some(from) = sources[lead].parent() {
                    add(format!("   in {}", folder_text(from, root)), MUTED, false);
                }
            }
            Action::TrashDirectory(folder) => {
                add(
                    format!("Folder {}", folder_text(&folder.path, root)),
                    INK,
                    false,
                );
                add(
                    format!(
                        "   with everything in it ({})",
                        count(folder.inventory.files.len(), "file", "files")
                    ),
                    MUTED,
                    false,
                );
            }
            _ => {}
        },
        Line::Updated(group) => {
            if let Action::Metadata {
                source,
                rating,
                orientation,
                target,
                ..
            } = &plan.actions[group]
            {
                add(file_name(&source.path), INK, false);
                add(
                    format!(
                        "   {}",
                        update_text(
                            *rating,
                            *orientation,
                            *target == crate::core::metadata::WriteTarget::Pixels
                        )
                    ),
                    MUTED,
                    false,
                );
            }
        }
        Line::FolderRenamed(group) => {
            if let Action::RenameDirectory(rename) = &plan.actions[group] {
                add(folder_text(&rename.from, root), INK, false);
                add("  →  ".into(), MUTED, false);
                add(folder_text(&rename.to, root), INK, false);
            }
        }
    }
    runs
}

/// The photos a plan changes, in cards by destination, Trash and updates.
/// Each line is one widget whose accessible value is the text it shows.
fn plain_rows(
    ui: &mut egui::Ui,
    plan: &Plan,
    index: &Rows,
    root: Option<&Path>,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<usize> {
    ui.scope(|ui| {
        // Lines of a card touch; the card gap is inside each heading line.
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical()
            .id_salt(("plan_plain", index.generation))
            .max_height(height)
            .auto_shrink([false, true])
            .show_rows(ui, LINE_HEIGHT, index.lines.len(), |ui, rows| {
                let rendered = rows.len();
                let body = egui::TextStyle::Body.resolve(ui.style());
                for row in rows {
                    let line = index.lines[row];
                    let last = index.lines.get(row + 1).is_none_or(|l| l.heading());
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), LINE_HEIGHT),
                        Sense::hover(),
                    );
                    let mut card = rect;
                    let radius = R_TILE;
                    let mut corners = egui::CornerRadius::ZERO;
                    if line.heading() {
                        card.min.y += CARD_GAP;
                        corners.nw = radius;
                        corners.ne = radius;
                    }
                    if last {
                        corners.sw = radius;
                        corners.se = radius;
                    }
                    ui.painter().rect_filled(card, corners, CARD);
                    let runs = plain_line(plan, index, line, root);
                    let mut job = LayoutJob::default();
                    let mut spoken = String::new();
                    for (text, color, strong) in &runs {
                        let font_id = if *strong {
                            medium(body.size)
                        } else {
                            body.clone()
                        };
                        job.append(
                            text,
                            0.0,
                            TextFormat {
                                font_id,
                                color: *color,
                                ..Default::default()
                            },
                        );
                        spoken.push_str(text.trim());
                        spoken.push(' ');
                    }
                    let mut text_rect = card.shrink2(egui::vec2(S3, 0.0));
                    // A card's photo count sits at its right edge.
                    if let Some(total) = heading_count(index, line) {
                        let galley = ui.painter().layout_no_wrap(
                            total.clone(),
                            egui::TextStyle::Small.resolve(ui.style()),
                            MUTED,
                        );
                        let at = egui::pos2(
                            text_rect.right() - galley.size().x,
                            text_rect.center().y - galley.size().y / 2.0,
                        );
                        text_rect.max.x = at.x - S3;
                        ui.painter().galley(at, galley, MUTED);
                        spoken.push_str(&total);
                    }
                    let spoken = spoken.trim_end().to_string();
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .id_salt(row)
                            .max_rect(text_rect)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    let response = child.add(egui::Label::new(job).truncate().selectable(false));
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken)
                    });
                    if let Line::Moved(group)
                    | Line::Trashed(group)
                    | Line::Updated(group)
                    | Line::FolderRenamed(group) = line
                    {
                        response.on_hover_ui(|ui| {
                            ui.label(plan.actions[group].label());
                        });
                    }
                }
                rendered
            })
    })
    .inner
}

/// The count shown at the right of a card heading.
fn heading_count(index: &Rows, line: Line) -> Option<String> {
    let summary = &index.summary;
    match line {
        Line::Into(i) if index.folders[i].photos > 0 => Some(photos(index.folders[i].photos)),
        Line::Trash if summary.trashed_folders == 0 => Some(photos(summary.trashed)),
        Line::Trash => Some(count(
            summary.trashed + summary.trashed_folders,
            "item",
            "items",
        )),
        _ => None,
    }
}

/// The folders a plan removes afterwards if nothing is left in them, in one line.
fn cleanup_text(plan: &Plan, index: &Rows, root: Option<&Path>, limit: usize) -> String {
    let names: Vec<String> = index
        .summary
        .cleanup
        .iter()
        .filter_map(|&group| match &plan.actions[group] {
            Action::RemoveEmptyDirectory { path, .. } => Some(folder_text(path, root)),
            _ => None,
        })
        .collect();
    let shown = limit.min(names.len());
    let mut text = names
        .iter()
        .take(shown)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > shown {
        text.push_str(&format!(" and {} more", names.len() - shown));
    }
    text
}

const ROW_HEIGHT: f32 = 46.0;

/// One path line. `spoken` prefixes the accessible text ("From  ", "To    ")
/// so screen readers keep the direction the arrow shows visually.
fn path_line(
    ui: &mut egui::Ui,
    spoken: &str,
    shown: &str,
    path: &Path,
    root: Option<&Path>,
    color: Color32,
) {
    let relative = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let display = if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.display().to_string()
    };
    // Extend inside the two-axis scroll area: long paths can be read without hover.
    let response = ui
        .add(egui::Label::new(RichText::new(format!("{shown}{display}")).color(color)).extend())
        .on_hover_text(path.display().to_string());
    let spoken = format!("{spoken}{display}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
}
fn path_label(ui: &mut egui::Ui, prefix: &str, path: &Path, root: Option<&Path>) {
    let to = prefix.trim() == "To";
    path_line(
        ui,
        prefix,
        if to { "→  " } else { "" },
        path,
        root,
        if to { MUTED } else { INK },
    );
}
/// The first line of a row: what happens, then the file it happens to.
fn source_line(
    ui: &mut egui::Ui,
    kind: &str,
    color: Color32,
    prefix: &str,
    path: &Path,
    root: Option<&Path>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        ui.label(RichText::new(kind).small().color(color));
        path_line(ui, prefix, "", path, root, INK);
    });
}
fn note_line(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(MUTED)).extend());
}

fn row_ui(
    ui: &mut egui::Ui,
    action: &Action,
    _group: usize,
    file: usize,
    root: Option<&Path>,
    output: Option<&Path>,
) {
    match action {
        Action::Move { files } | Action::FinishMove { files } if !files.is_empty() => {
            let (source, to) = &files[file];
            let kind = if matches!(action, Action::FinishMove { .. }) {
                "Finish staged move (remove verified source link)"
            } else if source.path.parent() == to.parent() {
                "Rename"
            } else if source.path.file_name() == to.file_name() {
                "Move"
            } else {
                "Move and rename"
            };
            source_line(ui, kind, ACCENT, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::Trash { files } if !files.is_empty() => {
            source_line(ui, "Trash", RED, "From  ", &files[file].path, root);
            note_line(ui, "→  System Trash");
        }
        Action::TrashDirectory(folder) => {
            if file == 0 {
                source_line(
                    ui,
                    &format!(
                        "Trash folder · {} files, {} subfolders",
                        folder.inventory.files.len(),
                        folder.inventory.directory_count().saturating_sub(1)
                    ),
                    RED,
                    "From  ",
                    &folder.path,
                    root,
                );
            } else {
                source_line(
                    ui,
                    &format!("In folder · {} of {}", file, folder.inventory.files.len()),
                    RED,
                    "File  ",
                    &folder.inventory.files[file - 1].path,
                    root,
                );
            }
            note_line(ui, "→  System Trash, with the whole folder");
        }
        Action::RenameFile { source, to } => {
            source_line(ui, "Rename", INK, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::RenameDirectory(rename) => {
            source_line(ui, "Rename folder", INK, "From  ", &rename.from, root);
            path_label(ui, "To    ", &rename.to, root);
        }
        Action::Metadata { source, .. } => {
            source_line(ui, "Metadata", INK, "File  ", &source.path, root);
            note_line(ui, &action.label());
        }
        Action::CreateDirectory { path } | Action::RemoveEmptyDirectory { path, .. } => {
            let create = matches!(action, Action::CreateDirectory { .. });
            source_line(
                ui,
                if create {
                    "Create folder"
                } else {
                    "Remove folder if empty"
                },
                if create { GREEN } else { MUTED },
                "Folder  ",
                path,
                output.or(root),
            );
        }
        _ => {
            ui.label("Empty file operation");
        }
    }
}

fn file_rows(
    ui: &mut egui::Ui,
    plan: &Plan,
    index: &Rows,
    root: Option<&Path>,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<usize> {
    egui::ScrollArea::both()
        .id_salt(("plan_files", index.generation))
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, index.len(), |ui, rows| {
            let rendered = rows.len();
            for row in rows {
                let (group, file) = index.at(row);
                ui.push_id(row, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            row_ui(
                                ui,
                                &plan.actions[group],
                                group,
                                file,
                                root,
                                plan.output_root.as_deref(),
                            );
                        },
                    );
                });
            }
            rendered
        })
}

impl PhotoSortApp {
    pub(super) fn set_review_plan(&mut self, plan: Option<Plan>) {
        self.retry_edit = None;
        self.plan_rows.replace(plan.as_ref());
        self.plan = plan;
    }

    pub(super) fn plan_review_dialog(&mut self, ctx: &egui::Context) {
        if !self.plan_open {
            return;
        }
        if self.retry_edit.is_some() {
            self.destination_editor(ctx);
            return;
        }
        let mut edit = false;
        let mut apply = false;
        let mut cancel = false;
        let mut discard = false;
        let mut save_quit = false;
        modal(egui::Id::new("review_changes")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 720.0));
            let top = ui.cursor().top();
            if let Some(plan) = &self.plan {
                let retry = self.retry_plan.is_some();
                let rows = &self.plan_rows;
                ui.heading(if retry { "Unfinished file operations" } else { "Review your changes" });
                ui.horizontal_wrapped(|ui| {
                    for (i, (part, color)) in rows.summary.parts().into_iter().enumerate() {
                        if i > 0 { ui.label(RichText::new("·").color(FAINT)); }
                        ui.label(RichText::new(part).font(medium(13.5)).color(color));
                    }
                });
                // Reassurance first; the library is named, its full path is on hover.
                let library = self.root.as_deref().map(|root| (folder_text(root, Some(root)), root));
                let note = if retry {
                    "Changes that finished are already applied. These are the ones left.".to_string()
                } else {
                    let mut note = match &library {
                        Some((name, _)) => format!("Nothing in {name} has changed yet."),
                        None => "Nothing has changed yet.".into(),
                    };
                    if rows.summary.trashed + rows.summary.trashed_folders > 0 {
                        note.push_str(" Anything moved to the Trash can be restored from your system Trash.");
                    }
                    note
                };
                let response = ui.add(egui::Label::new(RichText::new(note).color(MUTED)).wrap());
                if let Some((_, root)) = library { response.on_hover_text(root.display().to_string()); }
                if let Some(output) = &plan.output_root && self.root.as_ref() != Some(output) {
                    ui.add(egui::Label::new(RichText::new(format!("Photos go to {}", folder_text(output, Some(output)))).color(MUTED)).truncate())
                        .on_hover_text(output.display().to_string());
                }
                if !rows.summary.cleanup.is_empty() {
                    let n = rows.summary.cleanup.len();
                    ui.add(egui::Label::new(RichText::new(format!(
                        "{} left empty afterwards {} removed: {}",
                        count(n, "folder", "folders"),
                        if n == 1 { "is" } else { "are" },
                        cleanup_text(plan, rows, self.root.as_deref(), 3),
                    )).small().color(MUTED)).truncate())
                    .on_hover_text(cleanup_text(plan, rows, self.root.as_deref(), 40));
                }
                if retry && self.error.is_some() { ui.colored_label(RED, "Some operations need attention. Open details before retrying."); }
                // Details swap the plain list for every file operation, so the
                // list keeps one height budget. Retries start there.
                let details = egui::CollapsingHeader::new("Operation details").id_salt(("plan_details", retry)).default_open(retry).show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("plan_explanation").max_height(70.0).show(ui, |ui| {
                        ui.label(&plan.description);
                        if retry {
                            ui.label("Successful operations are already applied. Finish staged move rechecks that the destination is the same file before removing its source link. Edit destination saves a revised folder or base name for a pending move and all its companions. Retry uses the saved destinations and source checks. Resolve remaining conflicts first. Discard abandons only unfinished operations; the audit record is retained.");
                            if let Some(error) = &self.error { ui.colored_label(RED, error); }
                        } else {
                            ui.label(if self.rotation_plan { "Only confirmed corrections are included. Metadata rows identify embedded updates, XMP sidecars or lossless pixel rotations." } else { "All listed companion files are included. Metadata rows identify embedded updates, XMP sidecars or lossless pixel rotations. Trash entries go to your system's Trash. Moves never overwrite existing files. Folders are removed only when nothing is left in them." });
                        }
                    });
                });
                ui.separator();
                // Account for actual wrapped heading/notice heights and reserve the footer.
                let footer = if retry && matches!(self.after_apply, Some(Destination::Close)) { 230.0 } else { 130.0 };
                let height = (ctx.content_rect().height() - footer - (ui.cursor().top() - top)).clamp(40.0, 400.0);
                if details.openness > 0.0 || rows.lines.is_empty() {
                    file_rows(ui, plan, rows, self.root.as_deref(), height);
                } else {
                    plain_rows(ui, plan, rows, self.root.as_deref(), height);
                }
                ui.separator();
                if self.close_save_pending { ui.label("Saving before quitting…"); }
                if self.review_after_moves && !retry {
                    let staged = self.decisions.values().filter(|d| !d.is_empty()).count();
                    ui.label(RichText::new(format!(
                        "Next: {staged} {} with Trash marks, ratings or rotations, reviewed once these changes are applied.",
                        if staged == 1 { "photo" } else { "photos" }
                    )).color(MUTED));
                }
                if retry && matches!(self.after_apply, Some(Destination::Close)) && self.recovery_ticket.is_some() {
                    ui.label("Save your plan and review progress to resume next time. No remaining file operations will run.");
                    if self.has_unsaved_plan_extras() { ui.label("Go back to apply or reset other unsaved organization or confirmed rotation choices first."); }
                }
                let changes = rows.summary.changes();
                let apply_label = if retry {
                    "Retry unfinished changes".to_string()
                } else if changes == 0 {
                    "Apply changes".to_string()
                } else {
                    format!("Apply {}", count(changes, "change", "changes"))
                };
                ui.add_enabled_ui(!self.busy, |ui| { ui.horizontal_wrapped(|ui| {
                    if retry && self.recovery_ticket.is_some() && matches!(self.after_apply, Some(Destination::Close))
                        && ui.add_enabled(!self.has_unsaved_plan_extras(), egui::Button::new("Save plan and quit")).clicked() { save_quit = true; }
                    if ui.button("Back").clicked() { cancel = true; }
                    if retry && ui.add_enabled(!self.busy && self.recovery_ticket.is_some() && !rows.movable.is_empty(), egui::Button::new("Edit destination…")).clicked() { edit = true; }
                    if retry && ui.button("Discard unfinished plan").clicked() { discard = true; }
                    if ui.add_enabled(!plan.actions.is_empty() && !self.busy, primary(apply_label)).clicked() { apply = true; }
                }); });
            }
        });
        if save_quit {
            self.save_plan_and_quit();
        }
        if edit {
            self.begin_destination_edit();
        }
        if cancel {
            self.plan_open = false;
            self.after_apply = None;
            self.review_after_moves = false;
            if self.retry_plan.is_none() {
                self.rotation_plan = false;
            }
        }
        if discard {
            self.discard_remaining_operations();
        }
        if apply {
            self.apply_reviewed_plan();
        }
    }
}

pub(super) struct Edit {
    pub(super) group: usize,
    pub(super) folder: String,
    pub(super) stem: String,
    pub saving: bool,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub error: Option<String>,
}
impl PhotoSortApp {
    pub(super) fn begin_destination_edit(&mut self) {
        if self.busy || self.scanning || self.recovery_ticket.is_none() {
            return;
        }
        let Some(&group) = self.plan_rows.movable.first() else {
            return;
        };
        self.select_destination_group(group);
    }
    fn select_destination_group(&mut self, group: usize) {
        let Some(plan) = &self.plan else {
            return;
        };
        let Some(Action::Move { files }) = plan.actions.get(group) else {
            return;
        };
        let Some((_, destination)) = files.first() else {
            return;
        };
        let base = plan.output_root.as_deref().or(self.root.as_deref());
        let folder = destination
            .parent()
            .and_then(|p| base.and_then(|b| p.strip_prefix(b).ok()))
            .unwrap_or(Path::new(""));
        self.retry_edit = Some(Edit {
            group,
            folder: folder.display().to_string(),
            stem: String::new(),
            saving: false,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            error: None,
        });
    }
    pub(super) fn save_destination_edit(&mut self) {
        if self.busy || self.scanning {
            return;
        }
        let (Some(root), Some(ticket), Some(edit)) =
            (&self.root, &self.recovery_ticket, &mut self.retry_edit)
        else {
            return;
        };
        edit.cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match self.runtime.io(IoWork::AmendRecovery {
            root: root.clone(),
            ticket: ticket.clone(),
            group: edit.group,
            folder: PathBuf::from(&edit.folder),
            stem: (!edit.stem.is_empty()).then(|| edit.stem.clone()),
            cancelled: edit.cancelled.clone(),
        }) {
            Ok(()) => {
                edit.saving = true;
                edit.error = None;
                self.busy = true;
            }
            Err(error) => edit.error = Some(error),
        }
    }
    fn destination_editor(&mut self, ctx: &egui::Context) {
        let mut select = None;
        let mut save = false;
        let mut cancel = false;
        let Some(edit) = &mut self.retry_edit else {
            return;
        };
        let Some(plan) = &self.plan else {
            return;
        };
        modal(egui::Id::new("edit_pending_destination")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(240., 720.));
            ui.heading("Edit pending destination");
            ui.label("Choose a move. Photo companions stay together; extensions keep their original spelling.");
            ui.add_enabled_ui(!edit.saving, |ui| {
                egui::ScrollArea::vertical().id_salt("pending_move_groups")
                    .max_height((ctx.content_rect().height() * 0.22).clamp(50., 150.))
                    .show_rows(ui, button_row_height(ui), self.plan_rows.movable.len(), |ui, rows| {
                        for row in rows {
                            let group = self.plan_rows.movable[row];
                            if let Action::Move { files } = &plan.actions[group] {
                                let name = files[0].1.file_name().unwrap_or_default().to_string_lossy();
                                if ui.add(egui::Button::new(format!("Group {} · {} · {} file(s)", group + 1, name, files.len())).selected(edit.group == group).truncate()).clicked() { select = Some(group); }
                            }
                        }
                    });
                let base = plan.output_root.as_deref().or(self.root.as_deref()).unwrap_or(Path::new(""));
                ui.add(egui::Label::new(format!("Folder within {}", base.display())).truncate()).on_hover_text(base.display().to_string());
                ui.add(egui::TextEdit::singleline(&mut edit.folder).hint_text("Leave empty for the destination root").desired_width(f32::INFINITY));
                ui.label("New base name (optional)");
                ui.add(egui::TextEdit::singleline(&mut edit.stem).hint_text("Leave empty to keep reviewed names").desired_width(f32::INFINITY));
                if let Some(Action::Move { files }) = plan.actions.get(edit.group) {
                    egui::ScrollArea::both().id_salt("pending_names_preview").max_height(48.).show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace), files.len(), |ui, rows| {
                        for row in rows {
                            let old = &files[row].1;
                            let name = if edit.stem.is_empty() { old.file_name().unwrap_or_default().to_owned() }
                                else { crate::core::organize::renamed_file_name(old, edit.stem.as_ref()) };
                            let to = base.join(&edit.folder).join(name);
                            path_label(ui, "To  ", &to, Some(base));
                        }
                    });
                }
            });
            if let Some(error) = &edit.error {
                egui::ScrollArea::vertical().id_salt("destination_edit_error").max_height(55.).show(ui, |ui| { ui.colored_label(RED, error); });
            }
            ui.small("Saving updates the plan only. Existing files stay untouched until you review and retry.");
            if edit.saving { ui.label("Checking sources and saving…"); }
            ui.horizontal_wrapped(|ui| {
                if ui.button(if edit.saving { "Cancel saving" } else { "Cancel" }).clicked() { cancel = true; }
                if ui.add_enabled(!edit.saving, primary("Save destination")).clicked() { save = true; }
            });
        });
        if cancel {
            if edit.saving {
                edit.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            } else {
                self.retry_edit = None;
            }
        } else if let Some(group) = select {
            self.select_destination_group(group);
        } else if save {
            self.save_destination_edit();
        }
    }
}

#[cfg(test)]
#[path = "plan_review/tests.rs"]
mod tests;
