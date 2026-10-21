//! Measures two performance budgets on temporary copies:
//! time to first thumbnail on a real folder, and frame time while scrolling a
//! 10,000-photo grid. Usage: perf_budget <photo-folder> [grid-count]
//! Nothing outside temporary directories is written.
use eframe::egui;
use egui_kittest::Harness;
use photosort::{core::model::Step, ui::PhotoSortApp};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("photo folder"))?);
    let count: usize = args
        .next()
        .map(|v| v.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(10_000);
    let data = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    // SAFETY: set before any other thread exists.
    unsafe {
        std::env::set_var("PHOTOSORT_DATA_ROOT", data.path());
        std::env::set_var("PHOTOSORT_CACHE_ROOT", cache.path());
    }

    // 1. Time to first thumbnail on copies of the real photos, cold cache.
    let library = tempfile::tempdir()?;
    let mut copied = 0;
    for entry in walkdir::WalkDir::new(&source).into_iter().flatten() {
        let path = entry.path();
        // Copy every file; the catalog decides what it supports.
        if entry.file_type().is_file()
            && let Some(name) = path.file_name()
        {
            let name = format!("{copied:05}-{}", name.to_string_lossy());
            std::fs::copy(path, library.path().join(name))?;
            copied += 1;
        }
    }
    for cache in ["cold", "warm", "warm, second reopen"] {
        let (first, all) = time_to_thumbnails(library.path())?;
        println!(
            "first thumbnail: {:.0} ms · 24 thumbnails (first screen): {:.0} ms ({copied} files, {cache} cache)",
            first.as_secs_f64() * 1000.0,
            all.as_secs_f64() * 1000.0
        );
    }

    // 2. Grid frame time with `count` small generated photos.
    let grid = tempfile::tempdir()?;
    for i in 0..count {
        let shade = (i % 251) as u8;
        let image = image::RgbImage::from_fn(160, 120, |x, y| {
            image::Rgb([shade, (x as u8).wrapping_add(shade), (y as u8) ^ shade])
        });
        image.save(grid.path().join(format!("{i:05}.jpg")))?;
    }
    let folder = grid.path().to_path_buf();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
    settle(&mut harness, Duration::from_secs(120));
    harness.state_mut().gallery_step(Step::Cull);
    harness.state_mut().gallery_review(false);
    settle(&mut harness, Duration::from_secs(60));
    let mut frames = Vec::new();
    harness.hover_at(egui::pos2(700.0, 500.0));
    for i in 0..600 {
        let delta = if i < 300 { -120.0 } else { 120.0 };
        harness.event(egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, delta),
            modifiers: egui::Modifiers::NONE,
        });
        let start = Instant::now();
        harness.step();
        frames.push(start.elapsed());
        if i == 299 {
            // Proof the grid moved: the visible tiles after scrolling down.
            harness
                .render()
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .save(std::env::temp_dir().join("perf-scrolled.png"))?;
        }
    }
    frames.sort();
    let pct = |p: f64| frames[((frames.len() as f64 - 1.0) * p) as usize].as_secs_f64() * 1000.0;
    println!(
        "grid scroll ({count} photos, 1440×900, CPU frame): median {:.2} ms · p95 {:.2} ms · max {:.2} ms",
        pct(0.5),
        pct(0.95),
        pct(1.0)
    );
    // 3. Thumbnails filling the screen after a long jump down the grid.
    settle(&mut harness, Duration::from_secs(60));
    for _ in 0..40 {
        harness.event(egui::Event::MouseWheel {
            phase: egui::TouchPhase::Move,
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -2000.0),
            modifiers: egui::Modifiers::NONE,
        });
    }
    let start = Instant::now();
    harness.step();
    while harness.state().gallery_busy() && start.elapsed() < Duration::from_secs(60) {
        harness.step();
        std::thread::sleep(Duration::from_millis(2));
    }
    println!(
        "thumbnails after a jump: {:.0} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Time to the first thumbnail, and to a full first screen (24 tiles at 1440×900).
fn time_to_thumbnails(folder: &Path) -> anyhow::Result<(Duration, Duration)> {
    let folder = folder.to_path_buf();
    let start = Instant::now();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
    harness.state_mut().gallery_step(Step::Cull);
    let mut first = None;
    while start.elapsed() < Duration::from_secs(120) {
        harness.step();
        let textures = harness.state().gallery_textures();
        if std::env::var_os("PERF_TRACE").is_some() {
            eprintln!(
                "{:>6} ms textures {textures} busy {}",
                start.elapsed().as_millis(),
                harness.state().gallery_busy()
            );
        }
        if first.is_none() && textures > 0 {
            first = Some(start.elapsed());
        }
        if textures >= 24 {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let first = first.ok_or_else(|| anyhow::anyhow!("no thumbnail within 120 s"))?;
    let screen = start.elapsed();
    // Let metadata finish so the next (warm) run sees the same sorted screen.
    settle(&mut harness, Duration::from_secs(120));
    Ok((first, screen))
}

fn settle(harness: &mut Harness<'_, PhotoSortApp>, limit: Duration) {
    let start = Instant::now();
    let mut quiet = 0;
    while start.elapsed() < limit {
        harness.step();
        if harness.state().gallery_busy() {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet > 10 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
