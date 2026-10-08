//! Read-only startup diagnostics using the same model inventory as the UI.
use super::{
    model_assets::{Health, Inspector, Snapshot},
    models, session,
};
use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};
mod runtime_check;
pub use runtime_check::{RuntimeReport, check_runtime, check_runtime_with_backend};

#[derive(Serialize)]
pub struct Capabilities {
    pub raw: bool,
    pub heif: bool,
    pub embedded_metadata: bool,
    pub neural_inference: bool,
    pub model_downloads: bool,
    pub native_video: bool,
}
impl Default for Capabilities {
    fn default() -> Self {
        Self {
            raw: cfg!(feature = "raw"),
            heif: cfg!(feature = "heif"),
            embedded_metadata: cfg!(feature = "metadata-native"),
            neural_inference: cfg!(feature = "onnx"),
            model_downloads: cfg!(feature = "model-downloads"),
            native_video: super::playback::supported(),
        }
    }
}
#[derive(Serialize)]
pub struct Model {
    pub name: &'static str,
    pub selected: bool,
    pub status: &'static str,
    #[serde(serialize_with = "display_optional_path")]
    pub path: Option<PathBuf>,
    pub detail: Option<String>,
}
#[derive(Serialize)]
pub struct Runtime {
    #[serde(serialize_with = "display_optional_path")]
    pub path: Option<PathBuf>,
    pub status: &'static str,
    pub load_verified: bool,
}
#[derive(Serialize)]
pub struct Override {
    #[serde(serialize_with = "display_path")]
    pub path: PathBuf,
    pub regular_nonempty_file: bool,
    pub model_compatibility_verified: bool,
}
#[derive(Serialize)]
pub struct Report {
    pub schema_version: u8,
    pub version: &'static str,
    pub platform: &'static str,
    pub architecture: &'static str,
    pub capabilities: Capabilities,
    #[serde(serialize_with = "display_path")]
    pub data_directory: PathBuf,
    #[serde(serialize_with = "display_path")]
    pub cache_directory: PathBuf,
    pub models: Vec<Model>,
    pub runtime: Runtime,
    pub orientation_override: Option<Override>,
    pub files_ready: bool,
    pub note: &'static str,
}
pub fn collect() -> Result<Report> {
    collect_cancellable(&|| false)
}
fn collect_cancellable(cancelled: &impl Fn() -> bool) -> Result<Report> {
    let snapshot = Inspector::default().inspect(&models::resource_roots(), cancelled)?;
    Ok(from_snapshot(snapshot, Capabilities::default()))
}
fn from_snapshot(snapshot: Snapshot, capabilities: Capabilities) -> Report {
    let has_override = snapshot.orientation_override.is_some();
    let models: Vec<_> = snapshot
        .rows
        .into_iter()
        .map(|row| {
            let (status, path, detail) = match row.health {
                Health::Missing => ("missing", None, None),
                Health::Verified(path) => ("verified", Some(path), None),
                Health::Invalid(path, error) => ("invalid", Some(path), Some(error)),
            };
            Model {
                name: row.id.asset().name,
                selected: row.id != super::model_assets::Id::Rotation || !has_override,
                status,
                path,
                detail,
            }
        })
        .collect();
    let files_ready = !capabilities.neural_inference
        || (snapshot.runtime.is_some()
            && models.len() == super::model_assets::ALL.len()
            && models
                .iter()
                .filter(|model| model.selected)
                .all(|model| model.status == "verified")
            && snapshot
                .orientation_override
                .as_ref()
                .is_none_or(|(_, present)| *present));
    let runtime = Runtime {
        status: if !capabilities.neural_inference {
            "not_enabled"
        } else if snapshot.runtime.is_some() {
            "present_unverified"
        } else {
            "missing"
        },
        path: snapshot.runtime,
        load_verified: false,
    };
    Report {
        schema_version: 1,
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        architecture: std::env::consts::ARCH,
        capabilities,
        data_directory: session::data_dir(),
        cache_directory: session::cache_dir(),
        models,
        runtime,
        files_ready,
        orientation_override: snapshot
            .orientation_override
            .map(|(path, regular_nonempty_file)| Override {
                path,
                regular_nonempty_file,
                model_compatibility_verified: false,
            }),
        note: "Read-only file verification. No graphics initialization, runtime loading or inference was attempted. File readiness does not establish ABI, model compatibility or photographic accuracy. Paths are display text; non-UTF-8 bytes are replaced in JSON.",
    }
}
fn display_path<S: serde::Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}
fn display_optional_path<S: serde::Serializer>(
    path: &Option<PathBuf>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    path.as_deref()
        .map(|p| p.to_string_lossy())
        .serialize(serializer)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model_assets::{ALL, Row};
    pub(super) fn snapshot() -> Snapshot {
        Snapshot {
            rows: ALL
                .iter()
                .map(|a| Row {
                    id: a.id,
                    health: Health::Verified(a.name.into()),
                    bundled: None,
                    managed: None,
                })
                .collect(),
            runtime: Some("runtime".into()),
            orientation_override: None,
            hash_reads: 6,
        }
    }
    #[test]
    fn file_readiness_never_claims_runtime_or_custom_model_load_verification() {
        let mut snapshot = snapshot();
        snapshot.orientation_override = Some(("custom.onnx".into(), true));
        let report = from_snapshot(
            snapshot,
            Capabilities {
                neural_inference: true,
                ..Default::default()
            },
        );
        assert!(report.files_ready);
        assert!(!report.runtime.load_verified);
        assert_eq!(report.runtime.status, "present_unverified");
        assert!(
            !report
                .orientation_override
                .as_ref()
                .unwrap()
                .model_compatibility_verified
        );
        assert_eq!(serde_json::to_value(report).unwrap()["schema_version"], 1);
    }
    #[test]
    fn enabled_ai_requires_complete_verified_inventory_and_a_runtime_file() {
        for failure in 0..4 {
            let mut snapshot = snapshot();
            match failure {
                0 => snapshot.runtime = None,
                1 => snapshot.rows[0].health = Health::Missing,
                2 => snapshot.rows[0].health = Health::Invalid("bad".into(), "checksum".into()),
                _ => snapshot.orientation_override = Some(("missing".into(), false)),
            }
            assert!(
                !from_snapshot(
                    snapshot,
                    Capabilities {
                        neural_inference: true,
                        ..Default::default()
                    }
                )
                .files_ready
            );
        }
    }
    #[test]
    fn minimal_build_reports_unavailable_ai_without_requiring_model_files() {
        let snapshot = Snapshot {
            rows: Vec::new(),
            runtime: None,
            orientation_override: None,
            hash_reads: 0,
        };
        let report = from_snapshot(
            snapshot,
            Capabilities {
                neural_inference: false,
                ..Default::default()
            },
        );
        assert!(report.files_ready);
        assert_eq!(report.runtime.status, "not_enabled");
        assert!(!report.capabilities.neural_inference);
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;
    #[test]
    fn explicit_orientation_override_does_not_require_an_unused_bundled_orientation() {
        let mut snapshot = super::tests::snapshot();
        snapshot
            .rows
            .iter_mut()
            .find(|row| row.id == super::super::model_assets::Id::Rotation)
            .unwrap()
            .health = Health::Missing;
        snapshot.orientation_override = Some(("custom.onnx".into(), true));
        let report = from_snapshot(
            snapshot,
            Capabilities {
                neural_inference: true,
                ..Default::default()
            },
        );
        assert!(report.files_ready);
        assert!(
            !report
                .models
                .iter()
                .find(|m| m.name == "orientation.onnx")
                .unwrap()
                .selected
        );
    }
    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_still_produce_json_without_changing_filesystem_paths() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};
        let path = PathBuf::from(OsString::from_vec(b"/models/\xff.onnx".to_vec()));
        let mut snapshot = super::tests::snapshot();
        snapshot.rows[0].health = Health::Verified(path.clone());
        snapshot.runtime = Some(path.clone());
        snapshot.orientation_override = Some((path.clone(), true));
        let report = from_snapshot(snapshot, Capabilities::default());
        assert_eq!(report.runtime.path.as_ref(), Some(&path));
        let json = serde_json::to_value(&report).unwrap();
        assert!(
            json["runtime"]["path"]
                .as_str()
                .unwrap()
                .contains('\u{fffd}')
        );
        assert_eq!(
            json["models"][0]["path"],
            json["orientation_override"]["path"]
        );
    }
}
