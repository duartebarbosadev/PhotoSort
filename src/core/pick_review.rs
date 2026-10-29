//! Pick Best decisions, independent of presentation and shared with file planning.
//!
//! One group per run of similar photos. PhotoSort ranks each group and
//! suggests keeping the best photo and moving the rest to Trash. As in Easy
//! Delete, a suggestion is the default decision: once per photo, it is
//! staged in the shared decisions, which Review changes shows and applies.
//! The user changes only what they disagree with; each group can go back to
//! its suggestion.
use super::eyes::Eyes;
use super::model::{Decision, MediaId, update_trash};
use std::collections::{HashMap, HashSet};

/// What put the suggested photo ahead of the next best one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The other photos are clearly less sharp.
    Sharper,
    /// Someone's eyes are closed in the next best photo and open here.
    EyesOpen,
    /// The overall score: the aesthetic model, sharpness and faces.
    Score,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub key: usize,
    /// The suggested photo first, then the others from best to worst.
    pub ordered: Vec<MediaId>,
    /// Overall scores; a missing score never becomes a Trash suggestion.
    pub scores: HashMap<MediaId, f64>,
    /// Each photo's eyes against the rest of the group; empty without faces.
    pub eyes: HashMap<MediaId, Eyes>,
    /// Photos well below the sharpest of the group.
    pub soft: HashSet<MediaId>,
    pub reason: Reason,
}
impl Cluster {
    /// A group ranked by score alone, best first.
    pub fn new(key: usize, members: Vec<MediaId>, scores: HashMap<MediaId, f64>) -> Self {
        let mut ordered = members;
        ordered.sort_by(|a, b| {
            let score = |id: &MediaId| scores.get(id).copied().unwrap_or(f64::NEG_INFINITY);
            score(b).total_cmp(&score(a))
        });
        Self {
            key,
            ordered,
            scores,
            eyes: HashMap::new(),
            soft: HashSet::new(),
            reason: Reason::Score,
        }
    }
    /// The photo PhotoSort suggests keeping.
    pub fn best(&self) -> MediaId {
        self.ordered[0]
    }
    /// The suggested Keep flag of each photo, in order.
    pub fn suggestion(&self) -> impl Iterator<Item = (MediaId, bool)> + '_ {
        let best = self.best();
        self.ordered.iter().map(move |id| (*id, *id == best))
    }
    pub fn blinks(&self, id: MediaId) -> usize {
        self.eyes.get(&id).map_or(0, |e| e.closed)
    }
}

fn kept(decisions: &HashMap<MediaId, Decision>, id: MediaId) -> bool {
    !decisions.get(&id).is_some_and(|d| d.rejected)
}

