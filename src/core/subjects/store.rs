//! Shared model evidence, keyed by source fingerprint and model recipe.
use super::{DenseEmbedding, Evidence, Pair};
use crate::core::cache_record as codec;
use crate::core::faces::Faces;
use anyhow::Result;
use codec::Kind;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Features {
    pub dense: DenseEmbedding,
    pub faces: Option<Faces>,
    pub identities: Option<Vec<Evidence>>,
}
impl Features {
    fn valid(&self) -> bool {
        let vector = |v: &[f32]| v.len() == 384 && v.iter().all(|v| v.is_finite());
        let evidence = |e: &Evidence| {
            vector(&e.descriptor) && e.bbox.iter().all(|v| v.is_finite()) && e.area.is_finite()
        };
        vector(&self.dense.global)
            && self.dense.regions.len() == 16
            && self.dense.regions.iter().all(evidence)
            && self
                .identities
                .as_ref()
                .is_none_or(|v| v.len() <= 10 && v.iter().all(evidence))
            && self
                .faces
                .as_ref()
                .is_none_or(|f| f.count == f.geometry.len() && f.count <= 10)
            && self
                .identities
                .as_ref()
                .is_none_or(|v| self.faces.as_ref().is_some_and(|f| f.count == v.len()))
    }
}
/// Immutable per-key snapshots in a temporary working set protect an active job
/// from persistent-cache trimming. The last owner releases the working set.
pub struct Store {
    root: PathBuf,
    working: tempfile::TempDir,
    persistent: AtomicBool,
}
impl Store {
    pub fn new(root: PathBuf, persistent: bool) -> Result<Self> {
        Ok(Self {
            root,
            working: tempfile::tempdir()?,
            persistent: AtomicBool::new(persistent),
        })
    }
    pub fn key(source: &str, models: &str) -> String {
        blake3::hash(format!("subject-features-v1:{source}:{models}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn persistent(&self, enabled: bool) {
        self.persistent.store(enabled, Ordering::Release);
    }
    fn path(&self, key: &str) -> Result<PathBuf> {
        anyhow::ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid evidence key"
        );
        Ok(self.working.path().join(format!("{key}.bin")))
    }
    fn read<T: serde::de::DeserializeOwned + Serialize>(
        &self,
        key: &str,
        kind: Kind,
        valid: impl Fn(&T) -> bool,
    ) -> Option<T> {
        let working = self.path(key).ok()?;
        let persistent = self.persistent.load(Ordering::Acquire);
        let disk = self.root.join(format!("{key}.bin"));
        // A corrupt current record is a miss, never a fallback to an older value.
        let binary = if working.try_exists().ok()? {
            Some(working.clone())
        } else if persistent && disk.try_exists().ok()? {
            Some(disk)
        } else {
            None
        };
        if let Some(path) = binary {
            let bytes = codec::read(&path)?;
            let value = codec::decode(kind, &bytes)?;
            if !valid(&value) {
                return None;
            }
            if path != working {
                codec::write(&working, &bytes).ok()?;
            }
            return Some(value);
        }
        // Read existing JSON once, validate it, then pin the new representation.
        // Legacy files remain regenerable cache entries managed by the same budget.
        let bytes = codec::read(&working.with_extension("json")).or_else(|| {
            persistent
                .then(|| codec::read(&self.root.join(format!("{key}.json"))))
                .flatten()
        })?;
        let value = serde_json::from_slice(&bytes).ok()?;
        if !valid(&value) {
            return None;
        }
        self.write(key, kind, &value).ok()?;
        Some(value)
    }
    pub fn get(&self, key: &str) -> Option<Features> {
        self.read(key, Kind::Subjects, Features::valid)
    }
    pub fn measurements_key(source: &str) -> String {
        blake3::hash(format!("measurements-v1-neutral1024-area640-luma-tiles:{source}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn measurements(&self, key: &str) -> Option<crate::core::analysis::Measurements> {
        self.read(
            key,
            Kind::Measurements,
            crate::core::analysis::Measurements::valid,
        )
    }
    pub fn put_measurements(
        &self,
        key: &str,
        value: &crate::core::analysis::Measurements,
    ) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid image measurements");
        self.write(key, Kind::Measurements, value)
    }
    pub fn rotation_key(source: &str, model: &str) -> String {
        blake3::hash(
            format!(
                "rotation-v2-working2048-triangle416-crop384-imagenet-consensus021:{source}:{model}"
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string()
    }
    pub fn rotation(&self, key: &str) -> Option<crate::core::models::RotationSuggestion> {
        self.read(
            key,
            Kind::Rotation,
            crate::core::models::RotationSuggestion::valid,
        )
    }
    pub fn put_rotation(
        &self,
        key: &str,
        value: &crate::core::models::RotationSuggestion,
    ) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid rotation evidence");
        self.write(key, Kind::Rotation, value)
    }
    pub fn aesthetic_key(source: &str, model: &str) -> String {
        blake3::hash(format!("aesthetic-v1-neutral1024:{source}:{model}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn aesthetic(&self, key: &str) -> Option<f32> {
        self.read(key, Kind::Aesthetic, |v| (0.0..=1.0).contains(v))
    }
    pub fn put_aesthetic(&self, key: &str, value: f32) -> Result<()> {
        anyhow::ensure!((0.0..=1.0).contains(&value), "Invalid aesthetic score");
        self.write(key, Kind::Aesthetic, &value)
    }
    pub fn pair_key(a: &str, b: &str, ta: Option<i64>, tb: Option<i64>) -> String {
        blake3::hash(format!("subject-pair-v1:{a}:{b}:{ta:?}:{tb:?}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn pair(&self, key: &str) -> Option<Pair> {
        self.read(key, Kind::Pair, |p: &Pair| {
            [
                p.subject_similarity,
                p.scene_similarity,
                p.confidence,
                p.geometry.inlier_ratio,
                p.geometry.coverage_a,
                p.geometry.coverage_b,
            ]
            .iter()
            .all(|v| v.is_finite())
        })
    }
    pub fn regional_key(subject_key: &str) -> String {
        blake3::hash(format!("general-regions-v2-batched-five-crops:{subject_key}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn regional(&self, key: &str) -> Option<crate::core::regional::Embedding> {
        self.read(
            key,
            Kind::Regional,
            crate::core::regional::Embedding::valid_model_output,
        )
    }
    pub fn put_regional(&self, key: &str, value: &crate::core::regional::Embedding) -> Result<()> {
        anyhow::ensure!(
            value.valid_model_output(),
            "Invalid general regional embeddings"
        );
        self.write(key, Kind::Regional, value)
    }
    pub fn face_key(source: &str, models: &str) -> String {
        blake3::hash(format!("verified-faces-v1:{source}:{models}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn faces(&self, key: &str) -> Option<Faces> {
        self.read(key, Kind::Faces, Faces::complete)
    }
    pub fn put_faces(&self, key: &str, value: &Faces) -> Result<()> {
        anyhow::ensure!(value.complete(), "Invalid verified face evidence");
        self.write(key, Kind::Faces, value)
    }
    pub fn recognition_key(face_key: &str, models: &str) -> String {
        blake3::hash(
            format!("face-recognition-v3-sface112-all-main60:{face_key}:{models}").as_bytes(),
        )
        .to_hex()
        .to_string()
    }
    pub fn recognition(&self, key: &str) -> Option<crate::core::face_groups::Recognition> {
        self.read(
            key,
            Kind::Recognition,
            crate::core::face_groups::Recognition::valid,
        )
    }
    pub fn put_recognition(
        &self,
        key: &str,
        value: &crate::core::face_groups::Recognition,
    ) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid face recognition");
        self.write(key, Kind::Recognition, value)
    }
    pub fn duplicate_key(a: &str, b: &str) -> String {
        blake3::hash(format!("duplicate-pair-v1-proxy2048-rgb15:{a}:{b}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn duplicate(&self, key: &str) -> Option<crate::core::near_duplicates::Assessment> {
        self.read(key, Kind::Duplicate, |_| true)
    }
    pub fn put_duplicate(
        &self,
        key: &str,
        value: &crate::core::near_duplicates::Assessment,
    ) -> Result<()> {
        self.write(key, Kind::Duplicate, value)
    }
    pub fn put_pair(&self, key: &str, value: &Pair) -> Result<()> {
        self.write(key, Kind::Pair, value)
    }
    pub fn put(&self, key: &str, value: &Features) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid extracted subject evidence");
        self.write(key, Kind::Subjects, value)
    }
    fn write(&self, key: &str, kind: Kind, value: &impl Serialize) -> Result<()> {
        let bytes = codec::encode(kind, value)?;
        codec::write(&self.path(key)?, &bytes)?;
        if self.persistent.load(Ordering::Acquire) {
            // Persistence is optional; the pinned working set remains authoritative.
            let _ = std::fs::create_dir_all(&self.root);
            let _ = codec::write(&self.root.join(format!("{key}.bin")), &bytes);
        }
        Ok(())
    }
}
