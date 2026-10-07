//! Bounded ISO-BMFF/QuickTime movie headers; never reads media payloads.
//! See https://developer.apple.com/documentation/quicktime-file-format/movie_header_atom
use chrono::{DateTime, Local};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    ops::Range,
    path::Path,
};

const UNIX_EPOCH: u64 = 2_082_844_800;
const MAX_ATOMS: usize = 4096;

#[derive(Default)]
pub(super) struct Header {
    pub created: Option<DateTime<Local>>,
    pub dimensions: Option<(u32, u32)>,
}

pub(super) fn read_path(path: &Path, cancelled: &impl Fn() -> bool) -> Option<Header> {
    let extension = path.extension()?.to_str()?;
    if !["mp4", "mov", "m4v", "3gp", "3g2"]
        .iter()
        .any(|ext| extension.eq_ignore_ascii_case(ext))
    {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let end = file.metadata().ok()?.len();
    read(&mut file, end, cancelled)
}

fn read(
    reader: &mut (impl Read + Seek),
    end: u64,
    cancelled: &impl Fn() -> bool,
) -> Option<Header> {
    let mut remaining = MAX_ATOMS;
    let movie = find(reader, 0..end, b"moov", &mut remaining, cancelled)?;
    let created = find(reader, movie.clone(), b"mvhd", &mut remaining, cancelled)
        .and_then(|header| creation(reader, header, cancelled))
        .and_then(DateTime::from_timestamp_millis)
        .map(|time| time.with_timezone(&Local));
    let mut offset = movie.start;
    let mut dimensions = None;
    while let Some(track) = find(
        reader,
        offset..movie.end,
        b"trak",
        &mut remaining,
        cancelled,
    ) {
        offset = track.end;
        let Some(media) = find(reader, track.clone(), b"mdia", &mut remaining, cancelled) else {
            continue;
        };
        let Some(handler) = find(reader, media, b"hdlr", &mut remaining, cancelled) else {
            continue;
        };
        if handler.end - handler.start < 12 {
            continue;
        }
        reader.seek(SeekFrom::Start(handler.start + 8)).ok()?;
        let mut kind = [0; 4];
        reader.read_exact(&mut kind).ok()?;
        if &kind != b"vide" {
            continue;
        }
        // Match the first video track used by the playback owner; an invalid
        // header stays unknown instead of borrowing another track's geometry.
        dimensions = find(reader, track, b"tkhd", &mut remaining, cancelled)
            .and_then(|header| geometry(reader, header, cancelled));
        break;
    }
    (!cancelled()).then_some(Header {
        created,
        dimensions,
    })
}

fn creation(
    reader: &mut (impl Read + Seek),
    header: Range<u64>,
    cancelled: &impl Fn() -> bool,
) -> Option<i64> {
    if cancelled() || (header.end - header.start) < 8 {
        return None;
    }
    reader.seek(SeekFrom::Start(header.start)).ok()?;
    let mut flags = [0; 4];
    reader.read_exact(&mut flags).ok()?;
    let seconds = match flags[0] {
        0 => {
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes).ok()?;
            u64::from(u32::from_be_bytes(bytes))
        }
        1 if (header.end - header.start) >= 12 => {
            let mut bytes = [0; 8];
            reader.read_exact(&mut bytes).ok()?;
            u64::from_be_bytes(bytes)
        }
        _ => return None,
    };
    let seconds = seconds.checked_sub(UNIX_EPOCH)?;
    // Unset/pre-Unix and out-of-calendar-range camera timestamps are unknown.
    // Do not guess a timezone for old cameras that wrote local time into this field.
    if seconds == 0 || seconds > 253_402_300_799 || cancelled() {
        return None;
    }
    i64::try_from(seconds.checked_mul(1000)?).ok()
}

