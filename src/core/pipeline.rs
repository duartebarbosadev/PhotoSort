use super::{cache_record, metadata, model::*, session};
use anyhow::{Context, Result};
use image::{DynamicImage, GenericImageView, RgbaImage};
use lru::LruCache;
use std::{
    collections::HashMap,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const PROXY_EDGE: u32 = 2048;
pub const ANALYSIS_EDGE: u32 = 1024;
pub const THUMB_EDGE: u32 = 320;
const METADATA_CAPACITY: NonZeroUsize = NonZeroUsize::new(100_000).unwrap();
#[derive(Clone)]
pub struct Asset {
    pub pixels: Arc<RgbaImage>,
    pub thumbnail: Arc<RgbaImage>,
    pub metadata: Metadata,
    pub decoder_warning: Option<String>,
}
impl Asset {
    fn bytes(&self) -> usize {
        self.pixels.as_raw().len()
            + if Arc::ptr_eq(&self.pixels, &self.thumbnail) {
                0
            } else {
                self.thumbnail.as_raw().len()
            }
    }
}
#[derive(Clone)]
struct Prepared {
    image: Arc<DynamicImage>,
    warning: Option<String>,
    width: u32,
    height: u32,
    full_resolution: bool,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedRender {
    width: u32,
    height: u32,
    warning: Option<String>,
}
/// The grid thumbnail's own small artifact, so reopening a folder decodes a
/// 320 px image per tile instead of the 2048 px display proxy.
fn thumb_key(source_key: &str) -> String {
    blake3::hash(format!("thumb-{THUMB_EDGE}-v1:{source_key}").as_bytes())
        .to_hex()
        .to_string()
}
fn analysis_key(source_key: &str) -> String {
    blake3::hash(format!("analysis-1024-v1:{source_key}").as_bytes())
        .to_hex()
        .to_string()
}
struct DetailEntry {
    prepared: Prepared,
    tick: u64,
}
struct Entry {
    asset: Asset,
    tick: u64,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub decodes: u64,
    pub native_renders: u64,
    pub native_regions: u64,
    /// Rust color-manager counters; native preparation is counted separately.
    pub color_transforms: u64,
    pub color_initializations: u64,
    pub metadata_reads: u64,
    pub metadata_disk_hits: u64,
    pub raw_embedded: u64,
    pub raw_fallback: u64,
    pub hits: u64,
    pub disk_hits: u64,
    pub resident_bytes: usize,
    pub detail_bytes: usize,
    /// Bytes of the last image too large for full-size detail memory. It is
    /// kept anyway, alone and outside the budget, so zooming it stays fast,
    /// but a second one (comparing, going back) replaces it.
    pub detail_overflow: u64,
    /// Disk cache bytes this pipeline wrote. The runtime-wide count, with
    /// photos and trimmed bytes, is `cache::Writes`.
    pub disk_written: u64,
}
// Naive EXIF and movie-header dates depend on the system timezone. A cache
// hit must still represent that timezone at the photo's captured instant.
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedMetadata {
    metadata: Metadata,
    capture_offset: Option<i32>,
}
impl CachedMetadata {
    fn offset(meta: &Metadata) -> Option<i32> {
        chrono::DateTime::from_timestamp_millis(meta.captured_ms?)
            .map(|t| t.with_timezone(&chrono::Local).offset().local_minus_utc())
    }
    fn new(metadata: Metadata) -> Self {
        Self {
            capture_offset: Self::offset(&metadata),
            metadata,
        }
    }
    fn valid(&self) -> bool {
        (1..=8).contains(&self.metadata.orientation)
            && self
                .metadata
                .container_orientation
                .is_none_or(|o| (1..=8).contains(&o))
            && self.metadata.rating <= 5
            && self.capture_offset == Self::offset(&self.metadata)
            && self.metadata.location.is_none()
    }
}
/// Owned by one imaging actor. Concurrent UI requests are coalesced before entering it.
pub struct Pipeline {
    colors: super::color::Manager,
    geography: Option<super::geography::Geography>,
    geography_checked: bool,
    cache: HashMap<String, Entry>,
    metadata: LruCache<String, Metadata>,
    details: HashMap<String, DetailEntry>,
    /// The one entry of `details` over the detail budget, not counted in it.
    oversized: Option<String>,
    pressure: super::memory::Pressure,
    /// See `limit_quiet_allowance`.
    quiet_cap: Option<u64>,
    budget: usize,
    detail_budget: usize,
    disk_budget: u64,
    tick: u64,
    pub stats: Stats,
    disk: PathBuf,
    written_since_prune: u64,
    written_photos: std::collections::HashSet<PathBuf>,
    /// Shared with the other pipelines of a runtime; see `share_writes`.
    writes: Option<Arc<super::cache::Writes>>,
    started: std::time::SystemTime,
    raw_look: super::raw_look::RawLook,
    /// Enhanced-look levels per RAW source, so 1:1 crops match the preview.
    raw_levels: HashMap<String, super::raw_look::Levels>,
    /// Threads one proxy downscale may use; see `limit_threads`.
    threads: usize,
}
impl Pipeline {
    pub fn new(budget: usize, disk: PathBuf) -> Self {
        Self {
            colors: super::color::Manager::default(),
            geography: None,
            geography_checked: false,
            cache: HashMap::new(),
            metadata: LruCache::sparse(METADATA_CAPACITY),
            details: HashMap::new(),
            oversized: None,
            pressure: Default::default(),
            quiet_cap: None,
            budget,
            detail_budget: budget.saturating_mul(2),
            disk_budget: 1024 * 1024 * 1024,
            tick: 0,
            stats: Stats::default(),
            disk,
            written_since_prune: 0,
            written_photos: Default::default(),
            writes: None,
            started: std::time::SystemTime::now(),
            raw_look: Default::default(),
            raw_levels: HashMap::new(),
            threads: super::processing::available(),
        }
    }
    /// Pipelines that already run several at a time (background builders,
    /// analysis helpers) keep each downscale on their own thread, so they
    /// add up to the cores they were given rather than a multiple of them.
    pub fn limit_threads(&mut self, threads: usize) {
        self.threads = threads.max(1);
    }
    /// Whether another pipeline may prepare this photo next to the image
    /// worker's. libvips formats render under one lock the screen needs, and
    /// videos belong to the image worker, which owns playback.
    pub fn shared_decode(media: &Media) -> bool {
        media.kind != MediaKind::Video && !native_raster(media)
    }
    pub fn standard() -> Self {
        Self::new(128 * 1024 * 1024, session::cache_dir())
    }
    pub fn key(media: &Media) -> Result<String> {
        let fingerprint = Fingerprint::read(&media.path)?;
        let sidecar = Fingerprint::read(&metadata::sidecar(&media.path)).unwrap_or_default();
        let mut hash = blake3::Hasher::new();
        hash.update(media.path.as_os_str().as_encoded_bytes());
        hash.update(
            format!(
                "{fingerprint:?}-{sidecar:?}-v9-render-status-srgb-neutral-analysis-native_metadata=true"
            )
            .as_bytes(),
        );
        hash.update(
            format!(
                "vips-working-2048-shrink-on-load-v3:{}:{}:{}",
                photosort_imaging_vips::VIPS_VERSION,
                photosort_imaging_vips::LCMS_VERSION,
                photosort_imaging_vips::BUILD_ID
            )
            .as_bytes(),
        );
        if media.kind == MediaKind::Raw {
            hash.update(b"raw-shared-preview-v1");
        }
        if media.kind == MediaKind::Video {
            #[cfg(not(target_os = "macos"))]
            hash.update(b"gstreamer-rgba-poster-v1");
            hash.update(b"movie-source-geometry-v2");
        }
        #[cfg(target_os = "macos")]
        if media
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
        {
            hash.update(b"imageio-bounded-straight-alpha-v2");
        }
        #[cfg(not(target_os = "macos"))]
        if media
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
        {
            hash.update(
                concat!(
                    "heif-rgba-color-container-v1:",
                    env!("PHOTOSORT_HEIF_VERSION")
                )
                .as_bytes(),
            );
        }
        Ok(hash.finalize().to_hex().to_string())
    }
    /// Metadata-only browsing shares ownership and invalidation with all image resolutions.
    pub fn metadata(&mut self, media: &Media) -> Result<Metadata> {
        self.metadata_cancellable(media, &|| false)
    }
    pub fn metadata_cancellable(
        &mut self,
        media: &Media,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Metadata> {
        anyhow::ensure!(!cancelled(), "Metadata read cancelled");
        let key = Self::key(media)?;
        self.metadata_for_key(media, &key, cancelled)
    }
    fn metadata_for_key(
        &mut self,
        media: &Media,
        key: &str,
        cancelled: &impl Fn() -> bool,
    ) -> Result<Metadata> {
        if let Some(meta) = self.metadata.get(key) {
            return Ok(meta.clone());
        }
        // Raw metadata has its own recipe. Display geometry and resolved place
        // names are derived separately and must never become persisted EXIF.
        let record_key = blake3::hash(
            format!(
                "metadata-v6-subsec:heif=true:os={}:{key}",
                std::env::consts::OS
            )
            .as_bytes(),
        );
        let path = self.disk.join(format!("{}.bin", record_key.to_hex()));
        let cached = (self.disk_budget > 0)
            .then(|| cache_record::read(&path))
            .flatten()
            .and_then(|bytes| {
                cache_record::decode::<CachedMetadata>(cache_record::Kind::Metadata, &bytes)
            })
            .filter(CachedMetadata::valid)
            .map(|record| record.metadata);
        let hit = cached.is_some();
        let mut meta = if let Some(meta) = cached {
            meta
        } else {
            self.stats.metadata_reads += 1;
            let mut meta = metadata::read_cancellable(&media.path, cancelled)?;
            if media.kind == MediaKind::Video {
                meta.details.video = super::video_info::probe(&media.path);
            }
            meta
        };
        anyhow::ensure!(
            !cancelled() && Self::key(media)? == key,
            "Photo changed or metadata read cancelled"
        );
        if hit {
            self.stats.metadata_disk_hits += 1;
        } else if self.disk_budget > 0
            && let Ok(bytes) = cache_record::encode(
                cache_record::Kind::Metadata,
                &CachedMetadata::new(meta.clone()),
            )
            && bytes.len() as u64 <= self.disk_budget
            && cache_record::write(&path, &bytes).is_ok()
        {
            self.note_cache_write(None, bytes.len() as u64, cancelled);
        }
        // A cached model result can skip image preparation entirely. Restore
        // source geometry from the render descriptor without decoding pixels,
        // while keeping the persisted EXIF snapshot separate and unchanged.
        // Every still image needs this, not only RAW and HEIF: EXIF pixel
        // dimensions can be missing (the pure-Rust reader then has none) or
        // describe a different image, while a cold run publishes the decoded
        // size. Videos keep their movie-header geometry.
        if media.kind != MediaKind::Video {
            for render_key in [key.to_owned(), analysis_key(key)] {
                if let Some(render) = self.render_info(&render_key) {
                    (meta.width, meta.height) = (render.width, render.height);
                    break;
                }
            }
        }
        if let Some((lat, lon)) = meta.gps {
            if !self.geography_checked {
                self.geography_checked = true;
                if let Some(path) = super::models::resource_roots()
                    .into_iter()
                    .map(|p| p.join("locations.tsv"))
                    .find(|p| p.is_file())
                {
                    self.geography = super::geography::Geography::load(&path).ok();
                }
            }
            if let Some(geo) = &self.geography {
                meta.location = geo.nearest(lat, lon);
            }
        }

        anyhow::ensure!(
            !cancelled() && Self::key(media)? == key,
            "Photo changed or metadata read cancelled"
        );
        self.metadata.put(key.into(), meta.clone());
        Ok(meta)
    }
    fn render_info(&self, key: &str) -> Option<CachedRender> {
        if self.disk_budget == 0 {
            return None;
        }
        let path = self.disk.join(format!("{key}.json"));
        let info = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CachedRender>(&bytes).ok())
            .filter(|v| {
                v.width > 0 && v.height > 0 && v.warning.as_ref().is_none_or(|w| w.len() <= 4096)
            })?;
        super::cache::touch(&path);
        Some(info)
    }
    /// A grid thumbnail from its disk artifact, without the display proxy.
    /// `None` when it is not cached (or already in memory); load normally then.
    /// The returned asset carries only thumbnail pixels and is not cached.
    pub fn cached_thumbnail(
        &mut self,
        media: &Media,
        cancelled: impl Fn() -> bool,
    ) -> Option<Asset> {
        let source_key = Self::key(media).ok()?;
        let display_key = self.look_key(media, &source_key);
        if self.cache.contains_key(&display_key) {
            return None;
        }
        let saved = self.render_info(&source_key)?;
        let image = read_cached_image(&self.disk, &thumb_key(&display_key))?;
        let mut meta = self.metadata_for_key(media, &source_key, &cancelled).ok()?;
        if media.kind != MediaKind::Video {
            (meta.width, meta.height) = (saved.width, saved.height);
        }
        self.stats.disk_hits += 1;
        let thumbnail = Arc::new(image.to_rgba8());
        Some(Asset {
            pixels: thumbnail.clone(),
            thumbnail,
            metadata: meta,
            decoder_warning: saved.warning,
        })
    }
    /// Stores the grid thumbnail and returns it as a later session reads it
    /// back, so a fresh tile and a reopened one are the same pixels.
    fn save_thumbnail(
        &mut self,
        source_key: &str,
        thumbnail: &RgbaImage,
        cancelled: &impl Fn() -> bool,
    ) -> Option<RgbaImage> {
        if self.disk_budget == 0 || std::fs::create_dir_all(&self.disk).is_err() {
            return None;
        }
        let key = thumb_key(source_key);
        if ["jpg", "png"]
            .iter()
            .any(|ext| self.disk.join(format!("{key}.{ext}")).exists())
        {
            return None;
        }
        // Opaque tiles are stored as RGB, which lets them be JPEG.
        let image = DynamicImage::ImageRgba8(thumbnail.clone());
        let image = if thumbnail.pixels().all(|p| p[3] == 255) {
            DynamicImage::ImageRgb8(image.into_rgb8())
        } else {
            image
        };
        let (bytes, format, read_back) = encode_cached(&image, true)?;
        write_cached(&bytes, format, &self.disk, &key)?;
        self.note_cache_write(None, bytes.len() as u64, cancelled);
        read_back.map(|image| image.to_rgba8())
    }
    /// Counts new cache bytes and trims the folder now and then. A trim lists
    /// and sorts the whole folder, so it runs once a share of the budget was
    /// written, not every few photos. A failed trim never fails the read or
    /// preview that triggered it; the next one tries again.
    /// `preview` names the photo of a preview or analysis image, which is
    /// what a folder's cache size grows with; metadata records are small.
    fn note_cache_write(
        &mut self,
        preview: Option<&Path>,
        bytes: u64,
        cancelled: &impl Fn() -> bool,
    ) {
        const MIN_TRIM: u64 = 32 * 1024 * 1024;
        self.written_since_prune += bytes;
        self.stats.disk_written += bytes;
        use std::sync::atomic::Ordering::Relaxed;
        if let Some(writes) = &self.writes {
            writes.written.fetch_add(bytes, Relaxed);
        }
        if let Some(media) = preview
            && !self.written_photos.contains(media)
        {
            self.written_photos.insert(media.to_owned());
            if let Some(writes) = &self.writes {
                writes.photos.fetch_add(1, Relaxed);
            }
        }
        let every = (self.disk_budget / 16).max(MIN_TRIM).min(self.disk_budget);
        if self.written_since_prune >= every {
            match super::cache::trim(&self.disk, self.disk_budget, self.started, cancelled) {
                Ok(usage) => {
                    if let Some(writes) = &self.writes {
                        writes
                            .evicted
                            .fetch_add(usage.evicted_recent_bytes, Relaxed);
                    }
                }
                Err(error) => tracing::debug!(%error, "cache trim skipped"),
            }
            self.written_since_prune = 0;
        }
    }
    /// How RAW photos are displayed. Analysis is unaffected; display assets
    /// are cached per look, so switching never shows a stale render.
    pub fn set_raw_look(&mut self, look: super::raw_look::RawLook) {
        self.raw_look = look;
    }
    fn enhanced(&self, media: &Media) -> bool {
        media.kind == MediaKind::Raw && self.raw_look == super::raw_look::RawLook::Enhanced
    }
    /// Memory and thumbnail key for a display asset under the current look.
    fn look_key(&self, media: &Media, key: &str) -> String {
        if self.enhanced(media) {
            format!("{key}+enhanced")
        } else {
            key.to_owned()
        }
    }
    /// Adds this pipeline's disk cache writes to the runtime's shared count.
    pub fn share_writes(&mut self, writes: Arc<super::cache::Writes>) {
        self.writes = Some(writes);
    }
    /// Caps what a decode may use without the user asking. The image worker
    /// has the whole quiet allowance; background and copy pipelines run next
    /// to it, so they take a small share and fall back to smaller sources.
    pub fn limit_quiet_allowance(&mut self, bytes: u64) {
        self.quiet_cap = Some(bytes);
        self.colors.set_allowance(self.quiet_cap);
    }
    /// Makes the display preview and grid thumbnail on disk unless both are
    /// there already, and keeps nothing in memory. True when it made them.
    /// Videos are left to the image worker, which owns playback.
    pub fn build_preview(
        &mut self,
        media: &Media,
        cancelled: impl Fn() -> bool + Sync,
    ) -> Result<bool> {
        // libvips formats shrink while loading, so building them ahead gains
        // little and would hold libvips' one render lock against the screen.
        if self.disk_budget == 0 || media.kind == MediaKind::Video || native_raster(media) {
            return Ok(false);
        }
        let source_key = Self::key(media)?;
        let thumb = thumb_key(&self.look_key(media, &source_key));
        let ready = self.render_info(&source_key).is_some()
            && ["jpg", "png"]
                .iter()
                .any(|ext| self.disk.join(format!("{thumb}.{ext}")).exists());
        if ready {
            return Ok(false);
        }
        let built = self.load_image(media, false, true, &cancelled);
        self.clear_memory();
        built.map(|_| true)
    }
    /// Disk cache bytes this pipeline wrote, for a builder that must not
    /// push out what it just made.
    pub fn disk_written(&self) -> u64 {
        self.stats.disk_written
    }
    pub fn disk_budget(&self) -> u64 {
        self.disk_budget
    }
    pub fn load(&mut self, media: &Media) -> Result<Asset> {
        self.load_cancellable(media, || false)
    }
    pub fn load_cancellable(
        &mut self,
        media: &Media,
        cancelled: impl Fn() -> bool + Sync,
    ) -> Result<Asset> {
        self.load_image(media, false, true, &cancelled)
    }
    /// The display-sized working image without the RAW display look. Model input
    /// that must not depend on the display preference (rotation) uses this; it
    /// shares the neutral disk proxy and memory entry with the Neutral look.
    pub fn load_neutral_cancellable(
        &mut self,
        media: &Media,
        cancelled: impl Fn() -> bool + Sync,
    ) -> Result<Asset> {
        self.load_image(media, false, false, &cancelled)
    }
    /// Neutral, bounded input shared by measurements and every model workflow.
    /// Display and analysis derive separate proxies from shared prepared pixels;
    /// original detail retains its full rendering recipe. All purposes share
    /// the metadata owner, bounded source cache and disk budget.
    pub fn analysis(
        &mut self,
        media: &Media,
        cancelled: impl Fn() -> bool + Sync,
    ) -> Result<Asset> {
        self.load_image(media, true, false, &cancelled)
    }
    fn load_image(
        &mut self,
        media: &Media,
        analysis: bool,
        look: bool,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<Asset> {
        // Only display assets take the RAW look; analysis is always neutral.
        let look = look && !analysis && self.enhanced(media);
        anyhow::ensure!(!cancelled(), "Image preparation cancelled");
        let source_key = Self::key(media)?;
        let key = if analysis {
            analysis_key(&source_key)
        } else {
            source_key.clone()
        };
        // The disk proxy is always the neutral render; the look applies after.
        let memory_key = if look {
            self.look_key(media, &key)
        } else {
            key.clone()
        };
        self.tick += 1;
        if let Some(entry) = self.cache.get_mut(&memory_key) {
            entry.tick = self.tick;
            self.stats.hits += 1;
            return Ok(entry.asset.clone());
        }
        let mut meta = self.metadata_for_key(media, &source_key, cancelled)?;
        let info = self.disk.join(format!("{key}.json"));
        let saved = self.render_info(&key);
        let warning;
        let decoded = if let Some(saved) = saved
            && let Some(proxy) = read_cached_image(&self.disk, &key)
        {
            self.stats.disk_hits += 1;
            // Geometry and warnings are part of the same rendered artifact.
            // Never publish cached pixels without the status that produced them.
            if media.kind != MediaKind::Video {
                (meta.width, meta.height) = (saved.width, saved.height);
            }
            warning = saved.warning;
            proxy
        } else {
            let prepared = if media.kind == MediaKind::Raw {
                self.raw_preview(media, &source_key, &mut meta, cancelled)?
            } else {
                self.canonical(media, &source_key, &mut meta, false, cancelled)?
            };
            warning = prepared.warning;
            let oriented = prepared.image;
            anyhow::ensure!(!cancelled(), "Image preparation cancelled");
            let edge = if analysis { ANALYSIS_EDGE } else { PROXY_EDGE };
            let proxy = if (native_raster(media) || apple_heif(media))
                && analysis
                && (oriented.width() > edge || oriented.height() > edge)
            {
                // Derive geometry from the source, not from an already rounded
                // working image; otherwise two-stage resizing can add a row.
                let (width, height) = if (5..=8).contains(&meta.orientation) {
                    (meta.height, meta.width)
                } else {
                    (meta.width, meta.height)
                };
                let scale = f64::from(edge) / f64::from(width.max(height));
                super::resample::exact_lanczos3(
                    &oriented,
                    (f64::from(width) * scale).round().max(1.0) as u32,
                    (f64::from(height) * scale).round().max(1.0) as u32,
                    self.threads,
                )
            } else if oriented.width() > edge || oriented.height() > edge {
                // The same pixels as resize(.., Lanczos3) and resize(..,
                // Triangle), without their full-size float buffer and on
                // several threads.
                if analysis {
                    super::resample::fit_lanczos3(&oriented, edge, self.threads)
                } else {
                    super::resample::fit_triangle(&oriented, edge, self.threads)
                }
            } else {
                (*oriented).clone()
            };
            // Native preparation returns RGBA. Opaque proxies need no alpha
            // channel on disk; preserve exactly the same decoded RGBA pixels.
            let proxy = compact_opaque_proxy(proxy, cancelled)?;
            anyhow::ensure!(
                !cancelled() && Self::key(media)? == source_key,
                "Photo changed or preparation cancelled"
            );
            // Display previews are JPEG, encoded even when nothing is written,
            // so what is shown (and what the rotation model reads) never
            // depends on the cache. Analysis input stays lossless and is
            // encoded only to be written.
            let writing = self.disk_budget > 0 && std::fs::create_dir_all(&self.disk).is_ok();
            let encoded = if writing || is_jpeg_preview(&proxy, !analysis) {
                let Some(encoded) = encode_cached(&proxy, !analysis) else {
                    anyhow::bail!("Cannot encode the preview of {}", media.name());
                };
                Some(encoded)
            } else {
                None
            };
            let (proxy, encoded) = match encoded {
                Some((bytes, format, read_back)) => {
                    (read_back.unwrap_or(proxy), Some((bytes, format)))
                }
                None => (proxy, None),
            };
            if writing
                && let Some((bytes, format)) = encoded
                && write_cached(&bytes, format, &self.disk, &key).is_some()
            {
                let _ = metadata::atomic_write(
                    &info,
                    &serde_json::to_vec(&CachedRender {
                        width: meta.width,
                        height: meta.height,
                        warning: warning.clone(),
                    })?,
                );
                // A photo counts once, at its display preview; analysis
                // images add their bytes to it.
                let photo = (!analysis).then_some(media.path.as_path());
                self.note_cache_write(photo, bytes.len() as u64, cancelled);
            }
            proxy
        };
        anyhow::ensure!(
            !cancelled() && Self::key(media)? == source_key,
            "Photo changed or preparation cancelled"
        );
        let decoded = if look {
            let mut rgba = decoded.to_rgba8();
            let levels = super::raw_look::Levels::measure(&rgba);
            self.raw_levels.insert(source_key.clone(), levels);
            levels.apply(&mut rgba);
            DynamicImage::ImageRgba8(rgba)
        } else {
            decoded
        };
        let thumbnail = if analysis {
            None
        } else {
            Some(Arc::new(
                decoded.thumbnail(THUMB_EDGE, THUMB_EDGE).to_rgba8(),
            ))
        };
        let thumbnail = thumbnail.map(|thumbnail| {
            let display_key = if look {
                self.look_key(media, &source_key)
            } else {
                source_key.clone()
            };
            self.save_thumbnail(&display_key, &thumbnail, cancelled)
                .map_or(thumbnail, Arc::new)
        });
        let pixels = Arc::new(decoded.to_rgba8());
        let thumbnail = thumbnail.unwrap_or_else(|| pixels.clone());
        let asset = Asset {
            pixels,
            thumbnail,
            metadata: meta.clone(),
            decoder_warning: warning,
        };
        self.remember(source_key, memory_key, &asset);
        Ok(asset)
    }
    /// An analysis image another pipeline of this cache prepared for `media`
    /// (see `jobs::ahead`), kept as if this pipeline had prepared it: its
    /// metadata, with the geometry the decode found, and the image itself
    /// for later model stages and duplicate review.
    pub fn adopt_analysis(&mut self, media: &Media, asset: &Asset) -> Result<()> {
        let source_key = Self::key(media)?;
        let memory_key = analysis_key(&source_key);
        self.tick += 1;
        self.remember(source_key, memory_key, asset);
        Ok(())
    }
    fn remember(&mut self, source_key: String, memory_key: String, asset: &Asset) {
        self.metadata.put(source_key, asset.metadata.clone());
        let limit = self.preview_limit();
        while self.stats.resident_bytes + asset.bytes() > limit && !self.cache.is_empty() {
            let oldest = self
                .cache
                .iter()
                .min_by_key(|(_, v)| v.tick)
                .map(|(k, _)| k.clone())
                .unwrap();
            if let Some(entry) = self.cache.remove(&oldest) {
                self.stats.resident_bytes -= entry.asset.bytes();
            }
        }
        if asset.bytes() <= limit {
            self.stats.resident_bytes += asset.bytes();
            if let Some(old) = self.cache.insert(
                memory_key,
                Entry {
                    asset: asset.clone(),
                    tick: self.tick,
                },
            ) {
                self.stats.resident_bytes -= old.asset.bytes();
            }
        }
    }
    /// Camera preview (or neutral half-size fallback) shared by display and analysis.
    /// This recipe never supplies original inspection, even if a JPEG has source dimensions.
    fn raw_preview(
        &mut self,
        media: &Media,
        key: &str,
        meta: &mut Metadata,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<Prepared> {
        let cache_key = format!("raw-preview:{key}");
        self.tick += 1;
        if let Some(entry) = self.details.get_mut(&cache_key) {
            entry.tick = self.tick;
            (meta.width, meta.height) = (entry.prepared.width, entry.prepared.height);
            return Ok(entry.prepared.clone());
        }
        self.stats.decodes += 1;
        let decoded =
            decode_raw_recipe(&media.path, RawRecipe::Preview, &mut self.colors, cancelled);
        self.stats.color_transforms = self.colors.conversions;
        self.stats.color_initializations = self.colors.initializations;
        let decoded = decoded?;
        anyhow::ensure!(
            !cancelled() && Self::key(media)? == key,
            "Photo changed or RAW preview cancelled"
        );
        if decoded.embedded {
            self.stats.raw_embedded += 1;
        } else {
            self.stats.raw_fallback += 1;
        }
        (meta.width, meta.height) = decoded.source;
        let prepared = Prepared {
            image: Arc::new(orient(decoded.image, meta.orientation)),
            width: meta.width,
            height: meta.height,
            full_resolution: false,
            warning: None,
        };
        self.retain_detail(&cache_key, &prepared);
        Ok(prepared)
    }
    /// One bounded source cache serves every workflow. Native previews use a
    /// 2048-edge working image; full-resolution decoding is deferred to inspection.
    fn canonical(
        &mut self,
        media: &Media,
        key: &str,
        meta: &mut Metadata,
        original: bool,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<Prepared> {
        self.tick += 1;
        let bounded = native_raster(media) || apple_heif(media);
        let working_key = format!("working:{key}");
        let cache_key = if bounded && !original {
            &working_key
        } else {
            key
        };
        // Keep working and original recipes distinct: analysis must not depend
        // on whether the user opened original detail before asking for scores.
        let alternate = if original { &working_key } else { key };
        let reusable = if self.details.contains_key(cache_key) {
            cache_key
        } else if bounded
            && self.details.get(alternate).is_some_and(|entry| {
                entry.prepared.full_resolution
                    && entry.prepared.width <= PROXY_EDGE
                    && entry.prepared.height <= PROXY_EDGE
                    && entry.prepared.image.color().bits_per_pixel() == 32
            })
        {
            alternate
        } else {
            cache_key
        };
        if let Some(entry) = self.details.get_mut(reusable) {
            entry.tick = self.tick;
            if media.kind != MediaKind::Video {
                (meta.width, meta.height) = (entry.prepared.width, entry.prepared.height);
            }
            return Ok(entry.prepared.clone());
        }
        self.stats.decodes += 1;
        let (raw, width, height, warning, precision_retained, already_oriented) = {
            if native_raster(media) {
                self.stats.native_renders += 1;
                super::playback::locate_runtime()?;
                let rendered = photosort_imaging_vips::render(
                    &media.path,
                    photosort_imaging_vips::Request {
                        edge: (!original).then_some(PROXY_EDGE),
                        region: None,
                        memory: Some(self.colors.allowance()),
                    },
                    cancelled,
                )
                .map_err(|error| vips_failure(error, meta))?;
                let image = native_pixels(rendered.pixels, rendered.width, rendered.height)?;
                (
                    image,
                    rendered.source_width,
                    rendered.source_height,
                    rendered.decoder_warning,
                    original || rendered.source_sample_bytes == 1,
                    false,
                )
            } else {
                let decoded = decode(media, meta, original, &mut self.colors, cancelled)?;
                (
                    decoded.image,
                    decoded.source.0,
                    decoded.source.1,
                    None,
                    true,
                    decoded.oriented,
                )
            }
        };
        self.stats.color_transforms = self.colors.conversions;
        self.stats.color_initializations = self.colors.initializations;
        anyhow::ensure!(
            !cancelled() && Self::key(media)? == key,
            "Photo changed or preparation cancelled"
        );
        let full_resolution =
            precision_retained && (already_oriented || raw.dimensions() == (width, height));
        if media.kind != MediaKind::Video {
            (meta.width, meta.height) = (width, height);
        }
        let image = Arc::new(if media.kind == MediaKind::Video || already_oriented {
            raw
        } else {
            orient(raw, meta.orientation)
        });
        let prepared = Prepared {
            image,
            warning,
            width,
            height,
            full_resolution,
        };
        self.retain_detail(cache_key, &prepared);
        Ok(prepared)
    }
    fn retain_detail(&mut self, cache_key: &str, prepared: &Prepared) {
        let bytes = prepared.image.as_bytes().len();
        self.forget_detail(cache_key);
        let limit = self.detail_limit();
        if bytes <= limit {
            while self.stats.detail_bytes.saturating_add(bytes) > limit && self.evict_detail() {}
            self.stats.detail_bytes += bytes;
        } else if self.pressure == super::memory::Pressure::Critical {
            // The system is nearly out of memory: nothing over budget is kept.
            return;
        } else {
            // Too large for the budget: keep it as the one oversized entry,
            // so its next region or zoom step is not a whole new decode.
            // Smaller working images and tiles stay as they are.
            if let Some(previous) = self.oversized.take() {
                self.details.remove(&previous);
            }
            self.oversized = Some(cache_key.to_owned());
            self.stats.detail_overflow = bytes as u64;
        }
        self.details.insert(
            cache_key.to_owned(),
            DetailEntry {
                prepared: prepared.clone(),
                tick: self.tick,
            },
        );
    }
    fn forget_detail(&mut self, cache_key: &str) {
        if let Some(old) = self.details.remove(cache_key) {
            if self.oversized.as_deref() == Some(cache_key) {
                self.oversized = None;
            } else {
                self.stats.detail_bytes -= old.prepared.image.as_bytes().len();
            }
        }
    }
    /// Drops the least recently used budgeted entry; false when none is left.
    fn evict_detail(&mut self) -> bool {
        let oldest = self
            .details
            .iter()
            .filter(|(key, _)| self.oversized.as_ref() != Some(*key))
            .min_by_key(|(_, entry)| entry.tick)
            .map(|(key, _)| key.clone());
        match oldest {
            Some(key) => {
                self.forget_detail(&key);
                true
            }
            None => false,
        }
    }
    fn inspect_native_region(
        &mut self,
        request: &super::inspection::Request,
        key: &str,
        mut meta: Metadata,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<(Arc<RgbaImage>, Option<String>)> {
        // Keep source-resolution neighborhoods for nearby pans. Scaled fit-view
        // requests cache only their bounded output; both use the same detail LRU.
        const GRID: u32 = 512;
        let r = request.region;
        let right =
            r.x.checked_add(r.width)
                .context("Inspection bounds overflow")?;
        let bottom =
            r.y.checked_add(r.height)
                .context("Inspection bounds overflow")?;
        anyhow::ensure!(
            r.width > 0 && r.height > 0,
            "Inspection region must not be empty"
        );
        let edge = request.output_edge.clamp(1, 4096);
        let native = r.width.max(r.height) <= edge;
        let region_key = if native {
            format!(
                "region:{key}:grid:{},{},{},{}",
                r.x / GRID,
                r.y / GRID,
                right.div_ceil(GRID),
                bottom.div_ceil(GRID)
            )
        } else {
            format!(
                "region:{key}:scaled:{},{},{},{},{edge}",
                r.x, r.y, r.width, r.height
            )
        };
        self.tick += 1;
        let prepared = if let Some(entry) = self.details.get_mut(&region_key) {
            entry.tick = self.tick;
            entry.prepared.clone()
        } else {
            self.stats.decodes += 1;
            self.stats.native_renders += 1;
            self.stats.native_regions += 1;
            super::playback::locate_runtime()?;
            let rendered = photosort_imaging_vips::render_region(
                &request.media.path,
                photosort_imaging_vips::Request {
                    edge: (!native).then_some(edge),
                    memory: Some(self.colors.allowance()),
                    region: Some(photosort_imaging_vips::Region {
                        x: r.x,
                        y: r.y,
                        width: r.width,
                        height: r.height,
                    }),
                },
                meta.orientation,
                if native { GRID } else { 0 },
                cancelled,
            )?;
            let image = native_pixels(rendered.pixels, rendered.width, rendered.height)?;
            let full = if (5..=8).contains(&meta.orientation) {
                (rendered.source_height, rendered.source_width)
            } else {
                (rendered.source_width, rendered.source_height)
            };
            let prepared = Prepared {
                full_resolution: native && r.x < GRID && r.y < GRID && image.dimensions() == full,
                image: Arc::new(image),
                width: rendered.source_width,
                height: rendered.source_height,
                warning: rendered.decoder_warning,
            };
            anyhow::ensure!(
                !cancelled() && Self::key(&request.media)? == key,
                "Photo changed or inspection cancelled"
            );
            // A tile that is actually the entire original can also serve small
            // previews, avoiding an unnecessary second source decode.
            self.retain_detail(
                if prepared.full_resolution {
                    key
                } else {
                    &region_key
                },
                &prepared,
            );
            prepared
        };
        let full = if (5..=8).contains(&meta.orientation) {
            (prepared.height, prepared.width)
        } else {
            (prepared.width, prepared.height)
        };
        anyhow::ensure!(
            right <= full.0 && bottom <= full.1,
            "Inspection region is outside the source image"
        );
        let pixels = if native {
            prepared
                .image
                .crop_imm(r.x % GRID, r.y % GRID, r.width, r.height)
                .into_rgba8()
        } else {
            prepared.image.to_rgba8()
        };
        anyhow::ensure!(
            !cancelled() && Self::key(&request.media)? == key,
            "Photo changed or inspection cancelled"
        );
        (meta.width, meta.height) = (prepared.width, prepared.height);
        self.metadata.put(key.to_owned(), meta);
        Ok((Arc::new(pixels), prepared.warning))
    }
    pub fn inspect(&mut self, request: &super::inspection::Request) -> Result<Arc<RgbaImage>> {
        self.inspect_with_warning(request, &|| false)
            .map(|(pixels, _)| pixels)
    }
    /// Full-size decodes here may use half the memory, or more once the user
    /// chose to open this photo anyway (`allow_large`).
    pub fn inspect_with_warning(
        &mut self,
        request: &super::inspection::Request,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<(Arc<RgbaImage>, Option<String>)> {
        let machine = super::memory::Machine::current();
        // Under memory pressure a large photo asks first instead.
        let quiet = self.pressure.share(machine.quiet_allowance() as usize) as u64;
        self.colors.set_allowance(Some(if request.allow_large {
            machine.forced_allowance()
        } else {
            self.quiet_cap.map_or(quiet, |cap| cap.min(quiet))
        }));
        let result = self.inspect_allowed(request, cancelled);
        self.colors.set_allowance(self.quiet_cap);
        result
    }
    fn inspect_allowed(
        &mut self,
        request: &super::inspection::Request,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<(Arc<RgbaImage>, Option<String>)> {
        anyhow::ensure!(
            request.media.kind != MediaKind::Video,
            "Video frames use the playback service"
        );
        let key = Self::key(&request.media)?;
        // Another photo's oversized original is not kept once this one is
        // inspected: only the photo being looked at stays outside the budget.
        // Organize's face crops (slots with the top bit) are not looking.
        if let Some(oversized) = self.oversized.clone()
            && request.slot & (1 << 63) == 0
            && !oversized.contains(key.as_str())
        {
            self.forget_detail(&oversized);
        }
        let mut meta = self.metadata_for_key(&request.media, &key, cancelled)?;
        if native_raster(&request.media)
            && ![key.clone(), format!("working:{key}")].iter().any(|k| {
                self.details
                    .get(k)
                    .is_some_and(|entry| entry.prepared.full_resolution)
            })
        {
            return self.inspect_native_region(request, &key, meta, cancelled);
        }
        let prepared = self.canonical(&request.media, &key, &mut meta, true, cancelled)?;
        self.metadata.put(key.clone(), meta);
        let image = prepared.image;
        let r = request.region;
        anyhow::ensure!(
            r.width > 0
                && r.height > 0
                && r.x.checked_add(r.width).is_some_and(|x| x <= image.width())
                && r.y
                    .checked_add(r.height)
                    .is_some_and(|y| y <= image.height()),
            "Inspection region is outside the source image"
        );
        let crop = image.crop_imm(r.x, r.y, r.width, r.height);
        let edge = request.output_edge.clamp(1, 4096);
        let mut pixels = if crop.width() > edge || crop.height() > edge {
            crop.resize(edge, edge, image::imageops::FilterType::Triangle)
                .into_rgba8()
        } else {
            crop.into_rgba8()
        };
        if self.enhanced(&request.media) {
            // Same levels as the preview, so zooming in does not shift colour.
            let levels = match self.raw_levels.get(&key) {
                Some(levels) => *levels,
                None => {
                    let levels =
                        super::raw_look::Levels::measure(&image.thumbnail(1024, 1024).to_rgba8());
                    self.raw_levels.insert(key.clone(), levels);
                    levels
                }
            };
            levels.apply(&mut pixels);
        }
        anyhow::ensure!(
            !cancelled() && Self::key(&request.media)? == key,
            "Photo changed or inspection cancelled"
        );
        Ok((Arc::new(pixels), prepared.warning))
    }
    /// The whole photo as shown, upright, no larger than `edge` on either side.
    pub fn render_whole(&mut self, media: &Media, edge: u32) -> Result<Arc<RgbaImage>> {
        self.render_whole_cancellable(media, edge, &|| false)
    }
    /// The same, stopping with an error between decode stages once `cancelled`
    /// returns true, for example when a newer copy replaces this one.
    pub fn render_whole_cancellable(
        &mut self,
        media: &Media,
        edge: u32,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<Arc<RgbaImage>> {
        anyhow::ensure!(media.kind != MediaKind::Video, "Videos have no still image");
        anyhow::ensure!(!cancelled(), "Rendering cancelled");
        let key = Self::key(media)?;
        let mut meta = self.metadata_for_key(media, &key, cancelled)?;
        if let Some(image) = self.bounded_whole(media, &meta, edge, cancelled)? {
            anyhow::ensure!(
                !cancelled() && Self::key(media)? == key,
                "Photo changed or rendering cancelled"
            );
            return Ok(Arc::new(image));
        }
        let edge = edge.clamp(1, 4096);
        // The camera's own JPEG is enough when it covers the copy size, and
        // needs no sensor decode.
        if media.kind == MediaKind::Raw
            && let Ok(preview) = self.raw_preview(media, &key, &mut meta, cancelled)
            && preview.image.width().max(preview.image.height()) >= edge
        {
            return Ok(Arc::new(self.raw_copy(media, &preview, edge)));
        }
        let prepared = match self.canonical(media, &key, &mut meta, true, cancelled) {
            // Too large to decode at full size here: the camera's own
            // preview is used instead, so copying still works.
            Err(error) if error.is::<super::memory::TooLarge>() && media.kind == MediaKind::Raw => {
                let preview = self.raw_preview(media, &key, &mut meta, cancelled)?;
                return Ok(Arc::new(self.raw_copy(media, &preview, edge)));
            }
            other => other?,
        };
        let request = super::inspection::Request {
            slot: 0,
            media: media.clone(),
            region: super::inspection::Region::full(
                prepared.image.width(),
                prepared.image.height(),
            ),
            output_edge: edge,
            allow_large: false,
        };
        self.inspect_with_warning(&request, cancelled)
            .map(|(pixels, _)| pixels)
    }
    /// A RAW's preview at copy size, in the display look.
    fn raw_copy(&self, media: &Media, preview: &Prepared, edge: u32) -> RgbaImage {
        let mut pixels = if preview.image.width() > edge || preview.image.height() > edge {
            preview
                .image
                .resize(edge, edge, image::imageops::FilterType::Triangle)
                .into_rgba8()
        } else {
            preview.image.to_rgba8()
        };
        if self.enhanced(media) {
            super::raw_look::Levels::measure(&pixels).apply(&mut pixels);
        }
        pixels
    }
    /// The whole photo no larger than `edge`, from loaders that shrink while
    /// decoding, so a copy never holds the full original. None for formats
    /// that decode at full size first.
    fn bounded_whole(
        &mut self,
        media: &Media,
        meta: &Metadata,
        edge: u32,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<Option<RgbaImage>> {
        if native_raster(media) {
            self.stats.decodes += 1;
            self.stats.native_renders += 1;
            super::playback::locate_runtime()?;
            let rendered = photosort_imaging_vips::render(
                &media.path,
                photosort_imaging_vips::Request {
                    edge: Some(edge.clamp(1, 4096)),
                    region: None,
                    memory: Some(self.colors.allowance()),
                },
                cancelled,
            )?;
            let image = native_pixels(rendered.pixels, rendered.width, rendered.height)?;
            return Ok(Some(orient(image, meta.orientation).into_rgba8()));
        }
        #[cfg(target_os = "macos")]
        if apple_heif(media) {
            self.stats.decodes += 1;
            let decoded = decode_heif(
                &media.path,
                Some(edge.clamp(1, 4096)),
                self.colors.allowance(),
            )?;
            return Ok(Some(orient(decoded.image, meta.orientation).into_rgba8()));
        }
        Ok(None)
    }
    pub fn clear_memory(&mut self) {
        self.raw_levels.clear();
        self.colors.clear();
        self.cache.clear();
        self.metadata.clear();
        self.details.clear();
        self.oversized = None;
        self.stats.resident_bytes = 0;
        self.stats.detail_bytes = 0;
    }
    pub fn configure_cache(&mut self, mut limits: super::cache::Limits) {
        limits.normalize();
        self.budget = limits.previews_mib as usize * 1024 * 1024;
        self.detail_budget = limits.originals_mib as usize * 1024 * 1024;
        self.disk_budget = u64::from(limits.disk_mib) * super::cache::MIB;
        self.fit_budgets();
    }
    /// The memory budgets as the system's memory pressure allows now.
    fn preview_limit(&self) -> usize {
        self.pressure.share(self.budget)
    }
    fn detail_limit(&self) -> usize {
        self.pressure.share(self.detail_budget)
    }
    /// Follows the system's memory pressure: under pressure the memory caches
    /// keep half or a quarter of their budgets and, when critical, the
    /// oversized full-size image goes; they grow back once it clears. Cheap
    /// when nothing changed.
    pub fn apply_pressure(&mut self, pressure: super::memory::Pressure) {
        if pressure != self.pressure {
            self.pressure = pressure;
            self.fit_budgets();
        }
    }
    fn fit_budgets(&mut self) {
        let limit = self.preview_limit();
        while self.stats.resident_bytes > limit {
            let Some(key) = self
                .cache
                .iter()
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.cache.remove(&key) {
                self.stats.resident_bytes -= e.asset.bytes();
            }
        }
        while self.stats.detail_bytes > self.detail_limit() && self.evict_detail() {}
        if let Some(key) = self.oversized.take()
            && let Some(entry) = self.details.remove(&key)
            && self.pressure < super::memory::Pressure::Critical
        {
            // A larger budget may now hold it like any other entry.
            self.retain_detail(&key, &entry.prepared);
        }
    }
    pub fn maintain_cache(
        &self,
        clear: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<super::cache::Usage> {
        let mut usage = if clear {
            super::cache::maintain(&self.disk, Some(0), cancelled)?
        } else {
            super::cache::maintain_keeping_models(&self.disk, Some(self.disk_budget), cancelled)?
        };
        usage.preview_bytes = self.stats.resident_bytes;
        usage.original_bytes = self.stats.detail_bytes;
        usage.metadata_entries = self.metadata.len();
        Ok(usage)
    }
}
fn native_pixels(
    pixels: photosort_imaging_vips::Pixels,
    width: u32,
    height: u32,
) -> Result<DynamicImage> {
    match pixels {
        photosort_imaging_vips::Pixels::U8(pixels) => Ok(DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(width, height, pixels).context("Invalid RGBA image")?,
        )),
        photosort_imaging_vips::Pixels::U16(pixels) => Ok(DynamicImage::ImageRgba16(
            image::ImageBuffer::from_raw(width, height, pixels)
                .context("Invalid 16-bit RGBA image")?,
        )),
    }
}
// Keep this at the shared proxy boundary: analysis input is always lossless,
// and display previews read back exactly as stored. Original-detail precision
// is untouched.
/// Display previews and grid thumbnails are JPEG at this quality: about a
/// fifth of a PNG, so the same disk limit keeps several times more photos.
const PREVIEW_QUALITY: u8 = 92;
/// A cache image (see `encode_cached` for which are JPEG), marked as used.
/// image-rs decodes it, not libvips: each libvips call has a fixed cost that
/// dominates on small images, and image-rs needs none of libvips' one render
/// lock, which the screen waits on. Median decode times on Windows: a 2048 px
/// preview 13–16 ms against 17–19 ms, a 320 px thumbnail 0.3–0.5 ms against
/// 2.0–2.6 ms. Original photos are the opposite case; see `native_raster`.
fn read_cached_image(disk: &Path, key: &str) -> Option<DynamicImage> {
    ["jpg", "png"].iter().find_map(|ext| {
        let path = disk.join(format!("{key}.{ext}"));
        let image = image::open(&path).ok()?;
        super::cache::touch(&path);
        Some(image)
    })
}
/// Whether `encode_cached` makes this image a JPEG.
fn is_jpeg_preview(image: &DynamicImage, lossy: bool) -> bool {
    lossy && matches!(image, DynamicImage::ImageRgb8(_))
}
/// With `lossy`, an opaque 8-bit image becomes a JPEG and comes back as a
/// later read decodes it; anything else (analysis input, alpha, 16-bit) is a
/// lossless PNG and comes back as None (the pixels are unchanged).
fn encode_cached(
    image: &DynamicImage,
    lossy: bool,
) -> Option<(Vec<u8>, image::ImageFormat, Option<DynamicImage>)> {
    let mut bytes = Vec::new();
    if is_jpeg_preview(image, lossy) {
        image
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut bytes,
                PREVIEW_QUALITY,
            ))
            .ok()?;
        let read_back =
            image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok()?;
        Some((bytes, image::ImageFormat::Jpeg, Some(read_back)))
    } else {
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .ok()?;
        Some((bytes, image::ImageFormat::Png, None))
    }
}
/// Writes an encoded cache image through its own temporary file, so two
/// writers of the same photo (the clipboard has its own pipeline) never
/// publish a torn file and a failed write leaves nothing behind.
fn write_cached(bytes: &[u8], format: image::ImageFormat, disk: &Path, key: &str) -> Option<()> {
    let ext = if format == image::ImageFormat::Jpeg {
        "jpg"
    } else {
        "png"
    };
    let mut temp = tempfile::Builder::new()
        .prefix(&format!("{key}."))
        .suffix(&format!(".tmp.{ext}"))
        .tempfile_in(disk)
        .ok()?;
    std::io::Write::write_all(&mut temp, bytes).ok()?;
    super::replace::persist(temp, &disk.join(format!("{key}.{ext}"))).ok()?;
    Some(())
}
fn compact_opaque_proxy(
    image: DynamicImage,
    cancelled: &impl Fn() -> bool,
) -> Result<DynamicImage> {
    if let DynamicImage::ImageRgba8(rgba) = &image {
        for chunk in rgba.as_raw().chunks(64 * 1024) {
            anyhow::ensure!(!cancelled(), "Image preparation cancelled");
            if chunk.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
                return Ok(image);
            }
        }
        let rgb = image.into_rgb8();
        anyhow::ensure!(!cancelled(), "Image preparation cancelled");
        return Ok(DynamicImage::ImageRgb8(rgb));
    }
    Ok(image)
}

pub fn orient(image: DynamicImage, orientation: u8) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.rotate90().fliph(),
        6 => image.rotate90(),
        7 => image.rotate90().flipv(),
        8 => image.rotate270(),
        _ => image,
    }
}
struct Decoded {
    image: DynamicImage,
    /// Full source dimensions before the shared EXIF/sidecar transform.
    source: (u32, u32),
    oriented: bool,
}
impl Decoded {
    fn full(image: DynamicImage, orientation: u8, oriented: bool) -> Self {
        let source = if oriented && (5..=8).contains(&orientation) {
            (image.height(), image.width())
        } else {
            image.dimensions()
        };
        Self {
            image,
            source,
            oriented,
        }
    }
}
fn decode(
    media: &Media,
    metadata: &Metadata,
    _original: bool,
    colors: &mut super::color::Manager,
    cancelled: &impl Fn() -> bool,
) -> Result<Decoded> {
    if media.kind == MediaKind::Raw {
        return decode_raw_recipe(&media.path, RawRecipe::Original, colors, cancelled)
            .map(|decoded| Decoded::full(decoded.image, metadata.orientation, false));
    }
    if media.kind == MediaKind::Video {
        return video_thumbnail(&media.path, cancelled)
            .map(|image| Decoded::full(image, metadata.orientation, false));
    }
    if media
        .path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
    {
        #[cfg(not(target_os = "macos"))]
        return super::heif::decode(
            &media.path,
            metadata
                .container_orientation
                .context("Cannot read HEIF container orientation")?,
            metadata.orientation,
            colors,
            cancelled,
        )
        .map(|image| Decoded::full(image, metadata.orientation, true));
        #[cfg(target_os = "macos")]
        return decode_heif(
            &media.path,
            (!_original).then_some(PROXY_EDGE),
            colors.allowance(),
        );
    }
    let reader = image::ImageReader::open(&media.path)?.with_guessed_format()?;
    colors
        .decode(reader, cancelled)
        .with_context(|| format!("Cannot decode {}", media.name()))
        .map(|image| Decoded::full(image, metadata.orientation, false))
}
fn video_thumbnail(path: &Path, cancelled: &impl Fn() -> bool) -> Result<DynamicImage> {
    super::playback::thumbnail(path, cancelled)
}
#[derive(Clone, Copy)]
#[repr(i32)]
enum RawRecipe {
    Original = 0,
    Preview = 1,
    PreviewFallback = 2,
}
/// Only a sensor decode is bounded by memory (raw.c); an embedded JPEG is
/// bounded by its own size, so a 150 MP RAW still shows a preview and thumbnail.
fn check_raw_source(width: u32, height: u32) -> Result<()> {
    super::memory::check_size(width, height)
}
/// raw.c: a sensor decode would need more than its allowance.
const RAW_NEEDS_MEMORY: i32 = -200001;
/// raw.c: LibRaw could not size the result safely; never overridden.
const RAW_IMPOSSIBLE_SIZE: i32 = -200002;
/// LibRaw: the file or its mosaic is larger than LibRaw accepts.
const LIBRAW_TOO_BIG: i32 = -100012;
fn raw_failure(error: i32, needed: u64, width: u32, height: u32) -> anyhow::Error {
    match error {
        RAW_NEEDS_MEMORY => super::memory::TooLarge {
            needed,
            width,
            height,
        }
        .into(),
        RAW_IMPOSSIBLE_SIZE => {
            anyhow::anyhow!("This file reports an impossible size ({width} x {height})")
        }
        LIBRAW_TOO_BIG => anyhow::anyhow!(
            "LibRaw cannot read this RAW: the file is over 2 GB or its sensor data is too large"
        ),
        _ => anyhow::anyhow!("LibRaw decode failed ({error})"),
    }
}
struct RawDecoded {
    image: DynamicImage,
    source: (u32, u32),
    embedded: bool,
}
fn decode_raw_recipe(
    path: &Path,
    recipe: RawRecipe,
    colors: &mut super::color::Manager,
    cancelled: &impl Fn() -> bool,
) -> Result<RawDecoded> {
    use std::ffi::{CString, c_char, c_int, c_void};
    #[repr(C)]
    struct RawImage {
        kind: c_int,
        height: u16,
        width: u16,
        colors: u16,
        bits: u16,
        size: u32,
        data: [u8; 1],
    }
    unsafe extern "C" {
        fn photosort_raw_decode(
            path: *const c_char,
            recipe: c_int,
            allowance: u64,
            needed: *mut u64,
            embedded: *mut c_int,
            source_width: *mut u32,
            source_height: *mut u32,
            progress: unsafe extern "C" fn(*mut c_void, c_int, c_int, c_int) -> c_int,
            context: *mut c_void,
            error: *mut c_int,
        ) -> *mut RawImage;
        fn libraw_dcraw_clear_mem(image: *mut RawImage);
    }
    struct Owned(*mut RawImage);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { libraw_dcraw_clear_mem(self.0) }
            }
        }
    }
    let filename = CString::new(path.as_os_str().as_encoded_bytes())?;
    let mut error = 0;
    let mut embedded = 0;
    let mut needed = 0;
    let allowance = colors.allowance();
    let (mut source_width, mut source_height) = (0, 0);
    // The callback is synchronous; contain Rust panics at the native boundary.
    unsafe extern "C" fn progress(context: *mut c_void, _: c_int, _: c_int, _: c_int) -> c_int {
        let cancelled = unsafe { &*(context as *const &dyn Fn() -> bool) };
        i32::from(std::panic::catch_unwind(std::panic::AssertUnwindSafe(cancelled)).unwrap_or(true))
    }
    let callback: &dyn Fn() -> bool = cancelled;
    // SAFETY: C owns decoder lifetime, returns an allocated LibRaw image. The
    // RAII guard frees it after copying pixels; header layout is the stable C ABI.
    let owned = Owned(unsafe {
        photosort_raw_decode(
            filename.as_ptr(),
            recipe as c_int,
            allowance,
            &mut needed,
            &mut embedded,
            &mut source_width,
            &mut source_height,
            progress,
            (&callback as *const &dyn Fn() -> bool).cast_mut().cast(),
            &mut error,
        )
    });
    if owned.0.is_null() {
        return Err(raw_failure(error, needed, source_width, source_height));
    }
    anyhow::ensure!(!cancelled(), "RAW preparation cancelled");
    let raw = unsafe { &*owned.0 };
    // The pixels run past the one-byte `data` field. Take their address from
    // the C pointer: a slice through `raw` may only cover that one byte.
    let data = unsafe { std::ptr::addr_of!((*owned.0).data) }.cast::<u8>();
    check_raw_source(source_width, source_height)?;
    let source = (source_width, source_height);
    if embedded != 0 {
        anyhow::ensure!(
            raw.kind == 1 && raw.size > 0 && raw.size <= 300_000_000,
            "Invalid embedded RAW preview"
        );
        let bytes = unsafe { std::slice::from_raw_parts(data, raw.size as usize) };
        let reader =
            image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Jpeg);
        return match colors.decode(reader, cancelled) {
            Ok(image) => Ok(RawDecoded {
                image,
                source,
                embedded: true,
            }),
            Err(_) if !cancelled() => {
                // A corrupt camera JPEG must not hide an otherwise readable RAW.
                drop(owned);
                decode_raw_recipe(path, RawRecipe::PreviewFallback, colors, cancelled)
            }
            Err(error) => Err(error),
        };
    }
    anyhow::ensure!(
        raw.kind == 2 && raw.bits == 8 && raw.colors == 3,
        "Unsupported LibRaw output"
    );
    let size = usize::from(raw.width) * usize::from(raw.height) * 3;
    anyhow::ensure!(raw.size as usize == size, "Invalid RAW image dimensions");
    let pixels = unsafe { std::slice::from_raw_parts(data, size) }.to_vec();
    Ok(RawDecoded {
        image: DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(raw.width.into(), raw.height.into(), pixels)
                .context("Invalid RAW output")?,
        ),
        source,
        embedded: false,
    })
}

