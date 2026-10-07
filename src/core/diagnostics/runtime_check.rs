//! Exercises production inference services, without UI, a user library or cache writes.
use super::{Report, collect_cancellable};
use crate::core::{
    face_groups, faces,
    models::{self, Models},
    pipeline::Asset,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{sync::Arc, time::Instant};

#[derive(Clone, Copy)]
enum Check {
    Scene,
    Aesthetic,
    Faces,
    Recognition,
    Orientation,
}
const CHECKS: [Check; 5] = [
    Check::Scene,
    Check::Aesthetic,
    Check::Faces,
    Check::Recognition,
    Check::Orientation,
];
impl Check {
    fn name(self) -> &'static str {
        match self {
            Self::Scene => "scene_embedding",
            Self::Aesthetic => "aesthetics",
            Self::Faces => "face_geometry",
            Self::Recognition => "face_recognition",
            Self::Orientation => "orientation",
        }
    }
}
#[derive(Serialize)]
pub struct Step {
    pub name: &'static str,
    pub pass: u8,
    pub passed: bool,
    pub detail: String,
    pub elapsed_ms: u128,
}
#[derive(Serialize)]
pub struct RuntimeReport {
    pub diagnostics: Report,
    pub passed: bool,
    pub cancelled: bool,
    pub sessions_reused: bool,
    pub initializations: usize,
    pub inferences: u64,
    pub cpu_threads: usize,
    pub steps: Vec<Step>,
    pub fixture: &'static str,
    pub note: &'static str,
}
struct Outcome {
    result: Result<String>,
    initializations: usize,
    inferences: u64,
}
pub fn check_runtime(cancelled: &impl Fn() -> bool) -> Result<RuntimeReport> {
    ensure!(!cancelled(), "Runtime check cancelled");
    let mut report = RuntimeReport {
        diagnostics: collect_cancellable(cancelled)?,
        passed: false,
        cancelled: false,
        sessions_reused: false,
        initializations: 0,
        inferences: 0,
        cpu_threads: 1,
        steps: Vec::new(),
        fixture: include_str!("../../../tests/fixtures/recognition/SOURCE.md"),
        note: "Two native CPU inference passes using one decoded fixture and shared production services. Does not validate graphics, video/audio, all photo formats, GPU execution or photographic accuracy.",
    };
    if !report.diagnostics.capabilities.neural_inference || !report.diagnostics.files_ready {
        report.steps.push(Step {
            name: "prerequisites",
            pass: 0,
            passed: false,
            detail: "Enable ONNX support and resolve the file diagnostics before running inference"
                .into(),
            elapsed_ms: 0,
        });
        return Ok(report);
    }
    report.diagnostics.note = "File inventory was checked before native execution. Runtime and selected orientation compatibility flags reflect the outer runtime-check result. Paths use display text for non-UTF-8 names.";
    let mut engine = Engine::new()?;
    Ok(run(
        report,
        |check| engine.check(check, cancelled),
        cancelled,
    ))
}
fn run(
    mut report: RuntimeReport,
    mut probe: impl FnMut(Check) -> Outcome,
    cancelled: &impl Fn() -> bool,
) -> RuntimeReport {
    let mut first_initializations = 0;
    for pass in 1..=2 {
        for check in CHECKS {
            if cancelled() {
                report.cancelled = true;
                return report;
            }
            let started = Instant::now();
            let outcome = probe(check);
            report.initializations = outcome.initializations;
            report.inferences = outcome.inferences;
            report.steps.push(Step {
                name: check.name(),
                pass,
                passed: outcome.result.is_ok(),
                detail: outcome.result.unwrap_or_else(|e| format!("{e:#}")),
                elapsed_ms: started.elapsed().as_millis(),
            });
        }
        if cancelled() {
            report.cancelled = true;
            return report;
        }
        if report.steps.iter().any(|step| !step.passed) {
            return report;
        }
        if pass == 1 {
            first_initializations = report.initializations;
        }
    }
    report.sessions_reused =
        first_initializations == 6 && report.initializations == first_initializations;
    report.passed = report.sessions_reused && report.inferences >= 12;
    if !report.passed {
        report.steps.push(Step { name: "session_reuse", pass: 2, passed: false,
            detail: format!("Expected six reused sessions and at least twelve inferences; got {} initializations and {} inferences", report.initializations, report.inferences), elapsed_ms: 0 });
    }
    if report.passed {
        report.diagnostics.runtime.load_verified = true;
        report.diagnostics.runtime.status = "loaded_and_exercised";
        if let Some(overridden) = &mut report.diagnostics.orientation_override {
            overridden.model_compatibility_verified = true;
        }
    }
    report
}
struct Engine {
    models: Models,
    asset: Asset,
    faces: Option<faces::Faces>,
}
impl Engine {
    fn new() -> Result<Self> {
        let demo = image::load_from_memory(include_bytes!(
            "../../../tests/fixtures/recognition/sface-demo.jpg"
        ))?
        .to_rgba8();
        let pixels = Arc::new(image::imageops::crop_imm(&demo, 0, 12, 512, 488).to_image());
        let mut models = Models::default();
        models.configure_threads(1);
        Ok(Self {
            models,
            asset: Asset {
                decoder_warning: None,
                pixels: pixels.clone(),
                thumbnail: pixels,
                metadata: Default::default(),
            },
            faces: None,
        })
    }
    fn check(&mut self, check: Check, cancelled: &impl Fn() -> bool) -> Outcome {
        let result = (|| -> Result<String> {
            match check {
                Check::Scene => {
                    self.models.dense_embedding(&self.asset)?;
                    Ok("Validated 384-dimensional CLS and pooled spatial evidence".into())
                }
                Check::Aesthetic => {
                    let score = self.models.aesthetic(&self.asset)?;
                    ensure!(
                        score.is_finite() && (0.0..=1.0).contains(&score),
                        "Invalid aesthetic probability"
                    );
                    Ok(format!("Finite aesthetic probability: {score:.6}"))
                }
                Check::Faces => {
                    self.faces = None;
                    let faces =
                        faces::analyze_cancellable(&mut self.models, &self.asset, cancelled)?;
                    ensure!(
                        faces.count > 0 && faces.complete(),
                        "Fixture has no complete verified face geometry"
                    );
                    let detail = format!(
                        "{} verified face(s); detector and landmarks exercised",
                        faces.count
                    );
                    self.faces = Some(faces);
                    Ok(detail)
                }
                Check::Recognition => {
                    let faces = self
                        .faces
                        .as_ref()
                        .context("Face geometry check must pass first")?;
                    let recognition =
                        face_groups::describe(&mut self.models, &self.asset, faces, cancelled)?;
                    ensure!(
                        !recognition.vectors.is_empty() && recognition.valid(),
                        "Fixture has no valid normalized face descriptor"
                    );
                    Ok("Validated normalized 128-dimensional identity descriptor".into())
                }
                Check::Orientation => {
                    let path =
                        models::orientation_model().context("Orientation model is missing")?;
                    let suggestion =
                        self.models
                            .rotation_cancellable(&path, &self.asset, cancelled)?;
                    ensure!(
                        suggestion.turns <= 3
                            && suggestion.confidence.is_finite()
                            && (0.0..=1.0).contains(&suggestion.confidence),
                        "Invalid orientation output"
                    );
                    Ok(format!(
                        "Validated four-class orientation output; model score {:.6}, {} view(s), uncertain {}",
                        suggestion.confidence, suggestion.views, suggestion.uncertain
                    ))
                }
            }
        })();
        Outcome {
            result,
            initializations: self.models.initializations,
            inferences: self.models.inferences,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn report() -> RuntimeReport {
        RuntimeReport {
            diagnostics: super::super::from_snapshot(
                super::super::tests::snapshot(),
                super::super::Capabilities {
                    neural_inference: true,
                    ..Default::default()
                },
            ),
            passed: false,
            cancelled: false,
            sessions_reused: false,
            initializations: 0,
            inferences: 0,
            cpu_threads: 1,
            steps: Vec::new(),
            fixture: "fake",
            note: "test",
        }
    }
    #[test]
    fn runtime_success_requires_real_second_pass_session_reuse() {
        for reuse in [true, false] {
            let mut calls = 0;
            let report = run(
                report(),
                |_| {
                    calls += 1;
                    Outcome {
                        result: Ok("valid".into()),
                        initializations: if reuse || calls <= 5 { 6 } else { 12 },
                        inferences: calls * 2,
                    }
                },
                &|| false,
            );
            assert_eq!(calls, 10);
            assert_eq!(report.passed, reuse);
            assert_eq!(report.sessions_reused, reuse);
            assert_eq!(report.diagnostics.runtime.load_verified, reuse);
        }
    }
    #[test]
    fn failure_preserves_per_stage_evidence_and_skips_the_second_pass() {
        let mut calls = 0;
        let report = run(
            report(),
            |_| {
                calls += 1;
                Outcome {
                    result: if calls == 3 {
                        Err(anyhow::anyhow!("invalid output"))
                    } else {
                        Ok("valid".into())
                    },
                    initializations: 4,
                    inferences: calls,
                }
            },
            &|| false,
        );
        assert_eq!(calls, 5);
        assert!(
            !report.passed && !report.sessions_reused && !report.diagnostics.runtime.load_verified
        );
        assert!(report.steps[2].detail.contains("invalid output"));
    }
    #[test]
    fn cancellation_never_starts_a_model_check_or_claims_readiness() {
        let report = run(report(), |_| panic!("must stay lazy"), &|| true);
        assert!(report.cancelled && report.steps.is_empty() && !report.passed);
    }
}

#[cfg(test)]
mod completion_tests {
    use super::*;
    #[test]
    fn cancellation_during_last_inference_cannot_publish_success() {
        let cancelled = std::cell::Cell::new(false);
        let mut calls = 0;
        let report = run(
            super::tests::report(),
            |_| {
                calls += 1;
                if calls == 10 {
                    cancelled.set(true);
                }
                Outcome {
                    result: Ok("valid".into()),
                    initializations: 6,
                    inferences: calls * 2,
                }
            },
            &|| cancelled.get(),
        );
        assert_eq!(calls, 10);
        assert!(report.cancelled && !report.passed && !report.diagnostics.runtime.load_verified);
    }
}

#[cfg(all(test, feature = "onnx", target_os = "macos"))]
mod acceleration_probe {
    use super::*;
    #[test]
    #[ignore = "Exercises all six installed models with Apple Core ML, including two-pass session reuse"]
    fn all_models_coreml_runtime_check() -> Result<()> {
        use crate::core::processing::{Backend, Control, RunConfig};
        let mut engine = Engine::new()?;
        let control = Control::default();
        engine.models = Models::with_processing(control.clone());
        engine.models.configure(RunConfig {
            threads: 1,
            backend: Backend::CoreMl,
        });
        let mut initializations = 0;
        for pass in 1..=2 {
            for check in CHECKS {
                let start = Instant::now();
                let outcome = engine.check(check, &|| false);
                println!(
                    "{}",
                    serde_json::json!({"pass":pass,"check":check.name(),"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"result":format!("{:?}",outcome.result),"providers":format!("{:?}",control.status())})
                );
                outcome.result?;
            }
            if pass == 1 {
                initializations = engine.models.initializations;
            } else {
                ensure!(
                    engine.models.initializations == initializations,
                    "Repeated native model initialization"
                );
            }
        }
        ensure!(
            control.status().len() == 6,
            "Expected all six shared model sessions"
        );
        Ok(())
    }
}
