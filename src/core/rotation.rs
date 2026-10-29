//! Fix Rotation's suggestions, independent of presentation.
//!
//! A suggestion is the default decision: once per photo, its quarter turns
//! are staged in the shared decisions, which Review changes shows and
//! applies. A photo turned back to how it is on disk has no staged turn, so
//! it is no change at all. Only photos the model would turn are suggestions.
mod detection;
pub use detection::Detector;

use super::model::{Decision, MediaId};
use std::collections::{HashMap, HashSet};

/// What the model suggests for one photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// Clockwise quarter turns, never 0.
    pub turns: u8,
    /// The rotated views disagree: never staged without the user.
    pub uncertain: bool,
}

/// The staged quarter turns of one photo.
pub fn turns(decisions: &HashMap<MediaId, Decision>, id: MediaId) -> u8 {
    decisions.get(&id).map_or(0, |d| d.turns % 4)
}
/// Stages `turns` for one photo. Back to how it is on disk, nothing is
/// staged: an otherwise empty decision is removed.
pub fn set_turns(decisions: &mut HashMap<MediaId, Decision>, id: MediaId, turns: u8) {
    let decision = decisions.entry(id).or_default();
    decision.turns = turns % 4;
    if decision.is_empty() {
        decisions.remove(&id);
    }
}

