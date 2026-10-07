//! Save a revised destination as new durable intent, without applying files.
use super::*;
use crate::core::operations::Action;

impl Store {
    pub fn amend_destination(
        &self,
        root: &Path,
        ticket: &Ticket,
        group: usize,
        folder: &Path,
        stem: Option<&str>,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Restored> {
        let _lock = self.lock(root)?;
        let pointer = self
            .read_pointer(root)?
            .context("Unfinished plan was already resolved")?;
        let loaded = self.read(root, &pointer, cancelled)?;
        ensure!(
            &loaded.ticket == ticket,
            "Unfinished plan changed; reopen the library before editing"
        );
        ensure!(
            pointer.settled && loaded.uncertain == 0,
            "Reopen and resolve interrupted operations before editing their destinations"
        );
        ensure!(
            !folder.is_absolute()
                && folder
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_))),
            "Use a relative folder within the plan's destination, without . or .. components"
        );
        let mut plan = loaded.report.remaining;
        let output = plan.output_root.as_deref().unwrap_or(root);
        let target = output.join(folder);
        reject_symlink_components(&target)?;
        let target = super::super::normalized_destination(&target)?;
        ensure!(
            target.starts_with(output),
            "Destination must stay within the reviewed output library"
        );
        let action = plan
            .actions
            .get(group)
            .context("Operation group is no longer present")?;
        let Action::Move { files } = action else {
            anyhow::bail!("Only pending file moves can change destination folders");
        };
        ensure!(!files.is_empty(), "No files in the selected move");
        if let Some(stem) = stem {
            crate::core::organize::validate_file_stem(stem)?;
        }
        let revised: Vec<_> = files
            .iter()
            .map(|(source, to)| {
                Ok((
                    source.clone(),
                    target.join(match stem {
                        Some(stem) => crate::core::organize::renamed_file_name(to, stem.as_ref()),
                        None => to
                            .file_name()
                            .context("Destination has no filename")?
                            .to_owned(),
                    }),
                ))
            })
            .collect::<Result<_>>()?;
        ensure!(
            files
                .iter()
                .zip(&revised)
                .any(|((_, old), (_, new))| old != new),
            "Choose a different destination folder or filename"
        );
        let mut destinations = HashSet::new();
        let key = |p: &Path| p.to_string_lossy().to_lowercase();
        for (source, to) in &revised {
            ensure!(!cancelled(), "Destination edit cancelled");
            reject_symlink_components(&source.path)?;
            source.verify()?;
            reject_symlink_components(to)?;
            ensure!(
                matches!(fs::symlink_metadata(to), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
                "Destination already exists or cannot be checked: {}",
                to.display()
            );
            ensure!(
                destinations.insert(key(to)),
                "Companion destinations collide"
            );
        }
        // Do not leave a partly completed hard-link move behind under a new name.
        for (source, old) in files {
            ensure!(
                Fingerprint::read(old).ok() != Some(source.fingerprint),
                "A reviewed destination already contains this file; resolve the partial move first"
            );
        }
        for (index, action) in plan.actions.iter().enumerate() {
            ensure!(!cancelled(), "Destination edit cancelled");
            if index == group {
                continue;
            }
            let other: Vec<(&Path, bool)> = match action {
                Action::Move { files } | Action::FinishMove { files } => {
                    files.iter().map(|(_, p)| (p.as_path(), true)).collect()
                }
                Action::RenameFile { to, .. } => vec![(to, true)],
                Action::CreateDirectory { path } => vec![(path, false)],
                Action::RenameDirectory(rename) => vec![(&rename.to, true)],
                _ => vec![],
            };
            for (path, blocks_children) in other {
                let path = key(path);
                ensure!(
                    !destinations.iter().any(|to| to == &path
                        || (blocks_children
                            && to.starts_with(&(path.clone() + std::path::MAIN_SEPARATOR_STR)))
                        || path.starts_with(&(to.clone() + std::path::MAIN_SEPARATOR_STR))),
                    "Destination conflicts with another pending operation"
                );
            }
        }
        plan.actions[group] = Action::Move { files: revised };
        let prepared = self.prepare(&loaded.header.context, &plan, cancelled)?;
        // The old context is deliberately not replayed: all its completed intent
        // is already settled. Current ratings/bookmarks remain authoritative.
        if let Action::Move { files } = &plan.actions[group] {
            for (source, _) in files {
                ensure!(!cancelled(), "Destination edit cancelled");
                source.verify()?;
            }
        }
        ensure!(
            Fingerprint::read(&self.directory.join(&ticket.journal))? == ticket.fingerprint,
            "Unfinished plan changed during editing"
        );
        ensure!(
            !cancelled(),
            "Destination edit cancelled before publication"
        );
        let fingerprint = Fingerprint::read(prepared.path())?;
        let (_, path) = prepared.keep().map_err(|e| e.error)?;
        let journal = path
            .file_name()
            .unwrap()
            .to_str()
            .context("Journal name is not text")?
            .to_owned();
        if let Err(error) = self.write_pointer(
            root,
            &Pointer {
                journal: journal.clone(),
                settled: true,
            },
        ) {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Restored {
            ticket: Ticket {
                journal,
                fingerprint,
            },
            plan,
            rotation_only: loaded.header.context.rotation_only,
            uncertain: 0,
            errors: vec![],
        })
    }
}
