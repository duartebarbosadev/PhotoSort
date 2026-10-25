//! Faces on Organize's people folders: each person's clearest face is the
//! picture of the folders they are in, a name given to a person follows them
//! into every folder, and two people the grouping split can be marked as the
//! same person. Faces come from the evidence the Faces grouping already
//! cached; a picture is the photo's thumbnail texture drawn through a circle,
//! so nothing is decoded again.
use super::*;
use crate::core::{faces::Face, subjects::store::Store};
use std::{collections::BTreeMap, path::Path, sync::mpsc};

/// Photos looked at per person for their clearest faces.
const CANDIDATES: usize = 12;
/// Faces kept per person: the clearest, and a few more for the hover card.
const KEPT: usize = 5;
/// Sidebar rows grouped by faces, and the face in them.
pub(super) const ROW_HEIGHT: f32 = 34.0;
pub(super) const ROW_FACE: f32 = 26.0;
/// The room a sidebar row keeps for one face or two overlapping.
pub(super) const ROW_SLOT: f32 = 40.0;
/// The face in a board folder header.
pub(super) const HEADER_FACE: f32 = 32.0;
/// The faces on the hover card: the clearest large, up to four more in a
/// row under it.
pub(super) const CARD_FACE: f32 = 128.0;
pub(super) const CARD_MORE: f32 = 56.0;
const CARD_GAP: f32 = 6.0;
/// Sharp face crops asked for per frame, after the preview's own detail.
const SHARP_REQUESTS: usize = 4;
/// Sharp face crops kept; more empties the cache.
const SHARP_KEPT: usize = 256;

/// One person's face: the photo and the face's box in it, normalized to the
/// photo as shown.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Portrait {
    pub(super) id: MediaId,
    pub(super) bbox: [f32; 4],
}

/// A face ready to draw: a texture of the photo and the face's square in it.
#[derive(Clone, Copy)]
pub(super) struct FaceImage {
    pub(super) texture: egui::TextureId,
    pub(super) uv: egui::Rect,
}

/// A photo that may hold a person's face, and its size in original pixels
/// when its metadata has been read.
#[derive(Clone)]
struct Candidate {
    media: Media,
    pixels: Option<f32>,
}
/// Pixels assumed for a photo whose size is not known yet: a phone photo.
const UNKNOWN_PIXELS: f32 = 12_000_000.0;

/// Orders found faces (source pixels, [`quality`]) best first: the most
/// source pixels, so a large card is sharp. Quality decides between faces of
/// about the same size; it outweighs at most a third more pixels.
fn rank(found: &mut [(f32, f32, Portrait)]) {
    let score = |(pixels, quality, _): &(f32, f32, Portrait)| pixels.max(1.0).ln() + 0.3 * quality;
    found.sort_by(|a, b| score(b).total_cmp(&score(a)));
}
fn face_pixels(bbox: [f32; 4], pixels: Option<f32>) -> f32 {
    let [l, t, r, b] = bbox;
    ((r - l) * (b - t)).max(0.0) * pixels.unwrap_or(UNKNOWN_PIXELS)
}

/// Person number ("001") to their clearest faces, clearest first.
type Portraits = HashMap<String, Vec<Portrait>>;

#[derive(Default)]
pub(super) struct People {
    pub(super) portraits: Portraits,
    /// Folder path to the people its photos show, in folder-name order.
    pub(super) folders: HashMap<String, Vec<String>>,
    /// People marked as the same person: "003" to "001". Staged with the
    /// folder edits they made, and forgotten when those are.
    pub(super) merged: BTreeMap<String, String>,
    /// The face grouping `folders` was worked out from; portraits are looked
    /// up again when it changes.
    pub(super) signature: Option<u64>,
    looked_up: Option<u64>,
    loading: Option<mpsc::Receiver<(u64, Portraits)>>,
    /// Faces cut sharp from the original photo, by photo and region.
    pub(super) crops: HashMap<(MediaId, [u32; 4]), egui::TextureHandle>,
    /// Sharp crops wanted this frame, in the order they are shown.
    pub(super) wanted: Vec<InspectionRequest>,
}
impl People {
    /// Who `person` is after the merges: "003" marked as "001" is "001".
    fn canonical(&self, person: &str) -> String {
        let mut person = person.to_owned();
        // Merges point forward only; the bound guards against a cycle.
        for _ in 0..self.merged.len() {
            match self.merged.get(&person) {
                Some(into) => person = into.clone(),
                None => break,
            }
        }
        person
    }
}