#[cfg(target_os = "macos")]
fn decode_heif(path: &Path, edge: Option<u32>, allowance: u64) -> Result<Decoded> {
    let rendered = super::apple_image::decode(path, edge, allowance)?;
    Ok(Decoded {
        image: DynamicImage::ImageRgba8(rendered.pixels),
        source: rendered.source,
        oriented: false,
    })
}

fn apple_heif(media: &Media) -> bool {
    cfg!(target_os = "macos")
        && media.kind == MediaKind::Photo
        && media
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
}

/// A full-size libvips render refused for memory says what it needs.
fn vips_failure(error: photosort_imaging_vips::Error, meta: &Metadata) -> anyhow::Error {
    match error.needed() {
        Some(needed) => super::memory::TooLarge {
            needed,
            width: meta.width,
            height: meta.height,
        }
        .into(),
        None => error.into(),
    }
}

/// Original JPEG, PNG, TIFF and GIF photos decode through libvips, not image-rs.
///
/// Decoding one JPEG on its own, image-rs is about as fast as libvips or faster,
/// so a single-decode benchmark points the wrong way. The pipeline is where
/// libvips wins: JPEG shrink-on-load, analysis reusing the preview's decode,
/// and zoomed views reading only the visible region. `bench_pipeline` on 188
/// 24 MP camera JPEGs (Windows, Ryzen 7 7800X3D, release build):
///
/// | Stage | image-rs | libvips |
/// | --- | --- | --- |
/// | Cold 2048 px previews | 41.8–42.2 s | 38.5–38.9 s |
/// | Technical analysis | 28.7–29.5 s | 24.9–25.1 s |
/// | Original detail and revisit | 22.1–24.2 s | 2.5 s |
///
/// On 6–29 MB JPEGs previews take the same time, and analysis and original
/// detail are about 8× faster. Smaller JPEGs (embedded RAW previews, cache
/// images) stay on image-rs; see `color::Manager::decode` and
/// `read_cached_image`. Re-measure with
/// `cargo run --release --example bench_pipeline -- <folder>`.
fn native_raster(media: &Media) -> bool {
    media.kind == MediaKind::Photo
        && media.path.extension().is_some_and(|e| {
            ["jpg", "jpeg", "png", "tif", "tiff", "gif"]
                .iter()
                .any(|name| e.eq_ignore_ascii_case(name))
        })
}