#[derive(Default)]
pub struct Review {
    clusters: Vec<Cluster>,
    /// Every member's group.
    index: HashMap<MediaId, usize>,
    /// Photos whose suggestion was staged once; see `Session::pick_suggested`.
    suggested: HashSet<MediaId>,
}
impl Review {
    pub fn clusters(&self) -> &[Cluster] {
        &self.clusters
    }
    /// The group a photo belongs to.
    pub fn group_of(&self, id: MediaId) -> Option<usize> {
        self.index.get(&id).copied()
    }
    pub fn cluster_of(&self, id: MediaId) -> Option<&Cluster> {
        self.group_of(id).map(|i| &self.clusters[i])
    }
    /// The caller supplies capture ordering. Decisions are never touched:
    /// a new analysis keeps every choice already staged.
    pub fn replace(&mut self, mut clusters: Vec<Cluster>) {
        clusters.retain(|c| {
            c.ordered.len() >= 2
                && c.ordered
                    .iter()
                    .all(|id| c.scores.get(id).is_some_and(|s| s.is_finite()))
        });
        self.index = clusters
            .iter()
            .enumerate()
            .flat_map(|(i, c)| c.ordered.iter().map(move |id| (*id, i)))
            .collect();
        self.clusters = clusters;
    }
    pub fn suggested(&self) -> &HashSet<MediaId> {
        &self.suggested
    }
    pub fn restore_suggested(&mut self, ids: HashSet<MediaId>) {
        self.suggested = ids;
    }
    /// Stages the suggested Trash marks of photos seen for the first time.
    /// Never unmarks anything, and never repeats for a photo: a later
    /// analysis leaves the user's choices alone. A group whose suggested
    /// photo is already marked for Trash (by Easy Delete or by hand) is left
    /// as it is, so a whole burst never goes to Trash on its own. Returns the
    /// photos marked.
    pub fn stage_new_suggestions(
        &mut self,
        decisions: &mut HashMap<MediaId, Decision>,
    ) -> Vec<MediaId> {
        let mut marked = Vec::new();
        for cluster in &self.clusters {
            let best_kept = kept(decisions, cluster.best());
            for (id, keep) in cluster.suggestion() {
                if self.suggested.insert(id) && !keep && best_kept {
                    marked.push(id);
                }
            }
        }
        update_trash(decisions, marked.iter().map(|id| (*id, true)));
        marked
    }
    pub fn keeps(decisions: &HashMap<MediaId, Decision>, id: MediaId) -> bool {
        kept(decisions, id)
    }
    /// Whether a group's decisions differ from its suggestion.
    pub fn changed(&self, group: usize, decisions: &HashMap<MediaId, Decision>) -> bool {
        self.clusters
            .get(group)
            .is_some_and(|c| c.suggestion().any(|(id, keep)| kept(decisions, id) != keep))
    }
    /// How many of these groups differ from their suggestion.
    pub fn changed_count(&self, groups: &[usize], decisions: &HashMap<MediaId, Decision>) -> usize {
        groups
            .iter()
            .filter(|g| self.changed(**g, decisions))
            .count()
    }
    /// Switches one photo of a group between Keep and Trash.
    pub fn toggle(&self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) {
        if self.index.contains_key(&id) {
            update_trash(decisions, [(id, kept(decisions, id))]);
        }
    }
    /// Keeps only this photo of its group.
    pub fn keep_only(&self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) {
        if let Some(cluster) = self.cluster_of(id) {
            update_trash(decisions, cluster.ordered.iter().map(|o| (*o, *o != id)));
        }
    }
    /// Puts each group back to its suggestion.
    pub fn accept(&self, groups: &[usize], decisions: &mut HashMap<MediaId, Decision>) {
        let marks: Vec<_> = groups
            .iter()
            .filter_map(|g| self.clusters.get(*g))
            .flat_map(Cluster::suggestion)
            .map(|(id, keep)| (id, !keep))
            .collect();
        update_trash(decisions, marks);
    }
}

