//! GStreamer owns demuxing, decoding, audio and clock synchronization.
//! This adapter only maps its bounded RGBA sink into the shared playback owner.
use super::{Backend, Decoded};
use anyhow::{Context, Result};
use gst::prelude::*;
use gst_video::prelude::*;
use image::RgbaImage;
use std::{
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

const EDGE: i32 = 1280;
static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

fn initialize() -> Result<()> {
    INITIALIZED
        .get_or_init(|| gst::init().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| anyhow::anyhow!("GStreamer initialization: {e}"))?;
    Ok(())
}

pub struct Player {
    pipeline: gst::Element,
    sink: gst_app::AppSink,
    bus: gst::Bus,
    preroll: bool,
    ready: bool,
    ended: bool,
    error: Option<String>,
    segment: Option<gst::Seqnum>,
}

impl Player {
    pub fn open(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        Self::open_with_audio(path, cancelled, None)
    }

    pub(super) fn open_with_audio(
        path: &Path,
        cancelled: &dyn Fn() -> bool,
        audio: Option<gst::Element>,
    ) -> Result<Self> {
        anyhow::ensure!(!cancelled(), "Video opening cancelled");
        initialize()?;
        let uri = gst::glib::filename_to_uri(path, None)?;
        // The established local-file engine passes repeated concurrent preroll;
        // playbin3 stalled in that regression on the supported Linux runtime.
        let pipeline = gst::ElementFactory::make("playbin")
            .property("uri", uri.as_str())
            .build()
            .context("Install the GStreamer playback plugin")?;
        let sink = gst_app::AppSink::builder()
            .caps(
                &gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .field("width", gst::IntRange::<i32>::new(1, EDGE))
                    .field("height", gst::IntRange::<i32>::new(1, EDGE))
                    .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
                    .build(),
            )
            .max_buffers(1)
            .drop(true)
            .sync(true)
            .enable_last_sample(false)
            .wait_on_eos(false)
            .build();
        let flip = gst::ElementFactory::make("videoflip")
            .property_from_str("method", "automatic")
            .build()?;
        let scale = gst::ElementFactory::make("videoscale").build()?;
        let convert = gst::ElementFactory::make("videoconvert").build()?;
        let bin = gst::Bin::new();
        bin.add_many([&flip, &scale, &convert, sink.upcast_ref()])?;
        gst::Element::link_many([&flip, &scale, &convert, sink.upcast_ref()])?;
        let pad = gst::GhostPad::with_target(&flip.static_pad("sink").context("Video sink pad")?)?;
        bin.add_pad(&pad)?;
        pipeline.set_property("video-sink", &bin);
        // Photos are reviewed locally; subtitle overlays are not review evidence.
        pipeline.set_property_from_str("flags", "video+audio+soft-volume");
        if let Some(audio) = audio {
            pipeline.set_property("audio-sink", audio);
        }
        let bus = pipeline.bus().context("Playback bus")?;
        let player = Self {
            pipeline,
            sink,
            bus,
            preroll: true,
            ready: false,
            ended: false,
            error: None,
            segment: None,
        };
        anyhow::ensure!(!cancelled(), "Video opening cancelled");
        player
            .pipeline
            .set_state(gst::State::Paused)
            .context("Prepare video")?;
        // Preparation is asynchronous; the owner polls and enforces its deadline.
        if cancelled() {
            player.pipeline.set_state(gst::State::Null)?;
            anyhow::bail!("Video opening cancelled");
        }
        Ok(player)
    }

    fn controls(&mut self, playing: bool, seek: Option<f64>, volume: f32) -> Result<()> {
        self.pipeline.set_property("volume", f64::from(volume));
        if let Some(seconds) = seek {
            let seek = gst::event::Seek::new(
                1.0,
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::SeekType::Set,
                gst::ClockTime::from_seconds_f64(seconds.max(0.0)),
                gst::SeekType::None,
                gst::ClockTime::NONE,
            );
            let segment = seek.seqnum();
            anyhow::ensure!(self.pipeline.send_event(seek), "Video seek rejected");
            self.segment = Some(segment);
            self.preroll = true;
            self.ended = false;
        }
        self.pipeline.set_state(if playing {
            gst::State::Playing
        } else {
            gst::State::Paused
        })?;
        Ok(())
    }

    fn pixels(sample: &gst::Sample) -> Result<RgbaImage> {
        let info = gst_video::VideoInfo::from_caps(sample.caps().context("Missing video caps")?)?;
        anyhow::ensure!(
            info.format() == gst_video::VideoFormat::Rgba
                && info.width() <= EDGE as u32
                && info.height() <= EDGE as u32,
            "Unexpected video output geometry or format"
        );
        let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(
            sample.buffer().context("Missing video frame")?,
            &info,
        )?;
        let stride = usize::try_from(frame.plane_stride()[0]).context("Invalid video stride")?;
        let data = frame.plane_data(0)?;
        let row = info.width() as usize * 4;
        let height = info.height() as usize;
        anyhow::ensure!(
            stride >= row && height > 0 && data.len() >= stride * (height - 1) + row,
            "Incomplete video frame"
        );
        let mut pixels = Vec::with_capacity(row * height);
        for y in 0..height {
            pixels.extend_from_slice(&data[y * stride..y * stride + row]);
        }
        RgbaImage::from_raw(info.width(), info.height(), pixels).context("Invalid video dimensions")
    }
}

impl Backend for Player {
    fn control(&mut self, playing: bool, seek: Option<f64>, volume: f32) {
        if let Err(e) = self.controls(playing, seek, volume) {
            self.error = Some(format!("{e:#}"));
        }
    }
    fn poll(&mut self) -> Result<Decoded> {
        if let Some(error) = self.error.take() {
            anyhow::bail!(error);
        }
        // The streaming threads never write application state or queue RGBA copies.
        for message in self.bus.iter().take(64) {
            match message.view() {
                gst::MessageView::Error(error) => {
                    anyhow::bail!("Video playback: {}", error.error())
                }
                // An earlier segment can finish before the owner receives a seek.
                // Match GStreamer's sequence number rather than replaying its old EOS.
                gst::MessageView::Eos(..) if self.segment.is_none_or(|s| message.seqnum() == s) => {
                    self.ended = true
                }
                _ => {}
            }
        }
        let sample = if self.preroll {
            // Pulling the playing queue first clears appsink's paused preroll.
            let sample = self.sink.try_pull_preroll(gst::ClockTime::ZERO);
            if sample.is_some() {
                self.preroll = false;
            }
            sample
        } else {
            self.sink.try_pull_sample(gst::ClockTime::ZERO)
        };
        let frame = sample.as_ref().map(Self::pixels).transpose()?;
        self.ready |= frame.is_some();
        // A paused seek to the exact end has no preroll frame. The sink has
        // already reached EOS, but playbin need not post bus EOS until Playing.
        if self.ready && self.preroll && self.segment.is_some() && self.sink.is_eos() {
            self.ended = true;
        }
        let position = self
            .pipeline
            .query_position::<gst::ClockTime>()
            .map_or(0.0, |t| t.seconds_f64());
        let duration = self
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map_or(0.0, |t| t.seconds_f64());
        // Some decoders preroll their final frame at the exact endpoint instead
        // of sending EOS while paused. Only use the reported position after
        // preroll has completed, never the previous segment's position mid-seek.
        if self.ready && !self.preroll && duration > 0.0 && position >= duration {
            self.ended = true;
        }
        Ok(Decoded {
            position,
            duration,
            ready: self.ready,
            playing: self.pipeline.current_state() == gst::State::Playing && !self.ended,
            ended: self.ended,
            seeking: self.segment.is_some() && self.preroll && !self.ended,
            frame,
        })
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Poster requests remain on the shared image owner and use its existing caches.
pub fn thumbnail(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<image::DynamicImage> {
    anyhow::ensure!(!cancelled(), "Video thumbnail cancelled");
    initialize()?;
    let audio = gst::ElementFactory::make("fakesink").build()?;
    let mut player = Player::open_with_audio(path, cancelled, Some(audio))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        anyhow::ensure!(
            !cancelled() && Instant::now() < deadline,
            "Video thumbnail cancelled or timed out"
        );
        if let Some(frame) = player.poll()?.frame {
            return Ok(image::DynamicImage::ImageRgba8(frame));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(player: &mut Player, condition: impl Fn(&Decoded) -> bool) -> Decoded {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let value = player.poll().unwrap();
            if condition(&value) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "Playback timed out: position {}, duration {}, ready {}, playing {}, ended {}",
                value.position,
                value.duration,
                value.ready,
                value.playing,
                value.ended
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    #[test]
    fn real_video_playback_and_posters() {
        gst::init().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
        for (name, size) in [
            ("motion.mp4", (320, 180)),
            ("rotated.mp4", (180, 320)),
            ("large.mp4", (1280, 720)),
        ] {
            let path = root.join(name);
            let audio = gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()
                .unwrap();
            let mut player = Player::open_with_audio(&path, &|| false, Some(audio)).unwrap();
            let opened = wait(&mut player, |s| s.ready && s.frame.is_some());
            let pixels = opened.frame.unwrap();
            assert_eq!(pixels.dimensions(), size);
            assert!(pixels.pixels().all(|p| p.0[3] == 255));
            if name == "large.mp4" {
                let blue = pixels.get_pixel(640, 360).0;
                assert!(blue[2] > 200 && blue[0] < 30 && blue[1] < 30, "{blue:?}");
            }
            assert!(!opened.playing);
            let target = opened.duration * 0.5;
            player.control(false, Some(target), 0.0);
            let seek = wait(&mut player, |s| {
                (s.position - target).abs() < 0.08 && s.frame.is_some()
            });
            assert!(!seek.playing);
            player.control(true, None, 0.0);
            wait(&mut player, |s| {
                s.playing && s.position > target + opened.duration * 0.05
            });
            player.control(false, None, 0.0);
            let paused = wait(&mut player, |s| !s.playing);
            std::thread::sleep(Duration::from_millis(150));
            assert!((player.poll().unwrap().position - paused.position).abs() < 0.1);
            player.control(true, Some((opened.duration - 0.2).max(0.0)), 0.0);
            wait(&mut player, |s| s.ended);
            player.control(true, Some(0.0), 0.0);
            wait(&mut player, |s| {
                s.playing && s.position < 1.0 && s.frame.is_some()
            });
            drop(player);
            assert_eq!(
                thumbnail(&path, &|| false)
                    .unwrap()
                    .into_rgba8()
                    .dimensions(),
                size
            );
            println!(
                "PASS {name}: paused open, seek, playback, pause, EOF, replay, poster, dimensions {size:?}"
            );
        }
    }
    #[test]
    fn paused_end_seek_completes_and_can_seek_back() {
        gst::init().unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video/motion.mp4");
        let audio = gst::ElementFactory::make("fakesink").build().unwrap();
        let mut player = Player::open_with_audio(&path, &|| false, Some(audio)).unwrap();
        let opened = wait(&mut player, |s| s.ready && s.frame.is_some());
        player.control(false, Some(opened.duration), 0.0);
        let end = wait(&mut player, |s| !s.seeking && s.ended);
        assert!(!end.playing);
        player.control(false, Some(0.4), 0.0);
        let back = wait(&mut player, |s| {
            !s.seeking && s.frame.is_some() && (s.position - 0.4).abs() < 0.08
        });
        assert!(!back.ended);
        assert!(!back.playing);
    }

    #[test]
    fn cancelled_and_invalid_video_fail_without_waiting_for_a_frame() {
        let missing = Path::new("/not-a-video.mp4");
        assert!(Player::open(missing, &|| true).is_err());
        assert!(thumbnail(missing, &|| true).is_err());
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("invalid.mp4");
        std::fs::write(&path, "not video").unwrap();
        let start = Instant::now();
        assert!(thumbnail(&path, &|| false).is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video/large.mp4");
        let attempts = std::cell::Cell::new(0);
        assert!(
            thumbnail(&path, &|| {
                let n = attempts.get();
                attempts.set(n + 1);
                n >= 3
            })
            .is_err()
        );
    }

    #[test]
    fn real_posters_reuse_the_shared_pipeline_and_restart_cache() {
        use crate::core::{catalog::Catalog, pipeline::Pipeline};
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
        let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
        let cache = tempfile::tempdir().unwrap();
        let mut pipeline = Pipeline::new(32 * 1024 * 1024, cache.path().into());
        for media in &catalog.media {
            let first = pipeline.load(media).unwrap();
            for _ in crate::core::model::Step::ALL {
                assert!(std::sync::Arc::ptr_eq(
                    &first.pixels,
                    &pipeline.load(media).unwrap().pixels
                ));
            }
        }
        assert_eq!(pipeline.stats.decodes, catalog.media.len() as u64);
        let mut restarted = Pipeline::new(32 * 1024 * 1024, cache.path().into());
        for media in &catalog.media {
            let poster = restarted.load(media).unwrap();
            let expected = match media.name().as_str() {
                "large.mp4" => (3840, 2160),
                "rotated.mp4" => (180, 320),
                _ => (320, 180),
            };
            assert_eq!((poster.metadata.width, poster.metadata.height), expected);
        }
        assert_eq!(restarted.stats.decodes, 0);
        assert_eq!(restarted.stats.metadata_reads, 0);
    }
    #[test]
    fn seek_does_not_replay_a_queued_eof_from_the_previous_segment() {
        gst::init().unwrap();
        let audio = gst::ElementFactory::make("fakesink")
            .property("sync", true)
            .build()
            .unwrap();
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video/motion.mp4");
        let mut player = Player::open_with_audio(&path, &|| false, Some(audio)).unwrap();
        let opened = wait(&mut player, |s| s.frame.is_some());
        player.control(true, Some(opened.duration - 0.05), 0.0);
        std::thread::sleep(Duration::from_millis(300));
        // The UI seeks before the owner has consumed the previous EOF event.
        player.control(true, Some(0.0), 0.0);
        let value = player.poll().unwrap();
        assert!(!value.ended, "Queued old EOF ended the new segment");
        wait(&mut player, |s| {
            s.playing && s.position < 1.0 && s.frame.is_some()
        });
    }
    #[test]
    fn concurrent_poster_stress() {
        let threads: Vec<_> = (0..4)
            .map(|worker| {
                std::thread::spawn(move || {
                    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
                    for iteration in 0..40 {
                        let name =
                            ["motion.mp4", "rotated.mp4", "large.mp4"][(worker + iteration) % 3];
                        thumbnail(&root.join(name), &|| false).unwrap_or_else(|e| {
                            panic!("worker {worker}, iteration {iteration}, {name}: {e:#}")
                        });
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }
}
