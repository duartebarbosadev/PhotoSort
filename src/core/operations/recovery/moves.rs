//! Receipt recovery reads identities only. Removing staged source links always
//! remains an explicit action on the normal file-operation worker.
use super::*;
use crate::core::operations::{Action, verify_staged_move};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum State {
    Completed,
    Staged,
    Untouched,
}

/// Cross-device copies and platforms without an inode/device identity stay explicit.
pub(super) fn verify(action: &Action, cancelled: &impl Fn() -> bool) -> Result<Option<Vec<State>>> {
    ensure!(!cancelled(), "Recovery verification cancelled");
    let (Action::Move { files } | Action::FinishMove { files }) = action else {
        return Ok(None);
    };
    if files.is_empty() || !cfg!(unix) {
        return Ok(None);
    }
    let mut states = Vec::with_capacity(files.len());
    for (source, destination) in files {
        ensure!(!cancelled(), "Recovery verification cancelled");
        if source.fingerprint.identity[1] == 0
            || !source.path.is_absolute()
            || !destination.is_absolute()
            || reject_symlink_components(&source.path).is_err()
            || reject_symlink_components(destination).is_err()
        {
            return Ok(None);
        }
        let source_missing = matches!(fs::symlink_metadata(&source.path), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
        let state = if source_missing {
            if Fingerprint::read(destination).ok() != Some(source.fingerprint) {
                return Ok(None);
            }
            State::Completed
        } else if verify_staged_move(source, destination).is_ok() {
            State::Staged
        } else if matches!(action, Action::Move { .. })
            && Fingerprint::read(&source.path).ok() == Some(source.fingerprint)
            && matches!(fs::symlink_metadata(destination), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        {
            State::Untouched
        } else {
            // A different destination identity, changed source, symlink or
            // unknown filesystem error cannot justify any inferred operation.
            return Ok(None);
        };
        states.push(state);
    }
    ensure!(!cancelled(), "Recovery verification cancelled");
    Ok(states
        .iter()
        .any(|s| *s != State::Untouched)
        .then_some(states))
}

pub(super) fn restore(
    action: &Action,
    states: &[State],
    report: &mut Report,
) -> Result<Vec<Action>> {
    let (Action::Move { files } | Action::FinishMove { files }) = action else {
        anyhow::bail!("Move recovery record refers to another operation type");
    };
    ensure!(
        states.len() == files.len() && !files.is_empty(),
        "Invalid move recovery member count"
    );
    ensure!(
        states.iter().any(|s| *s != State::Untouched),
        "Move recovery has no verified progress"
    );
    ensure!(
        !matches!(action, Action::FinishMove { .. }) || !states.contains(&State::Untouched),
        "Staged move recovery cannot request a new copy"
    );
    let mut completed = Vec::new();
    let mut staged = Vec::new();
    let mut untouched = Vec::new();
    let mut relocated = Vec::new();
    for ((source, destination), state) in files.iter().zip(states) {
        match state {
            State::Completed => {
                completed.push((source.clone(), destination.clone()));
                relocated.push((
                    MediaId::for_path(&source.path),
                    MediaId::for_path(destination),
                    source.fingerprint,
                ));
            }
            State::Staged => staged.push((source.clone(), destination.clone())),
            State::Untouched => untouched.push((source.clone(), destination.clone())),
        }
    }
    if !completed.is_empty() {
        report.complete(&Action::Move { files: completed }, &relocated);
    }
    let mut remaining = Vec::new();
    // Each remaining unlink gets its own normal pending/completed receipt.
    // A second interruption can never hide progress inside another cohort.
    remaining.extend(
        staged
            .into_iter()
            .map(|file| Action::FinishMove { files: vec![file] }),
    );
    if !untouched.is_empty() {
        remaining.push(Action::Move { files: untouched });
    }
    Ok(remaining)
}

pub(super) fn write(
    journal: &mut File,
    index: usize,
    action: &Action,
    states: &[State],
) -> Result<()> {
    // Keep complete-cohort receipts compatible with existing audit readers.
    if states.iter().all(|s| *s == State::Completed) {
        let mut report = Report::default();
        restore(action, states, &mut report)?;
        return super::super::write_record(
            journal,
            "recovered",
            index,
            action,
            &report.relocated,
            None,
        );
    }
    #[derive(Serialize)]
    struct Receipt<'a> {
        status: &'static str,
        index: usize,
        action: &'a Action,
        move_states: &'a [State],
    }
    serde_json::to_writer(
        &mut *journal,
        &Receipt {
            status: "recovered-move",
            index,
            action,
            move_states: states,
        },
    )?;
    journal.write_all(b"\n")?;
    Ok(())
}