/// Reuse completed shared analysis; never initialize models or read photos here.
///
/// Each group is ranked by, in order: sharpness (a photo well below the
/// sharpest is never suggested over it), closed eyes (a photo where someone
/// blinks ranks below one where that person's eyes are open), then the
/// overall score.
pub fn from_analysis(
    media: &[super::model::Media],
    analysis: &HashMap<MediaId, super::analysis::Analysis>,
    groups: &HashMap<MediaId, usize>,
    metadata: &HashMap<MediaId, super::model::Metadata>,
) -> Vec<Cluster> {
    let mut grouped: std::collections::BTreeMap<usize, Vec<MediaId>> = Default::default();
    for m in media
        .iter()
        .filter(|m| m.kind != super::model::MediaKind::Video)
    {
        if let Some(group) = groups.get(&m.id) {
            grouped.entry(*group).or_default().push(m.id);
        }
    }
    let order: HashMap<_, _> = media
        .iter()
        .map(|m| (m.id, super::dates::capture_order(m, metadata.get(&m.id))))
        .collect();
    let mut clusters = Vec::new();
    for (key, mut members) in grouped {
        if members.len() < 2
            || members.iter().any(|id| {
                analysis.get(id).is_none_or(|a| {
                    a.aesthetic.is_none() || a.faces.is_none() || !a.score.is_finite()
                })
            })
        {
            continue;
        }
        members.sort_by_key(|id| &order[id]);
        let sharpness = |id: &MediaId| {
            analysis[id]
                .quality_sharpness
                .unwrap_or(analysis[id].sharpness)
        };
        let sharpest = members.iter().map(sharpness).fold(0., f64::max);
        let soft: HashSet<MediaId> = members
            .iter()
            .copied()
            .filter(|id| sharpest > 0. && sharpness(id) < sharpest * 0.6)
            .collect();
        let faces: Vec<(MediaId, &[super::faces::FaceEyes])> = members
            .iter()
            .map(|id| {
                let eyes = analysis[id].faces.as_ref().map_or(&[][..], |f| &f.eyes);
                (*id, eyes)
            })
            .collect();
        let eyes = super::eyes::compare(&faces);
        let scores: HashMap<MediaId, f64> =
            members.iter().map(|id| (*id, analysis[id].score)).collect();
        let rank = |id: &MediaId| (soft.contains(id), eyes.get(id).map_or(0, |e| e.closed));
        let mut ordered = members.clone();
        // Stable: equal photos keep capture order.
        ordered.sort_by(|a, b| {
            rank(a)
                .cmp(&rank(b))
                .then_with(|| scores[b].total_cmp(&scores[a]))
        });
        let (best, next) = (rank(&ordered[0]), rank(&ordered[1]));
        let reason = if best.0 != next.0 {
            Reason::Sharper
        } else if best.1 != next.1 {
            Reason::EyesOpen
        } else {
            Reason::Score
        };
        clusters.push(Cluster {
            key,
            ordered,
            scores,
            eyes: if eyes.values().any(|e| e.closed > 0) {
                eyes
            } else {
                HashMap::new()
            },
            soft,
            reason,
        });
    }
    clusters.sort_by_key(|c| c.ordered.iter().map(|id| &order[id]).min().cloned());
    clusters
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::analysis::Analysis;
    use crate::core::faces::{EyeState, FaceEyes, FaceSummary};
    use crate::core::model::{Fingerprint, Media, MediaKind};

    fn media(n: u64) -> Vec<Media> {
        (1..=n)
            .map(|i| Media {
                id: MediaId(i),
                path: format!("/{i}.jpg").into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect()
    }
    fn photo(score: f64, sharpness: f64, eyes: Vec<FaceEyes>) -> Analysis {
        Analysis {
            sharpness,
            quality_sharpness: Some(sharpness),
            aesthetic: Some(0.5),
            faces: Some(FaceSummary {
                count: eyes.len(),
                eyes,
                ..Default::default()
            }),
            score,
            ..Default::default()
        }
    }
    fn face(x: f32, state: EyeState, ratio: Option<f32>) -> FaceEyes {
        FaceEyes {
            centre: [x, 0.4],
            size: [0.1, 0.12],
            state,
            ratio,
        }
    }
    fn open(x: f32) -> FaceEyes {
        face(x, EyeState::Open, Some(0.3))
    }
    fn closed(x: f32) -> FaceEyes {
        face(x, EyeState::Closed, Some(0.05))
    }
    fn rank(photos: Vec<Analysis>) -> Cluster {
        let media = media(photos.len() as u64);
        let analysis = media.iter().map(|m| m.id).zip(photos).collect();
        let groups = media.iter().map(|m| (m.id, 0)).collect();
        from_analysis(&media, &analysis, &groups, &HashMap::new())
            .pop()
            .unwrap()
    }

    #[test]
    fn closed_eyes_rank_below_open_eyes_even_with_a_higher_score() {
        let c = rank(vec![
            photo(80., 300., vec![closed(0.3)]),
            photo(60., 300., vec![open(0.3)]),
        ]);
        assert_eq!(c.best(), MediaId(2));
        assert_eq!(c.reason, Reason::EyesOpen);
        assert_eq!(c.blinks(MediaId(1)), 1);
        assert_eq!(c.blinks(MediaId(2)), 0);
    }

    #[test]
    fn unknown_eyes_are_not_closed_so_the_score_decides() {
        // A small, sunglassed, squinting or legacy face is Unknown: it never
        // loses to an open one on eyes.
        let unknown = face(0.3, EyeState::Unknown, None);
        let c = rank(vec![
            photo(80., 300., vec![unknown]),
            photo(60., 300., vec![open(0.3)]),
        ]);
        assert_eq!(c.best(), MediaId(1));
        assert_eq!(c.reason, Reason::Score);
        assert!(c.eyes.is_empty(), "No eye facts to show");
    }

    #[test]
    fn eyes_closed_in_every_frame_leave_the_score_to_decide() {
        let c = rank(vec![
            photo(60., 300., vec![closed(0.3)]),
            photo(80., 300., vec![closed(0.3)]),
        ]);
        assert_eq!(c.best(), MediaId(2));
        assert_eq!(c.reason, Reason::Score);
    }

    #[test]
    fn group_photos_prefer_the_frame_where_everyone_has_eyes_open() {
        let c = rank(vec![
            photo(90., 300., vec![closed(0.2), open(0.7)]),
            photo(85., 300., vec![open(0.2), closed(0.7)]),
            photo(70., 300., vec![open(0.2), open(0.7)]),
        ]);
        assert_eq!(c.best(), MediaId(3));
        assert_eq!(c.ordered, vec![MediaId(3), MediaId(1), MediaId(2)]);
    }

    #[test]
    fn a_soft_photo_never_wins_even_with_eyes_open() {
        let c = rank(vec![
            photo(90., 300., vec![closed(0.3)]),
            photo(95., 100., vec![open(0.3)]),
        ]);
        assert_eq!(c.best(), MediaId(1));
        assert_eq!(c.reason, Reason::Sharper);
        assert!(c.soft.contains(&MediaId(2)));
    }

    fn review() -> Review {
        let mut review = Review::default();
        review.replace(vec![Cluster::new(
            1,
            vec![MediaId(1), MediaId(2), MediaId(3)],
            HashMap::from([(MediaId(1), 10.), (MediaId(2), 30.), (MediaId(3), 20.)]),
        )]);
        review
    }
    fn trashed(decisions: &HashMap<MediaId, Decision>) -> Vec<u64> {
        let mut ids: Vec<u64> = decisions
            .iter()
            .filter(|(_, d)| d.rejected)
            .map(|(id, _)| id.0)
            .collect();
        ids.sort();
        ids
    }

    #[test]
    fn suggestions_are_staged_once_and_undo_is_the_suggestion() {
        let mut review = review();
        let mut decisions = HashMap::new();
        assert_eq!(review.stage_new_suggestions(&mut decisions).len(), 2);
        assert_eq!(trashed(&decisions), vec![1, 3]);
        assert!(!review.changed(0, &decisions));
        // The user keeps a second photo: a change, staged at once.
        review.toggle(MediaId(3), &mut decisions);
        assert_eq!(trashed(&decisions), vec![1]);
        assert!(review.changed(0, &decisions));
        // A new analysis never re-trashes it.
        review.replace(vec![review.clusters()[0].clone()]);
        assert!(review.stage_new_suggestions(&mut decisions).is_empty());
        assert_eq!(trashed(&decisions), vec![1]);
        // Changing it back by hand is no change at all.
        review.toggle(MediaId(3), &mut decisions);
        assert!(!review.changed(0, &decisions));
        // Undo puts any change back to the suggestion.
        review.keep_only(MediaId(1), &mut decisions);
        assert_eq!(trashed(&decisions), vec![2, 3]);
        assert!(review.changed(0, &decisions));
        review.accept(&[0], &mut decisions);
        assert_eq!(trashed(&decisions), vec![1, 3]);
        assert_eq!(review.changed_count(&[0], &decisions), 0);
    }

    #[test]
    fn easy_delete_after_pick_best_never_trashes_both_copies() {
        use crate::core::delete_review::{self, Category, Entry};
        // Two identical files: Pick Best keeps 2, Easy Delete calls 2 the copy.
        let mut pick = Review::default();
        pick.replace(vec![Cluster::new(
            1,
            vec![MediaId(1), MediaId(2)],
            HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.)]),
        )]);
        let mut delete = delete_review::Review::default();
        delete.replace(vec![Entry {
            subject: MediaId(2),
            companion: Some(MediaId(1)),
            category: Category::Exact,
            tag: String::new(),
            reason: String::new(),
            keep_why: String::new(),
            eyes: Vec::new(),
            extra: Vec::new(),
        }]);
        let mut decisions = HashMap::new();
        pick.stage_new_suggestions(&mut decisions);
        delete.stage_new_suggestions(&mut decisions);
        assert_eq!(trashed(&decisions), vec![1]);
        // The other order holds too.
        let mut decisions = HashMap::new();
        let (mut pick, mut delete) = (pick, delete);
        pick.restore_suggested(Default::default());
        delete.restore_suggested(Default::default());
        delete.stage_new_suggestions(&mut decisions);
        pick.stage_new_suggestions(&mut decisions);
        assert_eq!(trashed(&decisions), vec![2]);
    }

    #[test]
    fn a_group_whose_best_photo_is_already_in_trash_is_never_all_trashed() {
        let mut review = review();
        let mut decisions = HashMap::new();
        update_trash(&mut decisions, [(MediaId(2), true)]);
        assert!(review.stage_new_suggestions(&mut decisions).is_empty());
        assert_eq!(trashed(&decisions), vec![2]);
        assert!(review.changed(0, &decisions));
        // Accepting keeps the best photo.
        review.accept(&[0], &mut decisions);
        assert_eq!(trashed(&decisions), vec![1, 3]);
    }
}
