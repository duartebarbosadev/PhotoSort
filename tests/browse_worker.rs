#[path = "../src/test_support.rs"]
mod test_support;
use photosort::core::{
    browsing::{self, Filter, Options, Query, RatingFilter, Seed, Sort, Update, Worker},
    catalog::Catalog,
    model::*,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
fn catalog(count: usize) -> Arc<Catalog> {
    Arc::new(Catalog {
        root: "/nonexistent".into(),
        media: (0..count)
            .map(|i| Media {
                id: MediaId(i as u64),
                path: format!("/nonexistent/folder-{}/Photo-{:06}.jpg", i % 7, count - i).into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint {
                    bytes: 1,
                    modified_ns: (i % 23) as u128 * 1_000_000,
                    ..Default::default()
                },
            })
            .collect(),
        ..Default::default()
    })
}
fn receive(worker: &Worker, run: u64) -> browsing::ResultSet {
    let deadline = Instant::now() + test_support::worker_patience();
    loop {
        let expired = Instant::now() >= deadline;
        if let Some(result) = worker.take_result() {
            assert_eq!(result.run, run, "obsolete query was published");
            return result;
        }
        assert!(!expired, "view query timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn worker_matches_shared_rating_sort_and_group_rules_without_filesystem_access() {
    let catalog = catalog(1031);
    let metadata: HashMap<_, _> = catalog
        .media
        .iter()
        .filter(|m| m.id.0 % 7 != 0)
        .map(|m| {
            (
                m.id,
                Metadata {
                    rating: (m.id.0 % 6) as u8,
                    captured_ms: Some((m.id.0 % 31) as i64),
                    ..Default::default()
                },
            )
        })
        .collect();
    let decisions = HashMap::from([(
        MediaId(7),
        Decision {
            rating: Some(0),
            ..Default::default()
        },
    )]);
    let groups: HashMap<_, _> = catalog
        .media
        .iter()
        .map(|m| (m.id, m.id.0 as usize % 5))
        .collect();
    let updates = catalog
        .media
        .iter()
        .map(|m| {
            (
                m.id,
                Update {
                    metadata: metadata.get(&m.id).cloned(),
                    group: groups.get(&m.id).copied(),
                    ..Default::default()
                },
            )
        })
        .collect();
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog: catalog.clone(),
        updates,
        decisions: decisions.clone(),
    }));
    for sort in Sort::ALL {
        for rating in RatingFilter::ALL {
            for group in [None, Some(3)] {
                let options = Options {
                    sort,
                    rating,
                    group,
                    ..Default::default()
                };
                let mut expected: Vec<_> = (0..catalog.media.len()).collect();
                browsing::apply(
                    &mut expected,
                    &catalog.media,
                    &metadata,
                    &decisions,
                    &groups,
                    &options,
                );
                let run = worker.request(Query {
                    options,
                    ..Default::default()
                });
                let result = receive(&worker, run);
                assert_eq!(result.indices, expected);
                assert_eq!(result.groups, vec![0, 1, 2, 3, 4]);
                assert_eq!(result.key_builds, 1031);
            }
        }
    }
}
#[test]
fn changed_records_and_staged_zero_reuse_sort_keys_and_update_every_filter() {
    let catalog = catalog(100);
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog,
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    let run = worker.request(Query::default());
    assert_eq!(receive(&worker, run).sorts, 1);
    worker.update(
        MediaId(10),
        Update {
            metadata: Some(Metadata {
                rating: 5,
                ..Default::default()
            }),
            group: Some(9),
            issues: true,
            duplicate: true,
            ..Default::default()
        },
    );
    for filter in [Filter::Rated, Filter::Issues, Filter::Duplicates] {
        let run = worker.request(Query {
            filter,
            text: "PHOTO".into(),
            ..Default::default()
        });
        let result = receive(&worker, run);
        assert_eq!(result.indices, vec![10]);
        assert_eq!(result.sorts, 1, "non-date changes must not sort again");
    }
    worker.decision(
        MediaId(10),
        Decision {
            rating: Some(0),
            rejected: true,
            ..Default::default()
        },
    );
    let run = worker.request(Query {
        filter: Filter::Rejected,
        options: Options {
            rating: RatingFilter::Exactly(0),
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(receive(&worker, run).indices, vec![10]);
    // Snapshot replacement clears earlier deltas; a later individual edit wins.
    worker.decision(
        MediaId(11),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    worker.decisions(HashMap::new());
    worker.decision(
        MediaId(12),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let run = worker.request(Query {
        options: Options {
            rating: RatingFilter::Exactly(4),
            ..Default::default()
        },
        ..Default::default()
    });
    assert_eq!(receive(&worker, run).indices, vec![12]);
    worker.update(
        MediaId(10),
        Update {
            metadata: Some(Metadata {
                captured_ms: Some(-123),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    let run = worker.request(Query::default());
    let result = receive(&worker, run);
    assert_eq!(result.indices[0], 10);
    assert_eq!(result.sorts, 2);
}
#[test]
fn bulk_analysis_reset_preserves_metadata_and_clears_pending_groups() {
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog: catalog(2),
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    worker.update(
        MediaId(0),
        Update {
            metadata: Some(Metadata {
                rating: 5,
                ..Default::default()
            }),
            issues: true,
            duplicate: true,
            group: Some(8),
            ..Default::default()
        },
    );
    worker.clear_analysis(true, true, true);
    let run = worker.request(Query {
        filter: Filter::Rated,
        ..Default::default()
    });
    let result = receive(&worker, run);
    assert_eq!(result.indices, vec![0]);
    assert!(result.groups.is_empty());
    for filter in [Filter::Issues, Filter::Duplicates] {
        let run = worker.request(Query {
            filter,
            ..Default::default()
        });
        assert!(receive(&worker, run).indices.is_empty());
    }
}
#[test]
fn latest_query_wins_and_new_folder_cannot_receive_old_indices() {
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog: catalog(100_000),
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    for n in 0..100 {
        worker.request(Query {
            text: format!("Photo-{n}"),
            options: Options {
                sort: if n % 2 == 0 { Sort::Name } else { Sort::Newest },
                ..Default::default()
            },
            ..Default::default()
        });
    }
    worker.cancel();
    assert!(worker.take_result().is_none());
    worker.reset(Some(Seed {
        catalog: catalog(3),
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    let run = worker.request(Query {
        text: "photo-000002".into(),
        ..Default::default()
    });
    let result = receive(&worker, run);
    assert_eq!(result.indices, vec![1]);
    assert_eq!(result.key_builds, 3);
    worker.reset(None);
    worker.request(Query::default());
    worker.cancel();
    assert!(worker.take_result().is_none());
}
#[test]
fn hundred_thousand_item_queries_reuse_keys_and_return_expected_results() {
    let worker = Worker::new(Arc::new(|| {}));
    let catalog = catalog(100_000);
    let start = Instant::now();
    worker.reset(Some(Seed {
        catalog,
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    let run = worker.request(Query::default());
    let initial = receive(&worker, run);
    assert_eq!(initial.indices.len(), 100_000);
    let cold = start.elapsed();
    let start = Instant::now();
    let submitted = Instant::now();
    let run = worker.request(Query {
        text: "PHOTO-000001".into(),
        ..Default::default()
    });
    let submit = submitted.elapsed();
    let result = receive(&worker, run);
    assert_eq!(result.indices, vec![99_999]);
    assert_eq!(result.key_builds, 100_000);
    assert_eq!(result.sorts, initial.sorts);
    eprintln!(
        "100k browsing: cold {cold:?}; warm query {:?}; submit {submit:?}; keys {}; sorts {}",
        start.elapsed(),
        result.key_builds,
        result.sorts
    );
}

#[test]
fn folder_scopes_reuse_scan_inventory_and_cached_keys_with_all_other_filters() {
    use browsing::FolderFilter;
    let names = [
        "root.jpg",
        "Trip/a.jpg",
        "Trip/nested/b.jpg",
        "Trip-2/c.jpg",
        ".hidden/d.jpg",
    ];
    let catalog = Arc::new(Catalog {
        root: "/no-files-here".into(),
        media: names
            .iter()
            .enumerate()
            .map(|(i, name)| Media {
                id: MediaId(i as u64),
                path: format!("/no-files-here/{name}").into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect(),
        directories: vec!["/no-files-here/.hidden/empty".into()],
        ..Default::default()
    });
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog: catalog.clone(),
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    let first = receive(&worker, worker.request(Query::default()));
    assert_eq!(
        first
            .folders
            .iter()
            .map(|f| f.label.as_str())
            .collect::<Vec<_>>(),
        [
            "Library root",
            ".hidden",
            ".hidden/empty",
            "Trip",
            "Trip/nested",
            "Trip-2"
        ]
    );
    for (folder, recursive, expected) in [
        ("", false, vec![0]),
        ("Trip", false, vec![1]),
        ("Trip", true, vec![1, 2]),
        (".hidden", true, vec![4]),
        (".hidden/empty", true, vec![]),
        ("missing", true, vec![]),
    ] {
        let options = Options {
            folder: FolderFilter {
                path: Some(catalog.root.join(folder)),
                recursive,
                ..Default::default()
            },
            ..Default::default()
        };
        let actual = receive(
            &worker,
            worker.request(Query {
                options: options.clone(),
                ..Default::default()
            }),
        );
        let mut pure: Vec<_> = (0..5).collect();
        browsing::apply(
            &mut pure,
            &catalog.media,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &options,
        );
        assert_eq!(actual.indices, expected);
        assert_eq!(actual.indices, pure);
        assert_eq!(actual.sorts, 1);
        assert_eq!(actual.key_builds, 5);
        assert!(
            Arc::ptr_eq(&first.folders, &actual.folders),
            "folder choices rebuilt per query"
        );
    }
    worker.update(
        MediaId(2),
        Update {
            group: Some(7),
            issues: true,
            ..Default::default()
        },
    );
    worker.decision(
        MediaId(2),
        Decision {
            rating: Some(5),
            ..Default::default()
        },
    );
    let actual = receive(
        &worker,
        worker.request(Query {
            navigator: Default::default(),
            text: "b.jpg".into(),
            filter: Filter::Issues,
            options: Options {
                folder: FolderFilter {
                    path: Some(catalog.root.join("Trip")),
                    ..Default::default()
                },
                group: Some(7),
                rating: RatingFilter::Exactly(5),
                ..Default::default()
            },
        }),
    );
    assert_eq!(actual.indices, vec![2]);
    assert_eq!(actual.sorts, 1);
    assert!(Arc::ptr_eq(&first.folders, &actual.folders));
    worker.reset(Some(Seed {
        catalog: Arc::new(Catalog {
            root: "/replacement".into(),
            ..Default::default()
        }),
        updates: HashMap::new(),
        decisions: HashMap::new(),
    }));
    let replaced = receive(&worker, worker.request(Query::default()));
    assert_eq!(replaced.folders.len(), 1);
    assert_eq!(replaced.folders[0].path.to_str(), Some("/replacement"));
    assert!(!Arc::ptr_eq(&first.folders, &replaced.folders));
}

#[test]
fn navigation_snapshot_tracks_visible_order_and_reuses_metadata_and_sort_keys() {
    use photosort::core::navigation::Preferences;
    let catalog = catalog(100_000);
    let worker = Worker::new(Arc::new(|| {}));
    let updates = catalog
        .media
        .iter()
        .map(|m| {
            (
                m.id,
                Update {
                    metadata: Some(Metadata {
                        rating: (m.id.0 % 6) as u8,
                        ..Default::default()
                    }),
                    group: Some(m.id.0 as usize / 10),
                    ..Default::default()
                },
            )
        })
        .collect();
    worker.reset(Some(Seed {
        catalog,
        updates,
        decisions: HashMap::new(),
    }));
    let query = Query {
        options: Options {
            sort: Sort::Name,
            ..Default::default()
        },
        ..Default::default()
    };
    let first = receive(&worker, worker.request(query.clone()));
    // Names are reverse ID order. Staged ratings must override cached EXIF.
    assert_eq!(
        first.navigation.target(
            None,
            1,
            false,
            Preferences {
                rating: Some(5),
                ..Default::default()
            },
            false,
            false
        ),
        Some(MediaId(99995))
    );
    worker.decision(
        MediaId(99999),
        Decision {
            rating: Some(5),
            ..Default::default()
        },
    );
    let second = receive(&worker, worker.request(query.clone()));
    assert_eq!(
        (second.key_builds, second.sorts),
        (first.key_builds, first.sorts)
    );
    assert_eq!(
        second.navigation.target(
            None,
            1,
            false,
            Preferences {
                rating: Some(5),
                ..Default::default()
            },
            false,
            false
        ),
        Some(MediaId(99999))
    );
    let filtered = receive(
        &worker,
        worker.request(Query {
            text: "Photo-000010".into(),
            ..query
        }),
    );
    assert_eq!(filtered.indices.len(), 1);
    assert_eq!(
        filtered
            .navigation
            .target(None, 1, true, Preferences::default(), false, false),
        Some(MediaId(99990))
    );
}

#[test]
fn group_pca_order_matches_captured_python_reference() {
    #[derive(serde::Deserialize)]
    struct Group {
        id: usize,
        time: i64,
        vectors: Vec<Vec<f32>>,
    }
    #[derive(serde::Deserialize)]
    struct Case {
        groups: Vec<Group>,
        expected: Vec<usize>,
    }
    let cases: Vec<Case> =
        serde_json::from_str(include_str!("group_order_reference.json")).unwrap();
    for case in cases {
        let count = case.groups.iter().map(|g| g.vectors.len().max(1)).sum();
        let worker = Worker::new(Arc::new(|| {}));
        let mut updates = HashMap::new();
        let mut i = 0;
        for group in case.groups {
            for k in 0..group.vectors.len().max(1) {
                updates.insert(
                    MediaId(i),
                    Update {
                        group: Some(group.id),
                        metadata: Some(Metadata {
                            captured_ms: Some((group.time + k as i64) * 1000),
                            ..Default::default()
                        }),
                        embedding: group.vectors.get(k).cloned().map(Arc::new),
                        ..Default::default()
                    },
                );
                i += 1;
            }
        }
        worker.reset(Some(Seed {
            catalog: catalog(count),
            updates,
            decisions: HashMap::new(),
        }));
        let result = receive(
            &worker,
            worker.request(Query {
                options: Options {
                    group_sort: browsing::GroupSort::Similarity,
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        assert_eq!(result.groups, case.expected);
        assert_eq!(result.indices.len(), count);
    }
}

#[test]
fn group_order_cache_reuses_embeddings_and_invalidates_only_relevant_changes() {
    use browsing::GroupSort;
    let worker = Worker::new(Arc::new(|| {}));
    let mut updates = HashMap::new();
    for (i, (group, time, x)) in [
        (10, 300, Some(-1.0)),
        (20, 100, Some(2.0)),
        (30, 200, Some(0.0)),
        (40, 50, None),
    ]
    .into_iter()
    .enumerate()
    {
        for k in 0..2 {
            updates.insert(
                MediaId((i * 2 + k) as u64),
                Update {
                    group: Some(group),
                    metadata: Some(Metadata {
                        captured_ms: Some(time + k as i64),
                        rating: 4,
                        ..Default::default()
                    }),
                    embedding: x.map(|x| Arc::new(vec![x, 0.0])),
                    ..Default::default()
                },
            );
        }
    }
    worker.reset(Some(Seed {
        catalog: catalog(9),
        updates: updates.clone(),
        decisions: HashMap::new(),
    }));
    let mut query = Query {
        options: Options {
            group_sort: GroupSort::Similarity,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = receive(&worker, worker.request(query.clone()));
    assert_eq!(result.groups, vec![10, 30, 20, 40]);
    assert_eq!(result.indices, vec![0, 1, 4, 5, 2, 3, 6, 7, 8]);
    assert_eq!(result.group_order_builds, 1);
    // Navigation follows precisely the displayed grouped order.
    assert_eq!(
        result
            .navigation
            .target(Some(MediaId(1)), 1, false, Default::default(), false, false),
        Some(MediaId(4))
    );
    query.options.rating = RatingFilter::Exactly(5);
    worker.decision(
        MediaId(4),
        Decision {
            rating: Some(5),
            ..Default::default()
        },
    );
    let result = receive(&worker, worker.request(query.clone()));
    assert_eq!(result.indices, vec![4]);
    assert_eq!((result.group_order_builds, result.sorts), (1, 1));
    let mut update = updates[&MediaId(1)].clone();
    update.metadata.as_mut().unwrap().rating = 5;
    worker.update(MediaId(1), update);
    assert_eq!(
        receive(&worker, worker.request(query.clone())).group_order_builds,
        1
    );
    query.options.rating = RatingFilter::All;
    query.options.group_sort = GroupSort::Time;
    let result = receive(&worker, worker.request(query.clone()));
    assert_eq!(result.groups, vec![40, 20, 30, 10]);
    assert_eq!(result.indices, vec![6, 7, 2, 3, 4, 5, 0, 1, 8]);
    query.options.group_sort = GroupSort::Similarity;
    assert_eq!(
        receive(&worker, worker.request(query.clone())).group_order_builds,
        2,
        "both order modes remain cached"
    );
    let mut update = updates[&MediaId(0)].clone();
    update.embedding = Some(Arc::new(vec![9.0, 0.0]));
    worker.update(MediaId(0), update);
    let result = receive(&worker, worker.request(query.clone()));
    assert_eq!(result.groups, vec![30, 20, 10, 40]);
    assert_eq!(result.group_order_builds, 3);
    worker.clear_analysis(false, false, true);
    let result = receive(&worker, worker.request(query));
    assert!(result.groups.is_empty());
    assert_eq!(result.indices.len(), 9, "unassigned photos are retained");
}

#[test]
fn hundred_thousand_grouped_rows_keep_order_cached_across_filters() {
    use browsing::GroupSort;
    let count = 100_000;
    let worker = Worker::new(Arc::new(|| {}));
    let updates = (0..count)
        .map(|i| {
            (
                MediaId(i as u64),
                Update {
                    group: Some(i % 100),
                    metadata: Some(Metadata {
                        captured_ms: Some((count - i) as i64),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            )
        })
        .collect();
    worker.reset(Some(Seed {
        catalog: catalog(count),
        updates,
        decisions: HashMap::new(),
    }));
    let mut query = Query {
        options: Options {
            group_sort: GroupSort::Time,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = receive(&worker, worker.request(query.clone()));
    assert_eq!(result.groups, (0..100).rev().collect::<Vec<_>>());
    assert_eq!(result.indices.len(), count);
    query.options.group = Some(45);
    let result = receive(&worker, worker.request(query));
    assert_eq!(result.indices.len(), 1000);
    assert_eq!(
        (result.group_order_builds, result.key_builds, result.sorts),
        (1, count, 1)
    );
}

fn tree_catalog() -> Arc<Catalog> {
    Arc::new(Catalog {
        root: "/library".into(),
        directories: vec![
            "/library/a".into(),
            "/library/a/deep".into(),
            "/library/b".into(),
            "/library/.hidden".into(),
            "/library/empty".into(),
        ],
        media: ["a/x.jpg", "a/deep/y.jpg", "a/z.jpg", "b/q.jpg"]
            .into_iter()
            .enumerate()
            .map(|(n, p)| Media {
                id: MediaId(n as u64),
                path: format!("/library/{p}").into(),
                kind: MediaKind::Photo,
                fingerprint: Default::default(),
            })
            .collect(),
        ..Default::default()
    })
}
fn tree_seed() -> Seed {
    Seed {
        catalog: tree_catalog(),
        decisions: HashMap::new(),
        updates: (0..4)
            .map(|n| {
                (
                    MediaId(n),
                    Update {
                        metadata: Some(Metadata {
                            captured: [
                                "2024:04:01 12:00:00",
                                "2024:04:01 12:00:00",
                                "2024:04:02 12:00:00",
                                "2024:05:01 12:00:00",
                            ][n as usize]
                                .into(),
                            rating: 5,
                            ..Default::default()
                        }),
                        group: Some(if n == 0 || n == 3 { 1 } else { 2 }),
                        ..Default::default()
                    },
                )
            })
            .collect(),
    }
}
#[test]
fn cached_tree_preserves_hierarchy_counts_empty_folders_and_siblings_without_io() {
    use browsing::tree::{Key, Mode, State};
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(tree_seed()));
    let mut query = Query {
        navigator: State {
            enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let first = receive(&worker, worker.request(query.clone()));
    let view = first.tree.unwrap();
    let tree = &view.tree;
    let index = |p: &str| tree.lookup[&Key::Folder(p.into())];
    assert_eq!(tree.nodes[index("/library")].count, 4);
    assert_eq!(tree.nodes[index("/library/a")].count, 3);
    assert_eq!(tree.nodes[index("/library/.hidden")].count, 0);
    assert_eq!(tree.nodes[index("/library/empty")].count, 0);
    assert_eq!(tree.nodes[index("/library")].end, tree.nodes.len());
    assert_eq!(
        tree.nodes[index("/library/a/deep")].parent,
        Some(index("/library/a"))
    );
    assert_eq!(
        tree.nodes[index("/library/b")].previous,
        Some(index("/library/a"))
    );
    assert_eq!(
        first
            .navigation
            .target(Some(MediaId(0)), 1, true, Default::default(), false, false),
        Some(MediaId(2)),
        "Folder cycling must skip a child folder"
    );
    query
        .navigator
        .collapsed
        .insert(Key::Folder("/library/a".into()));
    let collapsed = receive(&worker, worker.request(query.clone()));
    let projected = collapsed.tree.unwrap();
    assert!(Arc::ptr_eq(tree, &projected.tree));
    assert!(!projected.visible.contains(&index("/library/a/deep")));
    assert!(projected.visible.contains(&index("/library/b")));
    assert_eq!(
        (collapsed.tree_builds, collapsed.key_builds, collapsed.sorts),
        (1, 4, 1)
    );
    assert_eq!(
        collapsed.indices.len(),
        4,
        "Collapsing navigation must not silently hide photos"
    );
    assert!(tree.project(&query.navigator, || true).is_none());
    query.navigator.mode = Mode::Dates;
    let dated = receive(&worker, worker.request(query.clone()));
    let tree = dated.tree.unwrap().tree;
    assert_eq!(tree.nodes[tree.lookup[&Key::Date("2024".into())]].count, 4);
    assert_eq!(
        tree.nodes[tree.lookup[&Key::Date("2024-04".into())]].count,
        3
    );
    assert_eq!(
        dated
            .navigation
            .target(Some(MediaId(0)), 1, true, Default::default(), false, false),
        Some(MediaId(1)),
        "Date cycling uses the calendar day"
    );
    query.options.date = Some("2024-04".into());
    query.options.folder.path = Some("/library/a".into());
    query.options.folder.recursive = false;
    query.options.group = Some(1);
    query.options.rating = RatingFilter::Exactly(5);
    let intersection = receive(&worker, worker.request(query));
    assert_eq!(intersection.indices, vec![0]);
    assert_eq!(intersection.tree_builds, 2);
    assert!(Arc::ptr_eq(&tree, &intersection.tree.unwrap().tree));
}
#[test]
fn tree_invalidation_is_scoped_and_date_filters_reuse_shared_calendar_keys() {
    use browsing::tree::{Key, Mode, State};
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(tree_seed()));
    let mut query = Query {
        navigator: State {
            enabled: true,
            mode: Mode::Dates,
            ..Default::default()
        },
        ..Default::default()
    };
    let original = receive(&worker, worker.request(query.clone()))
        .tree
        .unwrap()
        .tree;
    worker.decision(
        MediaId(0),
        Decision {
            rating: Some(0),
            ..Default::default()
        },
    );
    let ratings = receive(&worker, worker.request(query.clone()));
    assert_eq!(ratings.tree_builds, 1);
    assert!(Arc::ptr_eq(&original, &ratings.tree.unwrap().tree));
    let mut update = tree_seed().updates.remove(&MediaId(0)).unwrap();
    update.metadata.as_mut().unwrap().captured = "2025:01:02 12:00:00".into();
    worker.update(MediaId(0), update);
    let changed = receive(&worker, worker.request(query.clone()));
    assert_eq!(changed.tree_builds, 2);
    assert!(
        changed
            .tree
            .unwrap()
            .tree
            .lookup
            .contains_key(&Key::Date("2025-01-02".into()))
    );
    query.navigator.mode = Mode::Groups;
    let groups = receive(&worker, worker.request(query.clone()));
    assert_eq!(groups.tree_builds, 3);
    assert_eq!(groups.tree.unwrap().tree.nodes.len(), 2);
    worker.clear_analysis(false, false, true);
    let cleared = receive(&worker, worker.request(query));
    assert_eq!(cleared.tree_builds, 4);
    assert!(cleared.tree.unwrap().tree.nodes.is_empty());
    assert!(browsing::tree::date_matches(Some(""), None));
    assert!(!browsing::tree::date_matches(
        Some("2024-0"),
        Some("2024-04-01")
    ));
    let old: Options = serde_json::from_str(
        r#"{"rating":"All","group":null,"sort":"Oldest","folder":{"path":null,"recursive":true}}"#,
    )
    .unwrap();
    assert!(old.date.is_none());
}
#[test]
fn hundred_thousand_tree_entries_are_cached_and_latest_projection_wins() {
    use browsing::tree::{Key, Mode, State};
    let worker = Worker::new(Arc::new(|| {}));
    let catalog = catalog(100_000);
    let updates = catalog
        .media
        .iter()
        .map(|m| {
            (
                m.id,
                Update {
                    group: Some(m.id.0 as usize),
                    ..Default::default()
                },
            )
        })
        .collect();
    worker.reset(Some(Seed {
        catalog,
        updates,
        decisions: HashMap::new(),
    }));
    let mut query = Query {
        navigator: State {
            enabled: true,
            mode: Mode::Groups,
            ..Default::default()
        },
        ..Default::default()
    };
    let first = receive(&worker, worker.request(query.clone()));
    let tree = first.tree.unwrap().tree;
    assert_eq!(tree.nodes.len(), 100_000);
    query.text = "Photo-000001".into();
    let narrow = receive(&worker, worker.request(query.clone()));
    assert_eq!(narrow.indices.len(), 1);
    assert_eq!(
        (narrow.tree_builds, narrow.sorts, narrow.key_builds),
        (1, 1, 100_000)
    );
    assert!(Arc::ptr_eq(&tree, &narrow.tree.unwrap().tree));
    query.navigator.mode = Mode::Folders;
    query
        .navigator
        .collapsed
        .insert(Key::Folder("/nonexistent".into()));
    worker.request(Query {
        navigator: State {
            enabled: true,
            mode: Mode::Dates,
            ..Default::default()
        },
        ..query.clone()
    });
    let last = receive(&worker, worker.request(query));
    let view = last.tree.unwrap();
    assert_eq!(view.mode, Mode::Folders);
    assert_eq!(view.visible, vec![0]);
    worker.reset(Some(tree_seed()));
    let replacement = receive(
        &worker,
        worker.request(Query {
            navigator: State {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        }),
    );
    assert_eq!(replacement.key_builds, 4);
    assert!(
        !replacement
            .tree
            .unwrap()
            .tree
            .lookup
            .contains_key(&Key::Folder("/nonexistent".into()))
    );
}

#[test]
fn date_tree_preserves_written_camera_day_across_timezone_offsets() {
    let metadata = Metadata {
        captured: "2024-04-01T00:30:00+14:00".into(),
        ..Default::default()
    };
    let millis = photosort::core::dates::timestamp(&metadata.captured);
    assert_eq!(
        photosort::core::dates::calendar(millis, Some(&metadata)).as_deref(),
        Some("2024-04-01")
    );
    assert_eq!(photosort::core::dates::calendar(None, None), None);
}

#[cfg(feature = "metadata-native")]
#[test]
#[ignore = "requires the seven original samples; read-only metadata/navigation check"]
fn calendar_and_folder_navigation_reuse_all_seven_sample_metadata_snapshots() -> anyhow::Result<()>
{
    use browsing::tree::{Key, Mode, State};
    use photosort::core::{operations, pipeline::Pipeline};
    let root = std::path::PathBuf::from(
        std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"),
    );
    let catalog = Arc::new(Catalog::scan(&root, || false, |_| {})?);
    assert_eq!(catalog.media.len(), 7);
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(32 * 1024 * 1024, cache.path().into());
    let mut metadata = HashMap::new();
    let mut hashes = HashMap::new();
    for media in &catalog.media {
        hashes.insert(media.id, operations::content_hash(&media.path, || false)?);
        metadata.insert(media.id, pipeline.metadata(media)?);
    }
    let before = pipeline.stats;
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog: catalog.clone(),
        updates: metadata
            .iter()
            .map(|(id, m)| {
                (
                    *id,
                    Update {
                        metadata: Some(m.clone()),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        decisions: HashMap::new(),
    }));
    let mut query = Query {
        navigator: State {
            enabled: true,
            mode: Mode::Dates,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = receive(&worker, worker.request(query.clone()));
    let tree = result.tree.unwrap().tree;
    assert_eq!(
        tree.nodes
            .iter()
            .filter(|n| n.parent.is_none())
            .map(|n| n.count)
            .sum::<usize>(),
        7
    );
    for node in &tree.nodes {
        let Key::Date(date) = &node.key else {
            continue;
        };
        query.options.date = Some(date.clone());
        let actual = receive(&worker, worker.request(query.clone()));
        assert_eq!(actual.indices.len(), node.count);
        let mut expected: Vec<_> = (0..7).collect();
        browsing::apply(
            &mut expected,
            &catalog.media,
            &metadata,
            &HashMap::new(),
            &HashMap::new(),
            &query.options,
        );
        assert_eq!(actual.indices, expected);
        assert!(Arc::ptr_eq(&tree, &actual.tree.unwrap().tree));
    }
    query.options.date = None;
    query.navigator.mode = Mode::Folders;
    let result = receive(&worker, worker.request(query));
    assert_eq!(result.indices.len(), 7);
    assert_eq!(result.tree.unwrap().tree.nodes[0].count, 7);
    for media in &catalog.media {
        let _ = pipeline.metadata(media)?;
        assert_eq!(
            hashes[&media.id],
            operations::content_hash(&media.path, || false)?
        );
    }
    assert_eq!(pipeline.stats.decodes, 0);
    assert_eq!(pipeline.stats.metadata_reads, before.metadata_reads);
    println!(
        "Seven original formats: {} metadata reads, zero image decodes; date/folder queries reused the tree and all source hashes are unchanged",
        before.metadata_reads
    );
    Ok(())
}

#[test]
fn atomic_manual_group_replacement_keeps_metadata_keys_filters_and_order_caches() {
    let catalog = catalog(1000);
    let worker = Worker::new(Arc::new(|| {}));
    worker.reset(Some(Seed {
        catalog: catalog.clone(),
        updates: catalog
            .media
            .iter()
            .map(|m| {
                (
                    m.id,
                    Update {
                        metadata: Some(Metadata {
                            rating: 5,
                            captured_ms: Some(100),
                            ..Default::default()
                        }),
                        group: Some(8),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        decisions: HashMap::new(),
    }));
    let first = receive(&worker, worker.request(Query::default()));
    worker.groups(Arc::new(HashMap::from([
        (MediaId(3), 12),
        (MediaId(4), 12),
    ])));
    let query = Query {
        options: Options {
            group: Some(12),
            rating: RatingFilter::Exactly(5),
            ..Default::default()
        },
        ..Default::default()
    };
    let grouped = receive(&worker, worker.request(query));
    assert_eq!(grouped.indices.len(), 2);
    assert_eq!(grouped.groups, vec![12]);
    assert_eq!(grouped.group_choices[&12].count, 2);
    assert_eq!(grouped.key_builds, first.key_builds);
    assert_eq!(grouped.sorts, first.sorts);
    let reused = receive(&worker, worker.request(Query::default()));
    assert!(Arc::ptr_eq(&grouped.group_choices, &reused.group_choices));
    worker.groups(Arc::default());
    let cleared = receive(&worker, worker.request(Query::default()));
    assert!(cleared.groups.is_empty());
    assert_eq!(cleared.indices.len(), 1000);
    assert_eq!(cleared.key_builds, first.key_builds);
}