// Track width/height are 16.16 values; the affine matrix carries rotation,
// mirroring and scale. Its perspective column is 2.30. Translation does not
// change the bounding size. Unsupported perspective remains unknown.
// https://developer.apple.com/documentation/quicktime-file-format/track_header_atom
// https://developer.apple.com/documentation/quicktime-file-format/matrices
fn geometry(
    reader: &mut (impl Read + Seek),
    header: Range<u64>,
    cancelled: &impl Fn() -> bool,
) -> Option<(u32, u32)> {
    if cancelled() || header.end - header.start < 84 {
        return None;
    }
    reader.seek(SeekFrom::Start(header.start)).ok()?;
    let mut version = [0];
    reader.read_exact(&mut version).ok()?;
    let offset = match version[0] {
        0 => 40,
        1 => 52,
        _ => return None,
    };
    if header.end - header.start < offset + 44 {
        return None;
    }
    reader.seek(SeekFrom::Start(header.start + offset)).ok()?;
    let mut bytes = [0; 44];
    reader.read_exact(&mut bytes).ok()?;
    let signed = |index| i32::from_be_bytes(bytes[index..index + 4].try_into().unwrap());
    if signed(8) != 0 || signed(20) != 0 || signed(32) != 1 << 30 {
        return None;
    }
    let fixed = |index| f64::from(signed(index)) / 65536.0;
    let width = f64::from(u32::from_be_bytes(bytes[36..40].try_into().ok()?)) / 65536.0;
    let height = f64::from(u32::from_be_bytes(bytes[40..44].try_into().ok()?)) / 65536.0;
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let w = fixed(0).abs() * width + fixed(12).abs() * height;
    let h = fixed(4).abs() * width + fixed(16).abs() * height;
    if !(1.0..=f64::from(u32::MAX)).contains(&w)
        || !(1.0..=f64::from(u32::MAX)).contains(&h)
        || cancelled()
    {
        return None;
    }
    Some((w.round() as u32, h.round() as u32))
}