#[cfg(test)]
mod metadata_cache_tests {
    use super::*;
    #[test]
    fn metadata_capacity_preserves_recent_photo_without_disk_reread() {
        let directory = tempfile::tempdir().unwrap();
        let make_media = |name: &str| {
            let path = directory.path().join(name);
            image::RgbaImage::new(1, 1).save(&path).unwrap();
            Media {
                id: MediaId::for_path(&path),
                fingerprint: Fingerprint::read(&path).unwrap(),
                kind: MediaKind::Photo,
                path,
            }
        };
        let recent = make_media("recent.png");
        let next = make_media("next.png");
        let mut pipeline = Pipeline::new(1024, directory.path().join("cache"));
        pipeline.metadata(&recent).unwrap();
        // Reach the actual production boundary without 100,000 filesystem reads.
        for index in 1..METADATA_CAPACITY.get() {
            pipeline
                .metadata
                .put(format!("unused-{index}"), Metadata::default());
        }
        pipeline.metadata(&recent).unwrap();
        pipeline.metadata(&next).unwrap();
        let disk_hits = pipeline.stats.metadata_disk_hits;
        pipeline.metadata(&recent).unwrap();
        assert_eq!(
            pipeline.stats.metadata_disk_hits, disk_hits,
            "adding one photo evicted recently used metadata and reread disk"
        );
        assert_eq!(pipeline.stats.metadata_reads, 2);
        assert_eq!(pipeline.stats.decodes, 0);
        assert_eq!(pipeline.metadata.len(), METADATA_CAPACITY.get());
        assert!(!pipeline.metadata.contains("unused-1"));
        assert!(pipeline.metadata.contains("unused-2"));
    }

