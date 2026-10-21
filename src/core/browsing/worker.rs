//! Derived, in-memory view index. No scanning, image decoding, metadata IO or model ownership.
use super::*;
use crate::core::catalog::Catalog;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

pub struct Seed {
    pub catalog: Arc<Catalog>,
    pub updates: HashMap<MediaId, Update>,
    pub decisions: HashMap<MediaId, Decision>,
}
#[derive(Clone, Default)]
pub struct Update {
    pub embedding: Option<Arc<Vec<f32>>>,
    pub metadata: Option<Metadata>,
    pub group: Option<usize>,
    pub issues: bool,
    pub duplicate: bool,
}
#[derive(Clone, Default)]
pub struct Query {
    pub text: String,
    pub filter: Filter,
    pub options: Options,
    pub navigator: tree::State,
}
#[derive(Clone, Copy)]
pub struct GroupChoice {
    pub count: usize,
    pub example: MediaId,
}
pub struct ResultSet {
    pub group_choices: Arc<HashMap<usize, GroupChoice>>,
    pub tree: Option<tree::View>,
    pub tree_builds: usize,
    pub navigation: Arc<crate::core::navigation::Index>,
    pub run: u64,
    pub indices: Vec<usize>,
    pub groups: Vec<usize>,
    pub folders: Arc<Vec<Folder>>,
    /// Diagnostic counters prove that view changes reuse normalized keys.
    pub key_builds: usize,
    pub sorts: usize,
    pub group_order_builds: usize,
}
struct Row {
    embedding: Option<Arc<Vec<f32>>>,
    name: String,
    search: String,
    captured: Option<i64>,
    date: Option<String>,
    rating: Option<u8>,
    group: Option<usize>,
    issues: bool,
    duplicate: bool,
}
struct Index {
    group_choices: Option<Arc<HashMap<usize, GroupChoice>>>,
    trees: HashMap<tree::Mode, Arc<tree::Tree>>,
    tree_builds: usize,
    catalog: Arc<Catalog>,
    positions: HashMap<MediaId, usize>,
    rows: Vec<Row>,
    folders: Arc<Vec<Folder>>,
    decisions: HashMap<MediaId, Decision>,
    order: Option<(Sort, Vec<usize>)>,
    sorts: usize,
    group_orders: HashMap<GroupSort, Vec<usize>>,
    group_order_builds: usize,
}
impl Index {
    fn new(seed: Seed, cancelled: impl Fn() -> bool) -> Option<Self> {
        let mut rows = Vec::with_capacity(seed.catalog.media.len());
        let mut positions = HashMap::with_capacity(seed.catalog.media.len());
        for (i, m) in seed.catalog.media.iter().enumerate() {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            let update = seed.updates.get(&m.id);
            let metadata = update.and_then(|u| u.metadata.as_ref());
            let captured = dates::capture_millis(m, metadata);
            rows.push(Row {
                embedding: update.and_then(|u| u.embedding.clone()),
                name: m.name().to_lowercase(),
                search: m.path.to_string_lossy().to_lowercase(),
                captured,
                date: dates::calendar(captured, metadata),
                rating: metadata.map(|m| m.rating),
                group: update.and_then(|u| u.group),
                issues: update.is_some_and(|u| u.issues),
                duplicate: update.is_some_and(|u| u.duplicate),
            });
            positions.insert(m.id, i);
        }
        // Reuse scan inventory, including hidden and empty folders. Include
        // ancestors for synthetic catalogs too; no filesystem walk is needed.
        let mut paths = std::collections::BTreeSet::new();
        paths.insert(seed.catalog.root.clone());
        for (i, path) in seed
            .catalog
            .directories
            .iter()
            .map(|p| p.as_path())
            .chain(seed.catalog.media.iter().filter_map(|m| m.path.parent()))
            .enumerate()
        {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            for parent in path
                .ancestors()
                .take_while(|p| p.starts_with(&seed.catalog.root))
            {
                if !paths.insert(parent.to_path_buf()) {
                    break;
                }
            }
        }
        let mut folders = Vec::with_capacity(paths.len());
        for (i, path) in paths.into_iter().enumerate() {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            let relative = path.strip_prefix(&seed.catalog.root).unwrap_or(&path);
            let label = if relative.as_os_str().is_empty() {
                "Library root".into()
            } else {
                relative.to_string_lossy().into_owned()
            };
            folders.push(Folder { path, label });
        }
        Some(Self {
            group_choices: None,
            trees: HashMap::new(),
            tree_builds: 0,
            folders: Arc::new(folders),
            catalog: seed.catalog,
            positions,
            rows,
            decisions: seed.decisions,
            order: None,
            sorts: 0,
            group_orders: HashMap::new(),
            group_order_builds: 0,
        })
    }
    fn update(&mut self, id: MediaId, update: Update) {
        let Some(&i) = self.positions.get(&id) else {
            return;
        };
        let row = &mut self.rows[i];
        let captured = dates::capture_millis(&self.catalog.media[i], update.metadata.as_ref());
        let same_embedding = match (&row.embedding, &update.embedding) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if captured != row.captured || row.group != update.group || !same_embedding {
            self.group_orders.clear();
        }
        if row.group != update.group {
            self.group_choices = None;
        }
        row.embedding = update.embedding;
        if captured != row.captured
            && self
                .order
                .as_ref()
                .is_some_and(|(sort, _)| *sort != Sort::Name)
        {
            self.order = None;
        }
        let date = dates::calendar(captured, update.metadata.as_ref());
        if date != row.date {
            self.trees.remove(&tree::Mode::Dates);
        }
        row.date = date;
        if row.group != update.group {
            self.trees.remove(&tree::Mode::Groups);
        }
        row.captured = captured;
        row.rating = update.metadata.map(|m| m.rating);
        row.group = update.group;
        row.issues = update.issues;
        row.duplicate = update.duplicate;
    }
    fn query(&mut self, run: u64, query: Query, cancelled: impl Fn() -> bool) -> Option<ResultSet> {
        if self
            .order
            .as_ref()
            .is_none_or(|(sort, _)| *sort != query.options.sort)
        {
            let mut order: Vec<_> = (0..self.rows.len()).collect();
            cancellable_sort(
                &mut order,
                |a, b| {
                    let (a, b) = (&self.rows[a], &self.rows[b]);
                    compare(query.options.sort, a.captured, b.captured, &a.name, &b.name)
                },
                |a, b| self.catalog.media[a].path.cmp(&self.catalog.media[b].path),
                &cancelled,
            )?;
            self.order = Some((query.options.sort, order));
            self.sorts += 1;
        }
        if query.options.group_sort != GroupSort::Flat
            && !self.group_orders.contains_key(&query.options.group_sort)
        {
            let order = super::group_order::order(
                self.rows.iter().filter_map(|row| {
                    row.group
                        .map(|g| (g, row.captured, row.embedding.as_deref().map(Vec::as_slice)))
                }),
                query.options.group_sort,
                &cancelled,
            )?;
            self.group_orders.insert(query.options.group_sort, order);
            self.group_order_builds += 1;
        }
        if self.group_choices.is_none() {
            let mut choices: HashMap<usize, GroupChoice> = HashMap::new();
            for (i, row) in self.rows.iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return None;
                }
                if let Some(group) = row.group {
                    choices
                        .entry(group)
                        .and_modify(|c| c.count += 1)
                        .or_insert(GroupChoice {
                            count: 1,
                            example: self.catalog.media[i].id,
                        });
                }
            }
            self.group_choices = Some(Arc::new(choices));
        }
        let text = query.text.to_lowercase();
        let mut indices = Vec::new();
        let mut groups = std::collections::BTreeSet::new();
        for (n, &i) in self.order.as_ref()?.1.iter().enumerate() {
            if n % 256 == 0 && cancelled() {
                return None;
            }
            let row = &self.rows[i];
            if let Some(group) = row.group {
                groups.insert(group);
            }
            let decision = self.decisions.get(&self.catalog.media[i].id);
            let rating = decision.and_then(|d| d.rating).or(row.rating);
            if (text.is_empty() || row.search.contains(&text))
                && query.filter.matches(
                    decision.is_some_and(|d| d.rejected),
                    rating,
                    row.issues,
                    row.duplicate,
                )
                && query.options.rating.matches(rating)
                && query.options.group.is_none_or(|g| row.group == Some(g))
                && query.options.folder.matches(&self.catalog.media[i].path)
                && tree::date_matches(query.options.date.as_deref(), row.date.as_deref())
            {
                indices.push(i);
            }
        }
        let groups = if let Some(order) = self.group_orders.get(&query.options.group_sort) {
            let ranks: HashMap<_, _> = order
                .iter()
                .enumerate()
                .map(|(rank, g)| (*g, rank))
                .collect();
            // Stable bucket assembly preserves the cached photo sort inside each group.
            let mut buckets = vec![Vec::new(); order.len() + 1];
            for (n, i) in indices.into_iter().enumerate() {
                if n % 256 == 0 && cancelled() {
                    return None;
                }
                let rank = self.rows[i]
                    .group
                    .and_then(|g| ranks.get(&g).copied())
                    .unwrap_or(order.len());
                buckets[rank].push(i);
            }
            indices = buckets.into_iter().flatten().collect();
            order.clone()
        } else {
            groups.into_iter().collect()
        };
        let mut navigation = crate::core::navigation::Index::build(
            indices.iter().map(|i| {
                let id = self.catalog.media[*i].id;
                let row = &self.rows[*i];
                let decision = self.decisions.get(&id);
                (
                    id,
                    row.group,
                    decision.and_then(|d| d.rating).or(row.rating),
                    decision.is_some_and(|d| d.rejected),
                )
            }),
            &cancelled,
        )?;
        let tree = if query.navigator.enabled {
            let mode = query.navigator.mode;
            if !self.trees.contains_key(&mode) {
                let snapshot = tree::build(
                    mode,
                    &self.folders,
                    self.rows
                        .iter()
                        .zip(&self.catalog.media)
                        .map(|(row, media)| tree::Entry {
                            path: &media.path,
                            date: row.date.as_deref(),
                            group: row.group,
                        }),
                    &cancelled,
                )?;
                self.trees.insert(mode, Arc::new(snapshot));
                self.tree_builds += 1;
            }
            let tree = self.trees[&mode].clone();
            if mode != tree::Mode::Groups {
                navigation.set_siblings(
                    indices.iter().map(|&i| {
                        let media = &self.catalog.media[i];
                        let key = if mode == tree::Mode::Folders {
                            tree::Key::Folder(
                                media.path.parent().unwrap_or(&self.catalog.root).to_owned(),
                            )
                        } else {
                            tree::Key::Date(self.rows[i].date.clone().unwrap_or_default())
                        };
                        (media.id, tree.lookup.get(&key).copied())
                    }),
                    &cancelled,
                )?;
            }
            Some(tree.project(&query.navigator, &cancelled)?)
        } else {
            None
        };
        Some(ResultSet {
            tree,
            tree_builds: self.tree_builds,
            navigation: Arc::new(navigation),
            run,
            indices,
            groups,
            group_choices: self.group_choices.clone().unwrap(),
            folders: self.folders.clone(),
            key_builds: self.rows.len(),
            sorts: self.sorts,
            group_order_builds: self.group_order_builds,
        })
    }
}
// Bounded chunk sorts followed by cancellable merge passes. Cancellation is
// checked between chunks and every 256 merged entries, including huge libraries.
fn cancellable_sort(
    values: &mut Vec<usize>,
    primary: impl Fn(usize, usize) -> std::cmp::Ordering,
    tie: impl Fn(usize, usize) -> std::cmp::Ordering,
    cancelled: impl Fn() -> bool,
) -> Option<()> {
    let compare = |a: &usize, b: &usize| primary(*a, *b).then_with(|| tie(*a, *b));
    for chunk in values.chunks_mut(512) {
        if cancelled() {
            return None;
        }
        chunk.sort_unstable_by(&compare);
    }
    let mut buffer = vec![0; values.len()];
    let mut width = 512;
    while width < values.len() {
        for start in (0..values.len()).step_by(width * 2) {
            let mid = (start + width).min(values.len());
            let end = (mid + width).min(values.len());
            let (mut a, mut b) = (start, mid);
            for (offset, target) in buffer[start..end].iter_mut().enumerate() {
                if offset % 256 == 0 && cancelled() {
                    return None;
                }
                if a < mid && (b == end || !compare(&values[a], &values[b]).is_gt()) {
                    *target = values[a];
                    a += 1;
                } else {
                    *target = values[b];
                    b += 1;
                }
            }
        }
        std::mem::swap(values, &mut buffer);
        width *= 2;
    }
    if cancelled() { None } else { Some(()) }
}
#[derive(Default)]
struct Mailbox {
    reset: bool,
    seed: Option<Seed>,
    updates: HashMap<MediaId, Update>,
    decisions: Option<HashMap<MediaId, Decision>>,
    decision_updates: HashMap<MediaId, Decision>,
    groups: Option<Arc<HashMap<MediaId, usize>>>,
    clear: (bool, bool, bool),
    query: Option<(u64, Query)>,
    result: Option<ResultSet>,
}
/// One worker and one coalescing mailbox per application, independent of decoding.
/// Each key has at most one pending update; only the newest query/result is kept.
pub struct Worker {
    mailbox: Arc<Mutex<Mailbox>>,
    generation: Arc<AtomicU64>,
    reset_generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    notify: mpsc::SyncSender<()>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let generation = Arc::new(AtomicU64::new(0));
        let reset_generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (notify, rx) = mpsc::sync_channel(1);
        let (state, current, resets, stopping) = (
            mailbox.clone(),
            generation.clone(),
            reset_generation.clone(),
            stop.clone(),
        );
        let thread = thread::Builder::new()
            .name("photosort-browse".into())
            .spawn(move || {
                let mut index: Option<Index> = None;
                while rx.recv().is_ok() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let (mut pending, reset) = {
                        let mut mailbox = state.lock().unwrap();
                        let result = mailbox.result.take();
                        let pending = std::mem::take(&mut *mailbox);
                        mailbox.result = result;
                        (pending, resets.load(Ordering::Acquire))
                    };
                    if pending.reset {
                        index = pending.seed.take().and_then(|seed| {
                            Index::new(seed, || {
                                stopping.load(Ordering::Acquire)
                                    || resets.load(Ordering::Acquire) != reset
                            })
                        });
                    }
                    let Some(index) = index.as_mut() else {
                        continue;
                    };
                    if pending.clear.2 {
                        index.group_choices = None;
                        index.group_orders.clear();
                        index.trees.remove(&tree::Mode::Groups);
                    }
                    if pending.clear != (false, false, false) {
                        for row in &mut index.rows {
                            if pending.clear.0 {
                                row.issues = false;
                            }
                            if pending.clear.1 {
                                row.duplicate = false;
                            }
                            if pending.clear.2 {
                                index.group_choices = None;
                                row.group = None;
                            }
                        }
                    }
                    for (id, update) in pending.updates {
                        index.update(id, update);
                    }
                    if let Some(groups) = pending.groups {
                        index.group_choices = None;
                        index.group_orders.clear();
                        index.trees.remove(&tree::Mode::Groups);
                        for (id, i) in &index.positions {
                            index.rows[*i].group = groups.get(id).copied();
                        }
                    }
                    if let Some(decisions) = pending.decisions {
                        index.decisions = decisions;
                    }
                    index.decisions.extend(pending.decision_updates);
                    let Some((run, query)) = pending.query else {
                        continue;
                    };
                    let cancelled = || {
                        stopping.load(Ordering::Acquire) || current.load(Ordering::Acquire) != run
                    };
                    if cancelled() {
                        continue;
                    }
                    if let Some(result) = index.query(run, query, cancelled) {
                        let mut mailbox = state.lock().unwrap();
                        if !cancelled() {
                            mailbox.result = Some(result);
                        }
                        drop(mailbox);
                        wake();
                    }
                }
            })
            .expect("Start browsing worker");
        Self {
            mailbox,
            generation,
            reset_generation,
            stop,
            notify,
            thread: Some(thread),
        }
    }
    pub fn reset(&self, seed: Option<Seed>) {
        let mut mailbox = self.mailbox.lock().unwrap();
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.reset_generation.fetch_add(1, Ordering::AcqRel);
        *mailbox = Mailbox {
            reset: true,
            seed,
            ..Default::default()
        };
        drop(mailbox);
        let _ = self.notify.try_send(());
    }
    pub fn update(&self, id: MediaId, update: Update) {
        self.mailbox.lock().unwrap().updates.insert(id, update);
        // Updates are consumed with the next query, coalescing background batches.
    }
    /// Atomic group-only replacement preserves metadata, embeddings and normalized keys.
    pub fn groups(&self, groups: Arc<HashMap<MediaId, usize>>) {
        self.mailbox.lock().unwrap().groups = Some(groups);
    }
    pub fn decisions(&self, decisions: HashMap<MediaId, Decision>) {
        let mut mailbox = self.mailbox.lock().unwrap();
        mailbox.decisions = Some(decisions);
        mailbox.decision_updates.clear();
    }
    pub fn decision(&self, id: MediaId, decision: Decision) {
        self.decision_batch([(id, decision)]);
    }
    pub fn decision_batch(&self, changes: impl IntoIterator<Item = (MediaId, Decision)>) {
        self.mailbox
            .lock()
            .unwrap()
            .decision_updates
            .extend(changes);
    }
    pub fn clear_analysis(&self, issues: bool, duplicates: bool, groups: bool) {
        let mut mailbox = self.mailbox.lock().unwrap();
        mailbox.clear.0 |= issues;
        mailbox.clear.1 |= duplicates;
        mailbox.clear.2 |= groups;
        if groups {
            mailbox.groups = None;
        }
        for update in mailbox.updates.values_mut() {
            if issues {
                update.issues = false;
            }
            if duplicates {
                update.duplicate = false;
            }
            if groups {
                update.group = None;
            }
        }
    }
    pub fn request(&self, query: Query) -> u64 {
        let mut mailbox = self.mailbox.lock().unwrap();
        let run = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        mailbox.result = None;
        mailbox.query = Some((run, query));
        drop(mailbox);
        let _ = self.notify.try_send(());
        run
    }
    pub fn cancel(&self) {
        let mut mailbox = self.mailbox.lock().unwrap();
        self.generation.fetch_add(1, Ordering::AcqRel);
        mailbox.query = None;
        mailbox.result = None;
    }
    pub fn take_result(&self) -> Option<ResultSet> {
        self.mailbox.lock().unwrap().result.take()
    }
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.notify.try_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_sort_cancels_inside_large_merge_and_sorts_partial_tail() {
        let expected: Vec<_> = (0..2051).collect();
        let mut values: Vec<_> = expected.iter().rev().copied().collect();
        let checks = std::cell::Cell::new(0);
        let result = cancellable_sort(
            &mut values,
            |a, b| a.cmp(&b),
            |_, _| std::cmp::Ordering::Equal,
            || {
                checks.set(checks.get() + 1);
                checks.get() == 8 // five chunk checks, then cancellation during first merge pass
            },
        );
        assert!(result.is_none());
        assert_eq!(checks.get(), 8);
        cancellable_sort(
            &mut values,
            |a, b| a.cmp(&b),
            |_, _| std::cmp::Ordering::Equal,
            || false,
        )
        .unwrap();
        assert_eq!(values, expected);
    }
}
