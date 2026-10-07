//! Rotation evidence resolution. Review decisions never enter this cache.
use crate::core::{
    model::{Fingerprint, Media},
    models::{Models, RotationSuggestion},
    pipeline::Pipeline,
    subjects::store::Store,
};
use anyhow::{Result, ensure};
use std::path::PathBuf;

/// Immutable model snapshot for one accepted run. The existing image actor owns
/// this object and injects its one pipeline, evidence store and model registry.
pub struct Detector {
    path: PathBuf,
    fingerprint: Fingerprint,
    signature: String,
}
impl Detector {
    pub fn new(path: PathBuf) -> Result<Self> {
        let fingerprint = Fingerprint::read(&path)?;
        ensure!(fingerprint.bytes > 0, "Rotation model is empty");
        let mut signature = blake3::Hasher::new();
        signature.update(path.as_os_str().as_encoded_bytes());
        signature.update(format!("{fingerprint:?}").as_bytes());
        Ok(Self {
            path,
            fingerprint,
            signature: signature.finalize().to_hex().to_string(),
        })
    }
    pub fn cache_key(&self, media: &Media) -> Result<String> {
        Ok(Store::rotation_key(&Pipeline::key(media)?, &self.signature))
    }
    fn model_unchanged(&self) -> Result<()> {
        ensure!(
            Fingerprint::read(&self.path)? == self.fingerprint,
            "Rotation model changed; restart detection"
        );
        Ok(())
    }
    pub fn detect(
        &self,
        media: &Media,
        pipeline: &mut Pipeline,
        models: &mut Models,
        store: &Store,
        cancelled: &(dyn Fn() -> bool + Sync),
    ) -> Result<RotationSuggestion> {
        ensure!(!cancelled(), "Rotation cancelled");
        self.model_unchanged()?;
        let source = Pipeline::key(media)?;
        let key = Store::rotation_key(&source, &self.signature);
        let cached = store.rotation(&key);
        // Resolve evidence before touching the image pipeline: a warm run needs
        // no metadata load, pixels, model initialization, or native inference.
        let result = if let Some(value) = cached {
            value
        } else {
            // Use the same neutral, bounded working image as browsing. Rotation
            // already resizes to its trained 416-square input; an intermediate
            // 1024-edge analysis resize changes fine detail and can flip a vote.
            // This reuses shared decoding, source orientation and cached pixels.
            let asset = pipeline.load_cancellable(media, cancelled)?;
            models.rotation_cancellable(&self.path, &asset, cancelled)?
        };
        ensure!(!cancelled(), "Rotation cancelled");
        self.model_unchanged()?;
        ensure!(
            Pipeline::key(media)? == source,
            "Photo changed during rotation detection"
        );
        if cached.is_none() {
            store.put_rotation(&key, &result)?;
        }
        Ok(result)
    }
}
