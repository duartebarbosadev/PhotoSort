//! Progressive Pick Best comparisons. Image analysis and file mutation remain shared services.
use super::model::{Decision, MediaId, update_trash};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub key: usize,
    /// Anchor first, then descending anchor similarity, with stable missing-vector fallback.
    pub ordered: Vec<MediaId>,
    /// Cluster-adjusted scores; a missing score must never become a deletion recommendation.
    pub scores: HashMap<MediaId, f64>,
}
#[derive(Clone, Debug)]
pub struct Comparison {
    pub paths: Vec<MediaId>,
    pub ai_pick: MediaId,
    challenger: MediaId,
    keep: HashMap<MediaId, bool>,
    defaults: HashMap<MediaId, bool>,
    pub advancing: MediaId,
    pub confirmed: bool,
}
impl Comparison {
    fn new(
        mut paths: Vec<MediaId>,
        scores: &HashMap<MediaId, f64>,
        carried: Option<(MediaId, bool)>,
    ) -> Self {
        let challenger = *paths.last().unwrap();
        let ai_pick = *paths
            .iter()
            .enumerate()
            .max_by(|(ai, a), (bi, b)| scores[a].total_cmp(&scores[b]).then_with(|| bi.cmp(ai)))
            .unwrap()
            .1;
        paths.retain(|id| *id != ai_pick);
        paths.push(ai_pick);
        let mut keep: HashMap<_, _> = paths.iter().map(|id| (*id, *id == ai_pick)).collect();
        if let Some((id, value)) = carried {
            keep.insert(id, value);
        }
        let mut c = Self {
            paths,
            ai_pick,
            challenger,
            defaults: keep.clone(),
            keep,
            advancing: ai_pick,
            confirmed: false,
        };
        c.resolve();
        c
    }
    pub fn keeps(&self, id: MediaId) -> bool {
        self.keep.get(&id).copied().unwrap_or(true)
    }
    pub fn keep_all(&self) -> bool {
        self.paths.iter().all(|id| self.keeps(*id))
    }
    fn resolve(&mut self) {
        let kept: Vec<_> = self
            .paths
            .iter()
            .copied()
            .filter(|id| self.keeps(*id))
            .collect();
        self.advancing = match kept.as_slice() {
            [only] => *only,
            many if many.len() > 1 && many.contains(&self.challenger) => self.challenger,
            _ => self.ai_pick,
        };
    }
    fn marks(&self) -> impl Iterator<Item = (MediaId, bool)> + '_ {
        self.paths.iter().map(|id| (*id, !self.keeps(*id)))
    }
}
#[derive(Clone, Debug)]
pub struct Tournament {
    pub cluster: Cluster,
    pub comparisons: Vec<Comparison>,
    pub current: usize,
    next: usize,
    pub finalized: bool,
    prior: Option<HashMap<MediaId, bool>>,
}
impl Tournament {
    /// Candidates not yet introduced into a comparison, in tournament order.
    pub fn upcoming(&self) -> &[MediaId] {
        &self.cluster.ordered[self.next..]
    }

