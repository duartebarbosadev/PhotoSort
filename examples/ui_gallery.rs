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
                    .is_some_and(|x| {
                        matches!(
                            x.to_ascii_lowercase().as_str(),
                            "jpg" | "jpeg" | "png" | "arw"
                        )
                    })
        })
        .map(|e| e.into_path())
        .collect();
    photos.sort();
    // A plain folder name for the title bar; photos keep their own names unless
    // two sources share one.
    let root = library.path().join("Photos");
    std::fs::create_dir(&root)?;
    for (i, photo) in photos.iter().take(limit).enumerate() {
        let name = photo.file_name().unwrap().to_string_lossy();
        let mut target = root.join(name.as_ref());
        if target.exists() {
            target = root.join(format!("{i:03}-{name}"));
        }
        std::fs::copy(photo, target)?;
    }

    write_demo_photos(&root)?;

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
            "03c-organize-compare-wide",
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
            "04b-easy-delete-grid-wide",
            Some(Step::EasyDelete),
            false,
            [1440.0, 900.0],
        ),
        (
            "05-rotation-wide",
            Some(Step::Rotation),
            true,
            [1440.0, 900.0],
        ),
        (
            "05b-rotation-grid-wide",
            Some(Step::Rotation),
            false,
            [1440.0, 900.0],
        ),
        (
            "06b-pick-best-grid-wide",
            Some(Step::PickBest),
            false,
            [1440.0, 900.0],
        ),
        (
            "06c-pick-best-review-left-strip-no-names",
            Some(Step::PickBest),
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
            "08b-easy-delete-grid-compact",
            Some(Step::EasyDelete),
            false,
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
        (
            "03e-organize-grouping-wide",
            Some(Step::Organize),
            false,
            [1440.0, 900.0],
        ),
        (
            "10b-organize-grouping-compact",
            Some(Step::Organize),
            false,
            [900.0, 700.0],
        ),
        (
            "03f-organize-destination-wide",
            Some(Step::Organize),
            false,
            [1440.0, 900.0],
        ),
        (
            "03h-organize-shortcuts-wide",
            Some(Step::Organize),
            false,
            [1440.0, 900.0],
        ),
        (
            "03g-organize-faces-wide",
            Some(Step::Organize),
            false,
            [1440.0, 900.0],
        ),
        (
            "06d-pick-best-preparing-wide",
            Some(Step::PickBest),
            false,
            [1440.0, 900.0],
        ),
        (
            "01b-cull-analyzing-wide",
            Some(Step::Cull),
            false,
            [1440.0, 900.0],
        ),
    ];
    // Each scene reopens the same folder, which restores its saved session, so a
    // scene that leaves staged folder changes must run last (see the end).
    let shoot = |folder: &std::path::Path,
                 name: &str,
                 step: Option<Step>,
                 review: bool,
                 size: [f32; 2]|
     -> anyhow::Result<()> {
        let folder = folder.to_path_buf();
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
            harness.state_mut().gallery_step(step);
            settle(&mut harness);
            if name.contains("by-date") {
                harness.state_mut().gallery_organize_by_date();
                settle(&mut harness);
            }
            if name.contains("faces") {
                harness.state_mut().gallery_organize_by_faces();
                settle(&mut harness);
                harness.state_mut().gallery_face_portraits();
                settle(&mut harness);
                harness.state_mut().gallery_select(1);
                settle(&mut harness);
            }
            harness.state_mut().gallery_review(review);
            settle(&mut harness);
            harness.state_mut().gallery_close_model_dialog();
            settle(&mut harness);
            if name.contains("compare") {
                harness.state_mut().gallery_select(3);
                settle(&mut harness);
            }
            if name.contains("destination") {
                // The destination chip in the Organize toolbar opens its popover.
                use egui_kittest::kittest::Queryable;
                let chip = ["Organize into", "Move to"]
                    .into_iter()
                    .find_map(|label| harness.query_all_by_label_contains(label).next())
                    .ok_or_else(|| anyhow::anyhow!("destination chip"))?;
                chip.click();
                for _ in 0..5 {
                    harness.step();
                }
            }
            if name.contains("preparing") {
                harness.state_mut().gallery_preparing_model();
                for _ in 0..5 {
                    harness.step();
                }
            }
            if name.contains("shortcuts") {
                // The Shortcuts button in the bottom bar opens its sheet.
                use egui_kittest::kittest::Queryable;
                harness.get_by_label("Keyboard shortcuts").click();
                for _ in 0..5 {
                    harness.step();
                }
            }
            if name.contains("grouping") || name.contains("analyzing") {
                // A pretend run: settling would wait for it to finish.
                harness.state_mut().gallery_running_analysis();
                for _ in 0..5 {
                    harness.step();
                }
            }
        }
        let image = harness.render().map_err(|e| anyhow::anyhow!("{e}"))?;
        image.save(out.join(format!("{name}.png")))?;
        eprintln!("{name}");
        Ok(())
    };
    for (name, step, review, size) in shots {
        shoot(&root, name, *step, *review, *size)?;
    }
    // Dialogs over the Cull grid.
    for (name, open) in [
        ("11-settings", Dialog::Settings),
        ("15-about", Dialog::About),
        ("16-organize-new-folder", Dialog::OrganizeNewFolder),
        ("17-cull-compare", Dialog::Compare),
        ("18-ai-models", Dialog::Models),
        // Last: it stages Trash marks, which the folder's saved session keeps.
        ("12-plan-review", Dialog::PlanReview),
    ] {
        let folder = root.clone();
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
            Dialog::OrganizeNewFolder => {
                harness.state_mut().gallery_step(Step::Organize);
                settle(&mut harness);
                harness.state_mut().gallery_organize_by_folders();
                settle(&mut harness);
                harness.state_mut().gallery_organize_new_folder();
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
    // The first-run intro over the welcome screen, once the film shows a frame
    // and the card has faded in.
    harness.state_mut().gallery_intro();
    for _ in 0..400 {
        harness.step();
        if harness.state().gallery_intro_ready() {
            break;
        }
        pump_main();
    }
    for _ in 0..40 {
        harness.step();
        pump_main();
    }
    harness
        .render()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .save(out.join("19-intro.png"))?;
    eprintln!("19-intro");
    // Grouping by date stages folder changes, and the scenes above stage Trash
    // marks. Each folder restores its own session, so use a separate copy.
    let dated = tempfile::tempdir()?;
    let dated_root = dated.path().join("Photos");
    std::fs::create_dir(&dated_root)?;
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            std::fs::copy(entry.path(), dated_root.join(entry.file_name()))?;
        }
    }
    shoot(
        &dated_root,
        "03b-organize-by-date-wide",
        Some(Step::Organize),
        false,
        [1440.0, 900.0],
    )?;
    // A date grouping, where most photos move: held backslash shows the
    // folders as they are now, then one review covers moves and Trash marks.
    let folder = dated_root.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
    settle(&mut harness);
    harness.state_mut().gallery_step(Step::Organize);
    settle(&mut harness);
    harness.state_mut().gallery_organize_by_date();
    settle(&mut harness);
    harness.key_down(egui::Key::Backslash);
    settle(&mut harness);
    harness
        .render()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .save(out.join("16b-organize-original-by-date.png"))?;
    eprintln!("16b-organize-original-by-date");
    harness.key_up(egui::Key::Backslash);
    settle(&mut harness);
    harness.state_mut().gallery_plan_review(2);
    settle(&mut harness);
    harness
        .render()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .save(out.join("16c-organize-review-changes.png"))?;
    eprintln!("16c-organize-review-changes");
    // Photos from several folders gathered into a new one: its card groups
    // them by the folder each comes from. A library with subfolders, so the
    // photos really come from different folders.
    let sources = tempfile::tempdir()?;
    let sources_root = sources.path().join("Photos");
    let samples: Vec<_> = std::fs::read_dir(&root)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .collect();
    for (i, folder) in ["2023-04-28/group-1", "2023-04-28/group-2", "2024-04-18"]
        .into_iter()
        .enumerate()
    {
        let dir = sources_root.join(folder);
        std::fs::create_dir_all(&dir)?;
        for sample in samples.iter().skip(i * 2).take(2) {
            std::fs::copy(sample, dir.join(sample.file_name().unwrap()))?;
        }
    }
    let folder = sources_root.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(2.0)
        .build_eframe(|cc| PhotoSortApp::new(cc, Some(folder)));
    settle(&mut harness);
    harness.state_mut().gallery_step(Step::Organize);
    settle(&mut harness);
    harness.state_mut().gallery_organize_by_folders();
    settle(&mut harness);
    harness.state_mut().gallery_organize_gather(3);
    settle(&mut harness);
    harness
        .render()
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .save(out.join("16d-organize-review-sources.png"))?;
    eprintln!("16d-organize-review-sources");
    // The notice after a crash in the previous session.
    photosort::core::crash::write_report(
        &photosort::core::crash::directory(),
        "main",
        "gallery example",
        "examples/ui_gallery.rs",
        "",
    )?;
    let folder = root.clone();
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
    OrganizeNewFolder,
    Compare,
    Models,
}

