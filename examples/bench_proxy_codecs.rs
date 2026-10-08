//! Lossless codec experiment on actual shared display/analysis disk proxies.
//! Preparation and equality checks are excluded from in-memory codec timings.
use anyhow::{Result, ensure};
use image::{
    DynamicImage, ImageEncoder, ImageFormat,
    codecs::{
        png::{CompressionType, FilterType, PngEncoder},
        webp::WebPEncoder,
    },
};
use photosort::core::{catalog::Catalog, model::MediaKind, pipeline::Pipeline};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};

#[derive(Clone, Copy)]
enum Codec {
    PngFastAdaptive,
    PngFastPaeth,
    PngLevel1Adaptive,
    WebpLossless,
}
impl Codec {
    fn name(self) -> &'static str {
        match self {
            Self::PngFastAdaptive => "png-fast-adaptive",
            Self::PngFastPaeth => "png-fast-paeth",
            Self::PngLevel1Adaptive => "png-level1-adaptive",
            Self::WebpLossless => "webp-lossless",
        }
    }
    fn format(self) -> ImageFormat {
        if matches!(self, Self::WebpLossless) {
            ImageFormat::WebP
        } else {
            ImageFormat::Png
        }
    }
    fn encode(self, image: &DynamicImage) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let (width, height, color) = (image.width(), image.height(), image.color().into());
        if matches!(self, Self::WebpLossless) {
            // Call the typed encoder directly: never silently quantize 16-bit input.
            WebPEncoder::new_lossless(&mut bytes).write_image(
                image.as_bytes(),
                width,
                height,
                color,
            )?;
        } else {
            let (compression, filter) = match self {
                Self::PngFastAdaptive => (CompressionType::Fast, FilterType::Adaptive),
                Self::PngFastPaeth => (CompressionType::Fast, FilterType::Paeth),
                Self::PngLevel1Adaptive => (CompressionType::Level(1), FilterType::Adaptive),
                Self::WebpLossless => unreachable!(),
            };
            PngEncoder::new_with_quality(&mut bytes, compression, filter).write_image(
                image.as_bytes(),
                width,
                height,
                color,
            )?;
        }
        Ok(bytes)
    }
}

fn measure(label: &str, image: &DynamicImage, sequence: usize) -> Result<Value> {
    let codecs = [
        Codec::PngFastAdaptive,
        Codec::PngFastPaeth,
        Codec::PngLevel1Adaptive,
        Codec::WebpLossless,
    ];
    let high_depth = image.color().bits_per_pixel() / image.color().channel_count() as u16 > 8;
    let expected = image.to_rgba16();
    let mut measurements = Vec::new();
    for iteration in 0..3 {
        // Rotate ordering so the same encoder does not always run first.
        for offset in 0..codecs.len() {
            let codec = codecs[(sequence + iteration + offset) % codecs.len()];
            if high_depth && matches!(codec, Codec::WebpLossless) {
                continue;
            }
            let start = Instant::now();
            let bytes = codec.encode(image)?;
            let encode_ms = start.elapsed().as_secs_f64() * 1000.;
            let start = Instant::now();
            let decoded = image::load_from_memory_with_format(&bytes, codec.format())?;
            let decode_ms = start.elapsed().as_secs_f64() * 1000.;
            ensure!(
                decoded.to_rgba16() == expected,
                "{} changed pixels for {label}",
                codec.name()
            );
            measurements.push(json!({"codec":codec.name(),"iteration":iteration+1,
                "bytes":bytes.len(),"encode_ms":encode_ms,"decode_ms":decode_ms}));
        }
    }
    Ok(
        json!({"proxy":label,"dimensions":[image.width(),image.height()],
        "color":format!("{:?}",image.color()),"exact_pixels":true,
        "webp_unsupported_high_depth":high_depth,"measurements":measurements}),
    )
}

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("folder required"))?,
    )
    .canonicalize()?;
    ensure!(args.next().is_none(), "use one read-only photo folder");
    let mut catalog = Catalog::scan(&root, || false, |_| {})?;
    ensure!(
        catalog.warnings.is_empty(),
        "discovery warnings: {:?}",
        catalog.warnings
    );
    catalog.media.retain(|media| media.kind != MediaKind::Video);
    ensure!(!catalog.media.is_empty(), "empty photo library");
    let mut rows = Vec::new();
    let mut preparation_ms = 0.;
    for (index, media) in catalog.media.iter().enumerate() {
        let start = Instant::now();
        let cache = tempfile::tempdir()?;
        let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
        let display_key = Pipeline::key(media)?;
        pipeline.load(media)?;
        pipeline.analysis(media, || false)?;
        drop(pipeline);
        let mut paths = std::fs::read_dir(cache.path())?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
            .collect::<Vec<_>>();
        paths.sort();
        ensure!(
            paths.len() == 2,
            "expected display and analysis disk proxies"
        );
        preparation_ms += start.elapsed().as_secs_f64() * 1000.;
        for path in paths {
            let image = image::open(&path)?;
            let purpose = if path.file_stem().unwrap() == display_key.as_str() {
                "display"
            } else {
                "analysis"
            };
            let label = format!("{}:{purpose}", media.path.strip_prefix(&root)?.display());
            rows.push(measure(&label, &image, rows.len())?);
        }
        eprintln!(
            "prepared and verified {}/{}",
            index + 1,
            catalog.media.len()
        );
    }
    // Include hidden RGB under alpha=0, partial alpha, and full 16-bit precision.
    let alpha = DynamicImage::ImageRgba8(image::RgbaImage::from_fn(257, 193, |x, y| {
        image::Rgba([
            x as u8,
            y as u8,
            (x ^ y) as u8,
            [0, 127, 255][(x + y) as usize % 3],
        ])
    }));
    let deep = DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(257, 193, |x, y| {
        image::Rgba([
            (x * 251) as u16,
            (y * 337) as u16,
            ((x ^ y) * 199) as u16,
            [0, 32767, 65535][(x + y) as usize % 3],
        ])
    }));
    rows.push(measure("synthetic:rgba8-alpha", &alpha, rows.len())?);
    rows.push(measure("synthetic:rgba16-alpha", &deep, rows.len())?);
    println!(
        "{}",
        json!({"photo_count":catalog.media.len(),"preparation_ms":preparation_ms,
        "native_imaging":cfg!(feature="native-imaging"),"repetitions":3,"rows":rows})
    );
    Ok(())
}