    fn new(cluster: Cluster) -> Self {
        let comparisons = vec![Comparison::new(
            cluster.ordered[..2].to_vec(),
            &cluster.scores,
            None,
        )];
        Self {
            cluster,
            comparisons,
            current: 0,
            next: 2,
            finalized: false,
            prior: None,
        }
    }
    pub fn comparison(&self) -> &Comparison {
        &self.comparisons[self.current]
    }
    fn ensure_prior(&mut self, decisions: &HashMap<MediaId, Decision>) {
        if self.prior.is_none() {
            self.prior = Some(
                self.cluster
                    .ordered
                    .iter()
                    .map(|id| (*id, decisions.get(id).is_some_and(|d| d.rejected)))
                    .collect(),
            );
        }
    }
    fn expected(&self, before: usize) -> HashMap<MediaId, bool> {
        let mut marks = self.prior.clone().unwrap_or_default();
        for c in self.comparisons.iter().take(before).filter(|c| c.confirmed) {
            marks.extend(c.marks());
        }
        marks
    }
    fn invalidate(&mut self, decisions: &mut HashMap<MediaId, Decision>) {
        if self.prior.is_some() {
            update_trash(decisions, self.expected(self.current));
        }
        self.comparisons.truncate(self.current + 1);
        self.next = self
            .comparisons
            .iter()
            .flat_map(|c| c.paths.iter())
            .filter_map(|id| self.cluster.ordered.iter().position(|p| p == id))
            .max()
            .map_or(0, |i| i + 1);
        self.finalized = false;
        self.comparisons[self.current].confirmed = false;
    }
}
#[derive(Default)]
pub struct Review {
    tournaments: Vec<Tournament>,
    current: usize,
}
impl Review {
    pub fn restore_pair(&mut self, active: Option<MediaId>, pair: &[MediaId]) {
        let found = self
            .tournaments
            .iter()
            .position(|t| {
                pair.len() == 2
                    && pair[0] != pair[1]
                    && pair.iter().all(|id| t.cluster.ordered.contains(id))
            })
            .or_else(|| {
                active.and_then(|id| {
                    self.tournaments
                        .iter()
                        .position(|t| t.cluster.ordered.contains(&id))
                })
            });
        let Some(index) = found else {
            return;
        };
        self.current = index;
        let t = &mut self.tournaments[index];
        if pair.len() != 2
            || pair[0] == pair[1]
            || !pair.iter().all(|id| t.cluster.ordered.contains(id))
        {
            return;
        }
        if let Some(round) = t
            .comparisons
            .iter()
            .position(|c| pair.iter().all(|id| c.paths.contains(id)))
        {
            t.current = round;
        } else if !t.comparisons.iter().any(|c| c.confirmed) {
            let mut cluster = t.cluster.clone();
            cluster.ordered.retain(|id| !pair.contains(id));
            cluster.ordered.splice(0..0, pair.iter().copied());
            *t = Tournament::new(cluster);
        }
    }
    pub fn tournaments(&self) -> &[Tournament] {
        &self.tournaments
    }
    pub fn current_index(&self) -> usize {
        self.current
    }
    pub fn current(&self) -> Option<&Tournament> {
        self.tournaments.get(self.current)
    }
    pub fn comparison(&self) -> Option<&Comparison> {
        self.current().map(Tournament::comparison)
    }
    pub fn unfinished(&self) -> bool {
        self.comparison()
            .is_some_and(|c| !c.confirmed && c.keep != c.defaults)
    }
    pub fn any_completed(&self) -> bool {
        self.tournaments.iter().any(|t| t.finalized)
    }
    pub fn replace(
        &mut self,
        mut clusters: Vec<Cluster>,
        decisions: &mut HashMap<MediaId, Decision>,
    ) {
        clusters.retain(|c| {
            c.ordered.len() >= 2
                && c.ordered
                    .iter()
                    .all(|id| c.scores.get(id).is_some_and(|s| s.is_finite()))
        });
        if self
            .tournaments
            .iter()
            .map(|t| &t.cluster)
            .eq(clusters.iter())
        {
            return;
        }
        self.restore_all(decisions);
        self.tournaments = clusters.into_iter().map(Tournament::new).collect();
        self.current = 0;
    }
    pub fn select_cluster(&mut self, index: usize) -> bool {
        if self.unfinished() || index >= self.tournaments.len() {
            return false;
        }
        self.current = index;
        true
    }
    pub fn select_comparison(&mut self, index: usize) -> bool {
        if self.unfinished() {
            return false;
        }
        if let Some(t) = self.tournaments.get_mut(self.current)
            && index < t.comparisons.len()
        {
            t.current = index;
            return true;
        }
        false
    }
    pub fn toggle(&mut self, id: MediaId, decisions: &mut HashMap<MediaId, Decision>) {
        let Some(t) = self.tournaments.get_mut(self.current) else {
            return;
        };
        if !t.comparison().paths.contains(&id) {
            return;
        }
        if t.comparison().confirmed {
            t.invalidate(decisions);
        }
        let c = &mut t.comparisons[t.current];
        *c.keep.get_mut(&id).unwrap() ^= true;
        c.resolve();
    }
    pub fn reset_current(&mut self, decisions: &mut HashMap<MediaId, Decision>) {
        let Some(t) = self.tournaments.get_mut(self.current) else {
            return;
        };
        if t.comparison().confirmed || t.finalized {
            t.invalidate(decisions);
        }
        let c = &mut t.comparisons[t.current];
        c.keep = c.defaults.clone();
        c.resolve();
    }
    fn restore_all(&self, decisions: &mut HashMap<MediaId, Decision>) {
        for t in self.tournaments.iter().rev() {
            if let Some(prior) = &t.prior {
                update_trash(decisions, prior.iter().map(|(id, b)| (*id, *b)));
            }
        }
    }
    pub fn reset_all(&mut self, decisions: &mut HashMap<MediaId, Decision>) {
        self.restore_all(decisions);
        self.discard_confirmations();
    }
    /// Keep the current choices and rounds, but remove marks from the rollback
    /// baseline after an explicit library-wide clear.
    pub fn clear_prior_trash_marks(&mut self) {
        for tournament in &mut self.tournaments {
            if let Some(prior) = &mut tournament.prior {
                for marked in prior.values_mut() {
                    *marked = false;
                }
            }
        }
    }
    /// Shared application discard already cleared marks; never replay old snapshots.
    pub fn discard_confirmations(&mut self) {
        for t in &mut self.tournaments {
            *t = Tournament::new(t.cluster.clone());
        }
    }
    /// A newer decision from another workflow supersedes this tournament's history.
    pub fn reconcile_external(&mut self, decisions: &HashMap<MediaId, Decision>) {
        for t in &mut self.tournaments {
            if t.prior.is_some()
                && t.expected(t.comparisons.len())
                    .iter()
                    .any(|(id, mark)| decisions.get(id).is_some_and(|d| d.rejected) != *mark)
            {
                *t = Tournament::new(t.cluster.clone());
            }
        }
    }
    pub fn confirm(&mut self, keep_all: bool, decisions: &mut HashMap<MediaId, Decision>) -> bool {
        let Some(t) = self.tournaments.get_mut(self.current) else {
            return false;
        };
        if t.comparison().confirmed && (!keep_all || t.comparison().keep_all()) {
            return false;
        }
        if keep_all && (t.comparison().confirmed || t.finalized) {
            t.invalidate(decisions);
        }
        t.ensure_prior(decisions);
        let c = &mut t.comparisons[t.current];
        if keep_all {
            for value in c.keep.values_mut() {
                *value = true;
            }
        }
        c.resolve();
        c.confirmed = true;
        update_trash(decisions, c.marks());
        let carrier = c.advancing;
        let carry = c.keeps(carrier);
        let none_kept = c.keep.values().all(|keep| !*keep);
        if t.next >= t.cluster.ordered.len() {
            t.finalized = true;
            if let Some(next) = self
                .tournaments
                .iter()
                .enumerate()
                .skip(self.current + 1)
                .find(|(_, t)| !t.finalized)
                .map(|(i, _)| i)
            {
                self.current = next;
            }
        } else {
            let remaining = &t.cluster.ordered[t.next..];
            let (paths, carried) = if none_kept && remaining.len() >= 2 {
                t.next += 2;
                (remaining[..2].to_vec(), None)
            } else {
                t.next += 1;
                (vec![carrier, remaining[0]], Some((carrier, carry)))
            };
            t.comparisons.truncate(t.current + 1);
            t.comparisons
                .push(Comparison::new(paths, &t.cluster.scores, carried));
            t.current += 1;
        }
        true
    }
}

