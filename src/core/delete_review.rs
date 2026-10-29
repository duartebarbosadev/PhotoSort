//! Easy Delete decisions, independent of presentation and shared with file planning.
//!
//! A suggestion is the default decision: once per photo, its suggested Trash
//! marks are staged in the shared decisions, which Review changes shows and
//! applies. The user changes only the ones they disagree with; each card can
//! go back to its suggestion.
use super::eyes::{self, Blink};
use super::faces::EyeState;
use super::model::{Decision, MediaId, update_trash};
use super::near_duplicates::batch::{Keeper, Kind};
use std::collections::{HashMap, HashSet};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Category {
    Exact,
    /// One shot saved as a JPEG and a HEIC.
    Formats,
    Near,
    /// Someone's eyes are closed: with a better shot of the same moment, a
    /// Trash suggestion; without one, kept, for information.
    EyesClosed,
    Blur,
    Dark,
    Bright,
}
impl Category {
    pub const ALL: [Self; 7] = [
        Self::Exact,
        Self::Formats,
        Self::Near,
        Self::EyesClosed,
        Self::Blur,
        Self::Dark,
        Self::Bright,
    ];
    /// The group's tab.
    pub fn title(self) -> &'static str {
        match self {
            Self::Exact => "Exact copies",
            Self::Formats => "Two formats",
            Self::Near => "Near-duplicates",
            Self::EyesClosed => "Eyes closed",
            Self::Blur => "Blurry",
            Self::Dark => "Too dark",
            Self::Bright => "Too bright",
        }
    }
    /// What the group is, in a few words.
    pub fn heading(self) -> &'static str {
        match self {
            Self::Formats => "Same photo, two formats",
            other => other.title(),
        }
    }
}

/// Which file of a JPEG + HEIC pair to keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatChoice {
    Both,
    Heic,
    Jpeg,
}
impl FormatChoice {
    pub const ALL: [Self; 3] = [Self::Both, Self::Heic, Self::Jpeg];
    pub fn title(self) -> &'static str {
        match self {
            Self::Both => "Keep both",
            Self::Heic => "Keep HEIC",
            Self::Jpeg => "Keep JPEG",
        }
    }
    /// Keep flags for (JPEG, HEIC).
    fn keeps(self) -> [bool; 2] {
        match self {
            Self::Both => [true, true],
            Self::Heic => [false, true],
            Self::Jpeg => [true, false],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The suggested discard; for two formats, the JPEG.
    pub subject: MediaId,
    /// The suggested keeper; for two formats, the HEIC.
    pub companion: Option<MediaId>,
    pub category: Category,
    /// A few words on the card, such as "Sharper kept".
    pub tag: String,
    /// The full explanation, in the tag's tooltip.
    pub reason: String,
    /// Why the suggested keeper is the one to keep, shown when the pointer
    /// is over its Suggested mark. Empty when no single photo is suggested.
    pub keep_why: String,
    /// For closed eyes: where the eyes are in each photo, to show zoomed.
    pub eyes: Vec<EyeCrop>,
    /// More photos that share the subject's suggestion, each on the card:
    /// the other closed-eyes shots of a moment with one better shot, such as
    /// an identical copy or the next frame of a burst.
    pub extra: Vec<MediaId>,
}
/// The eyes of one person in one photo of a card.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeCrop {
    pub id: MediaId,
    /// Normalized `[left, top, right, bottom]` in the shown photo.
    pub rect: [f32; 4],
    pub closed: bool,
}
impl Entry {
    /// Closed eyes with no better shot: kept, and shown for information.
    pub fn informational(&self) -> bool {
        self.category == Category::EyesClosed && self.companion.is_none()
    }
    /// Every photo of the card, each with its own decision: the subject,
    /// the other photos that share its suggestion, then the companion.
    pub fn candidates(&self) -> Vec<MediaId> {
        std::iter::once(self.subject)
            .chain(self.extra.iter().copied())
            .chain(self.companion.filter(|id| *id != self.subject))
            .collect()
    }
    /// The suggested Keep flag of each candidate. Two formats of one shot
    /// keep both: neither is worse, and which to drop is a preference. A
    /// closed-eyes shot with no better one is kept: it may be the only one.
    pub fn suggestion(&self) -> Vec<bool> {
        self.candidates()
            .iter()
            .map(|id| {
                self.category == Category::Formats
                    || self.informational()
                    || (*id != self.subject && !self.extra.contains(id))
            })
            .collect()
    }
}

fn kept(decisions: &HashMap<MediaId, Decision>, id: MediaId) -> bool {
    !decisions.get(&id).is_some_and(|d| d.rejected)
}