#[derive(Clone, Debug, Default)]
pub struct Review {
    order: Vec<MediaId>,
    suggestions: HashMap<MediaId, Suggestion>,
    /// Photos whose suggestion was staged once; see `Session::rotations_staged`.
    staged: HashSet<MediaId>,
}
impl Review {
    pub fn ids(&self) -> &[MediaId] {
        &self.order
    }
    pub fn suggestion(&self, id: MediaId) -> Option<Suggestion> {
        self.suggestions.get(&id).copied()
    }
    pub fn staged(&self) -> &HashSet<MediaId> {
        &self.staged
    }
    pub fn restore_staged(&mut self, ids: HashSet<MediaId>) {
        self.staged = ids;
    }
    pub fn replace_suggestions(&mut self, suggestions: Vec<(MediaId, u8)>) {
        self.replace_predictions(
            suggestions
                .into_iter()
                .map(|(id, turns)| (id, turns, false))
                .collect(),
        );
    }
    /// The caller supplies capture ordering. A photo the model would keep
    /// as it is is not a suggestion.
    pub fn replace_predictions(&mut self, predictions: Vec<(MediaId, u8, bool)>) {
        self.order.clear();
        self.suggestions.clear();
        for (id, turns, uncertain) in predictions {
            let turns = turns % 4;
            if turns != 0 && !self.suggestions.contains_key(&id) {
                self.order.push(id);
                self.suggestions.insert(id, Suggestion { turns, uncertain });
            }
        }
    }
    /// Stages the turns of certain suggestions seen for the first time.
    /// Never repeats for a photo: a later detection leaves the user's
    /// choices alone. Returns the photos turned.
    pub fn stage_new_suggestions(
        &mut self,
        decisions: &mut HashMap<MediaId, Decision>,
    ) -> Vec<MediaId> {
        let mut turned = Vec::new();
        for id in &self.order {
            let suggestion = self.suggestions[id];
            if !suggestion.uncertain && self.staged.insert(*id) {
                set_turns(decisions, *id, suggestion.turns);
                turned.push(*id);
            }
        }
        turned
    }
    /// The turns a photo starts from: its suggestion once staged, otherwise
    /// as it is on disk.
    pub fn default_turns(&self, id: MediaId) -> u8 {
        if self.staged.contains(&id) {
            self.suggestion(id).map_or(0, |s| s.turns)
        } else {
            0
        }
    }
    /// Whether a photo's staged turns differ from where it started.
    pub fn changed(&self, id: MediaId, decisions: &HashMap<MediaId, Decision>) -> bool {
        turns(decisions, id) != self.default_turns(id)
    }
    /// Puts a photo back where it started.
    pub fn undo(&self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) {
        set_turns(decisions, id, self.default_turns(id));
    }
    pub fn relocate(&mut self, moves: &[(MediaId, MediaId, super::model::Fingerprint)]) {
        let mapping: HashMap<_, _> = moves.iter().map(|&(old, new, _)| (old, new)).collect();
        let at = |id: MediaId| mapping.get(&id).copied().unwrap_or(id);
        for id in &mut self.order {
            *id = at(*id);
        }
        self.suggestions = std::mem::take(&mut self.suggestions)
            .into_iter()
            .map(|(id, suggestion)| (at(id), suggestion))
            .collect();
        self.staged = std::mem::take(&mut self.staged)
            .into_iter()
            .map(at)
            .collect();
    }
    pub fn complete(&mut self, completed: &HashSet<MediaId>) {
        self.retain(|id| !completed.contains(&id));
    }
    pub fn retain(&mut self, mut keep: impl FnMut(MediaId) -> bool) {
        self.order.retain(|id| keep(*id));
        let ids: HashSet<_> = self.order.iter().copied().collect();
        self.suggestions.retain(|id, _| ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review() -> Review {
        let mut review = Review::default();
        review.replace_predictions(vec![
            (MediaId(1), 1, false),
            (MediaId(2), 0, false),
            (MediaId(3), 4, false),
            (MediaId(4), 3, true),
        ]);
        review
    }

    #[test]
    fn photos_the_model_would_keep_are_not_suggestions() {
        let review = review();
        assert_eq!(review.ids(), &[MediaId(1), MediaId(4)]);
        assert_eq!(review.suggestion(MediaId(2)), None);
        assert_eq!(review.suggestion(MediaId(3)), None);
    }

    #[test]
    fn certain_suggestions_are_staged_once_and_uncertain_ones_wait() {
        let mut review = review();
        let mut decisions = HashMap::new();
        assert_eq!(review.stage_new_suggestions(&mut decisions), [MediaId(1)]);
        assert_eq!(turns(&decisions, MediaId(1)), 1);
        assert_eq!(turns(&decisions, MediaId(4)), 0);
        assert!(!review.changed(MediaId(1), &decisions));
        assert!(!review.changed(MediaId(4), &decisions));
        // The user keeps the original; a new detection leaves it alone.
        set_turns(&mut decisions, MediaId(1), 0);
        assert!(decisions.is_empty());
        assert!(review.changed(MediaId(1), &decisions));
        review.replace_predictions(vec![(MediaId(1), 1, false)]);
        assert!(review.stage_new_suggestions(&mut decisions).is_empty());
        assert!(decisions.is_empty());
        review.undo(MediaId(1), &mut decisions);
        assert_eq!(turns(&decisions, MediaId(1)), 1);
    }

    #[test]
    fn turning_back_to_the_original_stages_nothing() {
        let mut decisions = HashMap::new();
        set_turns(&mut decisions, MediaId(9), 1);
        set_turns(&mut decisions, MediaId(9), 4);
        assert!(decisions.is_empty());
        // A rating stays staged on its own.
        decisions.entry(MediaId(9)).or_default().rating = Some(3);
        set_turns(&mut decisions, MediaId(9), 2);
        set_turns(&mut decisions, MediaId(9), 0);
        assert_eq!(decisions[&MediaId(9)].rating, Some(3));
        assert_eq!(decisions[&MediaId(9)].turns, 0);
    }

    #[test]
    fn relocation_keeps_suggestions_and_what_was_staged() {
        let mut review = review();
        review.stage_new_suggestions(&mut HashMap::new());
        review.relocate(&[(MediaId(1), MediaId(10), Default::default())]);
        assert_eq!(review.ids(), &[MediaId(10), MediaId(4)]);
        assert_eq!(review.default_turns(MediaId(10)), 1);
        assert!(review.staged().contains(&MediaId(10)));
    }
}
