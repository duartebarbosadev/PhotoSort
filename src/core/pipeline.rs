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
    budget: usize,
    detail_budget: usize,
    disk_budget: u64,
    tick: u64,
    pub stats: Stats,
    disk: PathBuf,
    written_since_prune: u64,
    raw_look: super::raw_look::RawLook,
    /// Enhanced-look levels per RAW source, so 1:1 crops match the preview.
    raw_levels: HashMap<String, super::raw_look::Levels>,
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
            budget,
            detail_budget: budget.saturating_mul(2),
            disk_budget: 1024 * 1024 * 1024,
            tick: 0,
            stats: Stats::default(),
            disk,
            written_since_prune: 0,
            raw_look: Default::default(),
            raw_levels: HashMap::new(),
        }
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
                "{fingerprint:?}-{sidecar:?}-v7-render-status-srgb-neutral-analysis-native_metadata={}",
                cfg!(feature = "metadata-native")
            )
            .as_bytes(),
        );
        #[cfg(feature = "native-imaging")]
        hash.update(
            format!(
                "vips-working-2048-v2:{}:{}:{}",
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
            #[cfg(all(feature = "portable-video", not(target_os = "macos")))]
            hash.update(b"gstreamer-rgba-poster-v1");
            hash.update(b"movie-source-geometry-v2");
        }
        #[cfg(all(feature = "heif", target_os = "macos"))]
        if media
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
        {
            hash.update(b"imageio-bounded-straight-alpha-v2");
        }
        #[cfg(all(feature = "heif", not(target_os = "macos")))]
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
                "metadata-v6-subsec:heif={}:os={}:{key}",
                cfg!(feature = "heif"),
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
            self.written_since_prune += bytes.len() as u64;
            if self.written_since_prune >= self.disk_budget.min(32 * 1024 * 1024) {
                super::cache::maintain_keeping_models(
                    &self.disk,
                    Some(self.disk_budget),
                    cancelled,
                )?;
                self.written_since_prune = 0;
            }
        }
        // A cached model result can skip image preparation entirely. Restore
        // source geometry from the render descriptor without decoding pixels,
        // while keeping the persisted EXIF snapshot separate and unchanged.
        if media.kind == MediaKind::Raw || apple_heif(media) {
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
        std::fs::read(self.disk.join(format!("{key}.json")))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<CachedRender>(&bytes).ok())
            .filter(|v| {
                v.width > 0 && v.height > 0 && v.warning.as_ref().is_none_or(|w| w.len() <= 4096)
            })
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
        let image = image::open(self.disk.join(format!("{}.png", thumb_key(&display_key)))).ok()?;
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
    fn save_thumbnail(&self, source_key: &str, thumbnail: &RgbaImage) {
        if self.disk_budget == 0 || std::fs::create_dir_all(&self.disk).is_err() {
            return;
        }
        let path = self.disk.join(format!("{}.png", thumb_key(source_key)));
        if path.exists() {
            return;
        }
        let temp = path.with_extension("tmp.png");
        // Opaque tiles are stored as RGB; the PNG is about a quarter smaller.
        let image = DynamicImage::ImageRgba8(thumbnail.clone());
        let image = if thumbnail.pixels().all(|p| p[3] == 255) {
            DynamicImage::ImageRgb8(image.into_rgb8())
        } else {
            image
        };
        if image.save(&temp).is_ok() && std::fs::rename(&temp, &path).is_err() {
            let _ = std::fs::remove_file(&temp);
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
        let disk = self.disk.join(format!("{key}.png"));
        let info = self.disk.join(format!("{key}.json"));
        let saved = self.render_info(&key);
        let warning;
        let decoded = if let Some(saved) = saved
            && let Ok(proxy) = image::open(&disk)
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
                oriented.resize_exact(
                    (f64::from(width) * scale).round().max(1.0) as u32,
                    (f64::from(height) * scale).round().max(1.0) as u32,
                    image::imageops::FilterType::Lanczos3,
                )
            } else if oriented.width() > edge || oriented.height() > edge {
                if analysis {
                    oriented.resize(edge, edge, image::imageops::FilterType::Lanczos3)
                } else {
                    // The same pixels as resize(.., Triangle), without its
                    // full-size float buffer.
                    super::resample::fit_triangle(&oriented, edge)
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
            if self.disk_budget > 0 && std::fs::create_dir_all(&self.disk).is_ok() {
                let temp = disk.with_extension("tmp.png");
                if proxy.save(&temp).is_ok() && std::fs::rename(temp, &disk).is_ok() {
                    let _ = metadata::atomic_write(
                        &info,
                        &serde_json::to_vec(&CachedRender {
                            width: meta.width,
                            height: meta.height,
                            warning: warning.clone(),
                        })?,
                    );
                    self.written_since_prune += std::fs::metadata(&disk).map_or(0, |m| m.len());
                    if self.written_since_prune >= self.disk_budget.min(32 * 1024 * 1024) {
                        let _ = self.prune_disk(self.disk_budget);
                        self.written_since_prune = 0;
                    }
                }
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
        if let Some(thumbnail) = &thumbnail {
            let display_key = if look {
                self.look_key(media, &source_key)
            } else {
                source_key.clone()
            };
            self.save_thumbnail(&display_key, thumbnail);
        }
        let pixels = Arc::new(decoded.to_rgba8());
        let thumbnail = thumbnail.unwrap_or_else(|| pixels.clone());
        let asset = Asset {
            pixels,
            thumbnail,
            metadata: meta.clone(),
            decoder_warning: warning,
        };
        self.metadata.put(source_key, meta);
        while self.stats.resident_bytes + asset.bytes() > self.budget && !self.cache.is_empty() {
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
        if asset.bytes() <= self.budget {
            self.stats.resident_bytes += asset.bytes();
            self.cache.insert(
                memory_key,
                Entry {
                    asset: asset.clone(),
                    tick: self.tick,
                },
            );
        }
        Ok(asset)
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
            #[cfg(feature = "native-imaging")]
            if native_raster(media) {
                self.stats.native_renders += 1;
                let rendered = photosort_imaging_vips::render(
                    &media.path,
                    photosort_imaging_vips::Request {
                        edge: (!original).then_some(PROXY_EDGE),
                        region: None,
                    },
                    cancelled,
                )?;
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
            #[cfg(not(feature = "native-imaging"))]
            {
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
        // Oversized sources/regions must not evict useful working images.
        if bytes <= self.detail_budget {
            if let Some(old) = self.details.remove(cache_key) {
                self.stats.detail_bytes -= old.prepared.image.as_bytes().len();
            }
            while self.stats.detail_bytes.saturating_add(bytes) > self.detail_budget
                && !self.details.is_empty()
            {
                let oldest = self
                    .details
                    .iter()
                    .min_by_key(|(_, entry)| entry.tick)
                    .map(|(key, _)| key.clone())
                    .unwrap();
                if let Some(entry) = self.details.remove(&oldest) {
                    self.stats.detail_bytes -= entry.prepared.image.as_bytes().len();
                }
            }
            self.stats.detail_bytes += bytes;
            self.details.insert(
                cache_key.to_owned(),
                DetailEntry {
                    prepared: prepared.clone(),
                    tick: self.tick,
                },
            );
        }
    }
    #[cfg(feature = "native-imaging")]
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
            let rendered = photosort_imaging_vips::render_region(
                &request.media.path,
                photosort_imaging_vips::Request {
                    edge: (!native).then_some(edge),
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
    pub fn inspect_with_warning(
        &mut self,
        request: &super::inspection::Request,
        cancelled: &(impl Fn() -> bool + Sync),
    ) -> Result<(Arc<RgbaImage>, Option<String>)> {
        anyhow::ensure!(
            request.media.kind != MediaKind::Video,
            "Video frames use the playback service"
        );
        let key = Self::key(&request.media)?;
        let mut meta = self.metadata_for_key(&request.media, &key, cancelled)?;
        #[cfg(feature = "native-imaging")]
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
    pub fn clear_memory(&mut self) {
        self.raw_levels.clear();
        self.colors.clear();
        self.cache.clear();
        self.metadata.clear();
        self.details.clear();
        self.stats.resident_bytes = 0;
        self.stats.detail_bytes = 0;
    }
    pub fn configure_cache(&mut self, mut limits: super::cache::Limits) {
        limits.normalize();
        self.budget = limits.previews_mib as usize * 1024 * 1024;
        self.detail_budget = limits.originals_mib as usize * 1024 * 1024;
        self.disk_budget = u64::from(limits.disk_mib) * super::cache::MIB;
        while self.stats.resident_bytes > self.budget {
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
        while self.stats.detail_bytes > self.detail_budget {
            let Some(key) = self
                .details
                .iter()
                .min_by_key(|(_, e)| e.tick)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(e) = self.details.remove(&key) {
                self.stats.detail_bytes -= e.prepared.image.as_bytes().len();
            }
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
    pub fn prune_disk(&self, max_bytes: u64) -> Result<()> {
        super::cache::maintain_keeping_models(&self.disk, Some(max_bytes), || false).map(|_| ())
    }
}
#[cfg(feature = "native-imaging")]
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
// Keep this at the shared proxy boundary: display and analysis in every workflow
// use the same lossless representation. Original-detail precision is untouched.
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
        #[cfg(all(feature = "heif", not(target_os = "macos")))]
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
        #[cfg(any(not(feature = "heif"), target_os = "macos"))]
        return decode_heif(&media.path, (!_original).then_some(PROXY_EDGE));
    }
    let reader = image::ImageReader::open(&media.path)?.with_guessed_format()?;
    colors
        .decode(reader, cancelled)
        .with_context(|| format!("Cannot decode {}", media.name()))
        .map(|image| Decoded::full(image, metadata.orientation, false))
}
#[cfg(any(target_os = "macos", feature = "portable-video"))]
fn video_thumbnail(path: &Path, cancelled: &impl Fn() -> bool) -> Result<DynamicImage> {
    super::playback::thumbnail(path, cancelled)
}
#[cfg(all(not(target_os = "macos"), not(feature = "portable-video")))]
fn video_thumbnail(path: &Path, cancelled: &impl Fn() -> bool) -> Result<DynamicImage> {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("frame.png");
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-threads", "1", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            "scale=1280:1280:force_original_aspect_ratio=decrease",
        ])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Video thumbnails need FFmpeg on PATH")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "FFmpeg could not decode this video");
            break;
        }
        if cancelled() || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Video thumbnail cancelled or timed out after 10 seconds");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(image::open(output)?)
}
#[derive(Clone, Copy)]
#[repr(i32)]
enum RawRecipe {
    Original = 0,
    Preview = 1,
    #[cfg(feature = "raw")]
    PreviewFallback = 2,
}
struct RawDecoded {
    image: DynamicImage,
    source: (u32, u32),
    embedded: bool,
}
#[cfg(feature = "raw")]
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
            &mut embedded,
            &mut source_width,
            &mut source_height,
            progress,
            (&callback as *const &dyn Fn() -> bool).cast_mut().cast(),
            &mut error,
        )
    });
    anyhow::ensure!(!owned.0.is_null(), "LibRaw decode failed ({error})");
    anyhow::ensure!(!cancelled(), "RAW preparation cancelled");
    let raw = unsafe { &*owned.0 };
    anyhow::ensure!(
        source_width > 0
            && source_height > 0
            && u64::from(source_width) * u64::from(source_height) <= 100_000_000,
        "Invalid RAW source dimensions"
    );
    let source = (source_width, source_height);
    if embedded != 0 {
        anyhow::ensure!(
            raw.kind == 1 && raw.size > 0 && raw.size <= 300_000_000,
            "Invalid embedded RAW preview"
        );
        let bytes = unsafe { std::slice::from_raw_parts(raw.data.as_ptr(), raw.size as usize) };
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
    anyhow::ensure!(
        raw.size as usize == size && size <= 300_000_000,
        "Invalid RAW image dimensions"
    );
    let pixels = unsafe { std::slice::from_raw_parts(raw.data.as_ptr(), size) }.to_vec();
    Ok(RawDecoded {
        image: DynamicImage::ImageRgb8(
            image::RgbImage::from_raw(raw.width.into(), raw.height.into(), pixels)
                .context("Invalid RAW output")?,
        ),
        source,
        embedded: false,
    })
}
#[cfg(not(feature = "raw"))]
fn decode_raw_recipe(
    _: &Path,
    _: RawRecipe,
    _: &mut super::color::Manager,
    _: &impl Fn() -> bool,
) -> Result<RawDecoded> {
    anyhow::bail!("This build has no LibRaw support. Build with the raw feature.")
}

#[cfg(not(feature = "heif"))]
fn decode_heif(_: &Path, _: Option<u32>) -> Result<Decoded> {
    anyhow::bail!("This build has no HEIF support. Build with the heif feature.")
}

#[cfg(all(feature = "heif", target_os = "macos"))]
fn decode_heif(path: &Path, edge: Option<u32>) -> Result<Decoded> {
    let rendered = super::apple_image::decode(path, edge)?;
    Ok(Decoded {
        image: DynamicImage::ImageRgba8(rendered.pixels),
        source: rendered.source,
        oriented: false,
    })
}

fn apple_heif(media: &Media) -> bool {
    cfg!(all(target_os = "macos", feature = "heif"))
        && media.kind == MediaKind::Photo
        && media
            .path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
}

fn native_raster(media: &Media) -> bool {
    cfg!(feature = "native-imaging")
        && media.kind == MediaKind::Photo
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
