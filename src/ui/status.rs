//! Status ownership is independent of the currently visible workflow.
use super::*;
use std::borrow::Cow;

impl PhotoSortApp {
    /// Shared work and filesystem recovery remain visible while changing steps.
    pub(super) fn set_status(&mut self, message: String) {
        self.status = message;
        self.status_scope = None;
    }
    /// The producer names its owner, including when it completes in the background.
    pub(super) fn set_workflow_status(&mut self, step: Step, message: String) {
        self.status = message;
        self.status_scope = Some((step, (step == Step::Organize).then_some(self.group_mode)));
    }
    pub(super) fn set_organize_status(&mut self, mode: GroupMode, message: String) {
        self.status = message;
        self.status_scope = Some((Step::Organize, Some(mode)));
    }
    pub(super) fn workflow_status(&self) -> Cow<'_, str> {
        if let Some(status) = self.preparing_model() {
            return Cow::Owned(format!(
                "Preparing {} · {} · first use can take minutes",
                status.model,
                status.backend.label()
            ));
        }
        if self.step == Step::Organize
            && let Some(run) = self.analysis_run
            && self.organize_run == Some((run, GroupMode::Faces))
            && let Some((done, total)) = self.progress
        {
            return Cow::Owned(format!("Finding faces · {done} of {total}"));
        }
        if self.status_scope.is_none_or(|(step, mode)| {
            self.step == step && mode.is_none_or(|mode| self.group_mode == mode)
        }) {
            return Cow::Borrowed(&self.status);
        }
        Cow::Owned(if self.root.is_some() {
            format!("{} items · {} ready", self.media.len(), self.step.title())
        } else {
            "Open a folder to begin".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(temp: &tempfile::TempDir) -> PhotoSortApp {
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.root = Some(temp.path().into());
        app
    }
    #[test]
    fn one_unreadable_photo_is_reported_without_stopping_pick_best() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = app(&temp);
        app.media = vec![Media {
            id: MediaId(4),
            path: temp.path().join("broken.jpg"),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        }];
        app.index.insert(MediaId(4), 0);
        app.analysis_run = Some(9);
        app.pick_run = Some(9);
        app.on_analysis_error(9, MediaId(4), "Cannot decode broken.jpg".into());
        assert_eq!((app.analysis_run, app.pick_run), (Some(9), Some(9)));
        assert!(app.error.is_none(), "no dialog per photo");
        assert!(app.status.contains("broken.jpg could not be analyzed"));
        app.finish_pick_analysis();
        assert!(app.status.contains("1 photo could not be analyzed"));
        // Nothing analyzable at all: stop once with the cause, not per photo.
        app.analysis_run = Some(11);
        app.pick_run = Some(11);
        app.analysis_failures.clear();
        app.analysis_successes = 0;
        for n in 0..5 {
            app.on_analysis_error(11, MediaId(4 + n), "Model file missing".into());
        }
        assert_eq!((app.analysis_run, app.pick_run), (None, None));
        assert!(
            app.error
                .as_deref()
                .is_some_and(|e| e.contains("Model file missing"))
        );
        app.error = None;
        // A late error from a superseded run is ignored.
        app.analysis_run = Some(10);
        app.analysis_failures.clear();
        app.on_analysis_error(9, MediaId(4), "stale".into());
        assert!(app.analysis_failures.is_empty());
        app.runtime.shutdown();
    }
    #[cfg(feature = "onnx")]
    #[test]
    fn model_preparation_is_visible_only_while_the_shared_run_is_active() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = app(&temp);
        app.runtime
            .processing
            .preparing("dinov2", crate::core::processing::Backend::CoreMl);
        app.analysis_run = Some(42);
        assert!(app.workflow_status().contains("Preparing dinov2"));
        app.cancel_analysis();
        assert!(!app.workflow_status().contains("Preparing dinov2"));
        app.rotation_run = Some(43);
        assert!(app.workflow_status().contains("Apple Core ML"));
        app.runtime.processing.report(
            "dinov2",
            crate::core::processing::Backend::Cpu,
            "CPU fallback".into(),
        );
        assert!(!app.workflow_status().contains("Preparing dinov2"));
        assert_eq!(app.stats.decodes, 0);
        app.runtime.shutdown();
    }
    #[test]
    fn workflow_status_follows_all_step_transitions_without_changing_reviews_or_workers() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = app(&temp);
        app.decisions.insert(
            MediaId(77),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        for from in Step::ALL {
            for to in Step::ALL {
                app.set_step(from);
                app.set_workflow_status(from, "Instructions belonging to the previous step".into());
                assert_eq!(
                    app.workflow_status(),
                    "Instructions belonging to the previous step"
                );
                app.set_step(to);
                assert_eq!(
                    app.workflow_status().contains("Instructions belonging"),
                    from == to
                );
                assert_eq!(app.decisions[&MediaId(77)].rating, Some(4));
                assert!(
                    app.requested.is_empty()
                        && app.analysis_run.is_none()
                        && app.rotation_run.is_none()
                );
                assert_eq!(app.stats.decodes, 0);
            }
        }
        app.runtime.shutdown();
    }
    #[test]
    fn asynchronous_review_completions_keep_their_origin_in_the_rendered_footer() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = app(&temp);
        app.step = Step::Cull;
        app.finish_pick_analysis();
        // An empty fixture: no groups and nothing to clean up.
        assert!(app.status.contains("No similar photos found"));
        assert_eq!(app.status_scope, Some((Step::PickBest, None)));
        assert!(!app.workflow_status().contains("similar photos"));
        app.finish_delete_analysis();
        assert!(app.status.contains("Nothing to clean up"));
        assert_eq!(app.status_scope, Some((Step::EasyDelete, None)));
        assert!(!app.workflow_status().contains("clean up"));
        app.finish_rotation_detection();
        assert_eq!(app.status_scope, Some((Step::Rotation, None)));
        assert!(!app.workflow_status().contains("rotation suggestions"));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let output = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| app.footer(ui),
        );
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.value() == Some("0 items · Cull ready"))
        );
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, n)| n.value().is_some_and(|v| v.contains("confirm choices")))
        );
        app.set_step(Step::Rotation);
        assert!(app.workflow_status().contains("rotation suggestions"));
        app.runtime.shutdown();
    }
    #[test]
    fn shared_activity_recovery_and_organize_mode_status_keep_their_correct_scope() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = app(&temp);
        app.set_organize_status(GroupMode::Faces, "People groups ready".into());
        app.step = Step::Organize;
        app.group_mode = GroupMode::Faces;
        assert_eq!(app.workflow_status(), "People groups ready");
        app.set_group_mode(GroupMode::Current);
        assert!(!app.workflow_status().contains("People groups"));
        app.set_step(Step::Cull);
        app.set_organize_status(GroupMode::Mixed, "Date and similarity groups ready".into());
        assert!(!app.workflow_status().contains("similarity groups"));
        for message in [
            "Discovering photos…",
            "Analyzing locally · previews stay available",
            "Applying reviewed changes…",
            "3 unfinished operations · Review changes to resolve",
        ] {
            app.set_status(message.into());
            for step in Step::ALL {
                app.set_step(step);
                assert_eq!(app.workflow_status(), message);
                assert!(app.status_scope.is_none());
            }
        }
        app.runtime.shutdown();
    }
}
