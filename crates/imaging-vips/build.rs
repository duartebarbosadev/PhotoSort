fn main() {
    println!("cargo:rerun-if-changed=native/imaging.c");
    let mut build = cc::Build::new();
    build.file("native/imaging.c").std("c11").warnings(true);
    let dependencies = [("vips", "8.18"), ("lcms2", "2.16")];
    for (name, version) in dependencies {
        let library = pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version(version)
            .probe(name)
            .unwrap_or_else(|e| {
                panic!("Install {name} >= {version}, or set PKG_CONFIG_PATH to its prefix: {e}")
            });
        let key = if name == "vips" {
            "PHOTOSORT_VIPS_VERSION"
        } else {
            "PHOTOSORT_LCMS_VERSION"
        };
        println!("cargo:rustc-env={key}={}", library.version);
        for include in library.include_paths {
            build.include(include);
        }
    }
    // Private builds publish codec versions and options (including SIMD) in the
    // pkg-config recipe. Rebuild when it changes, even at the same prefix.
    if let Ok(directory) = pkg_config::get_variable("vips", "pcfiledir") {
        println!("cargo:rerun-if-changed={directory}/vips.pc");
    }
    let recipe = pkg_config::get_variable("vips", "photosort_recipe")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "external".into());
    assert!(
        recipe == "external"
            || (recipe.len() == 64 && recipe.bytes().all(|b| b.is_ascii_hexdigit())),
        "Invalid native image build recipe"
    );
    let target = std::env::var("TARGET").expect("Cargo build target");
    println!("cargo:rustc-env=PHOTOSORT_VIPS_BUILD_ID={target}:{recipe}");
    build.compile("photosort_imaging_vips");
    // Keep dynamic dependencies after their static caller for ELF --as-needed linking.
    for (name, version) in dependencies {
        pkg_config::Config::new()
            .atleast_version(version)
            .probe(name)
            .unwrap_or_else(|error| panic!("Native image link configuration for {name}: {error}"));
    }
}