/// Reuse completed shared analysis; never initialize models or read photos here.
pub fn from_analysis(
    media: &[super::model::Media],
    analysis: &HashMap<MediaId, super::analysis::Analysis>,
    groups: &HashMap<MediaId, usize>,
    metadata: &HashMap<MediaId, super::model::Metadata>,
    embeddings: &HashMap<MediaId, std::sync::Arc<Vec<f32>>>,
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
    let mut clusters = Vec::new();
    for (key, mut ordered) in grouped {
        if ordered.len() < 2
            || ordered.iter().any(|id| {
                analysis.get(id).is_none_or(|a| {
                    a.aesthetic.is_none() || a.faces.is_none() || !a.score.is_finite()
                })
            })
        {
            continue;
        }
        let sharpest = ordered
            .iter()
            .map(|id| {
                analysis[id]
                    .quality_sharpness
                    .unwrap_or(analysis[id].sharpness)
            })
            .fold(0., f64::max);
        let eligible = |id: &MediaId| {
            sharpest <= 0.
                || analysis[id]
                    .quality_sharpness
                    .unwrap_or(analysis[id].sharpness)
                    >= sharpest * 0.6
        };
        let floor = ordered
            .iter()
            .filter(|id| eligible(id))
            .map(|id| analysis[id].score)
            .fold(f64::INFINITY, f64::min);
        let ceiling = ordered
            .iter()
            .filter(|id| !eligible(id))
            .map(|id| analysis[id].score)
            .fold(f64::NEG_INFINITY, f64::max);
        let offset = (ceiling - floor + 0.2).max(0.);
        let scores = ordered
            .iter()
            .map(|id| {
                (
                    *id,
                    analysis[id].score - if eligible(id) { 0. } else { offset },
                )
            })
            .collect();
        if let Some(anchor) = ordered.first().and_then(|id| embeddings.get(id)) {
            ordered[1..].sort_by(|a, b| {
                let similarity =
                    |id: &MediaId| embeddings.get(id).and_then(|other| cosine(anchor, other));
                match (similarity(a), similarity(b)) {
                    (Some(a), Some(b)) => b.total_cmp(&a),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    _ => std::cmp::Ordering::Equal,
                }
            });
        }
        clusters.push(Cluster {
            key,
            ordered,
            scores,
        });
    }
    let order: HashMap<_, _> = media
        .iter()
        .map(|m| {
            let captured = metadata
                .get(&m.id)
                .map(|m| m.captured.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("");
            (m.id, (captured.is_empty(), captured, m.path.as_path()))
        })
        .collect();
    clusters.sort_by_key(|c| c.ordered.iter().map(|id| order[id]).min());
    clusters
}
fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let (mut dot, mut aa, mut bb) = (0., 0., 0.);
    for (a, b) in a.iter().zip(b) {
        let (a, b) = (f64::from(*a), f64::from(*b));
        dot += a * b;
        aa += a * a;
        bb += b * b;
    }
    let value = dot / (aa * bb).sqrt();
    value.is_finite().then_some(value.clamp(-1., 1.))
}
