//! Compare the previous full-original inspection allocation with bounded regions.
#[cfg(feature = "native-imaging")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use image::DynamicImage;
    use photosort::core::{
        inspection::{Region, Request},
        model::*,
        pipeline::{Pipeline, orient},
    };
    use std::{path::PathBuf, sync::Arc, time::Instant};
    let args: Vec<_> = std::env::args().collect();
    let mode = args.get(1).context("mode: generate, full, or regions")?;
    let path = PathBuf::from(args.get(2).context("image path")?);
    if mode == "generate" {
        ensure!(!path.exists(), "Choose a new fixture path");
        image::RgbaImage::from_fn(6000, 4000, |x, y| {
            image::Rgba([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8, 255])
        })
        .save(path)?;
        return Ok(());
    }
    ensure!(matches!(mode.as_str(), "full" | "regions"), "Unknown mode");
    let media = Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path)?,
        kind: MediaKind::from_path(&path).context("photo")?,
        path: path.clone(),
    };
    let source_hash = blake3::hash(&std::fs::read(&path)?);
    let disk = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, disk.path().into());
    let start = Instant::now();
    let preview = pipeline.load(&media)?;
    let orientation = preview.metadata.orientation;
    let (width, height) = if (5..=8).contains(&orientation) {
        (preview.metadata.height, preview.metadata.width)
    } else {
        (preview.metadata.width, preview.metadata.height)
    };
    ensure!(width >= 800 && height >= 600, "Use a large fixture");
    let mut legacy = None;
    let mut hashes = Vec::new();
    let mut times = Vec::new();
    for step in Step::ALL {
        for pan in [0, 16, 32] {
            let request = Request {
                slot: step.index() as u64,
                media: media.clone(),
                region: Region {
                    x: (width - 640) / 2 + pan,
                    y: (height - 480) / 2,
                    width: 640,
                    height: 480,
                },
                output_edge: 640,
            };
            let begin = Instant::now();
            let pixels = if mode == "full" {
                if legacy.is_none() {
                    let native =
                        photosort_imaging_vips::render(&path, Default::default(), || false)?;
                    let image = match native.pixels {
                        photosort_imaging_vips::Pixels::U8(v) => DynamicImage::ImageRgba8(
                            image::RgbaImage::from_raw(native.width, native.height, v)
                                .context("RGBA8")?,
                        ),
                        photosort_imaging_vips::Pixels::U16(v) => DynamicImage::ImageRgba16(
                            image::ImageBuffer::from_raw(native.width, native.height, v)
                                .context("RGBA16")?,
                        ),
                    };
                    legacy = Some(orient(image, orientation));
                }
                let r = request.region;
                Arc::new(
                    legacy
                        .as_ref()
                        .unwrap()
                        .crop_imm(r.x, r.y, r.width, r.height)
                        .into_rgba8(),
                )
            } else {
                pipeline.inspect(&request)?
            };
            times.push(begin.elapsed().as_secs_f64() * 1000.);
            hashes.push(blake3::hash(pixels.as_raw()).to_hex().to_string());
        }
    }
    let elapsed = start.elapsed().as_secs_f64() * 1000.;
    ensure!(
        blake3::hash(&std::fs::read(&path)?) == source_hash,
        "Source changed"
    );
    std::hint::black_box(&legacy);
    println!(
        "{}",
        serde_json::json!({"mode":mode,"source_dimensions":[width,height],"elapsed_ms":elapsed,"inspection_ms":times,"pixel_hashes":hashes,"source_blake3":source_hash.to_hex().to_string(),"pipeline_decodes":pipeline.stats.decodes,"native_regions":pipeline.stats.native_regions,"pipeline_detail_bytes":pipeline.stats.detail_bytes,"legacy_retained_bytes":legacy.as_ref().map(|i|i.as_bytes().len()).unwrap_or(0)})
    );
    Ok(())
}
#[cfg(not(feature = "native-imaging"))]
fn main() {
    eprintln!("Build with --features native-imaging");
}