/// "Person 003" is `["003"]`, "People 001 + 003" is `["001", "003"]`: the
/// folder names the Faces grouping makes.
pub(super) fn people_in(label: &str) -> Option<Vec<String>> {
    let people = label
        .strip_prefix("Person ")
        .or_else(|| label.strip_prefix("People "))?;
    let people: Vec<String> = people.split(" + ").map(str::to_owned).collect();
    people
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
        .then_some(people)
}

/// The folder name for `people` from the names given so far: "Ana", or
/// "Person 001 + Ana". With no names it is the grouping's own name.
pub(super) fn folder_name(people: &[String], names: &HashMap<String, String>) -> String {
    let named = |p: &String| names.get(p).cloned();
    match people {
        [one] => named(one).unwrap_or_else(|| format!("Person {one}")),
        _ if people.iter().all(|p| named(p).is_none()) => format!("People {}", people.join(" + ")),
        _ => people
            .iter()
            .map(|p| named(p).unwrap_or_else(|| format!("Person {p}")))
            .collect::<Vec<_>>()
            .join(" + "),
    }
}

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A face's quality apart from its size, 0 to 1: facing the camera, eyes
/// open and not cut off at the photo's edge.
pub(super) fn quality(face: &Face) -> f32 {
    let [l, t, r, b] = face.bbox;
    let (w, h) = (r - l, b - t);
    if !(w > 0.0 && h > 0.0) || !face.bbox.iter().all(|v| v.is_finite()) {
        return 0.0;
    }
    // Eyes level and centred in the box mean the face looks at the camera.
    let [a, e] = face.eyes;
    let middle = (a[0] + e[0]) / 2.0;
    let centred = 1.0 - ((middle - (l + r) / 2.0).abs() / (w / 2.0)).min(1.0);
    let level = 1.0 - ((a[1] - e[1]).abs() / w).min(1.0);
    let open = face
        .blink
        .map_or(0.5, |[x, y]| 1.0 - x.max(y).clamp(0.0, 1.0));
    let whole = if l >= 0.0 && t >= 0.0 && r <= 1.0 && b <= 1.0 {
        1.0
    } else {
        0.6
    };
    (0.4 + 0.6 * centred * level) * (0.6 + 0.4 * open) * whole
}

/// The face's square in a `size` texture, just around the box so the face
/// fills the circle, moved rather than squashed at the photo's edges.
pub(super) fn face_uv(bbox: [f32; 4], size: egui::Vec2) -> egui::Rect {
    let [l, t, r, b] = bbox.map(|v| v.clamp(0.0, 1.0));
    let side = ((r - l) * size.x).max((b - t) * size.y) * 1.1;
    let half = egui::vec2(
        (side / 2.0 / size.x.max(1.0)).min(0.5),
        (side / 2.0 / size.y.max(1.0)).min(0.5),
    );
    let center = egui::pos2(
        ((l + r) / 2.0).clamp(half.x, 1.0 - half.x),
        ((t + b) / 2.0).clamp(half.y, 1.0 - half.y),
    );
    egui::Rect::from_center_size(center, half * 2.0)
}

/// Paints `face` as a round picture with a thin ring in `ring`, the colour
/// behind it, so two overlapping faces read as two.
pub(super) fn paint_face(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    face: FaceImage,
    ring: Color32,
) {
    const STEPS: u32 = 48;
    let mut mesh = egui::Mesh::with_texture(face.texture);
    let uv = face.uv;
    mesh.vertices.push(egui::epaint::Vertex {
        pos: center,
        uv: uv.center(),
        color: Color32::WHITE,
    });
    for step in 0..=STEPS {
        let angle = step as f32 / STEPS as f32 * std::f32::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + radius * egui::vec2(cos, sin),
            uv: uv.center() + egui::vec2(cos * uv.width(), sin * uv.height()) / 2.0,
            color: Color32::WHITE,
        });
    }
    for step in 1..=STEPS {
        mesh.add_triangle(0, step, step + 1);
    }
    painter.add(mesh);
    painter.circle_stroke(center, radius, Stroke::new(1.5, ring));
}

/// Paints up to two faces in `slot`, the second overlapping the first.
pub(super) fn paint_faces(
    painter: &egui::Painter,
    slot: egui::Rect,
    faces: &[FaceImage],
    ring: Color32,
) {
    let radius = slot.height() / 2.0;
    let step = if faces.len() > 1 {
        (slot.width() - 2.0 * radius).max(0.0)
    } else {
        0.0
    };
    for (i, face) in faces.iter().take(2).enumerate().rev() {
        let center = egui::pos2(slot.left() + radius + step * i as f32, slot.center().y);
        paint_face(painter, center, radius, *face, ring);
    }
}

