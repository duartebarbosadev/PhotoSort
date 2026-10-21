//! One catalog and verification/install policy for every native model.
use super::{model::Fingerprint, models};
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};
mod worker;
pub use worker::{Progress, State, Work, Worker};
#[cfg(feature = "model-downloads")]
mod download;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    Scene,
    Aesthetic,
    Detector,
    Landmarks,
    Blendshapes,
    People,
    Rotation,
}
#[derive(Clone, Copy)]
pub struct Asset {
    pub id: Id,
    pub name: &'static str,
    pub title: &'static str,
    pub purpose: &'static str,
    pub bytes: u64,
    /// Direct download of the pinned file.
    pub url: &'static str,
    pub license: &'static str,
    /// Terms to read before downloading, shown beside the Download action.
    pub download_note: Option<&'static str>,
    /// The upstream model or project page (resources/ASSETS.md).
    pub page: &'static str,
    /// Where the file comes from and what kind of model it is.
    pub origin: &'static str,
    /// What it does, in plain language.
    pub about: &'static str,
    /// Needed only by an optional mode or step (People folders, Fix
    /// Rotation) rather than by the analysis most steps share.
    pub optional: bool,
}
pub const ALL: [Asset; 7] = [
    Asset {
        id: Id::Scene,
        name: "dinov2-small.onnx",
        title: "Scene and subject similarity",
        purpose: "Visual/Mixed Organize, Easy Delete, Pick Best and Cull",
        bytes: 88_532_934,
        url: "https://huggingface.co/onnx-community/dinov2-small/resolve/8b1f705a3a7f6f062f6bdd21986c1583d3ef105d/onnx/model.onnx",
        license: "Apache-2.0",
        download_note: None,
        page: "https://huggingface.co/onnx-community/dinov2-small/tree/8b1f705a3a7f6f062f6bdd21986c1583d3ef105d",
        origin: "Meta DINOv2 small (ViT-S/14), ONNX export by onnx-community, pinned revision",
        about: "Recognises what each photo shows, so similar shots and bursts can be grouped.",
        optional: false,
    },
    Asset {
        id: Id::Aesthetic,
        name: "aesthetic.onnx",
        title: "Aesthetic scoring",
        purpose: "Pick Best",
        // Basic-level ORT folding of the pinned export (examples/fold_model.rs).
        bytes: 534_515_358,
        url: "https://github.com/duartebarbosadev/PhotoSort/releases/download/models-1/aesthetic.onnx",
        license: "AGPL-3.0 model card",
        download_note: Some(
            "This model is licensed under AGPL-3.0, unlike the others. Its licence and model card are on its Source page.",
        ),
        page: "https://huggingface.co/cafeai/cafe_aesthetic/tree/48a343764f786abc1ca8aaddfcd60a688a70da9b",
        origin: "cafe_aesthetic (BEiT image classifier), exported to ONNX and folded for this app",
        about: "Rates how appealing a photo looks, to rank the candidates in Pick Best.",
        optional: false,
    },
    Asset {
        id: Id::Detector,
        name: "face_detector.onnx",
        title: "Face detection",
        purpose: "People Organize, Easy Delete, Pick Best and Cull",
        bytes: 425_659,
        url: "https://github.com/duartebarbosadev/PhotoSort/releases/download/models-1/face_detector.onnx",
        license: "MediaPipe / Apache-2.0",
        download_note: None,
        page: "https://github.com/google-ai-edge/mediapipe",
        origin: "Google MediaPipe face detector from face_landmarker.task, converted to ONNX",
        about: "Finds the faces in each photo.",
        optional: false,
    },
    Asset {
        id: Id::Landmarks,
        name: "face_landmarks_detector.onnx",
        title: "Face landmarks",
        purpose: "People Organize, Easy Delete, Pick Best and Cull",
        // Pooling pads made explicit for Core ML (scripts/explicit_pads.py).
        bytes: 4_996_107,
        url: "https://github.com/duartebarbosadev/PhotoSort/releases/download/models-1/face_landmarks_detector.onnx",
        license: "MediaPipe / Apache-2.0",
        download_note: None,
        page: "https://storage.googleapis.com/mediapipe-assets/Model%20Card%20MediaPipe%20Face%20Mesh%20V2.pdf",
        origin: "Google MediaPipe Face Mesh V2, converted to ONNX",
        about: "Maps the points of each face, used for eyes and alignment.",
        optional: false,
    },
    Asset {
        id: Id::Blendshapes,
        name: "face_blendshapes.onnx",
        title: "Eye openness",
        purpose: "Closed-eye checks in Easy Delete, Pick Best and Cull",
        // MediaPipe face_blendshapes.tflite via scripts/convert_blendshapes.py.
        bytes: 1_884_171,
        url: "https://github.com/duartebarbosadev/PhotoSort/releases/download/models-1/face_blendshapes.onnx",
        license: "MediaPipe / Apache-2.0",
        download_note: None,
        page: "https://github.com/google-ai-edge/mediapipe",
        origin: "Google MediaPipe face blendshapes (52 expression scores), converted to ONNX",
        about: "Tells whether eyes are open or closed.",
        optional: false,
    },
    Asset {
        id: Id::People,
        name: "face_recognition_sface.onnx",
        title: "Person recognition",
        purpose: "People Organize",
        bytes: 38_696_353,
        url: "https://media.githubusercontent.com/media/opencv/opencv_zoo/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface/face_recognition_sface_2021dec.onnx",
        license: "Apache-2.0",
        download_note: None,
        page: "https://github.com/opencv/opencv_zoo/tree/47534e27c9851bb1128ccc0102f1145e27f23f98/models/face_recognition_sface",
        origin: "OpenCV Zoo SFace (MobileFaceNet), unmodified, pinned revision",
        about: "Recognises the same person across photos, for People folders in Organize.",
        optional: true,
    },
    Asset {
        id: Id::Rotation,
        name: "orientation.onnx",
        title: "Orientation detection",
        purpose: "Fix Rotation suggestions",
        bytes: 80_578_424,
        url: "https://github.com/duartebarbosadev/deep-image-orientation-detection/releases/download/v2/orientation_model_v2_0.9882.onnx",
        license: "MIT (upstream repository)",
        download_note: None,
        page: "https://github.com/duartebarbosadev/deep-image-orientation-detection/releases/tag/v2",
        origin: "deep-image-orientation-detection v2 release asset, unmodified",
        about: "Spots photos that are sideways or upside down.",
        optional: true,
    },
];
impl Id {
    pub fn asset(self) -> &'static Asset {
        ALL.iter().find(|a| a.id == self).unwrap()
    }
}
impl Asset {
    pub fn checksum(&self) -> &'static str {
        let path = format!("models/{}", self.name);
        include_str!("../../resources/SHA256SUMS")
            .lines()
            .find_map(|line| {
                let (sum, file) = line.split_once("  ")?;
                (file == path).then_some(sum)
            })
            .expect("Every managed asset has a pinned checksum")
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issue {
    pub missing: Vec<Id>,
    pub runtime_missing: bool,
    pub unsupported: bool,
}
impl Issue {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && !self.runtime_missing && !self.unsupported
    }
}
pub fn required(profile: super::jobs::AnalysisProfile) -> &'static [Id] {
    use super::jobs::AnalysisProfile::*;
    match profile {
        Technical => &[],
        Visual | Mixed => &[Id::Scene],
        Faces => &[Id::Detector, Id::Landmarks, Id::Blendshapes, Id::People],
        Duplicates | Subjects => &[Id::Scene, Id::Detector, Id::Landmarks, Id::Blendshapes],
        Quality => &[
            Id::Scene,
            Id::Aesthetic,
            Id::Detector,
            Id::Landmarks,
            Id::Blendshapes,
        ],
    }
}
fn runtime_file() -> Option<PathBuf> {
    models::runtime_path().filter(|path| usable_runtime_file(path))
}
fn usable_runtime_file(path: &Path) -> bool {
    Fingerprint::read(path).is_ok_and(|f| f.bytes > 0)
}
/// Cheap worker-side prerequisite check. Full hash verification is explicit in
/// model setup and on import; never hash hundreds of MB on a UI frame.
pub fn preflight(ids: &[Id]) -> Issue {
    if ids.is_empty() {
        return Issue::default();
    }
    Issue {
        unsupported: !cfg!(feature = "onnx"),
        runtime_missing: runtime_file().is_none(),
        missing: ids
            .iter()
            .copied()
            .filter(|id| {
                let path = if *id == Id::Rotation {
                    models::orientation_model()
                } else {
                    models::model_path(id.asset().name)
                };
                path.and_then(|p| Fingerprint::read(&p).ok())
                    .is_none_or(|f| {
                        f.bytes == 0 || (*id != Id::Rotation && f.bytes != id.asset().bytes)
                    })
            })
            .collect(),
    }
}
#[derive(Clone, Debug)]
pub enum Health {
    Missing,
    Verified(PathBuf),
    Invalid(PathBuf, String),
}
#[derive(Clone)]
pub struct Row {
    pub id: Id,
    pub health: Health,
    pub bundled: Option<PathBuf>,
    pub managed: Option<ManagedCopy>,
}
#[derive(Clone)]
pub struct Snapshot {
    pub orientation_override: Option<(PathBuf, bool)>,
    pub rows: Vec<Row>,
    pub runtime: Option<PathBuf>,
    pub hash_reads: usize,
}
#[derive(Default)]
pub struct Inspector {
    cache: HashMap<PathBuf, (Fingerprint, &'static str, bool)>,
    pub hash_reads: usize,
}
pub struct Installed {
    pub path: PathBuf,
    pub fingerprint: Fingerprint,
}
impl Inspector {
    fn remember_install(&mut self, asset: &Asset, installed: &Installed) {
        self.cache.insert(
            installed.path.clone(),
            (installed.fingerprint, asset.checksum(), true),
        );
    }
    pub fn verify(
        &mut self,
        asset: &Asset,
        path: &Path,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool> {
        self.verify_expected(path, asset.bytes, asset.checksum(), cancelled)
    }
    fn verify_expected(
        &mut self,
        path: &Path,
        bytes: u64,
        checksum: &'static str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<bool> {
        let before = Fingerprint::read(path)?;
        if before.bytes != bytes {
            return Ok(false);
        }
        if let Some((fp, expected, valid)) = self.cache.get(path)
            && *fp == before
            && *expected == checksum
        {
            return Ok(*valid);
        }
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        let mut buf = [0; 64 * 1024];
        loop {
            ensure!(!cancelled(), "Cancelled");
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        ensure!(
            Fingerprint::read(path)? == before,
            "Model changed while verifying"
        );
        let valid = hex(&hash.finalize()) == checksum;
        self.hash_reads += 1;
        self.cache.insert(path.into(), (before, checksum, valid));
        Ok(valid)
    }
    pub fn inspect(
        &mut self,
        roots: &[PathBuf],
        cancelled: &impl Fn() -> bool,
    ) -> Result<Snapshot> {
        let mut rows = Vec::new();
        for asset in ALL {
            ensure!(!cancelled(), "Cancelled");
            let candidates: Vec<_> = roots
                .iter()
                .map(|root| root.join("models").join(asset.name))
                .collect();
            let current = candidates.iter().find(|p| p.symlink_metadata().is_ok());
            let health = match current {
                None => Health::Missing,
                Some(path) => match self.verify(&asset, path, cancelled) {
                    Ok(true) => Health::Verified(path.clone()),
                    Ok(false) => Health::Invalid(
                        path.clone(),
                        "Size or SHA-256 differs from this build’s model".into(),
                    ),
                    Err(e) => {
                        ensure!(!cancelled(), "Cancelled");
                        Health::Invalid(path.clone(), e.to_string())
                    }
                },
            };
            let managed = roots
                .first()
                .and_then(|root| review_managed(root, asset.id).ok().flatten());
            let bundled = candidates.into_iter().skip(1).find(|p| p.is_file());
            rows.push(Row {
                id: asset.id,
                health,
                bundled,
                managed,
            });
        }
        Ok(Snapshot {
            orientation_override: std::env::var_os("PHOTOSORT_ROTATION_MODEL")
                .map(PathBuf::from)
                .map(|p| {
                    let valid = Fingerprint::read(&p).is_ok_and(|fp| fp.bytes > 0);
                    (p, valid)
                }),
            rows,
            runtime: runtime_file(),
            hash_reads: self.hash_reads,
        })
    }
}
#[derive(Debug, Default)]
pub struct ClearReport {
    pub removed: Vec<PathBuf>,
    pub bytes: u64,
    pub errors: Vec<String>,
    pub cancelled: bool,
}
/// An opaque review of one catalog-owned file. It cannot name arbitrary files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedCopy {
    id: Id,
    path: PathBuf,
    fingerprint: Fingerprint,
    directory: PathBuf,
}
impl ManagedCopy {
    pub fn id(&self) -> Id {
        self.id
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn bytes(&self) -> u64 {
        self.fingerprint.bytes
    }
}
fn managed_directory(root: &Path) -> Result<Option<PathBuf>> {
    let directory = root.join("models");
    for path in [root, directory.as_path()] {
        match path.symlink_metadata() {
            Ok(meta) => ensure!(
                meta.is_dir() && !meta.file_type().is_symlink(),
                "Managed model directory is not a regular directory: {}",
                path.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Some(directory.canonicalize()?))
}
pub fn review_managed(root: &Path, id: Id) -> Result<Option<ManagedCopy>> {
    let Some(directory) = managed_directory(root)? else {
        return Ok(None);
    };
    let path = root.join("models").join(id.asset().name);
    match path.symlink_metadata() {
        Ok(_) => Ok(Some(ManagedCopy {
            id,
            fingerprint: Fingerprint::read(&path)?,
            path,
            directory,
        })),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
/// Shared CLI/UI removal boundary. Recheck the exact review before unlinking;
/// bundled resources, overrides, unknown names and links are never candidates.
pub fn remove_reviewed(
    root: &Path,
    reviewed: &[ManagedCopy],
    cancelled: &impl Fn() -> bool,
) -> Result<ClearReport> {
    let mut report = ClearReport::default();
    for copy in reviewed {
        if cancelled() {
            report.cancelled = true;
            break;
        }
        let result = (|| -> Result<()> {
            ensure!(
                copy.path == root.join("models").join(copy.id.asset().name),
                "Model review belongs to another directory"
            );
            ensure!(
                managed_directory(root)?.as_ref() == Some(&copy.directory),
                "Managed model directory changed; refresh and review again"
            );
            ensure!(
                Fingerprint::read(&copy.path)? == copy.fingerprint,
                "Model changed since review; refresh and review again"
            );
            std::fs::remove_file(&copy.path)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                report.bytes += copy.bytes();
                report.removed.push(copy.path.clone());
            }
            Err(error) => report
                .errors
                .push(format!("{}: {error:#}", copy.path.display())),
        }
    }
    Ok(report)
}
/// Clear only known managed copies, with an all-files preflight before removal.
pub fn clear_managed(root: &Path, cancelled: &impl Fn() -> bool) -> Result<ClearReport> {
    ensure!(!cancelled(), "Cancelled");
    if managed_directory(root)?.is_none() {
        return Ok(ClearReport::default());
    }
    let mut reviewed = Vec::new();
    for asset in ALL {
        ensure!(!cancelled(), "Cancelled");
        if let Some(copy) = review_managed(root, asset.id)? {
            reviewed.push(copy);
        }
    }
    remove_reviewed(root, &reviewed, cancelled)
}

/// Streaming installation owns only a private temporary file until exact size
/// and content are verified. Existing installed files survive every failure.
struct Install {
    file: tempfile::NamedTempFile,
    destination: PathBuf,
    prior: Option<Fingerprint>,
    hash: Sha256,
    received: u64,
}
impl Install {
    fn new(root: &Path, name: &str) -> Result<Self> {
        let directory = root.join("models");
        std::fs::create_dir_all(&directory)?;
        ensure!(
            !root.symlink_metadata()?.file_type().is_symlink()
                && !directory.symlink_metadata()?.file_type().is_symlink(),
            "Model directory must not be a symbolic link"
        );
        let destination = directory.join(name);
        let prior = if destination.symlink_metadata().is_ok() {
            Some(Fingerprint::read(&destination)?)
        } else {
            None
        };
        Ok(Self {
            file: tempfile::NamedTempFile::new_in(directory)?,
            destination,
            prior,
            hash: Sha256::new(),
            received: 0,
        })
    }
    fn write(&mut self, chunk: &[u8], expected: u64, cancelled: &impl Fn() -> bool) -> Result<()> {
        ensure!(!cancelled(), "Cancelled");
        ensure!(
            self.received + chunk.len() as u64 <= expected,
            "Download is larger than the expected model"
        );
        self.file.write_all(chunk)?;
        self.hash.update(chunk);
        self.received += chunk.len() as u64;
        Ok(())
    }
    fn finish(
        mut self,
        bytes: u64,
        checksum: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Installed> {
        ensure!(!cancelled(), "Cancelled");
        ensure!(
            self.received == bytes,
            "Model is incomplete: {} of {bytes} bytes",
            self.received
        );
        ensure!(
            hex(&self.hash.finalize()) == checksum,
            "Model SHA-256 verification failed"
        );
        self.file.flush()?;
        self.file.as_file().sync_all()?;
        let now = if self.destination.symlink_metadata().is_ok() {
            Some(Fingerprint::read(&self.destination)?)
        } else {
            None
        };
        ensure!(
            now == self.prior,
            "Installed model changed during transfer; refresh and retry"
        );
        ensure!(!cancelled(), "Cancelled");
        let verified = Fingerprint::read(self.file.path())?;
        if self.prior.is_none() {
            self.file.persist_noclobber(&self.destination)?;
        } else {
            self.file.persist(&self.destination)?;
        }
        ensure!(
            Fingerprint::read(&self.destination)? == verified,
            "Installed model changed before verification completed"
        );
        Ok(Installed {
            path: self.destination,
            fingerprint: verified,
        })
    }
}
pub fn import(
    asset: &Asset,
    source: &Path,
    root: &Path,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64),
) -> Result<Installed> {
    let before = Fingerprint::read(source)?;
    ensure!(
        before.bytes == asset.bytes,
        "Choose this build’s {} ({} bytes)",
        asset.name,
        asset.bytes
    );
    let mut input = File::open(source).context("Open prepared ONNX model")?;
    let mut install = Install::new(root, asset.name)?;
    let mut buffer = [0; 64 * 1024];
    loop {
        ensure!(!cancelled(), "Cancelled");
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        install.write(&buffer[..n], asset.bytes, cancelled)?;
        progress(install.received);
    }
    ensure!(
        Fingerprint::read(source)? == before,
        "Source model changed during import"
    );
    install.finish(asset.bytes, asset.checksum(), cancelled)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut result, "{byte:02x}").unwrap();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    fn count(root: &Path) -> usize {
        std::fs::read_dir(root.join("models")).unwrap().count()
    }
    #[test]
    fn streamed_install_rejects_truncation_corruption_oversize_and_cancel_without_replacing_model()
    {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("models")).unwrap();
        let path = dir.path().join("models/a.onnx");
        std::fs::write(&path, b"old").unwrap();
        for bytes in [b"ab".as_slice(), b"xyz", b"abcd"] {
            let mut install = Install::new(dir.path(), "a.onnx").unwrap();
            let result = install
                .write(bytes, 3, &|| false)
                .and_then(|_| install.finish(3, ABC, &|| false));
            assert!(result.is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"old");
            assert_eq!(count(dir.path()), 1);
        }
        let mut install = Install::new(dir.path(), "a.onnx").unwrap();
        install.write(b"abc", 3, &|| false).unwrap();
        assert!(install.finish(3, ABC, &|| true).is_err());
        assert_eq!(count(dir.path()), 1);
        let mut install = Install::new(dir.path(), "a.onnx").unwrap();
        install.write(b"a", 3, &|| false).unwrap();
        install.write(b"bc", 3, &|| false).unwrap();
        let installed = install.finish(3, ABC, &|| false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"abc");
        assert_eq!(installed.fingerprint, Fingerprint::read(&path).unwrap());
        assert_eq!(count(dir.path()), 1);
    }
    #[test]
    fn install_rechecks_destination_and_refuses_symbolic_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut install = Install::new(dir.path(), "a.onnx").unwrap();
        install.write(b"abc", 3, &|| false).unwrap();
        let path = dir.path().join("models/a.onnx");
        std::fs::write(&path, b"new").unwrap();
        assert!(install.finish(3, ABC, &|| false).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        {
            let link = dir.path().join("models/link.onnx");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(Install::new(dir.path(), "link.onnx").is_err());
        }
    }
    #[test]
    fn verification_cache_reuses_identity_but_rechecks_replacements_and_expected_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.onnx");
        std::fs::write(&path, b"abc").unwrap();
        let mut inspector = Inspector::default();
        assert!(inspector.verify_expected(&path, 3, ABC, &|| false).unwrap());
        assert!(inspector.verify_expected(&path, 3, ABC, &|| false).unwrap());
        assert_eq!(inspector.hash_reads, 1);
        assert!(
            !inspector
                .verify_expected(&path, 3, "bad", &|| false)
                .unwrap()
        );
        assert_eq!(inspector.hash_reads, 2);
        let replacement = dir.path().join("replacement");
        std::fs::write(&replacement, b"xyz").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert!(!inspector.verify_expected(&path, 3, ABC, &|| false).unwrap());
        assert_eq!(inspector.hash_reads, 3);
        let checks = std::cell::Cell::new(0);
        std::fs::write(&path, vec![4; 512 * 1024]).unwrap();
        assert!(
            inspector
                .verify_expected(&path, 512 * 1024, ABC, &|| {
                    checks.set(checks.get() + 1);
                    checks.get() > 2
                })
                .is_err()
        );
        assert_eq!(
            inspector.hash_reads, 3,
            "Canceled hashes must not enter the cache"
        );
    }
    #[test]
    fn catalog_has_pinned_hashes_unique_filenames_and_one_dependency_map_for_every_workflow() {
        let mut names = std::collections::HashSet::new();
        for a in ALL {
            assert!(names.insert(a.name));
            assert_eq!(a.checksum().len(), 64);
            assert!(!a.name.contains('/'));
            assert!(a.url.starts_with("https://"));
        }
        // The converted exports have no upstream file; they are this
        // repository's release assets, published under their own names.
        for id in [Id::Aesthetic, Id::Detector, Id::Landmarks, Id::Blendshapes] {
            let asset = id.asset();
            assert_eq!(
                asset.url,
                format!(
                    "https://github.com/duartebarbosadev/PhotoSort/releases/download/models-1/{}",
                    asset.name
                )
            );
        }
        assert!(
            Id::Aesthetic
                .asset()
                .download_note
                .unwrap()
                .contains("AGPL-3.0")
        );
        use super::super::jobs::AnalysisProfile::*;
        assert!(required(Technical).is_empty());
        assert_eq!(required(Visual), required(Mixed));
        assert_eq!(required(Duplicates), required(Subjects));
        assert!(required(Quality).contains(&Id::Aesthetic));
        assert!(!required(Faces).contains(&Id::Scene));
        assert!(preflight(&[]).is_empty());
    }
    #[test]
    fn lazy_worker_does_no_initial_scan_and_reports_missing_models_without_inference() {
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        let dir = tempfile::tempdir().unwrap();
        let mut worker = Worker::new(vec![dir.path().into()], Arc::new(|| {}));
        assert!(!worker.state().running);
        assert!(worker.state().snapshot.is_none());
        worker.start(Work::Inspect).unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while worker.state().running {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        let state = worker.state();
        assert!(state.error.is_none());
        let snapshot = state.snapshot.unwrap();
        assert_eq!(snapshot.rows.len(), ALL.len());
        assert!(
            snapshot
                .rows
                .iter()
                .all(|r| matches!(r.health, Health::Missing))
        );
        assert_eq!(snapshot.hash_reads, 0);
        worker.shutdown();
        assert!(worker.start(Work::Inspect).is_err());
    }
    #[test]
    fn worker_shutdown_cancels_a_pending_large_import_and_removes_partial_files() {
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("managed");
        std::fs::create_dir_all(root.join("models")).unwrap();
        let source = dir.path().join("source.onnx");
        File::create(&source)
            .unwrap()
            .set_len(Id::Aesthetic.asset().bytes)
            .unwrap();
        let destination = root.join("models").join(Id::Aesthetic.asset().name);
        std::fs::write(&destination, b"prior model").unwrap();
        let mut worker = Worker::new(vec![root.clone()], Arc::new(|| {}));
        worker.start(Work::Import(Id::Aesthetic, source)).unwrap();
        assert!(
            worker.start(Work::Inspect).is_err(),
            "Bound the queue to one accepted operation"
        );
        let start = Instant::now();
        worker.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2));
        assert_eq!(std::fs::read(destination).unwrap(), b"prior model");
        assert_eq!(count(&root), 1);
    }
}

#[cfg(test)]
mod runtime_presence_tests {
    use super::*;
    #[test]
    fn runtime_presence_requires_a_nonempty_regular_file_but_does_not_claim_loadability() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("runtime");
        assert!(!usable_runtime_file(&path));
        std::fs::write(&path, []).unwrap();
        assert!(!usable_runtime_file(&path));
        std::fs::write(&path, b"not a dynamic library").unwrap();
        assert!(usable_runtime_file(&path));
        #[cfg(unix)]
        {
            let link = root.path().join("linked");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(!usable_runtime_file(&link));
        }
    }
}
