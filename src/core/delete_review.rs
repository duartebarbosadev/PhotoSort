//! Easy Delete decisions, independent of presentation and shared with file planning.
//!
//! A suggestion is the default decision: once per photo, its suggested Trash
//! marks are staged in the shared decisions, which Review changes shows and
//! applies. The user changes only the ones they disagree with; each card can
//! go back to its suggestion.
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
    Blur,
    Dark,
    Bright,
}
impl Category {
    pub const ALL: [Self; 6] = [
        Self::Exact,
        Self::Formats,
        Self::Near,
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
}
impl Entry {
    pub fn candidates(&self) -> Vec<MediaId> {
        std::iter::once(self.subject)
            .chain(self.companion.filter(|id| *id != self.subject))
            .collect()
    }
    /// The suggested Keep flag of each candidate. Two formats of one shot
    /// keep both: neither is worse, and which to drop is a preference.
    pub fn suggestion(&self) -> Vec<bool> {
        self.candidates()
            .iter()
            .map(|id| self.category == Category::Formats || *id != self.subject)
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
        });
    }
    let order: HashMap<_, _> = media
        .iter()
        .map(|m| (m.id, super::dates::capture_order(m, metadata.get(&m.id))))
        .collect();
    // Choose pair ownership before capture sorting so metadata updates cannot
    // transfer a pending decision to the other image in the pair.
    entries.sort_by_key(|entry| entry.category);
    let mut seen = HashSet::new();
    entries.retain(|e| {
        if seen.contains(&e.subject) || e.companion.is_some_and(|id| seen.contains(&id)) {
            return false;
        }
        seen.insert(e.subject);
        if let Some(other) = e.companion {
            seen.insert(other);
        }
        true
    });
    entries.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| order[&a.subject].cmp(&order[&b.subject]))
    });
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