#[derive(Default)]
pub struct Review {
    entries: Vec<Entry>,
    index: HashMap<MediaId, usize>,
    /// Photos whose suggestion was staged once; see `Session::suggested`.
    suggested: HashSet<MediaId>,
}
impl Review {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn entry(&self, id: MediaId) -> Option<&Entry> {
        self.index.get(&id).map(|i| &self.entries[*i])
    }
    /// The caller supplies category/capture ordering.
    pub fn replace(&mut self, entries: Vec<Entry>) {
        self.index = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.subject, i))
            .collect();
        self.entries = entries;
    }
    pub fn suggested(&self) -> &HashSet<MediaId> {
        &self.suggested
    }
    pub fn restore_suggested(&mut self, ids: HashSet<MediaId>) {
        self.suggested = ids;
    }
    /// Stages the suggested Trash marks of photos seen for the first time.
    /// Never unmarks anything, and never repeats for a photo: a later
    /// analysis leaves the user's choices alone. A card whose suggested
    /// keeper is already marked for Trash (by Pick Best or by hand) is left
    /// as it is, so both photos never go to Trash on their own. Returns the
    /// photos marked.
    pub fn stage_new_suggestions(
        &mut self,
        decisions: &mut HashMap<MediaId, Decision>,
    ) -> Vec<MediaId> {
        let mut marked = Vec::new();
        for entry in &self.entries {
            let keeper_kept = entry.companion.is_none_or(|id| kept(decisions, id));
            for (id, keep) in entry.candidates().into_iter().zip(entry.suggestion()) {
                if self.suggested.insert(id) && !keep && keeper_kept {
                    marked.push(id);
                }
            }
        }
        update_trash(decisions, marked.iter().map(|id| (*id, true)));
        marked
    }
    pub fn keeps(decisions: &HashMap<MediaId, Decision>, candidate: MediaId) -> bool {
        kept(decisions, candidate)
    }
    /// Whether a card's decisions differ from its suggestion.
    pub fn changed(&self, id: MediaId, decisions: &HashMap<MediaId, Decision>) -> bool {
        self.entry(id).is_some_and(|e| {
            e.candidates()
                .into_iter()
                .zip(e.suggestion())
                .any(|(id, keep)| kept(decisions, id) != keep)
        })
    }
    pub fn toggle(
        &self,
        id: MediaId,
        candidate: MediaId,
        decisions: &mut HashMap<MediaId, Decision>,
    ) {
        if self
            .entry(id)
            .is_some_and(|e| e.candidates().contains(&candidate))
        {
            update_trash(decisions, [(candidate, kept(decisions, candidate))]);
        }
    }
    /// Puts each card back to its suggestion.
    pub fn accept(&self, ids: &[MediaId], decisions: &mut HashMap<MediaId, Decision>) {
        let marks: Vec<_> = ids
            .iter()
            .filter_map(|id| self.entry(*id))
            .flat_map(|e| e.candidates().into_iter().zip(e.suggestion()))
            .map(|(id, keep)| (id, !keep))
            .collect();
        update_trash(decisions, marks);
    }
    /// How many of these cards differ from their suggestion.
    pub fn changed_count(&self, ids: &[MediaId], decisions: &HashMap<MediaId, Decision>) -> usize {
        ids.iter()
            .filter(|id| self.changed(**id, decisions))
            .count()
    }
    /// Applies one choice to every JPEG + HEIC card among `ids`.
    pub fn choose_formats(
        &self,
        ids: &[MediaId],
        choice: FormatChoice,
        decisions: &mut HashMap<MediaId, Decision>,
    ) {
        let marks: Vec<_> = self
            .format_pairs(ids)
            .flat_map(|pair| pair.into_iter().zip(choice.keeps()))
            .map(|(id, keep)| (id, !keep))
            .collect();
        update_trash(decisions, marks);
    }
    /// The choice every JPEG + HEIC card among `ids` shares, if they agree.
    pub fn format_choice(
        &self,
        ids: &[MediaId],
        decisions: &HashMap<MediaId, Decision>,
    ) -> Option<FormatChoice> {
        let mut choices = self.format_pairs(ids).map(|[jpeg, heic]| {
            let keeps = [kept(decisions, jpeg), kept(decisions, heic)];
            FormatChoice::ALL.into_iter().find(|c| c.keeps() == keeps)
        });
        let first = choices.next()??;
        choices.all(|c| c == Some(first)).then_some(first)
    }
    fn format_pairs<'a>(&'a self, ids: &'a [MediaId]) -> impl Iterator<Item = [MediaId; 2]> + 'a {
        ids.iter()
            .filter_map(|id| self.entry(*id))
            .filter(|e| e.category == Category::Formats)
            .filter_map(|e| Some([e.subject, e.companion?]))
    }
}

