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
    /// The next best photo is a smaller copy of this one.
    FullSize,
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
    /// Smaller copies of another photo of the group; see `smaller_copies`.
    pub copies: HashSet<MediaId>,
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
            copies: HashSet::new(),
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
    /// The stamp of the last change to the review itself.
    revision: u64,
}
impl Review {
    pub fn clusters(&self) -> &[Cluster] {
        &self.clusters
    }
    /// Changes whenever the review does, never with the decisions: what
    /// was worked out from its groups stays good while this is the same.
    pub fn revision(&self) -> u64 {
        self.revision
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
        self.revision = super::revision::next();
    }
    pub fn suggested(&self) -> &HashSet<MediaId> {
        &self.suggested
    }
    pub fn restore_suggested(&mut self, ids: HashSet<MediaId>) {
        self.suggested = ids;
        self.revision = super::revision::next();
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
        self.revision = super::revision::next();
        marked
    }
    pub fn keeps(decisions: &HashMap<MediaId, Decision>, id: MediaId) -> bool {
        kept(decisions, id)
    }
    /// The photos kept in a group that has a photo marked for Trash: each
    /// was chosen over that one, by the suggestion or by hand.
    pub fn chosen(&self, decisions: &HashMap<MediaId, Decision>) -> HashSet<MediaId> {
        self.clusters
            .iter()
            .filter(|c| c.ordered.iter().any(|id| !kept(decisions, *id)))
            .flat_map(|c| c.ordered.iter().copied())
            .filter(|id| kept(decisions, *id))
            .collect()
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
    /// The one photo still kept in each group whose others are all marked
    /// for Trash: marking it too would send the whole group there.
    pub fn last_kept(&self, decisions: &HashMap<MediaId, Decision>) -> HashSet<MediaId> {
        self.clusters
            .iter()
            .filter_map(|c| {
                let mut kept = c.ordered.iter().copied().filter(|id| kept(decisions, *id));
                let only = kept.next()?;
                kept.next().is_none().then_some(only)
            })
            .collect()
    }
    /// For suggestions applied in bulk, here or in Easy Delete: drops from
    /// `marks` (a photo, and whether it goes to Trash) what would leave a
    /// group with every photo marked. The best of the photos about to be
    /// marked stays as it is. Only the groups the marks touch are read.
    pub fn spare(&self, decisions: &HashMap<MediaId, Decision>, marks: &mut Vec<(MediaId, bool)>) {
        let after: HashMap<MediaId, bool> = marks.iter().copied().collect();
        let trashed = |id: &MediaId| after.get(id).copied().unwrap_or(!kept(decisions, *id));
        let groups: HashSet<usize> = marks
            .iter()
            .filter(|(id, trash)| *trash && kept(decisions, *id))
            .filter_map(|(id, _)| self.group_of(*id))
            .collect();
        let spared: HashSet<MediaId> = groups
            .into_iter()
            .map(|g| &self.clusters[g].ordered)
            .filter(|ordered| ordered.iter().all(trashed))
            .filter_map(|ordered| ordered.iter().copied().find(|id| kept(decisions, *id)))
            .collect();
        marks.retain(|(id, trash)| !(*trash && spared.contains(id)));
    }
    /// Whether Accept all acts on a group: a photo not decided by hand
    /// differs from its suggestion, and the group's suggested photo is not
    /// marked for Trash. A group whose suggested photo is marked is left as
    /// it is, as when suggestions are staged.
    pub fn pending(
        &self,
        group: usize,
        decisions: &HashMap<MediaId, Decision>,
        by_hand: &HashSet<MediaId>,
    ) -> bool {
        self.clusters
            .get(group)
            .is_some_and(|c| !Self::undecided_marks(c, decisions, by_hand).is_empty())
    }
    /// What Accept all sets in one group: the suggestion of each photo not
    /// decided by hand that differs from it. Nothing where the suggested
    /// photo is marked for Trash: its suggestions assume that photo stays,
    /// and the group would be left with no photo kept.
    fn undecided_marks(
        cluster: &Cluster,
        decisions: &HashMap<MediaId, Decision>,
        by_hand: &HashSet<MediaId>,
    ) -> Vec<(MediaId, bool)> {
        if !kept(decisions, cluster.best()) {
            return Vec::new();
        }
        cluster
            .suggestion()
            .filter(|(id, keep)| !by_hand.contains(id) && kept(decisions, *id) != *keep)
            .map(|(id, keep)| (id, !keep))
            .collect()
    }
    /// Accept all: every photo not decided by hand takes its suggestion. A
    /// choice made by hand stays, and so does every group whose suggested
    /// photo is marked for Trash, so no group is left with no photo kept.
    /// Returns the photos changed.
    pub fn accept_undecided(
        &self,
        decisions: &mut HashMap<MediaId, Decision>,
        by_hand: &HashSet<MediaId>,
    ) -> Vec<MediaId> {
        let marks: Vec<_> = self
            .clusters
            .iter()
            .flat_map(|c| Self::undecided_marks(c, decisions, by_hand))
            .collect();
        update_trash(decisions, marks.iter().copied());
        marks.into_iter().map(|(id, _)| id).collect()
    }
    /// Puts each group back to its suggestion in full: one group, asked for
    /// by hand.
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

/// Fewer pixels than this share of another photo's make a photo its smaller
/// copy, when the two show the same picture. Not any difference at all: a
/// camera's RAW file and the JPEG it saved of the same shot differ by up to
/// 1.3% (6024x4024 against 6000x4000 is 99.0%), and neither is a copy of the
/// other. 98% still takes in a crop of 1% off each side.
pub(crate) const COPY_PIXELS: f64 = 0.98;

/// The photos of a group that are a smaller copy of another one in it: the
/// same picture (nearly the same visual hash and average colour) with fewer
/// pixels (see `COPY_PIXELS`), whether it was resized or cropped a little,
/// on every side or on one. Every photo is measured after being reduced to
/// one size, where a resized copy scores within a few percent of its
/// original, above or below, depending only on how it was resized, and a
/// cropped one scores a little higher because it shows the same detail
/// larger. So the scores cannot tell the two apart, and the pixels decide.
/// A crop of more than about a twentieth is another framing by this test and
/// competes on its score. A photo of unknown size is never a copy and never
/// makes one.
fn smaller_copies(
    members: &[MediaId],
    analysis: &HashMap<MediaId, super::analysis::Analysis>,
    metadata: &HashMap<MediaId, super::model::Metadata>,
) -> HashSet<MediaId> {
    let sized: Vec<(MediaId, f64)> = members
        .iter()
        .filter(|id| analysis.contains_key(id))
        .filter_map(|id| {
            let m = metadata.get(id).filter(|m| m.width > 0 && m.height > 0)?;
            Some((*id, f64::from(m.width) * f64::from(m.height)))
        })
        .collect();
    let same_picture = |a: &MediaId, b: &MediaId| {
        let (a, b) = (&analysis[a], &analysis[b]);
        (a.hash ^ b.hash).count_ones() <= 6
            && a.color
                .iter()
                .zip(b.color)
                .map(|(a, b)| (a - b).abs())
                .sum::<f32>()
                < 0.08
    };
    sized
        .iter()
        .filter(|(id, pixels)| {
            sized
                .iter()
                .any(|(other, more)| *pixels < more * COPY_PIXELS && same_picture(id, other))
        })
        .map(|(id, _)| *id)
        .collect()
}

/// One group's photos from best to worst, and what set them apart.
pub struct Ranking {
    pub ordered: Vec<MediaId>,
    /// Each photo's eyes against the rest of the group.
    pub eyes: HashMap<MediaId, Eyes>,
    /// Photos well below the sharpest of the group.
    pub soft: HashSet<MediaId>,
    /// Smaller copies of another photo of the group; see `smaller_copies`.
    pub copies: HashSet<MediaId>,
    pub reason: Reason,
}

/// Ranks one group: the one order Pick Best suggests from and Cull's "Best
/// of" badge, its "2nd of" place and "Go to best" follow, so the two steps
/// never name a different best photo.
///
/// In order: size (a smaller copy of a photo is never ranked over it),
/// sharpness (a photo well below the sharpest is never ranked over it),
/// closed eyes (a photo where someone blinks ranks below one where that
/// person's eyes are open), then the overall score. `members` come in the
/// order that settles a tie: by capture. A photo with no analysis yet, which
/// Cull can show and Pick Best never ranks, goes last.
pub fn rank(
    members: &[MediaId],
    analysis: &HashMap<MediaId, super::analysis::Analysis>,
    metadata: &HashMap<MediaId, super::model::Metadata>,
) -> Ranking {
    let sharpness = |id: &MediaId| {
        analysis
            .get(id)
            .map_or(0., |a| a.quality_sharpness.unwrap_or(a.sharpness))
    };
    let sharpest = members.iter().map(sharpness).fold(0., f64::max);
    let soft: HashSet<MediaId> = members
        .iter()
        .copied()
        .filter(|id| analysis.contains_key(id) && sharpest > 0. && sharpness(id) < sharpest * 0.6)
        .collect();
    let faces: Vec<(MediaId, &[super::faces::FaceEyes])> = members
        .iter()
        .map(|id| {
            let eyes = analysis
                .get(id)
                .and_then(|a| a.faces.as_ref())
                .map_or(&[][..], |f| &f.eyes);
            (*id, eyes)
        })
        .collect();
    let eyes = super::eyes::compare(&faces);
    let copies = smaller_copies(members, analysis, metadata);
    let score = |id: &MediaId| {
        analysis
            .get(id)
            .map(|a| a.score)
            .filter(|score| score.is_finite())
            .unwrap_or(f64::NEG_INFINITY)
    };
    let place = |id: &MediaId| {
        (
            !analysis.contains_key(id),
            copies.contains(id),
            soft.contains(id),
            eyes.get(id).map_or(0, |e| e.closed),
        )
    };
    let mut ordered = members.to_vec();
    // Stable: equal photos keep the order they came in.
    ordered.sort_by(|a, b| {
        place(a)
            .cmp(&place(b))
            .then_with(|| score(b).total_cmp(&score(a)))
    });
    let reason = match ordered.as_slice() {
        [best, next, ..] => {
            let (best, next) = (place(best), place(next));
            if best.1 != next.1 {
                Reason::FullSize
            } else if best.2 != next.2 {
                Reason::Sharper
            } else if best.3 != next.3 {
                Reason::EyesOpen
            } else {
                Reason::Score
            }
        }
        _ => Reason::Score,
    };
    Ranking {
        ordered,
        eyes,
        soft,
        copies,
        reason,
    }
}

/// Reuse completed shared analysis; never initialize models or read photos here.
///
/// Each group whose photos all have their quality analysis is ranked by
/// `rank`.
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
        let Ranking {
            ordered,
            eyes,
            soft,
            copies,
            reason,
        } = rank(&members, analysis, metadata);
        clusters.push(Cluster {
            key,
            scores: members.iter().map(|id| (*id, analysis[id].score)).collect(),
            ordered,
            eyes: if eyes.values().any(|e| e.closed > 0) {
                eyes
            } else {
                HashMap::new()
            },
            soft,
            copies,
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
        // A small, sunglassed or squinting face is Unknown: it never
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

    #[test]
    fn a_smaller_copy_never_wins_over_the_photo_it_copies() {
        use crate::core::model::Metadata;
        let sized = |width, height| Metadata {
            width,
            height,
            ..Default::default()
        };
        let media = media(3);
        let groups: HashMap<_, _> = media.iter().map(|m| (m.id, 0)).collect();
        // 1 is the original; 2 is its half-size copy, which measures a
        // little sharper and scores higher; 3 is another frame of the burst.
        let mut analysis: HashMap<_, _> = [
            photo(70., 397., vec![]),
            photo(72., 414., vec![]),
            photo(60., 380., vec![]),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, a)| (MediaId(i as u64 + 1), a))
        .collect();
        analysis.get_mut(&MediaId(3)).unwrap().hash = u64::MAX;
        let metadata = HashMap::from([
            (MediaId(1), sized(6000, 4000)),
            // Stored turned: the long side still matches.
            (MediaId(2), sized(2000, 3000)),
            (MediaId(3), sized(6000, 4000)),
        ]);
        let rank = |analysis: &HashMap<_, _>, metadata: &HashMap<_, _>| {
            from_analysis(&media, analysis, &groups, metadata)
                .pop()
                .unwrap()
        };
        let c = rank(&analysis, &metadata);
        assert_eq!(c.ordered, vec![MediaId(1), MediaId(3), MediaId(2)]);
        assert_eq!(c.copies, HashSet::from([MediaId(2)]));
        assert_eq!(c.reason, Reason::Score);
        // With only the pair, the size is what decided. So it is for a copy
        // cropped by 3% all round, or on one side only, whose hash moved a
        // little: each scores higher and still never wins.
        let pair = &media[..2];
        for copy in [sized(2000, 3000), sized(5820, 3880), sized(5820, 4000)] {
            let mut metadata = metadata.clone();
            metadata.insert(MediaId(2), copy);
            analysis.get_mut(&MediaId(2)).unwrap().hash = 0b101;
            let c = from_analysis(pair, &analysis, &groups, &metadata)
                .pop()
                .unwrap();
            assert_eq!(c.best(), MediaId(1));
            assert_eq!(c.reason, Reason::FullSize);
        }
        // A camera's RAW and JPEG of one shot differ by a few rows of
        // pixels: neither is a copy. Nor is a smaller photo that shows
        // something else, or one of unknown size. Those compete on their
        // score as before.
        let mut near = metadata.clone();
        near.insert(MediaId(2), sized(5964, 3976));
        assert_eq!(rank(&analysis, &near).best(), MediaId(2));
        analysis.get_mut(&MediaId(2)).unwrap().hash = 0xFFFF;
        assert_eq!(rank(&analysis, &metadata).best(), MediaId(2));
        analysis.get_mut(&MediaId(2)).unwrap().hash = 0;
        assert_eq!(rank(&analysis, &HashMap::new()).best(), MediaId(2));
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
        // Accept all leaves it alone too; the group's own Undo keeps the
        // best photo.
        assert!(!review.pending(0, &decisions, &HashSet::new()));
        review.accept(&[0], &mut decisions);
        assert_eq!(trashed(&decisions), vec![1, 3]);
    }

    #[test]
    fn easy_delete_suggestions_never_trash_the_last_photo_kept_of_a_group() {
        use crate::core::delete_review::{self, Category, Entry};
        let soft = |n: u64| Entry {
            subject: MediaId(n),
            companion: None,
            category: Category::Blur,
            tag: String::new(),
            reason: String::new(),
            keep_why: String::new(),
            eyes: Vec::new(),
            extra: Vec::new(),
        };
        // Pick Best keeps 2 of its group; Easy Delete finds all three soft.
        let mut pick = review();
        let mut delete = delete_review::Review::default();
        delete.replace(vec![soft(1), soft(2), soft(3)]);
        let cards = [MediaId(1), MediaId(2), MediaId(3)];
        let mut decisions = HashMap::new();
        pick.stage_new_suggestions(&mut decisions);
        // Staged on its own or accepted all at once, 2 stays.
        let staged = delete.stage_new_suggestions_sparing(&pick, &mut decisions);
        assert!(!staged.contains(&MediaId(2)));
        delete.accept_sparing(&cards, &pick, &mut decisions, &HashSet::new());
        assert_eq!(trashed(&decisions), vec![1, 3]);
        // Its card can say so, and Accept all has nothing left to do.
        let last = pick.last_kept(&decisions);
        assert_eq!(delete.held(MediaId(2), &last), vec![MediaId(2)]);
        assert!(
            cards
                .iter()
                .all(|id| !delete.pending(*id, &decisions, &last, &HashSet::new()))
        );
        // Marked by hand, it goes.
        delete.toggle(MediaId(2), MediaId(2), &mut decisions);
        assert_eq!(trashed(&decisions), vec![1, 2, 3]);
        // Easy Delete first, with nothing marked yet: the best of the
        // group is the one left.
        let mut decisions = HashMap::new();
        delete.accept_sparing(&cards, &pick, &mut decisions, &HashSet::new());
        assert_eq!(trashed(&decisions), vec![1, 3]);
    }
}
