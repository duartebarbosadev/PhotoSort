use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaId(pub u64);
impl MediaId {
    pub fn for_path(path: &Path) -> Self {
        let hash = blake3::hash(path.as_os_str().as_encoded_bytes());
        Self(u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap()))
    }
}

/// Apply verified identity changes simultaneously, including swaps and chains.
/// Detach original values before inserting targets that may also be old keys.
pub(crate) fn remap_media_ids<T>(
    values: &mut HashMap<MediaId, T>,
    changes: &[(MediaId, MediaId, Fingerprint)],
) {
    let moved: Vec<_> = changes
        .iter()
        .filter_map(|(old, new, _)| values.remove(old).map(|value| (*new, value)))
        .collect();
    values.extend(moved);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Photo,
    Raw,
    Video,
}
impl MediaKind {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" | "bmp" | "gif" | "heic" | "heif" => {
                Self::Photo
            }
            "arw" | "cr2" | "cr3" | "nef" | "nrw" | "dng" | "orf" | "raf" | "rw2" | "pef"
            | "srw" | "raw" | "ptx" | "cap" | "iiq" | "eip" | "fff" | "mef" | "mdc" | "mos"
            | "mrw" | "erf" | "kdc" | "dcs" | "dcr" | "x3f" | "rwl" => Self::Raw,
            "mp4" | "mov" | "m4v" | "avi" | "mkv" | "mpg" | "mpeg" | "wmv" | "webm" => Self::Video,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fingerprint {
    pub bytes: u64,
    pub modified_ns: u128,
    /// Identity from the same stat snapshot, preventing same-size/mtime replacement reuse.
    #[serde(default)]
    pub identity: [u64; 4],
}
impl Fingerprint {
    pub fn read(path: &Path) -> std::io::Result<Self> {
        let m = std::fs::symlink_metadata(path)?;
        if !m.is_file() {
            return Err(std::io::Error::other("Not a regular file"));
        }
        Self::from_metadata(path, &m)
    }
    pub(crate) fn read_directory(path: &Path) -> std::io::Result<Self> {
        let m = std::fs::symlink_metadata(path)?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(std::io::Error::other("Not a regular directory"));
        }
        Self::from_metadata(path, &m)
    }
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn from_metadata(path: &Path, m: &std::fs::Metadata) -> std::io::Result<Self> {
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            // Birth time stays stable while our no-overwrite move creates/removes hardlinks.
            let birth = m
                .created()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .unwrap_or_default();
            [
                m.dev(),
                m.ino(),
                birth.as_secs(),
                u64::from(birth.subsec_nanos()),
            ]
        };
        #[cfg(windows)]
        let identity = {
            use std::os::windows::fs::MetadataExt;
            // NTFS "tunnelling" hands a new file or folder the old creation
            // time when it reuses a name within seconds, so creation time
            // alone cannot tell a replacement apart. The volume serial number
            // and file index are the NTFS equivalent of device and inode.
            let info = winapi_util::file::information(winapi_util::Handle::from_path_any(path)?)?;
            [
                info.volume_serial_number(),
                info.file_index(),
                m.creation_time(),
                0,
            ]
        };
        #[cfg(not(any(unix, windows)))]
        let identity = [0; 4];
        Ok(Self {
            identity,
            bytes: m.len(),
            modified_ns: m
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        })
    }
}

/// Secondary EXIF values shown by the Details panel. Enumerations are stored as
/// their raw EXIF codes and formatted at display time, so both metadata backends
/// agree and the cached record stays small.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Details {
    pub make: Option<String>,
    pub software: Option<String>,
    pub focal_length_mm: Option<f32>,
    pub exposure_bias_ev: Option<f32>,
    pub flash: Option<u16>,
    pub white_balance: Option<u16>,
    pub metering: Option<u16>,
    pub exposure_mode: Option<u16>,
    pub scene: Option<u16>,
    pub color_space: Option<u16>,
    pub bit_depth: Option<u16>,
    #[serde(default)]
    pub video: Option<super::video_info::VideoInfo>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub width: u32,
    pub height: u32,
    #[serde(default, alias = "exif_fields")]
    pub metadata_fields: usize,
    #[serde(default)]
    pub embedded_metadata_writable: bool,
    pub camera: String,
    pub lens: String,
    pub captured: String,
    /// Capture and filesystem creation times in Unix milliseconds.
    #[serde(default)]
    pub captured_ms: Option<i64>,
    #[serde(default)]
    pub created_ms: Option<i64>,
    #[serde(default)]
    pub grouping_date: Option<String>,
    pub exposure: String,
    pub aperture: String,
    pub iso: String,
    pub orientation: u8,
    /// Native container transform before an authoritative sidecar override.
    #[serde(default)]
    pub container_orientation: Option<u8>,
    pub rating: u8,
    pub gps: Option<(f64, f64)>,
    #[serde(default)]
    pub location: Option<super::geography::Location>,
    #[serde(default)]
    pub details: Details,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Media {
    pub id: MediaId,
    pub path: PathBuf,
    pub kind: MediaKind,
    pub fingerprint: Fingerprint,
}
impl Media {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Decision {
    pub rejected: bool,
    pub rating: Option<u8>,
    pub turns: u8,
}
impl Decision {
    pub fn is_empty(&self) -> bool {
        !self.rejected && self.rating.is_none() && self.turns == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Step {
    #[default]
    Organize,
    EasyDelete,
    Rotation,
    PickBest,
    Cull,
}
impl Step {
    pub const ALL: [Self; 5] = [
        Self::Organize,
        Self::EasyDelete,
        Self::Rotation,
        Self::PickBest,
        Self::Cull,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap()
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Organize => "Organize",
            Self::EasyDelete => "Easy Delete",
            Self::Rotation => "Fix Rotation",
            Self::PickBest => "Pick Best",
            Self::Cull => "Cull",
        }
    }
    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Organize => "Give every story a home.",
            Self::EasyDelete => "Less noise. More moments.",
            Self::Rotation => "A fresh perspective.",
            Self::PickBest => "Find the frame that feels right.",
            Self::Cull => "Keep what matters.",
        }
    }
}

/// Publish review marks while preserving independently staged metadata changes.
pub(crate) fn update_trash(
    decisions: &mut std::collections::HashMap<MediaId, Decision>,
    values: impl IntoIterator<Item = (MediaId, bool)>,
) {
    for (id, rejected) in values {
        decisions.entry(id).or_default().rejected = rejected;
    }
}