/// Adapt the application's existing analysis output; never scan or decode here.
pub fn from_analysis(
    media: &[super::model::Media],
    analysis: &HashMap<MediaId, super::analysis::Analysis>,
    metadata: &HashMap<MediaId, super::model::Metadata>,
    pairs: &[super::near_duplicates::batch::Pair],
    blinks: &[Blink],
) -> Vec<Entry> {
    let mut entries = Vec::new();
    let present: HashSet<_> = media.iter().map(|m| m.id).collect();
    for item in media {
        let Some(a) = analysis.get(&item.id) else {
            continue;
        };
        if let Some(issue) = a.issues.first() {
            let category = match issue.as_str() {
                "Very dark" => Category::Dark,
                "Possibly soft" => Category::Blur,
                "Overexposed" => Category::Bright,
                _ => continue,
            };
            entries.push(Entry {
                subject: item.id,
                companion: None,
                category,
                tag: category.title().into(),
                keep_why: String::new(),
                eyes: Vec::new(),
                extra: Vec::new(),
                reason: format!(
                    "{issue}: sharpness {:.1}, brightness {:.1}/255. PhotoSort suggests moving it to Trash.",
                    a.sharpness, a.brightness
                ),
            });
        }
    }
    for pair in pairs {
        if !(pair.kind == Kind::Formats || pair.assessment.accepted())
            || pair.discard == pair.keep
            || !present.contains(&pair.discard)
            || !present.contains(&pair.keep)
            || [pair.discard, pair.keep]
                .iter()
                .any(|id| !analysis.contains_key(id))
        {
            continue;
        }
        // A pair outranks a single-photo issue for either member: comparing the
        // two is the more useful review, and the keeper is already the sharper
        // one. Say so instead of hiding the duplicate behind the issue.
        let keep_why = match (pair.kind, pair.keeper) {
            (Kind::Formats, _) => "",
            (Kind::Exact, _) => {
                "The two files are identical, so either can go. This one keeps its place in the library."
            }
            (Kind::Near, Keeper::Sharper) => "It is the sharper photo of the two.",
            (Kind::Near, Keeper::MoreMetadata) => {
                "Both are equally sharp, and this one carries more metadata, such as camera settings and location."
            }
            (Kind::Near, Keeper::LargerFile) => {
                "Both are equally sharp, and this one is the larger file, so it holds more detail."
            }
            (Kind::Near, Keeper::First) => {
                "Nothing tells the two apart, so PhotoSort keeps the one that comes first in the library."
            }
        };
        let (category, tag) = match (pair.kind, pair.keeper) {
            (Kind::Exact, _) => (Category::Exact, "Identical copy"),
            (Kind::Formats, _) => (Category::Formats, "JPEG + HEIC"),
            (Kind::Near, Keeper::Sharper) => (Category::Near, "Sharper kept"),
            (Kind::Near, Keeper::MoreMetadata) => (Category::Near, "Fuller metadata kept"),
            (Kind::Near, Keeper::LargerFile) => (Category::Near, "Larger file kept"),
            (Kind::Near, Keeper::First) => (Category::Near, "Near-duplicate"),
        };
        let mut reason = pair.reason.replace(" · ", ". ");
        reason.push('.');
        if category != Category::Formats {
            for (role, id) in [("suggested discard", pair.discard), ("keeper", pair.keep)] {
                if let Some(issue) = analysis[&id].issues.first() {
                    reason.push_str(&format!(" The {role} is also {}.", issue.to_lowercase()));
                }
            }
        }
        entries.push(Entry {
            subject: pair.discard,
            companion: Some(pair.keep),
            category,
            tag: tag.into(),
            reason: sentences(&reason),
            keep_why: keep_why.into(),
            eyes: Vec::new(),
            extra: Vec::new(),
        });
    }
    entries.extend(eyes_entries(media, analysis, pairs, blinks));
    let order: HashMap<_, _> = media
        .iter()
        .map(|m| (m.id, super::dates::capture_order(m, metadata.get(&m.id))))
        .collect();
    // Choose ownership before capture sorting so metadata updates cannot
    // transfer a pending decision to the other image in the pair. Each photo
    // belongs to one card, the first by precedence, with one exception: the
    // better shot of an Eyes closed card, kept there, may also be kept on one
    // more card of its own (its identical copy, its other format) and be the
    // better shot of other Eyes closed cards. A card that would show a photo
    // a second time otherwise is dropped, so a photo never gets two
    // suggestions and Trash comes from one card only.
    entries.sort_by_key(precedence);
    // Each photo's card count and whether it is an Eyes closed keeper.
    let mut owned: HashMap<MediaId, (usize, bool)> = HashMap::new();
    entries.retain(|e| {
        let better = e.category == Category::EyesClosed && e.companion.is_some();
        let suggestion: Vec<_> = e.candidates().into_iter().zip(e.suggestion()).collect();
        let fits = suggestion.iter().all(|(id, keep)| match owned.get(id) {
            None => true,
            // Another Eyes closed card with the same better shot.
            Some((_, true)) if better && e.companion == Some(*id) => true,
            // The better shot, kept on one card of its own as well.
            Some((1, true)) => *keep && !better && !e.informational(),
            _ => false,
        });
        if fits {
            for (id, _) in suggestion {
                let keeper = better && e.companion == Some(id);
                let slot = owned.entry(id).or_insert((0, false));
                slot.0 += 1;
                slot.1 |= keeper;
            }
        }
        fits
    });
    entries.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| order[&a.subject].cmp(&order[&b.subject]))
    });
    entries
}

