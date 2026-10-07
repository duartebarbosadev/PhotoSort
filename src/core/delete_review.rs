//! Easy Delete decisions, independent of presentation and shared with file planning.
use super::model::{Decision, MediaId, update_trash};
use std::collections::{HashMap, HashSet};

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Category {
    Exact,
    Near,
    Blur,
    Dark,
    Bright,
}
impl Category {
    pub const ALL: [Self; 5] = [
        Self::Exact,
        Self::Near,
        Self::Blur,
        Self::Dark,
        Self::Bright,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Exact => "Duplicates",
            Self::Near => "Near-duplicates",
            Self::Blur => "Blur",
            Self::Dark => "Dark",
            Self::Bright => "Bright",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub subject: MediaId,
    pub companion: Option<MediaId>,
    pub category: Category,
    pub reason: String,
}
impl Entry {
    pub fn candidates(&self) -> Vec<MediaId> {
        std::iter::once(self.subject)
            .chain(self.companion.filter(|id| *id != self.subject))
            .collect()
    }
    fn defaults(&self) -> Vec<bool> {
        self.candidates()
            .iter()
            .map(|id| *id != self.subject)
            .collect()
    }
}
#[derive(Clone, Debug)]
struct State {
    keep: Vec<bool>,
    prior: Option<Vec<bool>>,
}
#[derive(Default)]
pub struct Review {
    entries: Vec<Entry>,
    index: HashMap<MediaId, usize>,
    states: HashMap<MediaId, State>,
    confirmation_order: Vec<MediaId>,
}
impl Review {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    pub fn entry(&self, id: MediaId) -> Option<&Entry> {
        self.index.get(&id).map(|i| &self.entries[*i])
    }
    /// The caller supplies category/capture ordering. Repeated identical results
    /// preserve decisions; changed results forget confirmations without replaying them.
    pub fn replace(&mut self, entries: Vec<Entry>) {
        if self.entries == entries {
            return;
        }
        self.index = entries
            .iter()
            .enumerate()
            .map(|(i, e)| (e.subject, i))
            .collect();
        self.states = entries
            .iter()
            .map(|e| {
                (
                    e.subject,
                    State {
                        keep: e.defaults(),
                        prior: None,
                    },
                )
            })
            .collect();
        self.entries = entries;
        self.confirmation_order.clear();
    }
    pub fn confirmed(&self, id: MediaId) -> bool {
        self.states.get(&id).is_some_and(|s| s.prior.is_some())
    }
    pub fn keeps(&self, review: MediaId, candidate: MediaId) -> bool {
        self.entry(review)
            .and_then(|e| e.candidates().iter().position(|id| *id == candidate))
            .and_then(|i| self.states.get(&review).map(|s| s.keep[i]))
            .unwrap_or(true)
    }
    pub fn unfinished(&self, id: Option<MediaId>) -> bool {
        id.and_then(|id| self.entry(id).zip(self.states.get(&id)))
            .is_some_and(|(e, s)| s.prior.is_none() && s.keep != e.defaults())
    }
    fn revoke(&mut self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) {
        let candidates = self.entry(id).map(Entry::candidates).unwrap_or_default();
        if let Some(prior) = self.states.get_mut(&id).and_then(|s| s.prior.take()) {
            update_trash(decisions, candidates.into_iter().zip(prior));
            self.confirmation_order.retain(|other| *other != id);
        }
    }
    pub fn toggle(
        &mut self,
        id: MediaId,
        candidate: MediaId,
        decisions: &mut HashMap<MediaId, Decision>,
    ) {
        let Some(index) = self
            .entry(id)
            .and_then(|e| e.candidates().iter().position(|other| *other == candidate))
        else {
            return;
        };
        self.revoke(id, decisions);
        self.states.get_mut(&id).unwrap().keep[index] ^= true;
    }
    /// Returns true when newly confirmed, false when revoked or unknown.
    pub fn confirm(&mut self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) -> bool {
        if self.confirmed(id) {
            self.revoke(id, decisions);
            return false;
        }
        let Some(candidates) = self.entry(id).map(Entry::candidates) else {
            return false;
        };
        let state = self.states.get_mut(&id).unwrap();
        state.prior = Some(
            candidates
                .iter()
                .map(|id| decisions.get(id).is_some_and(|d| d.rejected))
                .collect(),
        );
        update_trash(
            decisions,
            candidates
                .into_iter()
                .zip(state.keep.iter().map(|keep| !*keep)),
        );
        self.confirmation_order.push(id);
        true
    }
    pub fn reset(&mut self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) {
        self.revoke(id, decisions);
        if let Some(defaults) = self.entry(id).map(Entry::defaults) {
            self.states.get_mut(&id).unwrap().keep = defaults;
        }
    }
    pub fn reset_all(&mut self, decisions: &mut HashMap<MediaId, Decision>) {
        for id in self.confirmation_order.clone().into_iter().rev() {
            self.revoke(id, decisions);
        }
        for entry in &self.entries {
            self.states.get_mut(&entry.subject).unwrap().keep = entry.defaults();
        }
    }
    /// Batch defaults capture the pre-batch state before publishing any marks.
    pub fn accept_defaults(&mut self, ids: &[MediaId], decisions: &mut HashMap<MediaId, Decision>) {
        let mut marks = HashMap::new();
        for id in ids {
            let Some(entry) = self.entry(*id).cloned() else {
                continue;
            };
            let candidates = entry.candidates();
            let state = self.states.get_mut(id).unwrap();
            if state.prior.is_none() {
                state.prior = Some(
                    candidates
                        .iter()
                        .map(|id| decisions.get(id).is_some_and(|d| d.rejected))
                        .collect(),
                );
                self.confirmation_order.push(*id);
            }
            state.keep = entry.defaults();
            marks.extend(
                candidates
                    .into_iter()
                    .zip(state.keep.iter().map(|keep| !*keep)),
            );
        }
        update_trash(decisions, marks);
    }
    /// Other workflows own their newer changes. A stale review must never undo them.
    pub fn reconcile_external(&mut self, decisions: &HashMap<MediaId, Decision>) {
        let stale: HashSet<_> = self
            .confirmation_order
            .iter()
            .copied()
            .filter(|id| {
                self.entry(*id)
                    .unwrap()
                    .candidates()
                    .iter()
                    .enumerate()
                    .any(|(i, candidate)| {
                        decisions.get(candidate).is_some_and(|d| d.rejected)
                            == self.states[id].keep[i]
                    })
            })
            .collect();
        for id in &stale {
            self.states.get_mut(id).unwrap().prior = None;
        }
        self.confirmation_order.retain(|id| !stale.contains(id));
    }
    /// Called after the application explicitly discards all shared Trash marks.
    pub fn discard_confirmations(&mut self) {
        self.confirmation_order.clear();
        for e in &self.entries {
            self.states.insert(
                e.subject,
                State {
                    keep: e.defaults(),
                    prior: None,
                },
            );
        }
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
                reason: format!(
                    "{issue} · sharpness {:.1} · brightness {:.1}/255",
                    a.sharpness, a.brightness
                ),
            });
        }
    }
    for pair in pairs {
        if !pair.assessment.accepted()
            || pair.discard == pair.keep
            || !present.contains(&pair.discard)
            || !present.contains(&pair.keep)
            || [pair.discard, pair.keep]
                .iter()
                .any(|id| analysis.get(id).is_none_or(|a| !a.issues.is_empty()))
        {
            continue;
        }
        entries.push(Entry {
            subject: pair.discard,
            companion: Some(pair.keep),
            category: if pair.assessment.decision
                == super::near_duplicates::Decision::ExactDuplicate
            {
                Category::Exact
            } else {
                Category::Near
            },
            reason: pair.reason.clone(),
        });
    }
    let order: HashMap<_, _> = media
        .iter()
        .map(|m| {
            let captured = metadata
                .get(&m.id)
                .filter(|m| !m.captured.is_empty())
                .map(|m| m.captured.clone());
            (
                m.id,
                (
                    captured.is_none(),
                    captured.unwrap_or_default(),
                    m.path.clone(),
                ),
            )
        })
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