/// The hover card of a people folder: each person's clearest face large,
/// a few more of them small, and their name; two people side by side.
pub(super) fn face_card(ui: &mut egui::Ui, people: &[(String, Vec<FaceImage>)]) -> f32 {
    let width: f32 = people
        .iter()
        .map(|(_, faces)| card_width(faces.len()))
        .sum::<f32>()
        + 2.0 * CARD_GAP * people.len().saturating_sub(1) as f32;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0 * CARD_GAP;
        for (name, faces) in people {
            let column = card_width(faces.len());
            ui.vertical(|ui| {
                ui.set_width(column);
                ui.spacing_mut().item_spacing.y = CARD_GAP;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(column, CARD_FACE), Sense::hover());
                if let Some(face) = faces.first() {
                    paint_face(ui.painter(), rect.center(), CARD_FACE / 2.0, *face, PANEL);
                }
                let more = &faces[faces.len().min(1)..];
                if !more.is_empty() {
                    let (row, _) =
                        ui.allocate_exact_size(egui::vec2(column, CARD_MORE), Sense::hover());
                    let shown = more.len().min(4);
                    let span = shown as f32 * CARD_MORE + (shown - 1) as f32 * CARD_GAP;
                    let left = row.center().x - span / 2.0;
                    for (i, face) in more.iter().take(4).enumerate() {
                        let center = egui::pos2(
                            left + CARD_MORE / 2.0 + i as f32 * (CARD_MORE + CARD_GAP),
                            row.center().y,
                        );
                        paint_face(ui.painter(), center, CARD_MORE / 2.0, *face, PANEL);
                    }
                }
                ui.vertical_centered(|ui| {
                    ui.add(egui::Label::new(RichText::new(name).font(medium(12.0))).truncate());
                });
            });
        }
    });
    width
}
/// One person's column on the hover card with `faces` faces: as wide as
/// the row of smaller faces, and never narrower than the large one.
pub(super) fn card_width(faces: usize) -> f32 {
    let more = faces.saturating_sub(1).min(4) as f32;
    (more * CARD_MORE + (more - 1.0).max(0.0) * CARD_GAP).max(CARD_FACE)
}