/// Which card keeps a photo shared by several. A closed-eyes shot with a
/// better shot of the same moment goes first: its exact copy, its other
/// format or its near-duplicate shows the same closed eyes, so each gets its
/// own Eyes closed card and goes to Trash for the better shot. Copies and
/// formats come next (they are certain), then near-duplicates, then single
/// problem photos. A closed-eyes shot with no better shot is only a note,
/// shown when no other card has the photo.
fn precedence(entry: &Entry) -> u8 {
    match entry.category {
        Category::EyesClosed if entry.companion.is_some() => 0,
        Category::Exact => 1,
        Category::Formats => 2,
        Category::Near => 3,
        Category::Blur | Category::Dark | Category::Bright => 4,
        Category::EyesClosed => 5,
    }
}

/// A closed-eyes shot, with its better shot and the people matched in both.
type Choice<'a> = (
    &'a eyes::Frame,
    Option<(&'a eyes::Frame, Vec<(usize, usize)>)>,
);

/// The most closed-eyes shots on one card, beside their better shot.
const MAX_SHOTS: usize = 3;

/// The most people whose eyes a card shows zoomed.
const EYE_CROPS: usize = 3;

/// Cards for photos with closed eyes. A better shot is another photo of the
/// moment with the same people where everyone whose eyes were closed has
/// them open and nobody is worse off (`eyes::better_shot`). It must not be a
/// photo another card suggests moving to Trash for a reason the eyes do not
/// outrank: a blurry, dark or bright photo, or the copy of an identical pair
/// (its twin is the same shot and is preferred). The keeper of a pair is
/// preferred over its discard, then the closest in time.
///
/// One card per better shot: every closed-eyes shot that chose it, such as
/// an identical copy or the next frame of a burst, is shown together with it
/// instead of repeating the better shot on a card each. A closed-eyes shot
/// with no better shot gets a card of its own that keeps it.
fn eyes_entries(
    media: &[super::model::Media],
    analysis: &HashMap<MediaId, super::analysis::Analysis>,
    pairs: &[super::near_duplicates::batch::Pair],
    blinks: &[Blink],
) -> Vec<Entry> {
    let names: HashMap<_, _> = media.iter().map(|m| (m.id, m.name())).collect();
    let issue = |id: &MediaId| {
        analysis.get(id).is_none_or(|a| {
            a.issues.first().is_some_and(|i| {
                matches!(i.as_str(), "Very dark" | "Possibly soft" | "Overexposed")
            })
        })
    };
    let accepted = || {
        pairs
            .iter()
            .filter(|p| p.kind == Kind::Formats || p.assessment.accepted())
    };
    let discards: HashSet<MediaId> = accepted().map(|p| p.discard).collect();
    let copies: HashSet<MediaId> = accepted()
        .filter(|p| p.kind == Kind::Exact)
        .map(|p| p.discard)
        .collect();
    let identical = |a: MediaId, b: MediaId| {
        accepted().any(|p| {
            p.kind == Kind::Exact
                && ((p.discard, p.keep) == (a, b) || (p.discard, p.keep) == (b, a))
        })
    };
    // Each closed-eyes shot with its better shot, if any.
    let mut chosen: Vec<Choice> = Vec::new();
    for blink in blinks {
        let shot = &blink.frame;
        if !names.contains_key(&shot.id)
            || !analysis.contains_key(&shot.id)
            || !shot.people.iter().any(|p| p.state == EyeState::Closed)
        {
            continue;
        }
        let better = blink
            .others
            .iter()
            .filter(|o| {
                o.id != shot.id
                    && names.contains_key(&o.id)
                    && !issue(&o.id)
                    && !copies.contains(&o.id)
            })
            .filter_map(|o| Some((o, eyes::better_shot(&shot.people, &o.people)?)))
            .min_by_key(|(o, _)| (discards.contains(&o.id), (o.at_ms - shot.at_ms).abs()));
        chosen.push((shot, better));
    }
    let closed_crops = |shot: &eyes::Frame| -> Vec<EyeCrop> {
        shot.people
            .iter()
            .filter(|p| p.state == EyeState::Closed)
            .filter_map(|p| {
                Some(EyeCrop {
                    id: shot.id,
                    rect: p.eyes?,
                    closed: true,
                })
            })
            .take(EYE_CROPS)
            .collect()
    };
    let who = |shot: &eyes::Frame| {
        let closed = shot
            .people
            .iter()
            .filter(|p| p.state == EyeState::Closed)
            .count();
        match (closed, shot.people.len()) {
            (_, 1) => "Eyes closed".to_owned(),
            (1, n) => format!("Eyes closed for 1 of {n} people"),
            (c, n) if c == n => format!("Eyes closed for all {n} people"),
            (c, n) => format!("Eyes closed for {c} of {n} people"),
        }
    };
    let mut entries = Vec::new();
    let mut grouped: Vec<MediaId> = Vec::new();
    for (shot, better) in &chosen {
        let Some((other, matched)) = better else {
            entries.push(Entry {
                subject: shot.id,
                companion: None,
                category: Category::EyesClosed,
                tag: "No shot with eyes open - kept".into(),
                reason: format!(
                    "{}. No other photo of this moment has everyone's eyes open, so PhotoSort keeps it: a photo with closed eyes can be better than none. Choose Trash if you do not want it.",
                    who(shot)
                ),
                keep_why: String::new(),
                eyes: closed_crops(shot),
                extra: Vec::new(),
            });
            continue;
        };
        if grouped.contains(&shot.id) {
            continue;
        }
        // Every closed-eyes shot that chose the same better shot, a few to a
        // card so each one shows at a useful size.
        let members: Vec<&eyes::Frame> = chosen
            .iter()
            .filter(|(_, b)| b.as_ref().is_some_and(|(o, _)| o.id == other.id))
            .map(|(s, _)| *s)
            .collect();
        grouped.extend(members.iter().map(|m| m.id));
        let everyone = if shot.people.len() == 1 {
            "with the eyes open"
        } else {
            "with the same people and everyone's eyes open"
        };
        for group in members.chunks(MAX_SHOTS) {
            let first = group[0];
            let extra: Vec<MediaId> = group[1..].iter().map(|m| m.id).collect();
            let copies = extra.iter().filter(|e| identical(**e, first.id)).count();
            let lead = match (extra.len(), copies) {
                (0, _) => format!("{}.", who(first)),
                (n, 0) => format!("{} in {} shots of this moment.", who(first), n + 1),
                (n, c) if c == n => format!(
                    "{}, in this photo and {}.",
                    who(first),
                    if c == 1 {
                        "an identical copy".to_owned()
                    } else {
                        format!("{c} identical copies")
                    }
                ),
                (n, c) => format!(
                    "{} in {} shots of this moment, {c} of them identical copies.",
                    who(first),
                    n + 1
                ),
            };
            // Each closed-eyes shot's closed eyes, then the same people in
            // the better shot with their eyes open.
            let mut eyes: Vec<EyeCrop> = group.iter().flat_map(|m| closed_crops(m)).collect();
            eyes.extend(
                matched
                    .iter()
                    .filter(|(i, _)| shot.people[*i].state == EyeState::Closed)
                    .filter_map(|(_, j)| {
                        Some(EyeCrop {
                            id: other.id,
                            rect: other.people[*j].eyes?,
                            closed: false,
                        })
                    })
                    .take(EYE_CROPS),
            );
            entries.push(Entry {
                subject: first.id,
                companion: Some(other.id),
                category: Category::EyesClosed,
                tag: "Keep the shot with eyes open".into(),
                reason: format!(
                    "{lead} {} shows the same moment {everyone}. PhotoSort suggests moving the closed-eyes {} to Trash and keeping that one.",
                    names[&other.id],
                    if extra.is_empty() { "shot" } else { "shots" }
                ),
                keep_why: "Everyone's eyes are open here.".into(),
                eyes,
                extra,
            });
        }
    }
    entries
}

/// Starts every sentence with a capital letter.
fn sentences(text: &str) -> String {
    text.split(". ")
        .map(|sentence| {
            let mut chars = sentence.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(". ")
}