// A single shared count bounds every header lookup; checked ranges never cross
// their parent, even for malformed/extended/zero-sized atoms. No sample payloads
// are descended and no storage is allocated from a file-provided length.
fn find(
    reader: &mut (impl Read + Seek),
    bounds: Range<u64>,
    kind: &[u8; 4],
    remaining: &mut usize,
    cancelled: &impl Fn() -> bool,
) -> Option<Range<u64>> {
    let mut offset = bounds.start;
    while bounds.end.checked_sub(offset)? >= 8 && *remaining > 0 {
        if cancelled() {
            return None;
        }
        *remaining -= 1;
        reader.seek(SeekFrom::Start(offset)).ok()?;
        let mut bytes = [0; 8];
        reader.read_exact(&mut bytes).ok()?;
        let short = u32::from_be_bytes(bytes[..4].try_into().ok()?);
        let (size, header) = match short {
            0 => (bounds.end - offset, 8),
            1 => {
                if bounds.end - offset < 16 {
                    return None;
                }
                let mut extended = [0; 8];
                reader.read_exact(&mut extended).ok()?;
                (u64::from_be_bytes(extended), 16)
            }
            _ => (u64::from(short), 8),
        };
        let end = offset.checked_add(size)?;
        if size < header || end > bounds.end {
            return None;
        }
        if &bytes[4..] == kind {
            return Some(offset + header..end);
        }
        offset = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, io::Cursor};

    fn atom(kind: &[u8; 4], body: &[u8], extended: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let size = body.len() as u64 + if extended { 16 } else { 8 };
        out.extend(if extended { 1_u32 } else { size as u32 }.to_be_bytes());
        out.extend(kind);
        if extended {
            out.extend(size.to_be_bytes());
        }
        out.extend(body);
        out
    }
    fn header(version: u8, seconds: u64) -> Vec<u8> {
        let mut body = vec![version, 0, 0, 0];
        if version == 1 {
            body.extend(seconds.to_be_bytes());
        } else {
            body.extend((seconds as u32).to_be_bytes());
        }
        atom(b"mvhd", &body, false)
    }
    fn parse(bytes: Vec<u8>) -> Option<i64> {
        let end = bytes.len() as u64;
        read(&mut Cursor::new(bytes), end, &|| false)?
            .created
            .map(|time| time.timestamp_millis())
    }
    fn track(version: u8, kind: &[u8; 4], matrix: [i32; 9], width: u32, height: u32) -> Vec<u8> {
        let offset = if version == 1 { 52 } else { 40 };
        let mut header = vec![0; offset];
        header[0] = version;
        for value in matrix {
            header.extend(value.to_be_bytes());
        }
        header.extend(width.to_be_bytes());
        header.extend(height.to_be_bytes());
        let mut handler = vec![0; 8];
        handler.extend(kind);
        let mut body = atom(b"tkhd", &header, false);
        body.extend(atom(b"mdia", &atom(b"hdlr", &handler, false), false));
        atom(b"trak", &body, false)
    }
    const IDENTITY: [i32; 9] = [65536, 0, 0, 0, 65536, 0, 0, 0, 1 << 30];
    fn dimensions(body: Vec<u8>) -> Option<(u32, u32)> {
        let bytes = atom(b"moov", &body, false);
        let end = bytes.len() as u64;
        read(&mut Cursor::new(bytes), end, &|| false)?.dimensions
    }
    #[test]
    fn track_dimensions_ignore_audio_and_apply_orientation_without_media_reads() {
        for version in [0, 1] {
            let audio = track(version, b"soun", IDENTITY, 99 << 16, 77 << 16);
            for (matrix, expected) in [
                (IDENTITY, (3840, 2160)),
                (
                    [0, 65536, 0, -65536, 0, 0, 2160 << 16, 0, 1 << 30],
                    (2160, 3840),
                ),
                ([-65536, 0, 0, 0, -65536, 0, 0, 0, 1 << 30], (3840, 2160)),
                ([32768, 0, 0, 0, 32768, 0, 0, 0, 1 << 30], (1920, 1080)),
            ] {
                let mut body = audio.clone();
                body.extend(track(version, b"vide", matrix, 3840 << 16, 2160 << 16));
                assert_eq!(dimensions(body), Some(expected));
            }
        }
        // The first video track defines playback, not a later larger track.
        let mut body = track(0, b"vide", IDENTITY, 320 << 16, 180 << 16);
        body.extend(track(0, b"vide", IDENTITY, 3840 << 16, 2160 << 16));
        assert_eq!(dimensions(body), Some((320, 180)));
    }
    #[test]
    fn invalid_geometry_is_unknown_without_crossing_atom_bounds() {
        for version in [0, 1] {
            let valid = track(version, b"vide", IDENTITY, 3840 << 16, 2160 << 16);
            for len in 0..valid.len() {
                assert_eq!(dimensions(valid[..len].to_vec()), None);
            }
        }
        let mut perspective = IDENTITY;
        perspective[2] = 1;
        for matrix in [perspective, [0; 9]] {
            assert_eq!(
                dimensions(track(0, b"vide", matrix, 3840 << 16, 2160 << 16)),
                None
            );
        }
        assert_eq!(
            dimensions(track(2, b"vide", IDENTITY, 3840 << 16, 2160 << 16)),
            None
        );
        assert_eq!(dimensions(track(0, b"vide", IDENTITY, 0, 2160 << 16)), None);
        assert_eq!(
            dimensions(track(0, b"soun", IDENTITY, 3840 << 16, 2160 << 16)),
            None
        );
        let mut invalid_first = track(0, b"vide", IDENTITY, 0, 2160 << 16);
        invalid_first.extend(track(0, b"vide", IDENTITY, 3840 << 16, 2160 << 16));
        assert_eq!(dimensions(invalid_first), None);
        let mut bounded = atom(b"free", &[], false).repeat(MAX_ATOMS);
        bounded.extend(track(0, b"vide", IDENTITY, 3840 << 16, 2160 << 16));
        assert_eq!(dimensions(bounded), None);
    }
    #[test]
    fn both_versions_extended_sizes_zero_size_and_absolute_timestamp() {
        for version in [0, 1] {
            for extended in [false, true] {
                let mut body = atom(b"free", &[0; 4], false);
                body.extend(header(version, UNIX_EPOCH + 1_700_000_000));
                let mut bytes = atom(b"ftyp", b"isom", false);
                bytes.extend(atom(b"moov", &body, extended));
                assert_eq!(parse(bytes), Some(1_700_000_000_000));
                let mut zero = atom(b"moov", &body, false);
                zero[..4].copy_from_slice(&[0; 4]);
                assert_eq!(parse(zero), Some(1_700_000_000_000));
            }
        }
        // v1 represents dates beyond the unsigned 32-bit QuickTime epoch.
        assert_eq!(
            parse(atom(b"moov", &header(1, UNIX_EPOCH + 4_102_444_800), false)),
            Some(4_102_444_800_000)
        );
    }
    #[test]
    fn invalid_timestamps_versions_and_parent_overruns_are_unknown() {
        for seconds in [0, UNIX_EPOCH - 1, UNIX_EPOCH, u64::MAX] {
            assert_eq!(parse(atom(b"moov", &header(1, seconds), false)), None);
        }
        assert_eq!(
            parse(atom(b"moov", &header(2, UNIX_EPOCH + 1), false)),
            None
        );
        let valid = atom(b"moov", &header(1, UNIX_EPOCH + 1), false);
        for length in 0..valid.len() {
            assert_eq!(parse(valid[..length].to_vec()), None);
        }
        // An mvhd cannot borrow its timestamp from a following sibling atom.
        let mut short = atom(b"moov", &atom(b"mvhd", &[1, 0, 0, 0], false), false);
        short.extend(atom(b"free", &(UNIX_EPOCH + 1).to_be_bytes(), false));
        assert_eq!(parse(short), None);
        for size in [2_u64, 15, u64::MAX] {
            let mut invalid = vec![];
            invalid.extend(1_u32.to_be_bytes());
            invalid.extend(b"moov");
            invalid.extend(size.to_be_bytes());
            assert_eq!(parse(invalid), None);
        }
    }
    #[test]
    fn bounded_atom_count_and_cancellation_do_not_scan_payloads() {
        let mut bytes = atom(b"free", &[], false).repeat(MAX_ATOMS);
        bytes.extend(atom(b"moov", &header(0, UNIX_EPOCH + 1), false));
        assert_eq!(parse(bytes.clone()), None);
        let checks = Cell::new(0);
        let end = bytes.len() as u64;
        assert_eq!(
            read(&mut Cursor::new(bytes), end, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 5
            })
            .and_then(|h| h.created),
            None
        );
        assert!((5..=8).contains(&checks.get()));
    }
    #[test]
    fn large_media_payload_is_skipped_with_bounded_io() {
        // A sparse file makes this independent of available RAM or disk capacity.
        let mut file = tempfile::tempfile().unwrap();
        use std::io::Write;
        let payload_end = 5_u64 * 1024 * 1024 * 1024;
        file.write_all(&1_u32.to_be_bytes()).unwrap();
        file.write_all(b"mdat").unwrap();
        file.write_all(&payload_end.to_be_bytes()).unwrap();
        file.seek(SeekFrom::Start(payload_end)).unwrap();
        file.write_all(&atom(b"moov", &header(0, UNIX_EPOCH + 100), false))
            .unwrap();
        struct Counting {
            file: File,
            bytes: usize,
        }
        impl Read for Counting {
            fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
                let n = self.file.read(out)?;
                self.bytes += n;
                Ok(n)
            }
        }
        impl Seek for Counting {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                self.file.seek(pos)
            }
        }
        let end = file.metadata().unwrap().len();
        let mut input = Counting { file, bytes: 0 };
        assert_eq!(
            read(&mut input, end, &|| false)
                .and_then(|h| h.created)
                .map(|t| t.timestamp_millis()),
            Some(100_000)
        );
        assert!(input.bytes <= 64, "Only bounded movie headers are read");
    }
}
