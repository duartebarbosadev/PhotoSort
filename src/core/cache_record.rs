//! Versioned, typed binary cache envelope. Schema changes require a version bump.
use anyhow::{Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::Path,
};
const MAGIC: &[u8; 8] = b"PSCE\0\0\0\x01";
const HEADER: usize = 8 + 1 + 32;
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Kind {
    Subjects = 1,
    Measurements = 2,
    Aesthetic = 3,
    Pair = 4,
    Regional = 5,
    Faces = 6,
    Recognition = 7,
    Duplicate = 8,
    Rotation = 9,
    Metadata = 10,
    #[cfg(any(test, all(feature = "onnx", target_os = "macos")))]
    Compiled = 11,
}
pub(crate) fn encode(kind: Kind, value: &impl Serialize) -> Result<Vec<u8>> {
    let payload = postcard::to_allocvec(value)?;
    ensure!(
        payload.len() <= MAX_BYTES - HEADER,
        "Evidence exceeds cache record limit"
    );
    let mut bytes = Vec::with_capacity(HEADER + payload.len());
    bytes.extend_from_slice(MAGIC);
    bytes.push(kind as u8);
    bytes.extend_from_slice(blake3::hash(&payload).as_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}
pub(crate) fn decode<T: DeserializeOwned>(kind: Kind, bytes: &[u8]) -> Option<T> {
    if bytes.len() < HEADER
        || bytes.len() > MAX_BYTES
        || &bytes[..8] != MAGIC
        || bytes[8] != kind as u8
    {
        return None;
    }
    let payload = &bytes[HEADER..];
    if blake3::hash(payload).as_bytes() != &bytes[9..HEADER] {
        return None;
    }
    let (value, rest) = postcard::take_from_bytes(payload).ok()?;
    rest.is_empty().then_some(value)
}
/// Regenerable records need atomic visibility, not a physical flush per photo.
/// An interrupted or lost cache write is a miss; user edits and recovery journals
/// continue to use metadata::atomic_write with sync_all.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() <= MAX_BYTES, "Cache record exceeds size limit");
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing cache directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".photosort-cache-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp.write_all(bytes)?;
    let mut attempt = 0;
    loop {
        match temp.persist(path) {
            Ok(_) => return Ok(()),
            Err(error) if replace_in_progress(&error.error, attempt) => {
                temp = error.file;
                attempt += 1;
            }
            Err(error) => return Err(error.error.into()),
        }
    }
}
/// Windows refuses to replace or open a file for a moment while another
/// thread replaces it ("Access is denied"). Workers race to publish the same
/// regenerable record, so wait briefly and retry there; elsewhere, and after
/// about a third of a second, the error stands.
fn replace_in_progress(error: &std::io::Error, attempt: u32) -> bool {
    if cfg!(windows) && error.kind() == std::io::ErrorKind::PermissionDenied && attempt < 24 {
        std::thread::sleep(std::time::Duration::from_millis(u64::from(attempt) + 1));
        true
    } else {
        false
    }
}
pub(crate) fn read(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_BYTES as u64
    {
        return None;
    }
    let mut bytes = Vec::new();
    let mut attempt = 0;
    let file = loop {
        match std::fs::File::open(path) {
            Ok(file) => break file,
            Err(error) if replace_in_progress(&error, attempt) => attempt += 1,
            Err(_) => return None,
        }
    };
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= MAX_BYTES).then_some(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codec_preserves_float_bits_and_rejects_corruption_type_version_and_trailing_data() {
        let values = vec![
            0.0f32,
            -0.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            f32::MAX,
            -f32::MAX,
            0.12345679,
        ];
        let bytes = encode(Kind::Regional, &values).unwrap();
        let actual: Vec<f32> = decode(Kind::Regional, &bytes).unwrap();
        assert_eq!(
            actual.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            values.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
        assert!(decode::<Vec<f32>>(Kind::Subjects, &bytes).is_none());
        for index in [0, 7, 9, bytes.len() - 1] {
            let mut bad = bytes.clone();
            bad[index] ^= 1;
            assert!(decode::<Vec<f32>>(Kind::Regional, &bad).is_none());
        }
        for length in [0, 8, 40, bytes.len() - 1] {
            assert!(decode::<Vec<f32>>(Kind::Regional, &bytes[..length]).is_none());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        let hash = blake3::hash(&trailing[HEADER..]);
        trailing[9..HEADER].copy_from_slice(hash.as_bytes());
        assert!(decode::<Vec<f32>>(Kind::Regional, &trailing).is_none());
    }
    #[test]
    fn atomic_cache_writers_publish_complete_records_and_preserve_failed_destinations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("record.bin");
        std::thread::scope(|scope| {
            for value in 0..16u32 {
                let path = &path;
                scope.spawn(move || {
                    let bytes = encode(Kind::Measurements, &vec![value; 4096]).unwrap();
                    write(path, &bytes).unwrap();
                    let actual: Vec<u32> =
                        decode(Kind::Measurements, &read(path).unwrap()).unwrap();
                    assert!(actual.len() == 4096 && actual.iter().all(|x| *x == actual[0]));
                });
            }
        });
        let blocked = dir.path().join("directory");
        std::fs::create_dir(&blocked).unwrap();
        std::fs::write(blocked.join("keep"), b"foreign").unwrap();
        assert!(write(&blocked, b"record").is_err());
        assert_eq!(std::fs::read(blocked.join("keep")).unwrap(), b"foreign");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            2,
            "No partial temporary files"
        );
    }
    #[test]
    fn oversized_or_non_file_cache_entries_are_misses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_BYTES as u64 + 1).unwrap();
        assert!(read(&path).is_none());
        assert!(read(dir.path()).is_none());
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(read(&link).is_none());
        }
    }
}
