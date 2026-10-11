//! Developer alignment benchmark. Decoding is excluded from timed measurements.
use anyhow::{Context, Result, ensure};
use photosort::core::{
    catalog::Catalog,
    near_duplicates::{Aligner, Gray},
    pipeline::Pipeline,
    pixels,
};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::Path,
    time::{Duration, Instant},
};

fn write_plane(
    path: &Path,
    width: u32,
    height: u32,
    values: impl Iterator<Item = f32>,
) -> Result<()> {
    let mut out = std::io::BufWriter::new(fs::File::create(path)?);
    out.write_all(b"PSF1")?;
    out.write_all(&width.to_le_bytes())?;
    out.write_all(&height.to_le_bytes())?;
    for value in values {
        out.write_all(&value.to_le_bytes())?;
    }
    out.flush()?;
    Ok(())
}
fn read_plane(path: &Path) -> Result<(Vec<f32>, usize, usize)> {
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() >= 12 && &bytes[..4] == b"PSF1",
        "Invalid float plane"
    );
    let w = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let h = u32::from_le_bytes(bytes[8..12].try_into()?) as usize;
    ensure!(
        w.checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| n.checked_add(12))
            == Some(bytes.len()),
        "Invalid plane dimensions"
    );
    Ok((
        bytes[12..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v))
            .collect(),
        w,
        h,
    ))
}
fn prepare(output: &Path, samples: &Path) -> Result<()> {
    ensure!(!output.exists(), "Choose a new corpus directory");
    fs::create_dir_all(output)?;
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/near_duplicate");
    let mut sources = Vec::new();
    for entry in fs::read_dir(fixtures)? {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "png")
            || path
                .file_stem()
                .unwrap()
                .to_str()
                .unwrap()
                .parse::<usize>()
                .is_err()
        {
            continue;
        }
        let name = format!("fixture-{}", path.file_stem().unwrap().to_str().unwrap());
        let im = image::open(&path)?.to_luma8();
        write_plane(
            &output.join(format!("{name}.f32")),
            im.width(),
            im.height(),
            im.pixels().map(|p| f32::from(p[0]) / 255.),
        )?;
    }
    let catalog = Catalog::scan(samples, || false, |_| {})?;
    ensure!(
        catalog.media.len() == 7,
        "Expected the seven original samples"
    );
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().to_owned());
    for (index, media) in catalog.media.iter().enumerate() {
        let before = blake3::hash(&fs::read(&media.path)?);
        let asset = pipeline.analysis(media, || false)?;
        let name = format!("sample-{index}");
        write_plane(
            &output.join(format!("{name}.f32")),
            asset.pixels.width(),
            asset.pixels.height(),
            asset
                .pixels
                .pixels()
                .map(|p| f32::from(pixels::luminance(p[0], p[1], p[2])) / 255.),
        )?;
        ensure!(
            blake3::hash(&fs::read(&media.path)?) == before,
            "Sample changed"
        );
        sources.push(json!({"id":name,"name":media.name(),"blake3":before.to_hex().to_string(),"width":asset.pixels.width(),"height":asset.pixels.height()}));
    }
    fs::write(
        output.join("sources.json"),
        serde_json::to_vec_pretty(&sources)?,
    )?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("prepare") => prepare(
            Path::new(args.get(2).context("output directory")?),
            Path::new(args.get(3).context("sample directory")?),
        ),
        Some("run") => {
            let manifest = Path::new(args.get(2).context("manifest")?);
            let repeats: usize = args.get(3).context("repetitions")?.parse()?;
            ensure!(repeats > 0 && repeats <= 100, "Invalid repetitions");
            let cancel_ms: u64 = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(2);
            let mut aligner = Aligner::default();
            for line in fs::read_to_string(manifest)?.lines() {
                let columns: Vec<_> = line.split('\t').collect();
                ensure!(columns.len() == 3, "Invalid manifest row");
                let root = manifest.parent().unwrap();
                let (a, w, h) = read_plane(&root.join(columns[1]))?;
                let (b, bw, bh) = read_plane(&root.join(columns[2]))?;
                ensure!((w, h) == (bw, bh), "Mismatched images");
                let a = Gray::new(&a, w, h).context("Invalid first image")?;
                let b = Gray::new(&b, w, h).context("Invalid second image")?;
                let result = aligner
                    .prepare(a, b, || false)
                    .map(|r| (r.alignment(), r.overlap()));
                let mut ms = Vec::new();
                for _ in 0..repeats {
                    let start = Instant::now();
                    let output = aligner.prepare(a, b, || false);
                    ensure!(
                        output.is_some() == result.is_some(),
                        "Non-deterministic alignment acceptance"
                    );
                    drop(std::hint::black_box(output));
                    ms.push(start.elapsed().as_secs_f64() * 1000.);
                }
                let start = Instant::now();
                let cancelled =
                    aligner.prepare(a, b, || start.elapsed() >= Duration::from_millis(cancel_ms));
                println!(
                    "{}",
                    json!({"id":columns[0],"engine":"rust","ms":ms,"alignment":result.map(|r|r.0),"overlap":result.map(|r|r.1),"cancel_deadline_ms":cancel_ms,"cancel_ms":start.elapsed().as_secs_f64()*1000.,"cancel_returned_alignment":cancelled.is_some()})
                );
            }
            Ok(())
        }
        _ => anyhow::bail!(
            "Usage: bench_alignment prepare OUT SAMPLES | run MANIFEST REPEATS [CANCEL_MS]"
        ),
    }
}