impl PhotoSortApp {
    /// Works out which people each folder shows, from the faces the
    /// grouping recognized in its photos and the people marked the same: a
    /// folder renamed "Ana" still shows Ana. Called when the board is arranged.
    pub(super) fn arrange_people(&mut self) {
        self.board.people.folders.clear();
        // Merges are folder edits: gone with the grouping's draft.
        if !self.organize_drafts.contains_key(&GroupMode::Faces) {
            self.board.people.merged.clear();
        }
        let Some(groups) = self
            .face_groups
            .as_ref()
            .filter(|_| self.group_mode == GroupMode::Faces && !self.board.original)
        else {
            self.board.people.signature = None;
            return;
        };
        let mut hasher = std::hash::DefaultHasher::new();
        let mut labels: Vec<(u64, &String)> = groups
            .folders
            .iter()
            .filter(|(id, _)| groups.groups.get(id).is_some_and(|g| *g != 0))
            .map(|(id, label)| (id.0, label))
            .collect();
        labels.sort();
        std::hash::Hash::hash(&labels, &mut hasher);
        self.board.people.signature = Some(std::hash::Hasher::finish(&hasher));
        for section in &self.board.sections {
            let Some(path) = &section.path else {
                continue;
            };
            let mut counts = BTreeMap::<Vec<String>, usize>::new();
            for &index in &self.filtered[section.range.clone()] {
                let id = self.media[index].id;
                if groups.groups.get(&id).is_some_and(|g| *g != 0)
                    && let Some(people) = groups.folders.get(&id).and_then(|l| people_in(l))
                {
                    let mut people: Vec<String> = people
                        .iter()
                        .map(|p| self.board.people.canonical(p))
                        .collect();
                    people.sort();
                    people.dedup();
                    *counts.entry(people).or_default() += 1;
                }
            }
            // The most common, then the first by name for a steady choice.
            if let Some((people, _)) = counts
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
            {
                self.board.people.folders.insert(path.clone(), people);
            }
        }
    }
    /// Starts looking up each person's clearest faces once per grouping, on
    /// a background thread, and takes the result when it is ready.
    pub(super) fn update_people(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.board.people.loading {
            match rx.try_recv() {
                Ok((signature, portraits)) => {
                    self.board.people.loading = None;
                    if Some(signature) == self.board.people.signature {
                        self.board.people.portraits.extend(portraits);
                        self.earlier_portraits();
                    }
                }
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => self.board.people.loading = None,
            }
        }
        let Some(signature) = self.board.people.signature else {
            return;
        };
        if self.board.people.looked_up == Some(signature) {
            return;
        }
        self.board.people.looked_up = Some(signature);
        // People are numbered afresh by each grouping.
        self.board.people.portraits.clear();
        let jobs = self.portrait_candidates();
        let root = self.runtime.cache.directory().map(Path::to_path_buf);
        let (tx, rx) = mpsc::channel();
        self.board.people.loading = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((signature, look_up_portraits(root, jobs)));
            ctx.request_repaint();
        });
    }
    /// Photos showing exactly one person, so their largest face is that
    /// person's, up to [`CANDIDATES`] each.
    fn portrait_candidates(&self) -> Vec<(String, Vec<Candidate>)> {
        let Some(groups) = &self.face_groups else {
            return Vec::new();
        };
        let mut by_person = BTreeMap::<String, Vec<Candidate>>::new();
        for media in &self.media {
            if media.kind == MediaKind::Video
                || !groups.groups.get(&media.id).is_some_and(|g| *g != 0)
            {
                continue;
            }
            if let Some([person]) = groups
                .folders
                .get(&media.id)
                .and_then(|label| people_in(label))
                .as_deref()
            {
                let list = by_person.entry(person.clone()).or_default();
                if list.len() < CANDIDATES {
                    list.push(Candidate {
                        media: media.clone(),
                        pixels: self
                            .metadata
                            .get(&media.id)
                            .filter(|m| m.width > 0 && m.height > 0)
                            .map(|m| m.width as f32 * m.height as f32),
                    });
                }
            }
        }
        by_person.into_iter().collect()
    }
    /// A person without cached face geometry can still use the largest face
    /// an earlier Pick Best or Cull analysis found in one of their photos.
    fn earlier_portraits(&mut self) {
        for (person, candidates) in self.portrait_candidates() {
            if self.board.people.portraits.contains_key(&person) {
                continue;
            }
            let mut found: Vec<(f32, f32, Portrait)> = candidates
                .iter()
                .filter_map(|c| {
                    let bbox = self.analysis.get(&c.media.id)?.faces.as_ref()?.primary?;
                    let portrait = Portrait {
                        id: c.media.id,
                        bbox,
                    };
                    Some((face_pixels(bbox, c.pixels), 0.0, portrait))
                })
                .collect();
            rank(&mut found);
            if !found.is_empty() {
                self.board
                    .people
                    .portraits
                    .insert(person, found.into_iter().take(KEPT).map(|f| f.2).collect());
            }
        }
    }
    /// A person's faces, clearest first, with those of people marked as them.
    fn portraits_of(&self, person: &str) -> Vec<Portrait> {
        let people = &self.board.people;
        let mut faces = people.portraits.get(person).cloned().unwrap_or_default();
        for (other, list) in &people.portraits {
            if other != person && people.canonical(other) == person {
                faces.extend(list);
            }
        }
        faces
    }
    /// A face cut from the photo's thumbnail, or with `large` from its
    /// display image once that has loaded off the UI thread (the thumbnail
    /// stands in meanwhile).
    fn face_image(&mut self, portrait: Portrait, large: bool) -> Option<FaceImage> {
        let texture = large
            .then(|| self.texture(portrait.id, true))
            .flatten()
            .or_else(|| self.texture(portrait.id, false))?;
        Some(FaceImage {
            texture: texture.id(),
            uv: face_uv(portrait.bbox, texture.size_vec2()),
        })
    }
    /// The faces to draw for a folder; empty until their thumbnails are in.
    pub(super) fn folder_faces(&mut self, path: &str, points: f32) -> Vec<FaceImage> {
        let Some(people) = self.board.people.folders.get(path).cloned() else {
            return Vec::new();
        };
        people
            .iter()
            .take(2)
            .filter_map(|person| {
                let portrait = *self.portraits_of(person).first()?;
                self.sharp_face(portrait, points)
                    .or_else(|| self.face_image(portrait, false))
            })
            .collect()
    }
    /// Each person in a folder with their name and up to five faces, for
    /// the hover card. Asks for their sharp crops only while it shows; the
    /// current, softer faces stand in until they arrive.
    pub(super) fn folder_card(&mut self, path: &str, points: f32) -> Vec<(String, Vec<FaceImage>)> {
        let Some(people) = self.board.people.folders.get(path).cloned() else {
            return Vec::new();
        };
        let names = self.person_names();
        // The card's faces go first in line, ahead of the sidebar's.
        let sidebar = std::mem::take(&mut self.board.people.wanted);
        let card = people
            .iter()
            .take(2)
            .map(|person| {
                let faces = self
                    .portraits_of(person)
                    .into_iter()
                    .take(KEPT)
                    .enumerate()
                    .filter_map(|(i, portrait)| {
                        self.sharp_face(portrait, points)
                            .or_else(|| self.face_image(portrait, i == 0))
                    })
                    .collect();
                (folder_name(std::slice::from_ref(person), &names), faces)
            })
            .filter(|(_, faces): &(String, Vec<FaceImage>)| !faces.is_empty())
            .collect();
        self.board.people.wanted.extend(sidebar);
        card
    }
    /// The region of the original photo, in its shown orientation, that a
    /// face's circle covers, and the request that decodes it at the card's
    /// size. `None` for a video or before the photo's size is known.
    fn crop_request(&self, portrait: Portrait, points: f32) -> Option<InspectionRequest> {
        let media = &self.media[*self.index.get(&portrait.id)?];
        let meta = self.metadata.get(&portrait.id)?;
        if media.kind == MediaKind::Video || meta.width == 0 || meta.height == 0 {
            return None;
        }
        let (w, h) = if (5..=8).contains(&meta.orientation) {
            (meta.height, meta.width)
        } else {
            (meta.width, meta.height)
        };
        let uv = face_uv(portrait.bbox, egui::vec2(w as f32, h as f32));
        let x = ((uv.min.x * w as f32).floor() as u32).min(w - 1);
        let y = ((uv.min.y * h as f32).floor() as u32).min(h - 1);
        let region = inspection::Region {
            x,
            y,
            width: ((uv.width() * w as f32).round() as u32).clamp(1, w - x),
            height: ((uv.height() * h as f32).round() as u32).clamp(1, h - y),
        };
        // Decoded at the card's size, never larger than the photo has.
        let output_edge = ((CARD_FACE * points).ceil() as u32)
            .min(region.width.max(region.height))
            .max(1);
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(
            &(
                "face-crop",
                portrait.id.0,
                x,
                y,
                region.width,
                region.height,
                output_edge,
            ),
            &mut hasher,
        );
        Some(InspectionRequest {
            // Apart from the preview's own slots.
            slot: std::hash::Hasher::finish(&hasher) | 1 << 63,
            media: media.clone(),
            region,
            output_edge,
        })
    }
    /// A face cut sharp from the original photo: from the cache, or as soon
    /// as its decode, asked for here and done off the UI thread, arrives.
    pub(super) fn sharp_face(&mut self, portrait: Portrait, points: f32) -> Option<FaceImage> {
        let request = self.crop_request(portrait, points)?;
        let r = request.region;
        let key = (portrait.id, [r.x, r.y, r.width, r.height]);
        let texture = if let Some(texture) = self.board.people.crops.get(&key) {
            texture.clone()
        } else if let Some(texture) = self
            .inspection_frames
            .get(&request.slot)
            .filter(|frame| frame.request == request)
            .and_then(|frame| frame.texture.clone())
        {
            let crops = &mut self.board.people.crops;
            if crops.len() >= SHARP_KEPT {
                crops.clear();
            }
            crops.insert(key, texture.clone());
            texture
        } else {
            if !self.board.people.wanted.contains(&request) {
                self.board.people.wanted.push(request);
            }
            return None;
        };
        Some(FaceImage {
            texture: texture.id(),
            uv: egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        })
    }
    /// Hands this frame's sharp-crop requests to the decoder, after the
    /// preview's own detail requests and only a few at a time.
    pub(super) fn request_sharp_faces(&mut self) {
        let wanted = std::mem::take(&mut self.board.people.wanted);
        self.inspection_wanted
            .extend(wanted.into_iter().take(SHARP_REQUESTS));
    }
    /// Names given to people: a one-person folder called something other
    /// than "Person 003" names that person.
    pub(super) fn person_names(&self) -> HashMap<String, String> {
        self.board
            .people
            .folders
            .iter()
            .filter_map(|(path, people)| match people.as_slice() {
                [one] if leaf(path) != format!("Person {one}") => {
                    Some((one.clone(), leaf(path).to_owned()))
                }
                _ => None,
            })
            .collect()
    }
    /// A one-person folder still named "Person 003": it can take a name.
    pub(super) fn unnamed_person(&self, path: &str) -> bool {
        matches!(
            self.board.people.folders.get(path).map(Vec::as_slice),
            Some([one]) if leaf(path) == format!("Person {one}")
        )
    }
    /// The one person a folder shows, if it shows exactly one.
    pub(super) fn folder_person(&self, path: &str) -> Option<&String> {
        match self.board.people.folders.get(path).map(Vec::as_slice) {
            Some([one]) => Some(one),
            _ => None,
        }
    }
    /// Other one-person folders `path` could be the same person as.
    pub(super) fn same_person_choices(&self, path: &str) -> Vec<String> {
        let Some(person) = self.folder_person(path) else {
            return Vec::new();
        };
        let mut choices: Vec<String> = self
            .board
            .people
            .folders
            .iter()
            .filter(|(other, people)| {
                other.as_str() != path && people.len() == 1 && people[0] != *person
            })
            .map(|(other, _)| other.clone())
            .collect();
        choices.sort();
        choices
    }
    /// After `path`, a one-person folder, is renamed to `name`, renames
    /// every folder with that person in it whose name the grouping made, so
    /// "People 001 + 002" becomes "Person 001 + Ana". All or nothing.
    pub(super) fn rename_person(&mut self, path: &str, name: &str) -> anyhow::Result<String> {
        let people = self.board.people.folders.clone();
        let person = self.folder_person(path).cloned();
        let before = self.person_names();
        let mut after = before.clone();
        if let Some(person) = &person {
            after.insert(person.clone(), name.to_owned());
        }
        let draft = self
            .organize_drafts
            .get(&self.group_mode)
            .ok_or_else(|| anyhow::anyhow!("No folders to rename"))?;
        let mut next = draft.clone();
        let renamed = next.rename_folder(path, name)?;
        if let Some(person) = &person {
            let mut others: Vec<(&String, &Vec<String>)> = people
                .iter()
                .filter(|(other, group)| {
                    other.as_str() != path && group.len() > 1 && group.contains(person)
                })
                .collect();
            others.sort();
            for (other, group) in others {
                // A name typed for this folder stays.
                if leaf(other) != folder_name(group, &before) {
                    continue;
                }
                let target = folder_name(group, &after);
                // A folder moved by the rename above is found under its new path.
                let current = if other.starts_with(&format!("{path}/")) {
                    format!("{renamed}{}", &other[path.len()..])
                } else {
                    other.clone()
                };
                next.rename_folder(&current, &target)?;
            }
        }
        *self.organize_drafts.get_mut(&self.group_mode).unwrap() = next;
        Ok(renamed)
    }
    /// Marks the person of folder `from` as the person of folder `into`.
    /// Every folder either is in is renamed for the people it now shows,
    /// and folders that end up showing the same people combine: "Person 003"
    /// joins "Person 001", "People 002 + 003" joins "People 001 + 002".
    /// Folders named by hand keep their names. A staged edit, all or nothing.
    pub(super) fn same_person(&mut self, from: &str, into: &str) -> anyhow::Result<String> {
        let (Some(a), Some(b)) = (self.folder_person(from), self.folder_person(into)) else {
            anyhow::bail!("Choose two folders of one person each");
        };
        let (a, b) = (a.clone(), b.clone());
        anyhow::ensure!(a != b, "These folders are already the same person");
        anyhow::ensure!(self.board_draft().is_some(), "Folders cannot change now");
        let folders = self.board.people.folders.clone();
        let before = self.person_names();
        let mut after = before.clone();
        // The person keeps a name either of them had, theirs first.
        let name = after.remove(&a);
        if let Some(name) = name
            && !after.contains_key(&b)
        {
            after.insert(b.clone(), name);
        }
        let mut next = self.organize_drafts[&self.group_mode].clone();
        let mut touched: Vec<(&String, &Vec<String>)> = folders
            .iter()
            .filter(|(_, people)| people.contains(&a) || people.contains(&b))
            .collect();
        touched.sort();
        for (path, people) in touched {
            if !next.folders().contains(path.as_str()) {
                continue;
            }
            let mut merged: Vec<String> = people
                .iter()
                .map(|p| if *p == a { b.clone() } else { p.clone() })
                .collect();
            merged.sort();
            merged.dedup();
            if leaf(path) != folder_name(people, &before) {
                continue;
            }
            let name = folder_name(&merged, &after);
            let target = match path.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/{name}"),
                None => name.clone(),
            };
            if target == *path {
                continue;
            }
            if next.folders().contains(&target) {
                next.merge_folder(path, &target)?;
            } else {
                next.rename_folder(path, &name)?;
            }
        }
        *self.organize_drafts.get_mut(&self.group_mode).unwrap() = next;
        let people = &mut self.board.people;
        for target in people.merged.values_mut() {
            if *target == a {
                *target = b.clone();
            }
        }
        people.merged.insert(a.clone(), b.clone());
        Ok(folder_name(std::slice::from_ref(&b), &after))
    }
}

