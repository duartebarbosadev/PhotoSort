//! Every integration test builds into this one binary, so the library and its
//! dependencies are linked once instead of once per file. Cargo no longer finds
//! test files by itself (`autotests = false`): list each new file here.
#[path = "../src/test_support.rs"]
mod test_support;

mod analysis_inputs;
mod atomic_writes;
mod browse_worker;
mod browsing;
mod cache;
mod case_rename;
mod color_management;
#[path = "core.rs"]
mod core_api;
mod crash;
mod delete_review;
mod details;
mod directories;
mod duplicate_batch;
mod embedded_metadata;
mod existing_names;
mod eyes;
mod eyes_sample;
mod face_geometry;
mod face_groups;
mod file_case_rename;
mod folder_trash;
mod grouping;
mod heif_alpha;
mod heif_portable;
mod heif_preview;
mod hidden_inventory;
mod inspection;
mod manual_groups;
mod metadata_cache;
mod metadata_native;
mod model_setup;
mod model_storage;
mod native_imaging;
mod native_regions;
mod navigation;
mod near_alignment;
mod near_duplicates;
mod organize;
mod organize_drag;
mod organize_names;
mod organize_trash;
mod persistent_analysis;
mod pick_review;
mod pixel_rotation;
mod playback;
mod preferences;
mod processing;
mod raw_previews;
mod recovery;
mod reference_samples;
mod regional;
mod regional_store;
mod rotation_cache;
mod rotation_review;
mod selection;
mod startup_maintenance;
mod subject_store;
mod subjects;
mod technical_reference;
mod thresholds;
mod video_dates;
mod views;

#[test]
fn every_test_file_is_part_of_this_binary() {
    let root = include_str!("integration.rs");
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for entry in std::fs::read_dir(tests).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        if let Some(module) = name.strip_suffix(".rs")
            && module != "integration"
        {
            assert!(
                root.contains(&format!("mod {module};")) || root.contains(&format!("\"{name}\"")),
                "tests/{name} is not listed in tests/integration.rs"
            );
        }
    }
}