/// AVFoundation finishes loading on the main run loop, which a headless
/// harness never runs by itself (as in `video_probe`).
fn pump_main() {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::c_void;
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            static kCFRunLoopDefaultMode: *const c_void;
            fn CFRunLoopRunInMode(
                mode: *const c_void,
                seconds: f64,
                return_after_source: u8,
            ) -> i32;
        }
        // SAFETY: called on the main thread with the system's default mode.
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.025, 0);
        }
    }
    #[cfg(not(target_os = "macos"))]
    std::thread::sleep(Duration::from_millis(25));
}

/// Demo photos for the review scenes, derived from tests/samples and named as a
/// camera would name them (see `PhotoSortApp::gallery_seed_reviews`): for Easy
/// Delete an exact copy of the harbour, a softer, slightly shifted copy of the
/// island, an out-of-focus desert, an underexposed and an overexposed frame; a
/// sideways Chrysler Building for Fix Rotation, and a three-frame burst of the
/// street where only the first frame is sharp.
fn write_demo_photos(root: &std::path::Path) -> anyhow::Result<()> {
    use image::{DynamicImage, imageops::FilterType};
    let open = |name: &str| -> anyhow::Result<DynamicImage> {
        let image = image::open(root.join(name))?;
        // Demo copies need no more than 3000 pixels, and blurring stays fast.
        Ok(image.resize(3000, 3000, FilterType::Triangle))
    };
    let save = |image: &DynamicImage, name: &str| -> anyhow::Result<()> {
        let file = std::fs::File::create(root.join(name))?;
        let mut writer = std::io::BufWriter::new(file);
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, 90)
            .encode_image(&image.to_rgb8())?;
        Ok(())
    };
    let crop = |image: &DynamicImage, x: f32, y: f32| {
        let (w, h) = (image.width() as f32, image.height() as f32);
        image.crop_imm(
            (w * x) as u32,
            (h * y) as u32,
            (w * 0.96) as u32,
            (h * 0.96) as u32,
        )
    };
    // What a real library also holds: system files and other documents.
    std::fs::write(root.join(".DS_Store"), b"\0\0\0\x01Bud1")?;
    std::fs::write(root.join("notes.json"), b"{}")?;
    if root.join("20230428_035435_IMG_4666.JPG").is_file() {
        std::fs::copy(
            root.join("20230428_035435_IMG_4666.JPG"),
            root.join("20230428_035435_IMG_4666 copy.JPG"),
        )?;
    }
    if root.join("20230428_042445_IMG_4667.JPG").is_file() {
        save(
            &open("20230428_042445_IMG_4667.JPG")?.brighten(-110),
            "DSC01310.jpg",
        )?;
    }
    if root.join("20230428_034640_IMG_4658.JPG").is_file() {
        save(
            &open("20230428_034640_IMG_4658.JPG")?.brighten(120),
            "DSC01320.jpg",
        )?;
    }
    if root.join("GAB00915.jpg").is_file() {
        let island = open("GAB00915.jpg")?;
        save(&crop(&island, 0.03, 0.02).blur(4.0), "GAB00915 (2).jpg")?;
    }
    if root.join("GAB02000-min.png").is_file() {
        save(&open("GAB02000-min.png")?.blur(9.0), "GAB02000-2.jpg")?;
    }
    if root.join("20230428_034640_IMG_4658.JPG").is_file() {
        // A quarter turn counterclockwise; the seeded suggestion turns it back.
        let tower = open("20230428_034640_IMG_4658.JPG")?.rotate270();
        save(&tower, "DSC01180.jpg")?;
    }
    if root.join("jpg_sample.jpg").is_file() {
        let street = open("jpg_sample.jpg")?;
        save(&crop(&street, 0.02, 0.02), "DSC01231.jpg")?;
        save(&crop(&street, 0.03, 0.025).blur(6.0), "DSC01232.jpg")?;
        save(&crop(&street, 0.035, 0.03).blur(10.0), "DSC01233.jpg")?;
    }
    Ok(())
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
