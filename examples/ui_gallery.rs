//! Headless screenshot gallery of the real application UI.
//! Usage: ui_gallery <photo-folder> <new-output-dir> [max-photos]
//! Photos are copied into a temporary library; data and cache roots are
//! temporary too, so nothing outside the output directory is written.
use eframe::egui;
use egui_kittest::Harness;
use photosort::{core::model::Step, ui::PhotoSortApp};
use std::{path::PathBuf, time::Duration};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("photo folder"))?);
    let out = PathBuf::from(args.next().ok_or_else(|| anyhow::anyhow!("output dir"))?);
    let limit: usize = args
        .next()
        .map(|v| v.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(36);
    std::fs::create_dir(&out)?;
    let data = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    // SAFETY: set before any other thread exists.
    unsafe {
        std::env::set_var("PHOTOSORT_DATA_ROOT", data.path());
        std::env::set_var("PHOTOSORT_CACHE_ROOT", cache.path());
    }
    let library = tempfile::tempdir()?;
    let mut photos: Vec<_> = walkdir::WalkDir::new(&source)
        .into_iter()
        .flatten()
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
        })
        .map(|e| e.into_path())
        .collect();
    photos.sort();
    for (i, photo) in photos.iter().take(limit).enumerate() {
        let name = format!("{i:03}-{}", photo.file_name().unwrap().to_string_lossy());
        std::fs::copy(photo, library.path().join(name))?;
    }

    let shots: &[(&str, Option<Step>, bool, [f32; 2])] = &[
        (
            "01-cull-grid-wide",
            Some(Step::Cull),
            false,
            [1440.0, 900.0],
        ),
        (
            "02-cull-review-wide",
            Some(Step::Cull),
            true,
            [1440.0, 900.0],
        ),
        (
            "03-organize-wide",
            Some(Step::Organize),
            false,
            [1440.0, 900.0],
        ),
        (
            "03b-organize-by-date-wide",
            Some(Step::Organize),
            false,
            [1440.0, 900.0],
        ),
        (
            "04-easy-delete-wide",
            Some(Step::EasyDelete),
            true,
            [1440.0, 900.0],
        ),
        (
            "05-rotation-wide",
            Some(Step::Rotation),
            true,
            [1440.0, 900.0],
        ),
        (
            "06-pick-best-wide",
            Some(Step::PickBest),
            true,
            [1440.0, 900.0],
        ),
        (
            "02b-cull-review-left-strip",
            Some(Step::Cull),
            true,
            [1440.0, 900.0],
        ),
        (
            "02c-cull-grid-no-names",
            Some(Step::Cull),
            false,
            [1440.0, 900.0],
        ),
        (
            "02d-cull-review-left-strip-no-names",
            Some(Step::Cull),
            true,
            [1440.0, 900.0],
        ),
        (
            "07-cull-grid-compact",
            Some(Step::Cull),
            false,
            [900.0, 700.0],
        ),
        (
            "08-easy-delete-compact",
            Some(Step::EasyDelete),
            true,
            [900.0, 700.0],
        ),
        (
            "09-pick-best-compact",
            Some(Step::PickBest),
            true,
            [900.0, 700.0],
        ),
        (
            "10-organize-compact",
            Some(Step::Organize),
            false,
            [900.0, 700.0],
        ),
    ];
    for (name, step, review, size) in shots {
        let folder = library.path().to_path_buf();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(size[0], size[1]))
            .with_pixels_per_point(2.0)
            .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
        settle(&mut harness);
        harness.state_mut().gallery_seed_reviews();
        harness
            .state_mut()
            .gallery_left_strip(name.contains("left-strip"));
        harness
            .state_mut()
            .gallery_show_filenames(!name.contains("no-names"));
        if let Some(step) = step {
            harness.state_mut().gallery_step(*step);
            settle(&mut harness);
            if name.contains("by-date") {
                harness.state_mut().gallery_organize_by_date();
                settle(&mut harness);
            }
            harness.state_mut().gallery_review(*review);
            settle(&mut harness);
            harness.state_mut().gallery_close_model_dialog();
            settle(&mut harness);
        }
        let image = harness.render().map_err(|e| anyhow::anyhow!("{e}"))?;
        image.save(out.join(format!("{name}.png")))?;
        eprintln!("{name}");
    }
    // Dialogs over the Cull grid.
    for (name, open) in [
        ("11-settings", Dialog::Settings),
        ("12-plan-review", Dialog::PlanReview),
        ("15-about", Dialog::About),
        ("16-organize-editor", Dialog::OrganizeEditor),
        ("17-cull-compare", Dialog::Compare),
        ("18-ai-models", Dialog::Models),
    ] {
        let folder = library.path().to_path_buf();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_pixels_per_point(2.0)
            .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
        settle(&mut harness);
        harness.state_mut().gallery_step(Step::Cull);
        settle(&mut harness);
        match open {
            Dialog::Settings => harness.state_mut().gallery_settings(),
            Dialog::PlanReview => harness.state_mut().gallery_plan_review(3),
            Dialog::About => harness.state_mut().gallery_about(),
            Dialog::Compare => harness.state_mut().gallery_compare(3),
            Dialog::Models => harness.state_mut().gallery_models(),
            Dialog::OrganizeEditor => {
                harness.state_mut().gallery_step(Step::Organize);
                settle(&mut harness);
                harness.state_mut().gallery_organize_editor();
            }
        }
        settle(&mut harness);
        let image = harness.render().map_err(|e| anyhow::anyhow!("{e}"))?;
        image.save(out.join(format!("{name}.png")))?;
        eprintln!("{name}");
    }
    // The welcome screen, before any folder is chosen.
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .build_eframe(|cc| PhotoSortApp::new(cc, None));
    settle(&mut harness);
    harness
        .render()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .save(out.join("00-welcome.png"))?;
    // The notice after a crash in the previous session.
    photosort::core::crash::write_report(
        &photosort::core::crash::directory(),
        "main",
        "gallery example",
        "examples/ui_gallery.rs",
        "",
    )?;
    let folder = library.path().to_path_buf();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
    settle(&mut harness);
    harness
        .render()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .save(out.join("13-crash-notice.png"))?;
    eprintln!("13-crash-notice");
    Ok(())
}

enum Dialog {
    Settings,
    PlanReview,
    About,
    OrganizeEditor,
    Compare,
    Models,
}

/// Step frames until background work settles (or ~20 s pass).
fn settle(harness: &mut Harness<'_, PhotoSortApp>) {
    let mut quiet = 0;
    for _ in 0..400 {
        harness.step();
        if harness.state().gallery_busy() {
            quiet = 0;
        } else {
            quiet += 1;
            if quiet > 10 {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