/// The clearest faces of each person among their candidate photos, read
/// from the face evidence the Faces grouping cached. Nothing is decoded or
/// written; a miss (cache off, an old record) leaves the person out.
fn look_up_portraits(root: Option<PathBuf>, jobs: Vec<(String, Vec<Candidate>)>) -> Portraits {
    let Some(store) = root.and_then(|root| Store::new(root, true).ok()) else {
        return HashMap::new();
    };
    let signature = crate::core::models::face_signature();
    jobs.into_iter()
        .filter_map(|(person, candidates)| {
            let mut found: Vec<(f32, f32, Portrait)> = candidates
                .iter()
                .filter_map(|candidate| {
                    let media = &candidate.media;
                    let source = crate::core::pipeline::Pipeline::key(media).ok()?;
                    let faces = store.faces(&Store::face_key(&source, &signature))?;
                    // One person in the photo: the largest face is theirs.
                    let face = faces.geometry.iter().max_by(|a, b| {
                        let area = |f: &Face| (f.bbox[2] - f.bbox[0]) * (f.bbox[3] - f.bbox[1]);
                        area(a).total_cmp(&area(b))
                    })?;
                    Some((
                        face_pixels(face.bbox, candidate.pixels),
                        quality(face),
                        Portrait {
                            id: media.id,
                            bbox: face.bbox,
                        },
                    ))
                })
                .collect();
            rank(&mut found);
            (!found.is_empty())
                .then(|| (person, found.into_iter().take(KEPT).map(|f| f.2).collect()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::faces::Faces;

    fn face(bbox: [f32; 4], eyes: [[f32; 2]; 2], blink: [f32; 2]) -> Face {
        Face {
            bbox,
            eyes,
            landmarks: vec![[0.5, 0.5]],
            blink: Some(blink),
        }
    }

    #[test]
    fn folder_names_say_who_is_in_them_and_take_given_names() {
        assert_eq!(people_in("Person 003"), Some(vec!["003".to_owned()]));
        assert_eq!(
            people_in("People 001 + 003"),
            Some(vec!["001".to_owned(), "003".to_owned()])
        );
        for other in ["No people", "Person", "Person Ana", "People 001 + "] {
            assert_eq!(people_in(other), None, "{other}");
        }
        let pair = ["001".to_owned(), "002".to_owned()];
        let mut names = HashMap::new();
        assert_eq!(folder_name(&pair, &names), "People 001 + 002");
        assert_eq!(folder_name(&pair[1..], &names), "Person 002");
        names.insert("002".to_owned(), "Ana".to_owned());
        assert_eq!(folder_name(&pair, &names), "Person 001 + Ana");
        assert_eq!(folder_name(&pair[1..], &names), "Ana");
        names.insert("001".to_owned(), "Bo".to_owned());
        assert_eq!(folder_name(&pair, &names), "Bo + Ana");
    }

    #[test]
    fn a_clear_face_is_large_frontal_open_eyed_and_whole() {
        let level = [[0.42, 0.4], [0.58, 0.4]];
        let good = face([0.3, 0.2, 0.7, 0.6], level, [0.0, 0.0]);
        let small = face(
            [0.45, 0.35, 0.55, 0.45],
            [[0.47, 0.4], [0.53, 0.4]],
            [0.0, 0.0],
        );
        let turned = face(
            [0.3, 0.2, 0.7, 0.6],
            [[0.6, 0.35], [0.68, 0.45]],
            [0.0, 0.0],
        );
        let shut = face([0.3, 0.2, 0.7, 0.6], level, [0.9, 0.95]);
        let cut = face(
            [-0.1, 0.2, 0.3, 0.6],
            [[0.02, 0.4], [0.18, 0.4]],
            [0.0, 0.0],
        );
        assert!(face_pixels(good.bbox, None) > face_pixels(small.bbox, None));
        for (worse, why) in [
            (&turned, "turned away"),
            (&shut, "eyes shut"),
            (&cut, "cut off"),
        ] {
            assert!(quality(&good) > quality(worse), "{why}");
        }
        assert_eq!(quality(&face([0.5, 0.5, 0.5, 0.6], level, [0.0; 2])), 0.0);
    }

    #[test]
    fn a_face_crop_is_square_on_screen_and_stays_inside_the_photo() {
        // A landscape thumbnail: the crop is square in pixels, not in units.
        let size = egui::vec2(400.0, 200.0);
        let uv = face_uv([0.4, 0.4, 0.5, 0.6], size);
        let pixels = egui::vec2(uv.width() * size.x, uv.height() * size.y);
        assert!((pixels.x - pixels.y).abs() < 0.01, "{pixels:?}");
        assert!(
            pixels.x >= 40.0 * 1.09,
            "just around the face, so it fills the circle"
        );
        assert!(pixels.x <= 40.0 * 1.2);
        // A face at the edge is moved in, not squashed.
        let edge = face_uv([0.0, 0.8, 0.1, 1.0], size);
        assert!(edge.min.x >= 0.0 && edge.max.y <= 1.0 + 1e-6, "{edge:?}");
        assert!((edge.width() * size.x - edge.height() * size.y).abs() < 0.01);
    }

    #[test]
    fn portraits_come_from_the_cached_face_evidence_without_decoding() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("cache");
        let media: Vec<Media> = (0..3)
            .map(|i| {
                let path = temp.path().join(format!("p{i}.png"));
                image::RgbaImage::new(8, 8).save(&path).unwrap();
                Media {
                    id: MediaId(i),
                    path: path.clone(),
                    kind: MediaKind::Photo,
                    fingerprint: crate::core::model::Fingerprint::read(&path).unwrap(),
                }
            })
            .collect();
        let store = Store::new(root.clone(), true).unwrap();
        let signature = crate::core::models::face_signature();
        let level = [[0.42, 0.4], [0.58, 0.4]];
        // p0 has a small face, p1 a large one plus a background face; p2
        // has no record, as with the disk cache off.
        for (index, faces) in [
            (0, vec![face([0.45, 0.35, 0.55, 0.45], level, [0.0; 2])]),
            (
                1,
                vec![
                    face([0.3, 0.2, 0.7, 0.6], level, [0.0; 2]),
                    face([0.8, 0.8, 0.85, 0.85], level, [0.0; 2]),
                ],
            ),
        ] {
            let key = crate::core::pipeline::Pipeline::key(&media[index]).unwrap();
            let faces = Faces {
                count: faces.len(),
                closed_eyes: 0,
                primary: Some(faces[0].bbox),
                geometry: faces,
            };
            store
                .put_faces(&Store::face_key(&key, &signature), &faces)
                .unwrap();
        }
        drop(store);
        let candidates = |list: &[Media]| -> Vec<Candidate> {
            list.iter()
                .map(|media| Candidate {
                    media: media.clone(),
                    pixels: Some(12_000_000.0),
                })
                .collect()
        };
        let found = look_up_portraits(
            Some(root.clone()),
            vec![
                ("001".into(), candidates(&media)),
                ("002".into(), candidates(&media[2..])),
            ],
        );
        // Clearest first: p1's large face, then p0's small one.
        assert_eq!(
            found.get("001"),
            Some(&vec![
                Portrait {
                    id: MediaId(1),
                    bbox: [0.3, 0.2, 0.7, 0.6]
                },
                Portrait {
                    id: MediaId(0),
                    bbox: [0.45, 0.35, 0.55, 0.45]
                }
            ])
        );
        assert!(!found.contains_key("002"));
        assert!(look_up_portraits(None, vec![("001".into(), candidates(&media))]).is_empty());
    }

    #[test]
    fn the_face_with_the_most_source_pixels_leads_and_clarity_breaks_ties() {
        let level = [[0.35, 0.3], [0.45, 0.3]];
        let portrait = |n| Portrait {
            id: MediaId(n),
            bbox: [0.0; 4],
        };
        let clear = quality(&face([0.3, 0.2, 0.5, 0.4], level, [0.0; 2]));
        let shut = quality(&face([0.3, 0.2, 0.5, 0.4], level, [0.9; 2]));
        // The same face box in a 48 MP photo has four times the pixels of a
        // 12 MP one, and wins even with its eyes shut.
        let big = face_pixels([0.3, 0.2, 0.5, 0.4], Some(48e6));
        let small = face_pixels([0.3, 0.2, 0.5, 0.4], Some(12e6));
        let mut found = vec![(small, clear, portrait(1)), (big, shut, portrait(2))];
        rank(&mut found);
        assert_eq!(found[0].2.id, MediaId(2));
        // About the same size: the clearer face.
        let mut found = vec![
            (small, shut, portrait(1)),
            (small * 1.05, shut, portrait(2)),
            (small, clear, portrait(3)),
        ];
        rank(&mut found);
        assert_eq!(found[0].2.id, MediaId(3));
        // Unknown size counts as a phone photo.
        assert_eq!(face_pixels([0.0, 0.0, 0.5, 0.5], None), 3_000_000.0);
    }

    #[test]
    fn the_card_hugs_one_large_face_and_a_row_of_smaller_ones() {
        // A face alone, or with one more, keeps the large face's width.
        assert_eq!(card_width(1), CARD_FACE);
        assert_eq!(card_width(2), CARD_FACE);
        // Three more: 3 x 56 + 2 gaps; four or more: the row of four.
        assert_eq!(card_width(4), 3.0 * CARD_MORE + 2.0 * CARD_GAP);
        assert_eq!(card_width(5), 4.0 * CARD_MORE + 3.0 * CARD_GAP);
        assert_eq!(card_width(9), card_width(5));
        assert_eq!((CARD_FACE, CARD_MORE), (128.0, 56.0));
    }
}
