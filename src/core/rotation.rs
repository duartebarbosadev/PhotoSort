//! Rotation review state. Preview choices never become file operations implicitly.
mod detection;
pub use detection::Detector;

use super::model::{Decision, MediaId};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct Choice {
    pub suggested: u8,
    pub selected: bool,
    pub confirmed: bool,
    pub uncertain: bool,
    override_turns: Option<u8>,
}
impl Choice {
    pub fn turns(&self) -> u8 {
        self.override_turns.unwrap_or(self.suggested)
    }
    pub fn degrees(&self) -> u16 {
        u16::from(self.turns()) * 90
    }
    pub fn manual(&self) -> bool {
        self.override_turns.is_some()
    }
    pub fn unfinished(&self) -> bool {
        !self.confirmed && (self.selected == self.uncertain || self.manual())
    }
    fn new(suggested: u8, uncertain: bool) -> Self {
        Self {
            suggested: suggested % 4,
            selected: !uncertain,
            uncertain,
            confirmed: false,
            override_turns: None,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Review {
    order: Vec<MediaId>,
    choices: HashMap<MediaId, Choice>,
}
impl Review {
    pub fn ids(&self) -> &[MediaId] {
        &self.order
    }
    pub fn choice(&self, id: MediaId) -> Option<&Choice> {
        self.choices.get(&id)
    }
    pub fn unfinished(&self, id: Option<MediaId>) -> bool {
        id.and_then(|id| self.choice(id))
            .is_some_and(Choice::unfinished)
    }
    /// Identical detection results preserve reviewed decisions across step changes.
    pub fn replace_suggestions(&mut self, suggestions: Vec<(MediaId, u8)>) {
        self.replace_predictions(
            suggestions
                .into_iter()
                .map(|(id, turns)| (id, turns, false))
                .collect(),
        );
    }
    pub fn replace_predictions(&mut self, suggestions: Vec<(MediaId, u8, bool)>) {
        let suggestions: Vec<_> = suggestions
            .into_iter()
            .filter(|(_, turns, _)| turns % 4 != 0)
            .collect();
        if self.order.len() == suggestions.len()
            && suggestions
                .iter()
                .enumerate()
                .all(|(i, (id, turns, uncertain))| {
                    self.order[i] == *id
                        && self
                            .choice(*id)
                            .is_some_and(|c| c.suggested == turns % 4 && c.uncertain == *uncertain)
                })
        {
            return;
        }
        self.order.clear();
        self.choices.clear();
        for (id, turns, uncertain) in suggestions {
            self.ensure_prediction(id, turns, uncertain);
        }
    }
    pub fn ensure(&mut self, id: MediaId, suggestion: u8) {
        self.ensure_prediction(id, suggestion, false);
    }
    pub fn ensure_prediction(&mut self, id: MediaId, suggestion: u8, uncertain: bool) {
        if let std::collections::hash_map::Entry::Vacant(entry) = self.choices.entry(id) {
            entry.insert(Choice::new(suggestion, uncertain));
            self.order.push(id);
        }
    }
    pub fn select(&mut self, id: MediaId, corrected: bool) {
        if let Some(c) = self.choices.get_mut(&id) {
            if corrected && c.turns() == 0 {
                c.override_turns = None;
            }
            c.selected = corrected;
            c.confirmed = false;
        }
    }
    pub fn rotate(&mut self, id: MediaId, delta: u8) {
        self.ensure(id, 0);
        let c = self.choices.get_mut(&id).unwrap();
        let turns = ((if c.selected { c.turns() } else { 0 }) + delta) % 4;
        c.override_turns = (turns != c.suggested).then_some(turns);
        c.selected = turns != 0;
        c.confirmed = false;
    }
    /// Returns the next unconfirmed item after this one; never wraps.
    pub fn confirm(&mut self, id: MediaId) -> Option<MediaId> {
        let c = self.choices.get_mut(&id)?;
        c.confirmed = !c.confirmed;
        if !c.confirmed {
            return None;
        }
        let index = self.order.iter().position(|other| *other == id)?;
        self.order[index + 1..]
            .iter()
            .copied()
            .find(|id| !self.choices[id].confirmed)
    }
    pub fn confirm_all(&mut self) {
        for c in self.choices.values_mut() {
            if c.uncertain {
                continue; // Uncertain predictions require individual confirmation.
            }
            if c.turns() == 0 {
                c.override_turns = None;
            }
            c.selected = true;
            c.confirmed = true;
        }
    }
    pub fn reset(&mut self, id: MediaId) {
        if let Some(c) = self.choices.get_mut(&id) {
            *c = Choice::new(c.suggested, c.uncertain);
        }
    }
    pub fn reset_all(&mut self) {
        for c in self.choices.values_mut() {
            *c = Choice::new(c.suggested, c.uncertain);
        }
    }
    pub fn discard(&mut self) {
        for c in self.choices.values_mut() {
            c.selected = false;
            c.confirmed = false;
        }
    }
    pub fn pending_count(&self) -> usize {
        self.choices
            .values()
            .filter(|c| c.confirmed && c.selected && c.turns() != 0)
            .count()
    }
    pub fn pending(&self) -> HashMap<MediaId, Decision> {
        self.choices
            .iter()
            .filter(|(_, c)| c.confirmed && c.selected && c.turns() != 0)
            .map(|(id, c)| {
                (
                    *id,
                    Decision {
                        turns: c.turns(),
                        ..Decision::default()
                    },
                )
            })
            .collect()
    }
    pub fn relocate(&mut self, moves: &[(MediaId, MediaId, super::model::Fingerprint)]) {
        let mapping: HashMap<_, _> = moves.iter().map(|&(old, new, _)| (old, new)).collect();
        for id in &mut self.order {
            *id = mapping.get(id).copied().unwrap_or(*id);
        }
        self.choices = std::mem::take(&mut self.choices)
            .into_iter()
            .map(|(id, choice)| (mapping.get(&id).copied().unwrap_or(id), choice))
            .collect();
    }
    pub fn complete(&mut self, completed: &HashSet<MediaId>) {
        self.retain(|id| !completed.contains(&id));
    }
    pub fn retain(&mut self, mut keep: impl FnMut(MediaId) -> bool) {
        self.order.retain(|id| keep(*id));
        let ids: HashSet<_> = self.order.iter().copied().collect();
        self.choices.retain(|id, _| ids.contains(id));
    }
}