    #[test]
    #[cfg(unix)]
    fn a_failed_cache_trim_never_fails_the_metadata_read_that_triggered_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("photo.png");
        image::RgbaImage::new(1, 1).save(&path).unwrap();
        let media = Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            kind: MediaKind::Photo,
            path,
        };
        // Records still land, but maintenance refuses a symlinked cache folder.
        let real = directory.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let linked = directory.path().join("cache");
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        let mut pipeline = Pipeline::new(1024, linked);
        pipeline.written_since_prune = pipeline.disk_budget;
        assert!(pipeline.metadata(&media).is_ok());
        assert_eq!(pipeline.written_since_prune, 0, "the trim ran and failed");
    }

    #[test]
    fn whole_render_stops_when_cancelled_and_otherwise_matches() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("photo.png");
        image::RgbaImage::from_fn(40, 20, |x, y| image::Rgba([x as u8, y as u8, 7, 255]))
            .save(&path)
            .unwrap();
        let media = Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            kind: MediaKind::Photo,
            path,
        };
        let cache = directory.path().join("cache");
        let expected = Pipeline::new(1024 * 1024, cache.clone())
            .render_whole(&media, 16)
            .unwrap();
        // Cancelled after the first check: a newer copy replaced this one
        // while it was decoding.
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let later = || checks.fetch_add(1, std::sync::atomic::Ordering::Relaxed) > 0;
        let mut pipeline = Pipeline::new(1024 * 1024, directory.path().join("other"));
        let error = pipeline
            .render_whole_cancellable(&media, 16, &later)
            .unwrap_err();
        assert!(format!("{error:#}").contains("cancelled"), "{error:#}");
        assert_eq!(pipeline.stats.decodes, 0, "stopped before decoding");
        let rendered = Pipeline::new(1024 * 1024, cache)
            .render_whole_cancellable(&media, 16, &|| false)
            .unwrap();
        assert_eq!(*rendered, *expected);
    }

    #[test]
    fn raw_sizes_are_bounded_by_memory_and_refusals_say_why() {
        // Fujifilm GFX100 and a 400 MP pixel-shift file are photos.
        assert!(check_raw_source(11_648, 8_736).is_ok());
        assert!(check_raw_source(23_296, 17_472).is_ok());
        assert!(check_raw_source(0, 8_736).is_err());
        let refused = raw_failure(RAW_NEEDS_MEMORY, 3 << 30, 11_648, 8_736);
        assert!(
            refused
                .downcast_ref::<crate::core::memory::TooLarge>()
                .is_some()
        );
        assert!(
            raw_failure(LIBRAW_TOO_BIG, 0, 0, 0)
                .to_string()
                .contains("over 2 GB")
        );
    }

    #[test]
    fn persisted_metadata_rejects_changed_timezone_and_invalid_values() {
        let mut record = CachedMetadata::new(Metadata {
            captured_ms: Some(1_700_000_000_000),
            orientation: 1,
            ..Default::default()
        });
        assert!(record.valid());
        record.capture_offset = record.capture_offset.map(|offset| offset + 3600);
        assert!(!record.valid());
        record.capture_offset = CachedMetadata::offset(&record.metadata);
        record.metadata.rating = 6;
        assert!(!record.valid());
        record.metadata.rating = 5;
        record.metadata.orientation = 0;
        assert!(!record.valid());
    }
}
