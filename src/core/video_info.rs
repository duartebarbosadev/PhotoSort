//! Container facts for the Details panel, read from the MP4/MOV header only
//! (the `moov` box), so even multi-gigabyte clips probe in milliseconds.
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VideoInfo {
    pub duration_s: f64,
    pub frame_rate: Option<f32>,
    pub codec: Option<String>,
    /// Video track bitrate in bits per second.
    pub bitrate: Option<u32>,
    pub audio: bool,
}

/// `None` for containers the reader does not understand; Details then simply
/// omits the video rows.
pub fn probe(path: &Path) -> Option<VideoInfo> {
    let file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    // The header comes from an untrusted file; a parser bug skips the rows.
    super::crash::catch(|| {
        let reader = mp4::Mp4Reader::read_header(std::io::BufReader::new(file), size).ok()?;
        read_info(&reader)
    })
    .ok()
    .flatten()
}
/// The reader's duration, frame rate and bitrate divide by the header's
/// timescales unchecked, so a zero timescale is refused before any of them.
fn read_info<R: std::io::Read + std::io::Seek>(reader: &mp4::Mp4Reader<R>) -> Option<VideoInfo> {
    if reader.timescale() == 0 || reader.tracks().values().any(|t| t.timescale() == 0) {
        return None;
    }
    let mut info = VideoInfo {
        duration_s: reader.duration().as_secs_f64(),
        ..Default::default()
    };
    for track in reader.tracks().values() {
        match track.track_type() {
            Ok(mp4::TrackType::Video) if info.codec.is_none() => {
                info.codec = Some(
                    match track.media_type() {
                        Ok(mp4::MediaType::H264) => "H.264 (AVC)",
                        Ok(mp4::MediaType::H265) => "H.265 (HEVC)",
                        Ok(mp4::MediaType::VP9) => "VP9",
                        _ => "Other",
                    }
                    .to_owned(),
                );
                info.frame_rate = rate(track.sample_count(), track.duration());
                let bitrate = track.bitrate();
                info.bitrate = (bitrate > 0).then_some(bitrate);
            }
            Ok(mp4::TrackType::Audio) => info.audio = true,
            _ => {}
        }
    }
    (info.duration_s > 0.0).then_some(info)
}

/// Frames per second from the track's frame count and length. The reader's
/// own `frame_rate` divides whole numbers, which turns 29.97 into 29.
fn rate(frames: u32, length: std::time::Duration) -> Option<f32> {
    let rate = (f64::from(frames) / length.as_secs_f64()) as f32;
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

/// "1:05", or "1:02:05" past an hour.
pub fn duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// "25 fps", "29.97 fps".
pub fn frame_rate(rate: f32) -> String {
    let rounded = (rate * 100.0).round() / 100.0;
    if (rounded - rounded.round()).abs() < 0.005 {
        format!("{} fps", rounded.round())
    } else {
        format!("{rounded:.2} fps")
    }
}

/// "96 Mbit/s", "850 kbit/s".
pub fn bitrate(bits: u32) -> String {
    if bits >= 1_000_000 {
        format!("{:.0} Mbit/s", f64::from(bits) / 1_000_000.0)
    } else {
        format!("{:.0} kbit/s", f64::from(bits) / 1_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_values_read_naturally() {
        assert_eq!(duration(3.36), "0:03");
        assert_eq!(duration(1340.64), "22:21");
        assert_eq!(duration(3725.0), "1:02:05");
        assert_eq!(frame_rate(25.0), "25 fps");
        assert_eq!(frame_rate(29.97), "29.97 fps");
        // Ten seconds of NTSC video: the rate keeps its decimals.
        let shown = |frames, micros| {
            frame_rate(rate(frames, std::time::Duration::from_micros(micros)).unwrap())
        };
        assert_eq!(shown(300, 10_010_000), "29.97 fps");
        assert_eq!(shown(240, 10_010_000), "23.98 fps");
        assert_eq!(shown(600, 10_010_000), "59.94 fps");
        assert_eq!(shown(250, 10_000_000), "25 fps");
        assert_eq!(rate(0, std::time::Duration::from_secs(10)), None);
        assert_eq!(rate(300, std::time::Duration::ZERO), None);
        assert_eq!(bitrate(96_198_086), "96 Mbit/s");
        assert_eq!(bitrate(850_000), "850 kbit/s");
    }

    #[test]
    fn non_video_files_probe_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-video.mp4");
        std::fs::write(&path, b"plain text").unwrap();
        assert!(probe(&path).is_none());
        assert!(probe(&dir.path().join("missing.mp4")).is_none());
    }

    #[test]
    fn a_zero_timescale_header_probes_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zero-timescale.mp4");
        let config = mp4::Mp4Config {
            major_brand: str::parse("isom").unwrap(),
            minor_version: 512,
            compatible_brands: vec![],
            timescale: 0,
        };
        let file = std::fs::File::create(&path).unwrap();
        let mut writer = mp4::Mp4Writer::write_start(file, &config).unwrap();
        writer.write_end().unwrap();
        // The header itself parses; only its derived values would divide by zero.
        let bytes = std::fs::read(&path).unwrap();
        let size = bytes.len() as u64;
        assert!(mp4::Mp4Reader::read_header(std::io::Cursor::new(bytes), size).is_ok());
        assert!(probe(&path).is_none());
    }
}
