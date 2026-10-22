use super::*;
use crate::core::{
    catalog::Catalog,
    operations::{Action, Source},
    views::Bookmark,
};
use std::{
    cell::Cell,
    collections::HashMap,
    panic::{AssertUnwindSafe, catch_unwind},
};

fn fixture() -> (tempfile::TempDir, Store, Context, Plan) {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("library");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.jpg"), b"first photo").unwrap();
    fs::write(root.join("b.jpg"), b"second photo").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let first = catalog.media.iter().find(|m| m.name() == "a.jpg").unwrap();
    let session = Session {
        decisions: catalog
            .media
            .iter()
            .map(|m| {
                (
                    m.id,
                    Decision {
                        rating: Some(4),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        fingerprints: catalog
            .media
            .iter()
            .map(|m| (m.id, m.fingerprint))
            .collect(),
        ..Default::default()
    };
    let views = views::Saved {
        step: Some(Step::Cull),
        bookmarks: HashMap::from([(
            Step::Cull,
            Bookmark {
                active: Some(first.id),
                ..Default::default()
            },
        )]),
        fingerprints: HashMap::from([(first.id, first.fingerprint)]),
    };
    let actions = ["a.jpg", "b.jpg"]
        .into_iter()
        .map(|name| Action::Move {
            files: vec![(
                Source::capture(root.join(name)).unwrap(),
                root.join("Sorted").join(name),
            )],
        })
        .collect();
    let plan = Plan {
        actions,
        description: "Review moves".into(),
        output_root: Some(root.clone()),
        ..Default::default()
    };
    let context = Context {
        root,
        rotation_only: false,
        media: catalog.media,
        session,
        views,
    };
    (temp, Store::new(base.join("journals")), context, plan)
}
#[test]
fn recovery_details_preserve_source_failure_chain_through_restart_and_retry() {
    for kind in 0..5 {
        let (temp, store, context, mut plan) = fixture();
        let source = Source::capture(context.root.join("a.jpg")).unwrap();
        let destination = context
            .root
            .join(if kind == 4 { "A.jpg" } else { "Sorted/a.jpg" });
        let action = match kind {
            0 => Action::Move {
                files: vec![(source.clone(), destination.clone())],
            },
            1 => Action::Trash {
                files: vec![source.clone()],
            },
            2 | 3 => Action::Metadata {
                source: source.clone(),
                sidecar_before: None,
                target: if kind == 2 {
                    crate::core::metadata::WriteTarget::Sidecar
                } else {
                    crate::core::metadata::WriteTarget::Embedded
                },
                rating: Some(4),
                orientation: None,
            },
            _ => Action::RenameFile {
                source: source.clone(),
                to: destination.clone(),
            },
        };
        let label = action.label();
        plan.actions = vec![action];
        let held = temp.path().join("held.jpg");
        fs::rename(&source.path, &held).unwrap();
        let cause = fs::metadata(&source.path).unwrap_err().to_string();
        let report = store
            .execute(&context, &plan, None, || false, |_, _| {})
            .unwrap();
        assert_eq!(report.errors.len(), 1);
        assert!(report.errors[0].starts_with(&format!("{label}: ")));
        assert!(report.errors[0].contains(&format!(
            "Could not verify reviewed source {}",
            source.path.display()
        )));
        assert!(report.errors[0].contains(&cause));
        let ticket = report.recovery.as_ref().unwrap();
        let journal = fs::read_to_string(store.directory.join(&ticket.journal)).unwrap();
        let failed: serde_json::Value =
            serde_json::from_str(journal.lines().last().unwrap()).unwrap();
        let detail = failed["error"].as_str().unwrap();
        assert!(detail.contains(&cause));
        assert!(
            !detail.starts_with(&label),
            "The journal stores detail, not a duplicated action label"
        );
        let reopened = Store::new(store.directory.clone());
        for _ in 0..2 {
            let restored = reopened.restore(&context.root, &|| false).unwrap().unwrap();
            assert_eq!(restored.errors, report.errors);
            assert_eq!(restored.uncertain, 0);
            assert_eq!(restored.plan.actions.len(), 1);
        }
        let restored = reopened.restore(&context.root, &|| false).unwrap().unwrap();
        let retry = reopened
            .execute(
                &context,
                &restored.plan,
                Some(&restored.ticket),
                || false,
                |_, _| {},
            )
            .unwrap();
        assert_eq!(retry.errors, report.errors);
        assert!(!destination.exists());
        assert!(!crate::core::metadata::sidecar(&source.path).exists());
        assert_eq!(fs::read(&held).unwrap(), b"first photo");
        reopened
            .discard(&context.root, retry.recovery.as_ref().unwrap())
            .unwrap();
    }
}

#[test]
fn recovery_details_read_legacy_multiline_errors_and_empty_actions_without_panicking() {
    for action in [
        Action::Move { files: vec![] },
        Action::Trash { files: vec![] },
    ] {
        let (_temp, store, context, mut plan) = fixture();
        let detail = "Could not open café\nUnderlying failure: permission denied";
        let expected = format!("{}: {detail}", action.label());
        plan.actions = vec![action];
        let tail = format!(
            "{}\n{}\n",
            serde_json::json!({"status":"pending","index":0}),
            serde_json::json!({"status":"failed","index":0,"error":detail})
        );
        prepared(&store, &context, &plan, tail.as_bytes());
        let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
        assert_eq!(restored.errors, vec![expected]);
        assert_eq!(restored.uncertain, 0);
        store.discard(&context.root, &restored.ticket).unwrap();
    }
}

#[test]
fn recovery_details_keep_known_failures_beside_an_interrupted_action() {
    let (_temp, store, context, plan) = fixture();
    let detail = "Destination already exists: café/a.jpg";
    let tail = format!(
        "{}\n{}\n{}\n",
        serde_json::json!({"status":"pending","index":0}),
        serde_json::json!({"status":"failed","index":0,"error":detail}),
        serde_json::json!({"status":"pending","index":1})
    );
    prepared(&store, &context, &plan, tail.as_bytes());
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(
        restored.errors,
        vec![format!("{}: {detail}", plan.actions[0].label())]
    );
    assert_eq!(restored.uncertain, 1);
    assert_eq!(restored.plan.actions.len(), 2);
    assert!(context.root.join("a.jpg").exists());
    assert!(context.root.join("b.jpg").exists());
    assert!(!context.root.join("Sorted").exists());
    store.discard(&context.root, &restored.ticket).unwrap();
}

#[test]
fn interrupted_external_move_settles_destination_views_once_and_retry_preserves_them() {
    let (temp, store, context, mut plan) = fixture();
    let output = temp.path().canonicalize().unwrap().join("output");
    fs::create_dir(&output).unwrap();
    plan.output_root = Some(output.clone());
    for action in &mut plan.actions {
        if let Action::Move { files } = action {
            for (source, destination) in files {
                *destination = output.join(source.path.file_name().unwrap());
            }
        }
    }
    let unrelated = Bookmark {
        query: "existing workflow".into(),
        ..Default::default()
    };
    views::Saved {
        step: Some(Step::Organize),
        bookmarks: HashMap::from([(Step::Organize, unrelated.clone())]),
        ..Default::default()
    }
    .save(&output)
    .unwrap();
    assert!(
        catch_unwind(AssertUnwindSafe(|| store.execute(
            &context,
            &plan,
            None,
            || false,
            |_, _| panic!("after receipt sync")
        )))
        .is_err()
    );
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let mut destination = views::Saved::load(&output).unwrap();
    let new = MediaId::for_path(&output.join("a.jpg"));
    assert_eq!(destination.step, Some(Step::Cull));
    assert_eq!(destination.bookmarks[&Step::Cull].active, Some(new));
    assert_eq!(destination.bookmarks[&Step::Organize], unrelated);
    destination.bookmarks.get_mut(&Step::Cull).unwrap().query = "later change".into();
    destination.save(&output).unwrap();
    store.restore(&context.root, &|| false).unwrap();
    assert_eq!(views::Saved::load(&output).unwrap(), destination);
    let mut next = context.clone();
    next.session = Session::load(&context.root).unwrap();
    next.views = views::Saved::load(&context.root).unwrap();
    next.views.reconcile(
        &Catalog::scan(&context.root, || false, |_| {})
            .unwrap()
            .media,
    );
    let report = store
        .execute(
            &next,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(report.remaining.actions.is_empty());
    assert_eq!(views::Saved::load(&output).unwrap(), destination);
    destination.reconcile(&Catalog::scan(&output, || false, |_| {}).unwrap().media);
    assert_eq!(destination.bookmarks[&Step::Cull].active, Some(new));
}
fn prepared(store: &Store, context: &Context, plan: &Plan, tail: &[u8]) -> PathBuf {
    let _lock = store.lock(&context.root).unwrap();
    let (mut file, path) = tempfile::Builder::new()
        .prefix("operations-")
        .suffix(".jsonl")
        .tempfile_in(&store.directory)
        .unwrap()
        .keep()
        .unwrap();
    serde_json::to_writer(
        &mut file,
        &Header {
            version: 1,
            context: context.clone(),
            plan: plan.clone(),
        },
    )
    .unwrap();
    file.write_all(b"\n").unwrap();
    file.write_all(tail).unwrap();
    file.sync_all().unwrap();
    store
        .write_pointer(
            &context.root,
            &Pointer {
                journal: path.file_name().unwrap().to_str().unwrap().into(),
                settled: false,
            },
        )
        .unwrap();
    path
}
#[test]
fn interruption_after_completion_restores_remaining_plan_ratings_and_focus() {
    let (_temp, store, context, plan) = fixture();
    assert!(
        catch_unwind(AssertUnwindSafe(|| store.execute(
            &context,
            &plan,
            None,
            || false,
            |_, _| panic!("interrupted after journal sync")
        )))
        .is_err()
    );
    assert!(!context.root.join("a.jpg").exists());
    assert!(context.root.join("b.jpg").exists());
    let restored = Store::new(store.directory.clone())
        .restore(&context.root, &|| false)
        .unwrap()
        .unwrap();
    assert_eq!(restored.plan.actions.len(), 1);
    assert_eq!(restored.uncertain, 0);
    assert!(
        restored
            .plan
            .completed_dependencies
            .contains(&context.root.join("a.jpg"))
    );
    let new = MediaId::for_path(&context.root.join("Sorted/a.jpg"));
    let session = Session::load(&context.root).unwrap();
    assert_eq!(session.decisions[&new].rating, Some(4));
    assert_eq!(
        views::Saved::load(&context.root).unwrap().bookmarks[&Step::Cull].active,
        Some(new)
    );
    let mut next = context.clone();
    next.session = session;
    next.views = views::Saved::load(&context.root).unwrap();
    let report = store
        .execute(
            &next,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert_eq!(report.completed.len(), 1);
    assert!(report.remaining.actions.is_empty());
    assert!(store.restore(&context.root, &|| false).unwrap().is_none());
    assert_eq!(
        fs::read(context.root.join("Sorted/a.jpg")).unwrap(),
        b"first photo"
    );
    assert_eq!(
        fs::read(context.root.join("Sorted/b.jpg")).unwrap(),
        b"second photo"
    );
}
#[test]
fn cancellation_survives_restart_and_stale_windows_cannot_retry_or_discard() {
    let (_temp, store, context, plan) = fixture();
    let stop = Cell::new(false);
    let result = store
        .execute(&context, &plan, None, || stop.get(), |_, _| stop.set(true))
        .unwrap();
    assert!(result.cancelled);
    assert_eq!(result.completed.len(), 1);
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(result.recovery, Some(restored.ticket.clone()));
    let mut changed = restored.plan.clone();
    changed.actions.push(Action::CreateDirectory {
        path: context.root.join("unexpected"),
    });
    assert!(
        store
            .execute(
                &context,
                &changed,
                Some(&restored.ticket),
                || false,
                |_, _| {}
            )
            .is_err()
    );
    assert!(!context.root.join("unexpected").exists());
    assert!(
        store
            .execute(&context, &plan, None, || false, |_, _| {})
            .is_err()
    );
    store.discard(&context.root, &restored.ticket).unwrap();
    assert!(store.discard(&context.root, &restored.ticket).is_err());
    assert!(
        store
            .execute(
                &context,
                &restored.plan,
                Some(&restored.ticket),
                || false,
                |_, _| {}
            )
            .is_err()
    );
    assert!(store.directory.join(restored.ticket.journal).exists());
    assert!(context.root.join("Sorted/a.jpg").exists());
    assert!(context.root.join("b.jpg").exists());
}
#[test]
#[cfg(unix)]
fn verified_move_with_torn_receipt_is_checkpointed_without_repeating_it() {
    let (_temp, store, context, plan) = fixture();
    let log = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n{\"status\":\"completed\"",
    );
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::rename(
        context.root.join("a.jpg"),
        context.root.join("Sorted/a.jpg"),
    )
    .unwrap();
    let before = fs::read(&log).unwrap();
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 0);
    assert_eq!(restored.plan.actions.len(), 1);
    assert_ne!(store.directory.join(&restored.ticket.journal), log);
    assert!(
        fs::read_to_string(store.directory.join(&restored.ticket.journal))
            .unwrap()
            .contains("recovered")
    );
    let moved = MediaId::for_path(&context.root.join("Sorted/a.jpg"));
    assert_eq!(
        Session::load(&context.root).unwrap().decisions[&moved].rating,
        Some(4)
    );
    assert_eq!(
        views::Saved::load(&context.root).unwrap().bookmarks[&Step::Cull].active,
        Some(moved)
    );
    assert_eq!(fs::read(&log).unwrap(), before);
    let report = store
        .execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(report.errors.is_empty());
    assert!(report.remaining.actions.is_empty());
    assert_eq!(
        fs::read(context.root.join("Sorted/a.jpg")).unwrap(),
        b"first photo"
    );
    assert_eq!(
        fs::read(&log).unwrap(),
        before,
        "Retry retains original audit bytes"
    );
    assert!(store.restore(&context.root, &|| false).unwrap().is_none());
}
#[test]
fn malformed_receipts_wrong_roots_symlinks_and_concurrent_owners_fail_closed() {
    let (_temp, store, context, plan) = fixture();
    // The journal path is used only by the Unix symlink check below.
    #[cfg_attr(not(unix), allow(unused_variables))]
    let log = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"completed\",\"index\":0}\n",
    );
    assert!(store.restore(&context.root, &|| false).is_err());
    assert!(context.root.join("a.jpg").exists());
    let owner = store.lock(&context.root).unwrap();
    assert!(store.lock(&context.root).is_err());
    drop(owner);
    let unrelated = context.root.parent().unwrap().join("other");
    fs::create_dir(&unrelated).unwrap();
    assert!(store.restore(&unrelated, &|| false).unwrap().is_none());
    #[cfg(unix)]
    {
        fs::remove_file(&log).unwrap();
        std::os::unix::fs::symlink(context.root.join("a.jpg"), &log).unwrap();
        assert!(store.restore(&context.root, &|| false).is_err());
        assert_eq!(
            fs::read(context.root.join("a.jpg")).unwrap(),
            b"first photo"
        );
    }
}
#[test]
fn cancelled_reads_do_not_publish_or_settle_partial_recovery() {
    let (_temp, store, context, plan) = fixture();
    prepared(&store, &context, &plan, b"");
    assert!(store.restore(&context.root, &|| true).is_err());
    assert!(!store.read_pointer(&context.root).unwrap().unwrap().settled);
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.plan.actions.len(), 2);
    store.discard(&context.root, &restored.ticket).unwrap();
}
#[test]
fn source_changes_are_still_rejected_after_recovery() {
    let (_temp, store, context, plan) = fixture();
    prepared(&store, &context, &plan, b"");
    fs::write(context.root.join("a.jpg"), b"external replacement").unwrap();
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let report = store
        .execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.remaining.actions.len(), 1);
    assert_eq!(
        fs::read(context.root.join("a.jpg")).unwrap(),
        b"external replacement"
    );
    assert!(!context.root.join("Sorted/a.jpg").exists());
    store
        .discard(&context.root, report.recovery.as_ref().unwrap())
        .unwrap();
}
fn crash_after_first(store: &Store, context: &Context, plan: &Plan) {
    let input = context.root.parent().unwrap().join("crash-input.json");
    fs::write(
        &input,
        serde_json::to_vec(&(context, plan, &store.directory)).unwrap(),
    )
    .unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "core::operations::recovery::tests::process_exit_helper",
            "--nocapture",
        ])
        .env("PHOTOSORT_RECOVERY_CHILD", &input)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(75));
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("Recovery child did not exit");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
#[test]
fn process_exit_helper() {
    let Some(input) = std::env::var_os("PHOTOSORT_RECOVERY_CHILD") else {
        return;
    };
    let (context, plan, directory): (Context, Plan, PathBuf) =
        serde_json::from_slice(&fs::read(input).unwrap()).unwrap();
    Store::new(directory)
        .execute(
            &context,
            &plan,
            None,
            || false,
            |_, _| std::process::exit(75),
        )
        .unwrap();
    panic!("An operation should have completed before process exit");
}
#[test]
fn actual_process_exit_releases_the_lock_and_preserves_exact_remaining_actions() {
    let (_temp, store, context, plan) = fixture();
    crash_after_first(&store, &context, &plan);
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.plan.actions.len(), 1);
    assert_eq!(restored.uncertain, 0);
    assert_eq!(
        serde_json::to_value(&restored.plan.actions).unwrap(),
        serde_json::to_value(&plan.actions[1..]).unwrap()
    );
    assert!(context.root.join("b.jpg").exists());
    store.discard(&context.root, &restored.ticket).unwrap();
}
#[test]
fn rotation_only_recovery_preserves_unrelated_staged_rating() {
    let (_temp, store, mut context, mut plan) = fixture();
    context.rotation_only = true;
    let source = Source::capture(context.root.join("a.jpg")).unwrap();
    let id = MediaId::for_path(&source.path);
    plan.actions[0] = Action::Metadata {
        source,
        sidecar_before: None,
        target: metadata::WriteTarget::Sidecar,
        rating: None,
        orientation: Some(6),
    };
    crash_after_first(&store, &context, &plan);
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert!(restored.rotation_only);
    assert_eq!(
        Session::load(&context.root).unwrap().decisions[&id].rating,
        Some(4)
    );
    assert_eq!(metadata::read(&context.root.join("a.jpg")).orientation, 6);
    store.discard(&context.root, &restored.ticket).unwrap();
}
#[test]
#[ignore = "copies all seven PHOTOSORT_SAMPLE_DIR photos and exits a child between committed moves"]
fn seven_samples_survive_process_exit_and_reviewed_retry() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("library");
    fs::create_dir(&root).unwrap();
    let samples =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let originals = Catalog::scan(&samples, || false, |_| {}).unwrap();
    assert_eq!(originals.media.len(), 7);
    let mut hashes = HashMap::new();
    for media in &originals.media {
        hashes.insert(
            media.name(),
            crate::core::operations::content_hash(&media.path, || false).unwrap(),
        );
        fs::copy(&media.path, root.join(media.name())).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let plan = Plan {
        actions: catalog
            .media
            .iter()
            .map(|m| Action::Move {
                files: vec![(
                    Source::capture(m.path.clone()).unwrap(),
                    root.join("Sorted").join(m.name()),
                )],
            })
            .collect(),
        output_root: Some(root.clone()),
        ..Default::default()
    };
    let context = Context {
        root: root.clone(),
        rotation_only: false,
        session: Session {
            fingerprints: catalog
                .media
                .iter()
                .map(|m| (m.id, m.fingerprint))
                .collect(),
            ..Default::default()
        },
        views: Default::default(),
        media: catalog.media,
    };
    let store = Store::new(base.join("journals"));
    crash_after_first(&store, &context, &plan);
    let restored = store.restore(&root, &|| false).unwrap().unwrap();
    assert_eq!(restored.plan.actions.len(), 6);
    let mut resumed = context.clone();
    resumed.session = Session::load(&root).unwrap();
    let report = store
        .execute(
            &resumed,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(report.errors.is_empty() && report.remaining.actions.is_empty());
    for original in originals.media {
        assert_eq!(
            crate::core::operations::content_hash(&original.path, || false).unwrap(),
            hashes[&original.name()]
        );
        assert_eq!(
            crate::core::operations::content_hash(
                &root.join("Sorted").join(original.name()),
                || false
            )
            .unwrap(),
            hashes[&original.name()]
        );
    }
}
#[test]
fn oversized_acceptance_fails_before_mutation_and_removes_unaccepted_output() {
    let (_temp, mut store, context, plan) = fixture();
    store.max_header = 1;
    assert!(
        store
            .execute(&context, &plan, None, || false, |_, _| {})
            .is_err()
    );
    assert!(!store.has_pending(&context.root));
    assert!(context.root.join("a.jpg").exists() && context.root.join("b.jpg").exists());
    assert!(!context.root.join("Sorted").exists());
    assert!(fs::read_dir(&store.directory).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".jsonl")
    }));
}
#[test]
fn streamed_records_check_delimiters_limits_and_cancellation() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(b"{\"status\":\"pending\",\"index\":0}\n")
        .unwrap();
    let bytes = file.metadata().unwrap().len();
    let stop = Cell::new(false);
    let cancelled = || stop.get();
    let mut records = Records {
        reader: BufReader::new(Cancellable {
            file,
            cancelled: &cancelled,
        }),
        offset: 0,
        bytes,
    };
    assert!(records.next::<Entry>(5).is_err());
    stop.set(true);
    assert!(records.next::<Entry>(MAX_RECORD).is_err());
    stop.set(false);
    assert_eq!(records.next::<Entry>(MAX_RECORD).unwrap().unwrap().index, 0);
    assert!(records.next::<Entry>(MAX_RECORD).unwrap().is_none());
}
#[test]
#[ignore = "streams a 65 MiB record across the former whole-line limit without an input-sized byte buffer"]
fn streams_records_larger_than_the_old_limit() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(b"\"").unwrap();
    let chunk = vec![b'a'; 1024 * 1024];
    for _ in 0..65 {
        file.write_all(&chunk).unwrap();
    }
    file.write_all(b"\"\n").unwrap();
    let bytes = file.metadata().unwrap().len();
    let mut records = Records {
        reader: BufReader::new(Cancellable {
            file,
            cancelled: &|| false,
        }),
        offset: 0,
        bytes,
    };
    assert!(
        records
            .next::<serde::de::IgnoredAny>(MAX_HEADER)
            .unwrap()
            .is_some()
    );
    assert_eq!(records.offset, bytes);
    assert!(records.next::<Entry>(MAX_RECORD).unwrap().is_none());
}
#[test]
fn hundred_thousand_photo_context_stores_only_user_intent_from_the_shared_index() {
    use crate::core::manual_groups::{self, Edit};
    use std::sync::Arc;
    let media: Vec<_> = (0..100_000)
        .map(|id| Media {
            id: MediaId(id),
            path: format!("/unavailable/library/{id}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        })
        .collect();
    let index = media.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
    let manual = manual_groups::edit(
        1,
        Arc::default(),
        Arc::default(),
        &media,
        Edit::Move {
            selected: vec![MediaId(2), MediaId(3)],
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    let scene = manual_groups::edit(
        2,
        Arc::default(),
        Arc::default(),
        &media,
        Edit::Move {
            selected: vec![MediaId(5), MediaId(6)],
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    let session = Session {
        manual_groups: manual,
        scene_groups: scene,
        decisions: HashMap::from([(
            MediaId(1),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let views = views::Saved {
        fingerprints: HashMap::from([(MediaId(4), Fingerprint::default())]),
        ..Default::default()
    };
    let context = Context::capture(
        "/unavailable/library".into(),
        false,
        &media,
        &index,
        session,
        views,
    );
    assert_eq!(context.media.len(), 1);
    assert_eq!(context.session.fingerprints.len(), 5);
    assert!(
        context.session.manual_groups.contains(MediaId(2))
            && context.session.manual_groups.contains(MediaId(3))
    );
    assert!(context.views.fingerprints.contains_key(&MediaId(4)));
    assert!(
        serde_json::to_vec(&context).unwrap().len() < 4096,
        "One decision should not journal all 100,000 untouched photo identities"
    );
    let mut resumed = context.session;
    resumed.reconcile(&media);
    assert_eq!(resumed.decisions[&MediaId(1)].rating, Some(4));
    assert_eq!(resumed.manual_groups.len(), 2);
    assert_eq!(resumed.scene_groups.len(), 2);
    assert!(resumed.scene_groups.contains(MediaId(5)) && resumed.scene_groups.contains(MediaId(6)));
}
#[test]
fn interrupted_moves_preserve_scene_and_subject_membership_after_restart_and_retry() {
    use crate::core::manual_groups::{self, Edit};
    let (_temp, store, mut context, plan) = fixture();
    let group = |ids| {
        manual_groups::edit(
            1,
            Default::default(),
            Default::default(),
            &context.media,
            Edit::Move {
                selected: ids,
                target: None,
            },
            || false,
        )
        .unwrap()
        .manual
    };
    context.session.manual_groups = group(vec![context.media[0].id]);
    context.session.scene_groups = group(context.media.iter().map(|m| m.id).collect());
    crash_after_first(&store, &context, &plan);
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let first = MediaId::for_path(&context.root.join("Sorted/a.jpg"));
    let pending = MediaId::for_path(&context.root.join("b.jpg"));
    context.session = Session::load(&context.root).unwrap();
    assert!(context.session.manual_groups.contains(first));
    assert!(
        context.session.scene_groups.contains(first)
            && context.session.scene_groups.contains(pending)
    );
    let report = store
        .execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(report.errors.is_empty() && report.remaining.actions.is_empty());
    let catalog = Catalog::scan(&context.root, || false, |_| {}).unwrap();
    let mut session = Session::load(&context.root).unwrap();
    session.reconcile(&catalog.media);
    assert_eq!(session.manual_groups.len(), 1);
    assert_eq!(session.scene_groups.len(), 2);
    assert!(
        session
            .scene_groups
            .contains(MediaId::for_path(&context.root.join("Sorted/b.jpg")))
    );
}
#[test]
fn lock_scope_releases_on_return_and_unwind_even_with_a_duplicated_handle() {
    let (temp, store, context, _) = fixture();
    assert!(temp.path().exists());
    for panic in [false, true] {
        let mut inherited = None;
        let ended = catch_unwind(AssertUnwindSafe(|| {
            let guard = store.lock(&context.root).unwrap();
            inherited = Some(guard.file.try_clone().unwrap());
            assert!(
                store.lock(&context.root).is_err(),
                "Active owner must remain exclusive"
            );
            assert!(!panic, "Simulated operation unwind");
        }));
        assert_eq!(ended.is_err(), panic);
        let next = store
            .lock(&context.root)
            .expect("Scope end releases ownership while duplicate is alive");
        assert!(store.lock(&context.root).is_err());
        drop(inherited);
        assert!(
            store.lock(&context.root).is_err(),
            "Old descriptor must not release the new owner's lock"
        );
        drop(next);
        assert!(store.lock(&context.root).is_ok());
    }
}
#[test]
#[cfg(unix)]
fn recovery_never_confuses_copies_links_replacements_or_unstarted_moves_with_completion() {
    for mode in ["copy", "changed", "unstarted", "zero-identity", "symlink"] {
        let (_temp, store, context, mut plan) = fixture();
        let source = context.root.join("a.jpg");
        let destination = context.root.join("Sorted/a.jpg");
        fs::create_dir(destination.parent().unwrap()).unwrap();
        if mode == "zero-identity" {
            let Action::Move { files } = &mut plan.actions[0] else {
                unreachable!()
            };
            files[0].0.fingerprint.identity = [0; 4];
        }
        let log = prepared(
            &store,
            &context,
            &plan,
            if mode == "unstarted" {
                b""
            } else {
                b"{\"status\":\"pending\",\"index\":0}\n"
            },
        );
        match mode {
            "copy" => {
                fs::copy(&source, &destination).unwrap();
                fs::remove_file(&source).unwrap();
            }
            "linked" => fs::hard_link(&source, &destination).unwrap(),
            "symlink" => {
                let other = context.root.join("other.jpg");
                fs::rename(&source, &other).unwrap();
                std::os::unix::fs::symlink(other, &destination).unwrap();
            }
            _ => {
                fs::rename(&source, &destination).unwrap();
                if mode == "changed" {
                    fs::write(&destination, b"changed content").unwrap();
                }
            }
        }
        let journal = fs::read(&log).unwrap();
        let before = fs::read(&destination).unwrap();
        let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
        assert_eq!(restored.plan.actions.len(), 2, "{mode}");
        assert_eq!(
            restored.uncertain,
            usize::from(mode != "unstarted"),
            "{mode}"
        );
        assert_eq!(store.directory.join(&restored.ticket.journal), log);
        assert_eq!(fs::read(&log).unwrap(), journal);
        assert_eq!(fs::read(&destination).unwrap(), before);
        assert_eq!(source.exists(), mode == "linked");
        store.discard(&context.root, &restored.ticket).unwrap();
    }
}
#[test]
#[cfg(unix)]
fn partial_companions_recover_completed_members_and_explicitly_finish_remaining_links() {
    let (_temp, store, context, mut plan) = fixture();
    let Action::Move { files: second } = plan.actions.remove(1) else {
        unreachable!()
    };
    let Action::Move { files } = &mut plan.actions[0] else {
        unreachable!()
    };
    files.extend(second);
    fs::create_dir(context.root.join("Sorted")).unwrap();
    for (source, destination) in files.iter() {
        fs::hard_link(&source.path, destination).unwrap();
    }
    fs::remove_file(&files[0].0.path).unwrap();
    let log = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.plan.actions.len(), 1);
    assert_eq!(restored.uncertain, 0);
    assert_ne!(store.directory.join(&restored.ticket.journal), log);
    assert!(
        matches!(&restored.plan.actions[0], Action::FinishMove { files } if files.len() == 1 && files[0].0.path == context.root.join("b.jpg"))
    );
    assert!(
        restored
            .plan
            .completed_dependencies
            .contains(&context.root.join("a.jpg"))
    );
    assert_eq!(
        Session::load(&context.root).unwrap().decisions
            [&MediaId::for_path(&context.root.join("Sorted/a.jpg"))]
            .rating,
        Some(4)
    );
    assert!(context.root.join("b.jpg").exists());
    assert!(
        context.root.join("Sorted/a.jpg").exists() && context.root.join("Sorted/b.jpg").exists()
    );
    let original = fs::read(&log).unwrap();
    let current = Context {
        session: Session::load(&context.root).unwrap(),
        views: views::Saved::load(&context.root).unwrap(),
        ..context.clone()
    };
    let report = store
        .execute(
            &current,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(report.errors.is_empty() && report.remaining.actions.is_empty());
    assert!(!context.root.join("b.jpg").exists());
    assert_eq!(
        Session::load(&context.root).unwrap().decisions
            [&MediaId::for_path(&context.root.join("Sorted/b.jpg"))]
            .rating,
        Some(4)
    );
    assert_eq!(fs::read(&log).unwrap(), original);
}
#[test]
#[cfg(unix)]
fn cancelled_checkpoint_leaves_original_index_journal_and_files_untouched() {
    let (_temp, store, context, plan) = fixture();
    let log = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::rename(
        context.root.join("a.jpg"),
        context.root.join("Sorted/a.jpg"),
    )
    .unwrap();
    let before = fs::read(&log).unwrap();
    let mut pointer = store.read_pointer(&context.root).unwrap().unwrap();
    let loaded = store.read(&context.root, &pointer, &|| false).unwrap();
    let original_count = fs::read_dir(&store.directory).unwrap().count();
    let cancelled = || fs::read_dir(&store.directory).unwrap().count() > original_count;
    assert!(
        store
            .recover_moves(&context.root, &mut pointer, loaded, &cancelled)
            .is_err()
    );
    assert_eq!(
        fs::read_dir(&store.directory).unwrap().count(),
        original_count
    );
    assert_eq!(fs::read(&log).unwrap(), before);
    assert_eq!(
        store.read_pointer(&context.root).unwrap().unwrap().journal,
        log.file_name().unwrap().to_str().unwrap()
    );
    assert!(!store.read_pointer(&context.root).unwrap().unwrap().settled);
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 0);
    assert_eq!(restored.plan.actions.len(), 1);
    store.discard(&context.root, &restored.ticket).unwrap();
}
#[test]
#[cfg(unix)]
fn later_opens_do_not_replay_old_intent_over_new_decisions() {
    let (_temp, store, context, plan) = fixture();
    prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 1);
    let mut current = Session::load(&context.root).unwrap();
    let id = MediaId::for_path(&context.root.join("b.jpg"));
    current.decisions.get_mut(&id).unwrap().rating = Some(1);
    current.save(&context.root).unwrap();
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::rename(
        context.root.join("a.jpg"),
        context.root.join("Sorted/a.jpg"),
    )
    .unwrap();
    let again = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(again.ticket, restored.ticket);
    assert_eq!(again.uncertain, 1);
    assert_eq!(
        Session::load(&context.root).unwrap().decisions[&id].rating,
        Some(1)
    );
    store.discard(&context.root, &again.ticket).unwrap();
}
#[test]
#[cfg(unix)]
fn recovered_all_companions_restore_intent_and_finish_without_retry() {
    let (_temp, store, mut context, mut plan) = fixture();
    let Action::Move { files: second } = plan.actions.remove(1) else {
        unreachable!()
    };
    let Action::Move { files } = &mut plan.actions[0] else {
        unreachable!()
    };
    files.extend(second);
    context.session.scene_groups = crate::core::manual_groups::edit(
        1,
        Default::default(),
        Default::default(),
        &context.media,
        crate::core::manual_groups::Edit::Move {
            selected: context.media.iter().map(|m| m.id).collect(),
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    let original = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    fs::create_dir(context.root.join("Sorted")).unwrap();
    for name in ["a.jpg", "b.jpg"] {
        fs::rename(
            context.root.join(name),
            context.root.join("Sorted").join(name),
        )
        .unwrap();
    }
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert!(restored.plan.actions.is_empty());
    assert_eq!(restored.uncertain, 0);
    assert!(!store.has_pending(&context.root));
    assert!(original.exists());
    let session = Session::load(&context.root).unwrap();
    for name in ["a.jpg", "b.jpg"] {
        let id = MediaId::for_path(&context.root.join("Sorted").join(name));
        assert_eq!(session.decisions[&id].rating, Some(4));
        assert!(session.scene_groups.contains(id));
    }
    assert!(store.restore(&context.root, &|| false).unwrap().is_none());
}
#[test]
#[cfg(unix)]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR; only disposable copies are moved"]
fn seven_sample_unreceipted_move_recovers_ratings_groups_and_retry_without_changing_bytes() {
    seven_sample_move_recovery(false);
}
#[test]
#[cfg(unix)]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR; disposable RAW/XMP staged interruption"]
fn seven_sample_partial_move_recovers_and_retries_only_remaining_companions() {
    seven_sample_move_recovery(true);
}
#[cfg(unix)]
fn seven_sample_move_recovery(partial: bool) {
    let samples =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let native = std::env::var_os("PHOTOSORT_RECOVERY_KEEP_NATIVE").is_some();
    let scratch = if native {
        tempfile::Builder::new()
            .prefix("recovered-move-native-")
            .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join(".tools"))
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let root = scratch.path().join("library");
    let before = root.join("Before");
    fs::create_dir_all(&before).unwrap();
    let root = root.canonicalize().unwrap();
    let before = root.join("Before");
    let mut hashes = std::collections::BTreeMap::new();
    for item in fs::read_dir(&samples).unwrap() {
        let path = item.unwrap().path();
        if path.is_file() && MediaKind::from_path(&path).is_some() {
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            hashes.insert(
                name.clone(),
                super::super::content_hash(&path, || false).unwrap(),
            );
            fs::copy(&path, before.join(name)).unwrap();
        }
    }
    assert_eq!(hashes.len(), 7);
    if partial {
        fs::write(before.join("arw_sample.xmp"), b"test companion").unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let first = if partial {
        catalog
            .media
            .iter()
            .find(|m| m.name() == "arw_sample.ARW")
            .unwrap()
            .clone()
    } else {
        catalog.media[0].clone()
    };
    let scene = crate::core::manual_groups::edit(
        1,
        Default::default(),
        Default::default(),
        &catalog.media,
        crate::core::manual_groups::Edit::Move {
            selected: catalog.media.iter().map(|m| m.id).collect(),
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    let session = Session {
        decisions: catalog
            .media
            .iter()
            .map(|m| {
                (
                    m.id,
                    Decision {
                        rating: Some(4),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        scene_groups: scene,
        fingerprints: catalog
            .media
            .iter()
            .map(|m| (m.id, m.fingerprint))
            .collect(),
        ..Default::default()
    };
    let context = Context {
        root: catalog.root.clone(),
        rotation_only: false,
        media: catalog.media.clone(),
        session,
        views: views::Saved {
            step: Some(Step::Cull),
            bookmarks: HashMap::from([(
                Step::Cull,
                Bookmark {
                    active: Some(first.id),
                    ..Default::default()
                },
            )]),
            fingerprints: HashMap::from([(first.id, first.fingerprint)]),
        },
    };
    let labels = catalog
        .media
        .iter()
        .map(|m| (m.id, "After".into()))
        .collect();
    let mut plan = super::super::organize_plan(&catalog, &labels, &catalog.root, true).unwrap();
    if partial {
        let index = plan.actions.iter().position(|a| matches!(a, Action::Move { files } if files.iter().any(|(s,_)| s.path==first.path))).unwrap();
        plan.actions.swap(0, index);
    }
    let expected_remaining = plan.actions.len() - usize::from(!partial);
    assert!(matches!(plan.actions[0], Action::Move { .. }));
    let store = if native {
        Store::standard()
    } else {
        Store::new(scratch.path().canonicalize().unwrap().join("journals"))
    };
    let log = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    if partial {
        // Production stages the whole cohort before unlinking any source.
        let Action::Move { files } = &plan.actions[0] else {
            panic!()
        };
        assert_eq!(files.len(), 2);
        for (source, to) in files {
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            super::super::copy_exclusive(&source.path, to, &|| false).unwrap();
        }
        fs::remove_file(&first.path).unwrap();
    } else {
        super::super::apply(
            &plan.actions[0],
            &|| false,
            &mut super::super::case_rename::FileNames::default(),
        )
        .unwrap();
    }
    let original_journal = fs::read(&log).unwrap();
    if native {
        let preferences = crate::core::preferences::Preferences::path();
        let record = serde_json::json!({ "root": root, "fixture": scratch.path(), "original_journal": log, "expected_remaining": expected_remaining, "partial_companions":partial, "active_pointer": store.pointer(&root),
            "first_destination": root.join("After").join(first.name()), "sample_blake3": hashes,
            "preferences_before": fs::read_to_string(&preferences).ok() });
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(".tools/recovered-move-native-state.json"),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .unwrap();
        let _kept = scratch.keep();
        return;
    }
    let restored = store.restore(&catalog.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 0);
    assert_eq!(restored.plan.actions.len(), expected_remaining);
    if partial {
        assert!(
            matches!(&restored.plan.actions[0], Action::FinishMove { files } if files.len()==1 && files[0].0.path==before.join("arw_sample.xmp"))
        );
        assert!(before.join("arw_sample.xmp").exists());
    }
    assert_eq!(fs::read(&log).unwrap(), original_journal);
    let moved_id = MediaId::for_path(&root.join("After").join(first.name()));
    let session = Session::load(&catalog.root).unwrap();
    assert_eq!(session.decisions[&moved_id].rating, Some(4));
    assert!(session.scene_groups.contains(moved_id));
    assert_eq!(
        views::Saved::load(&catalog.root).unwrap().bookmarks[&Step::Cull].active,
        Some(moved_id)
    );
    let current = Catalog::scan(&catalog.root, || false, |_| {}).unwrap();
    let context = Context {
        root: catalog.root.clone(),
        rotation_only: false,
        media: current.media,
        session,
        views: views::Saved::load(&catalog.root).unwrap(),
    };
    let report = store
        .execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(report.errors.is_empty() && report.remaining.actions.is_empty());
    assert!(!before.exists());
    if partial {
        assert_eq!(
            fs::read(root.join("After/arw_sample.xmp")).unwrap(),
            b"test companion"
        );
    }
    let mut session = Session::load(&catalog.root).unwrap();
    let final_catalog = Catalog::scan(&catalog.root, || false, |_| {}).unwrap();
    session.reconcile(&final_catalog.media);
    assert_eq!(session.decisions.len(), 7);
    assert_eq!(session.scene_groups.len(), 7);
    for (name, hash) in hashes {
        assert_eq!(
            super::super::content_hash(&samples.join(&name), || false).unwrap(),
            hash
        );
        assert_eq!(
            super::super::content_hash(&root.join("After").join(&name), || false).unwrap(),
            hash
        );
    }
}
#[test]
fn amended_move_survives_restart_preserves_newer_intent_and_never_repeats_completed_actions() {
    let (_temp, store, mut context, plan) = fixture();
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::write(context.root.join("Sorted/b.jpg"), b"unrelated collision").unwrap();
    let report = store
        .execute(&context, &plan, None, || false, |_, _| {})
        .unwrap();
    assert_eq!(report.completed.len(), 1);
    let ticket = report.recovery.unwrap();
    let original = fs::read(store.directory.join(&ticket.journal)).unwrap();
    let b = MediaId::for_path(&context.root.join("b.jpg"));
    let mut latest = Session::load(&context.root).unwrap();
    latest.decisions.get_mut(&b).unwrap().rating = Some(5);
    latest.save(&context.root).unwrap();
    let mut views = views::Saved::load(&context.root).unwrap();
    views.bookmarks.get_mut(&Step::Cull).unwrap().query = "newer bookmark".into();
    views.save(&context.root).unwrap();
    let edited = store
        .amend_destination(
            &context.root,
            &ticket,
            0,
            Path::new("Corrected"),
            None,
            &|| false,
        )
        .unwrap();
    assert_eq!(edited.plan.actions.len(), 1);
    assert_ne!(edited.ticket, ticket);
    assert_eq!(
        edited.plan.completed_dependencies,
        report.remaining.completed_dependencies
    );
    assert!(!context.root.join("Corrected").exists());
    assert!(context.root.join("b.jpg").exists());
    assert_eq!(
        fs::read(store.directory.join(&ticket.journal)).unwrap(),
        original
    );
    let reopened = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(reopened.ticket, edited.ticket);
    assert_eq!(
        Session::load(&context.root).unwrap().decisions[&b].rating,
        Some(5)
    );
    assert_eq!(views::Saved::load(&context.root).unwrap(), views);
    assert!(
        store
            .amend_destination(&context.root, &ticket, 0, Path::new("Stale"), None, &|| {
                false
            })
            .is_err()
    );
    context.session = Session::load(&context.root).unwrap();
    context.views = views;
    let done = store
        .execute(
            &context,
            &reopened.plan,
            Some(&reopened.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert_eq!(done.completed.len(), 1);
    assert!(done.errors.is_empty() && done.remaining.actions.is_empty());
    assert_eq!(
        fs::read(context.root.join("Sorted/a.jpg")).unwrap(),
        b"first photo"
    );
    assert_eq!(
        fs::read(context.root.join("Sorted/b.jpg")).unwrap(),
        b"unrelated collision"
    );
    assert_eq!(
        fs::read(context.root.join("Corrected/b.jpg")).unwrap(),
        b"second photo"
    );
    let new = MediaId::for_path(&context.root.join("Corrected/b.jpg"));
    assert_eq!(
        Session::load(&context.root).unwrap().decisions[&new].rating,
        Some(5)
    );
}

#[test]
fn amendment_retargets_every_companion_and_rejects_collisions_changes_and_partial_moves() {
    let (_temp, store, context, mut plan) = fixture();
    fs::write(context.root.join("a.xmp"), b"sidecar").unwrap();
    if let Action::Move { files } = &mut plan.actions[0] {
        files.push((
            Source::capture(context.root.join("a.xmp")).unwrap(),
            context.root.join("Sorted/a.xmp"),
        ));
    }
    let path = prepared(&store, &context, &plan, b"");
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let original = fs::read(path).unwrap();
    for folder in ["../escape", "/absolute", "Sorted"] {
        assert!(
            store
                .amend_destination(
                    &context.root,
                    &restored.ticket,
                    0,
                    Path::new(folder),
                    None,
                    &|| false
                )
                .is_err()
        );
    }
    fs::create_dir(context.root.join("Blocked")).unwrap();
    fs::write(context.root.join("Blocked/a.xmp"), b"foreign").unwrap();
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("Blocked"),
                None,
                &|| false
            )
            .is_err()
    );
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::hard_link(
        context.root.join("a.jpg"),
        context.root.join("Sorted/a.jpg"),
    )
    .unwrap();
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("Other"),
                None,
                &|| false
            )
            .is_err()
    );
    fs::remove_file(context.root.join("Sorted/a.jpg")).unwrap();
    let edited = store
        .amend_destination(
            &context.root,
            &restored.ticket,
            0,
            Path::new("Other"),
            None,
            &|| false,
        )
        .unwrap();
    let Action::Move { files } = &edited.plan.actions[0] else {
        panic!()
    };
    assert_eq!(files.len(), 2);
    assert!(
        files
            .iter()
            .all(|(_, p)| p.parent() == Some(context.root.join("Other").as_path()))
    );
    assert_eq!(
        fs::read(store.directory.join(&restored.ticket.journal)).unwrap(),
        original
    );
    fs::write(context.root.join("a.jpg"), b"changed source").unwrap();
    assert!(
        store
            .amend_destination(
                &context.root,
                &edited.ticket,
                0,
                Path::new("Changed"),
                None,
                &|| false
            )
            .is_err()
    );
}

#[test]
fn cancelled_and_uncertain_amendments_leave_saved_plan_and_all_files_unchanged() {
    let (_temp, store, context, plan) = fixture();
    prepared(&store, &context, &plan, b"");
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let pointer = fs::read(store.pointer(&context.root)).unwrap();
    let records = || {
        fs::read_dir(&store.directory)
            .unwrap()
            .filter_map(|p| p.ok())
            .filter(|p| p.path().extension().is_some_and(|x| x == "jsonl"))
            .count()
    };
    let count = records();
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("Cancelled"),
                None,
                &|| records() > count
            )
            .is_err()
    );
    assert_eq!(records(), count);
    assert_eq!(fs::read(store.pointer(&context.root)).unwrap(), pointer);
    assert!(!context.root.join("Cancelled").exists());
    store.discard(&context.root, &restored.ticket).unwrap();
    prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    let uncertain = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(uncertain.uncertain, 1);
    assert!(
        store
            .amend_destination(
                &context.root,
                &uncertain.ticket,
                0,
                Path::new("Unknown"),
                None,
                &|| false
            )
            .is_err()
    );
    assert!(context.root.join("a.jpg").exists() && context.root.join("b.jpg").exists());
}
#[test]
fn amendment_checks_other_destinations_and_symlink_paths_without_changing_dependencies() {
    let (_temp, store, context, mut plan) = fixture();
    if let Action::Move { files } = &mut plan.actions[1] {
        files[0].1 = context.root.join("Reserved/a.jpg");
    }
    plan.actions.push(Action::CreateDirectory {
        path: context.root.join("Allowed"),
    });
    prepared(&store, &context, &plan, b"");
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("reserved"),
                None,
                &|| false
            )
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(context.root.join("Sorted"), context.root.join("Alias"))
            .unwrap();
        assert!(
            store
                .amend_destination(
                    &context.root,
                    &restored.ticket,
                    0,
                    Path::new("Alias"),
                    None,
                    &|| false
                )
                .is_err()
        );
    }
    let edited = store
        .amend_destination(
            &context.root,
            &restored.ticket,
            0,
            Path::new("Allowed"),
            None,
            &|| false,
        )
        .unwrap();
    assert_eq!(edited.plan.actions.len(), 3);
    assert_eq!(
        serde_json::to_value(&edited.plan.actions[1..]).unwrap(),
        serde_json::to_value(&plan.actions[1..]).unwrap()
    );
}

#[test]
fn amendment_renames_companions_in_place_and_migrates_intent_only_on_retry() {
    let (_temp, store, context, mut plan) = fixture();
    for name in ["a.ARW", "a.Xmp"] {
        fs::write(context.root.join(name), name.as_bytes()).unwrap();
        if let Action::Move { files } = &mut plan.actions[0] {
            files.push((
                Source::capture(context.root.join(name)).unwrap(),
                context.root.join("Sorted").join(name),
            ));
        }
    }
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::write(context.root.join("Sorted/a.jpg"), b"existing photo").unwrap();
    let report = store
        .execute(&context, &plan, None, || false, |_, _| {})
        .unwrap();
    assert_eq!(report.completed.len(), 1);
    let ticket = report.recovery.unwrap();
    let journal = fs::read(store.directory.join(&ticket.journal)).unwrap();
    let saved_session = Session::load(&context.root).unwrap();
    let edited = store
        .amend_destination(
            &context.root,
            &ticket,
            0,
            Path::new("Sorted"),
            Some("Chosen.v2"),
            &|| false,
        )
        .unwrap();
    assert_ne!(edited.ticket, ticket);
    assert!(context.root.join("a.jpg").exists());
    assert!(!context.root.join("Sorted/Chosen.v2.jpg").exists());
    assert_eq!(
        Session::load(&context.root).unwrap().decisions,
        saved_session.decisions
    );
    assert_eq!(
        fs::read(store.directory.join(&ticket.journal)).unwrap(),
        journal
    );
    let reopened = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(reopened.ticket, edited.ticket);
    let context = Context {
        session: saved_session,
        views: views::Saved::load(&context.root).unwrap(),
        ..context
    };
    let done = store
        .execute(
            &context,
            &reopened.plan,
            Some(&reopened.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(done.errors.is_empty() && done.remaining.actions.is_empty());
    for extension in ["jpg", "ARW", "Xmp"] {
        assert!(!context.root.join(format!("a.{extension}")).exists());
        assert!(
            context
                .root
                .join(format!("Sorted/Chosen.v2.{extension}"))
                .exists()
        );
    }
    assert_eq!(
        fs::read(context.root.join("Sorted/Chosen.v2.jpg")).unwrap(),
        b"first photo"
    );
    assert_eq!(
        fs::read(context.root.join("Sorted/a.jpg")).unwrap(),
        b"existing photo"
    );
    assert_eq!(
        Session::load(&context.root).unwrap().decisions
            [&MediaId::for_path(&context.root.join("Sorted/Chosen.v2.jpg"))]
            .rating,
        Some(4)
    );
}

#[test]
fn filename_amendment_rejects_invalid_names_collisions_and_cancellation_atomically() {
    let (_temp, store, context, plan) = fixture();
    prepared(&store, &context, &plan, b"");
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let pointer = fs::read(store.pointer(&context.root)).unwrap();
    let journal = fs::read(store.directory.join(&restored.ticket.journal)).unwrap();
    let mut invalid = vec![
        "",
        ".",
        "..",
        "../escape",
        "a/b",
        "a\\b",
        " trailing ",
        "bad:name",
        "b",
        "a",
    ];
    // Only Windows drops trailing dots from new names.
    if cfg!(windows) {
        invalid.push("trailing.");
    }
    for name in invalid {
        assert!(
            store
                .amend_destination(
                    &context.root,
                    &restored.ticket,
                    0,
                    Path::new("Sorted"),
                    Some(name),
                    &|| false
                )
                .is_err(),
            "{name}"
        );
    }
    fs::create_dir(context.root.join("Sorted")).unwrap();
    fs::write(context.root.join("Sorted/occupied.jpg"), b"foreign").unwrap();
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("Sorted"),
                Some("occupied"),
                &|| false
            )
            .is_err()
    );
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("Sorted"),
                Some("Free"),
                &|| true
            )
            .is_err()
    );
    assert_eq!(fs::read(store.pointer(&context.root)).unwrap(), pointer);
    assert_eq!(
        fs::read(store.directory.join(&restored.ticket.journal)).unwrap(),
        journal
    );
    assert_eq!(
        fs::read(context.root.join("a.jpg")).unwrap(),
        b"first photo"
    );
    assert_eq!(
        fs::read(context.root.join("Sorted/occupied.jpg")).unwrap(),
        b"foreign"
    );
}

#[test]
fn amendment_preserves_extensionless_names_and_refuses_collapsing_companions() {
    let (_temp, store, context, mut plan) = fixture();
    fs::write(context.root.join("Notes"), b"document").unwrap();
    plan.actions[0] = Action::Move {
        files: vec![(
            Source::capture(context.root.join("Notes")).unwrap(),
            context.root.join("Sorted/Notes"),
        )],
    };
    prepared(&store, &context, &plan, b"");
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let revised = store
        .amend_destination(
            &context.root,
            &restored.ticket,
            0,
            Path::new("Sorted"),
            Some("Read me"),
            &|| false,
        )
        .unwrap();
    assert!(
        matches!(&revised.plan.actions[0], Action::Move { files } if files[0].1 == context.root.join("Sorted/Read me"))
    );
    store.discard(&context.root, &revised.ticket).unwrap();
    plan.actions = vec![Action::Move {
        files: ["a.jpg", "b.jpg"]
            .into_iter()
            .map(|name| {
                (
                    Source::capture(context.root.join(name)).unwrap(),
                    context.root.join("Sorted").join(name),
                )
            })
            .collect(),
    }];
    prepared(&store, &context, &plan, b"");
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let error = store
        .amend_destination(
            &context.root,
            &restored.ticket,
            0,
            Path::new("Sorted"),
            Some("Same"),
            &|| false,
        )
        .unwrap_err();
    assert!(error.to_string().contains("Companion destinations collide"));
}

#[test]
#[ignore = "copies seven original samples; optional PHOTOSORT_AMEND_KEEP_NATIVE leaves a native fixture"]
fn seven_sample_move_amendment_keeps_completed_files_companions_and_bytes() {
    let samples = PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let originals = Catalog::scan(&samples, || false, |_| {}).unwrap();
    assert_eq!(originals.media.len(), 7);
    let keep = std::env::var_os("PHOTOSORT_AMEND_KEEP_NATIVE").is_some();
    let scratch = if keep {
        tempfile::Builder::new()
            .prefix("amend-native-")
            .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join(".tools"))
            .unwrap()
    } else {
        tempfile::tempdir().unwrap()
    };
    let base = scratch.path().canonicalize().unwrap();
    let root = base.join("library");
    fs::create_dir_all(root.join("Before")).unwrap();
    let mut hashes = HashMap::new();
    for photo in &originals.media {
        fs::copy(
            &photo.path,
            root.join("Before").join(photo.path.file_name().unwrap()),
        )
        .unwrap();
        hashes.insert(
            photo.name(),
            crate::core::operations::content_hash(&photo.path, || false).unwrap(),
        );
    }
    fs::write(root.join("Before/arw_sample.xmp"), b"test companion").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let labels = catalog
        .media
        .iter()
        .map(|m| (m.id, "Sorted".into()))
        .collect();
    let plan = crate::core::operations::organize_plan(&catalog, &labels, &root, true).unwrap();
    fs::create_dir_all(root.join("Sorted/arw_sample.ARW")).unwrap();
    let context = Context {
        root: root.clone(),
        rotation_only: false,
        media: catalog.media.clone(),
        session: Session {
            decisions: catalog
                .media
                .iter()
                .map(|m| {
                    (
                        m.id,
                        Decision {
                            rating: Some(4),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            fingerprints: catalog
                .media
                .iter()
                .map(|m| (m.id, m.fingerprint))
                .collect(),
            ..Default::default()
        },
        views: Default::default(),
    };
    let store = if keep {
        Store::standard()
    } else {
        Store::new(base.join("journals"))
    };
    let report = store
        .execute(&context, &plan, None, || false, |_, _| {})
        .unwrap();
    let ticket = report.recovery.unwrap();
    assert_eq!(report.remaining.actions.len(), 2);
    let Action::Move { files } = &report.remaining.actions[0] else {
        panic!()
    };
    assert_eq!(files.len(), 2);
    if keep {
        let state = serde_json::json!({"root":root,"fixture":base,"sample_blake3":hashes,"original_journal":store.directory.join(&ticket.journal),"active_pointer":store.pointer(&root)});
        fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join(".tools/amend-native-state.json"),
            serde_json::to_vec_pretty(&state).unwrap(),
        )
        .unwrap();
        let _ = scratch.keep();
        return;
    }
    let revised = store
        .amend_destination(
            &root,
            &ticket,
            0,
            Path::new("Corrected"),
            Some("Recovered"),
            &|| false,
        )
        .unwrap();
    let restored = store.restore(&root, &|| false).unwrap().unwrap();
    assert_eq!(revised.ticket, restored.ticket);
    let next = Context {
        session: Session::load(&root).unwrap(),
        views: views::Saved::load(&root).unwrap(),
        ..context
    };
    let done = store
        .execute(
            &next,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(done.errors.is_empty() && done.remaining.actions.is_empty());
    assert!(!root.join("Before").exists());
    assert_eq!(
        fs::read(root.join("Corrected/Recovered.xmp")).unwrap(),
        b"test companion"
    );
    assert!(root.join("Sorted/arw_sample.ARW").is_dir());
    for original in originals.media {
        let destination = if original.name() == "arw_sample.ARW" {
            root.join("Corrected/Recovered.ARW")
        } else {
            root.join("Sorted").join(original.path.file_name().unwrap())
        };
        assert_eq!(
            crate::core::operations::content_hash(&destination, || false).unwrap(),
            hashes[&original.name()]
        );
        assert_eq!(
            crate::core::operations::content_hash(&original.path, || false).unwrap(),
            hashes[&original.name()]
        );
    }
    assert_eq!(Session::load(&root).unwrap().decisions.len(), 7);
}
#[test]
#[cfg(unix)]
fn staged_move_recovery_cancels_retries_restarts_and_keeps_one_receipt_per_file() {
    let (_temp, store, context, mut plan) = fixture();
    let Action::Move { files: second } = plan.actions.remove(1) else {
        panic!()
    };
    let Action::Move { files } = &mut plan.actions[0] else {
        panic!()
    };
    files.extend(second);
    fs::create_dir(context.root.join("Sorted")).unwrap();
    for (source, to) in files {
        fs::hard_link(&source.path, to).unwrap();
    }
    let original = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 0);
    assert_eq!(restored.plan.actions.len(), 2);
    assert!(
        restored
            .plan
            .actions
            .iter()
            .all(|a| matches!(a, Action::FinishMove { files } if files.len()==1))
    );
    assert!(context.root.join("a.jpg").exists() && context.root.join("b.jpg").exists());
    assert!(
        store
            .amend_destination(
                &context.root,
                &restored.ticket,
                0,
                Path::new("Elsewhere"),
                None,
                &|| false
            )
            .is_err()
    );
    let stop = Cell::new(false);
    let report = store
        .execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || stop.get(),
            |_, _| stop.set(true),
        )
        .unwrap();
    assert!(report.cancelled && report.completed.len() == 1 && report.remaining.actions.len() == 1);
    let again = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(again.uncertain, 0);
    assert_eq!(again.plan.actions.len(), 1);
    let first = MediaId::for_path(&context.root.join("Sorted/a.jpg"));
    let mut session = Session::load(&context.root).unwrap();
    session.decisions.get_mut(&first).unwrap().rating = Some(5);
    session.save(&context.root).unwrap();
    let reopen = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(reopen.ticket, again.ticket);
    assert_eq!(
        Session::load(&context.root).unwrap().decisions[&first].rating,
        Some(5)
    );
    let current = Context {
        session,
        ..context.clone()
    };
    let done = store
        .execute(
            &current,
            &reopen.plan,
            Some(&reopen.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(done.errors.is_empty() && done.remaining.actions.is_empty());
    assert!(original.exists());
    assert!(!context.root.join("a.jpg").exists() && !context.root.join("b.jpg").exists());
}
#[test]
#[cfg(unix)]
fn staged_retry_rechecks_destination_source_and_symlinks_before_removing_any_source() {
    for mode in [
        "destination-replaced",
        "destination-missing",
        "source-changed",
        "symlink",
        "cancel",
    ] {
        let (_temp, store, context, plan) = fixture();
        let source = context.root.join("a.jpg");
        let to = context.root.join("Sorted/a.jpg");
        fs::create_dir(to.parent().unwrap()).unwrap();
        fs::hard_link(&source, &to).unwrap();
        prepared(
            &store,
            &context,
            &plan,
            b"{\"status\":\"pending\",\"index\":0}\n",
        );
        let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
        assert!(matches!(
            restored.plan.actions[0],
            Action::FinishMove { .. }
        ));
        match mode {
            "destination-replaced" => {
                fs::remove_file(&to).unwrap();
                fs::write(&to, b"first photo").unwrap();
            }
            "destination-missing" => fs::remove_file(&to).unwrap(),
            "source-changed" => fs::write(&source, b"changed source").unwrap(),
            "symlink" => {
                fs::remove_file(&to).unwrap();
                std::os::unix::fs::symlink(&source, &to).unwrap();
            }
            _ => {}
        }
        let bytes = fs::read(&source).unwrap();
        let result = store.execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || mode == "cancel",
            |_, _| {},
        );
        if mode == "cancel" {
            assert!(result.is_err());
        } else {
            assert!(!result.unwrap().errors.is_empty(), "{mode}");
        }
        assert_eq!(fs::read(&source).unwrap(), bytes, "{mode}");
    }
}
#[test]
#[cfg(unix)]
fn partial_recovery_does_not_guess_when_any_companion_has_a_conflicting_identity() {
    for mode in ["copied", "changed", "missing", "symlink"] {
        let (_temp, store, context, mut plan) = fixture();
        let Action::Move { files: second } = plan.actions.remove(1) else {
            panic!()
        };
        let Action::Move { files } = &mut plan.actions[0] else {
            panic!()
        };
        files.extend(second);
        fs::create_dir(context.root.join("Sorted")).unwrap();
        fs::rename(&files[0].0.path, &files[0].1).unwrap();
        match mode {
            "copied" => {
                fs::copy(&files[1].0.path, &files[1].1).unwrap();
            }
            "changed" => {
                fs::hard_link(&files[1].0.path, &files[1].1).unwrap();
                fs::write(&files[1].1, b"changed").unwrap();
            }
            "missing" => {
                fs::remove_file(&files[1].0.path).unwrap();
            }
            "symlink" => {
                std::os::unix::fs::symlink(&files[1].0.path, &files[1].1).unwrap();
            }
            _ => unreachable!(),
        }
        let log = prepared(
            &store,
            &context,
            &plan,
            b"{\"status\":\"pending\",\"index\":0}\n",
        );
        let before = fs::read(&log).unwrap();
        let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
        assert_eq!(restored.uncertain, 1, "{mode}");
        assert_eq!(store.directory.join(restored.ticket.journal), log);
        assert_eq!(fs::read(&log).unwrap(), before);
    }
}
#[test]
fn malformed_partial_move_receipts_are_rejected() {
    for tail in [
        b"{\"status\":\"recovered-move\",\"index\":0,\"move_states\":[\"staged\"]}\n".as_slice(),
        b"{\"status\":\"pending\",\"index\":0}\n{\"status\":\"recovered-move\",\"index\":0,\"move_states\":[]}\n",
        b"{\"status\":\"pending\",\"index\":0}\n{\"status\":\"recovered-move\",\"index\":0,\"move_states\":[\"untouched\"]}\n",
    ] {
        let (_temp, store, context, plan) = fixture();
        prepared(&store, &context, &plan, tail);
        assert!(store.restore(&context.root, &|| false).is_err());
        assert!(context.root.join("a.jpg").exists());
    }
}
#[test]
#[cfg(unix)]
fn staged_move_recovery_handles_another_crash_after_one_source_removal() {
    let (_temp, store, context, mut plan) = fixture();
    let Action::Move { files: second } = plan.actions.remove(1) else {
        panic!()
    };
    let Action::Move { files } = &mut plan.actions[0] else {
        panic!()
    };
    files.extend(second);
    fs::create_dir(context.root.join("Sorted")).unwrap();
    for (source, to) in files {
        fs::hard_link(&source.path, to).unwrap();
    }
    let first_log = prepared(
        &store,
        &context,
        &plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    let restored = store.restore(&context.root, &|| false).unwrap().unwrap();
    let second_log = prepared(
        &store,
        &context,
        &restored.plan,
        b"{\"status\":\"pending\",\"index\":0}\n",
    );
    fs::remove_file(context.root.join("a.jpg")).unwrap();
    let recovered = store.restore(&context.root, &|| false).unwrap().unwrap();
    assert_eq!(recovered.uncertain, 0);
    assert_eq!(recovered.plan.actions.len(), 1);
    assert!(
        matches!(&recovered.plan.actions[0], Action::FinishMove { files } if files.len()==1 && files[0].0.path==context.root.join("b.jpg"))
    );
    assert_eq!(
        Session::load(&context.root).unwrap().decisions
            [&MediaId::for_path(&context.root.join("Sorted/a.jpg"))]
            .rating,
        Some(4)
    );
    assert!(first_log.exists() && second_log.exists() && context.root.join("b.jpg").exists());
}
#[test]
#[cfg(unix)]
fn staged_move_never_removes_the_only_destination_through_an_alias() {
    let (_temp, _store, context, _plan) = fixture();
    let source = Source::capture(context.root.join("a.jpg")).unwrap();
    fs::create_dir(context.root.join("nested")).unwrap();
    for to in [
        source.path.clone(),
        context.root.join("nested/../a.jpg"),
        context.root.join("A.JPG"),
    ] {
        let plan = Plan {
            actions: vec![Action::FinishMove {
                files: vec![(source.clone(), to)],
            }],
            ..Default::default()
        };
        let report =
            super::super::execute(&plan, &context.root.join("audit"), || false, |_, _| {}).unwrap();
        assert!(!report.errors.is_empty());
        assert_eq!(fs::read(&source.path).unwrap(), b"first photo");
    }
    // Distinct same-directory names really are two links and remain valid.
    let destination = context.root.join("renamed.jpg");
    fs::hard_link(&source.path, &destination).unwrap();
    let plan = Plan {
        actions: vec![Action::FinishMove {
            files: vec![(source.clone(), destination.clone())],
        }],
        ..Default::default()
    };
    let report =
        super::super::execute(&plan, &context.root.join("audit"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty());
    assert!(!source.path.exists());
    assert_eq!(fs::read(destination).unwrap(), b"first photo");
}
