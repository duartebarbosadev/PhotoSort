fn main() {
    if std::env::var_os("CARGO_FEATURE_METADATA_NATIVE").is_some() {
        println!("cargo:rerun-if-changed=native/metadata.cpp");
        let library = pkg_config::Config::new().cargo_metadata(false).atleast_version("0.28").probe("exiv2").expect("Install Exiv2 (macOS: brew install exiv2; Ubuntu: libexiv2-dev), or disable metadata-native");
        let mut build = cc::Build::new();
        build.cpp(true).std("c++17").file("native/metadata.cpp");
        for include in library.include_paths {
            build.include(include);
        }
        build.compile("photosort_metadata");
        // ELF linkers may discard shared libraries seen before their static callers.
        pkg_config::Config::new()
            .atleast_version("0.28")
            .probe("exiv2")
            .expect("Exiv2 link configuration");
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=native/file_attributes.c");
        cc::Build::new()
            .file("native/file_attributes.c")
            .compile("photosort_file_attributes");
        println!("cargo:rerun-if-changed=native/application.m");
        cc::Build::new()
            .file("native/application.m")
            .compile("photosort_application");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rerun-if-changed=native/video.m");
        cc::Build::new()
            .file("native/video.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("photosort_video");
        for framework in [
            "AVFoundation",
            "CoreMedia",
            "CoreVideo",
            "CoreGraphics",
            "Accelerate",
        ] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && std::env::var_os("CARGO_FEATURE_HEIF").is_some()
    {
        println!("cargo:rerun-if-changed=native/apple_image.m");
        println!("cargo:rerun-if-changed=native/apple_image.h");
        cc::Build::new()
            .file("native/apple_image.m")
            .compile("photosort_apple_image");
        for framework in ["Foundation", "CoreFoundation", "CoreGraphics", "ImageIO"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos")
        && std::env::var_os("CARGO_FEATURE_HEIF").is_some()
    {
        println!("cargo:rerun-if-changed=native/heif_metadata.c");
        let library = pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version("1.23")
            .probe("libheif")
            .expect("Install libheif >= 1.23 for portable HEIF support");
        println!("cargo:rustc-env=PHOTOSORT_HEIF_VERSION={}", library.version);
        let mut build = cc::Build::new();
        build.file("native/heif_metadata.c");
        for include in library.include_paths {
            build.include(include);
        }
        build.compile("photosort_heif_metadata");
        pkg_config::Config::new()
            .atleast_version("1.23")
            .probe("libheif")
            .expect("HEIF link configuration");
    }

    println!("cargo:rerun-if-changed=native/raw.c");
    if std::env::var_os("CARGO_FEATURE_RAW").is_some() {
        let library = pkg_config::Config::new().cargo_metadata(false).atleast_version("0.21").probe("libraw").expect("Install LibRaw (macOS: brew install libraw; Ubuntu: libraw-dev), or build --no-default-features for raster-only support");
        let mut build = cc::Build::new();
        build.file("native/raw.c");
        for include in &library.include_paths {
            build.include(include);
        }
        build.compile("photosort_raw");
        for path in library.link_paths {
            println!("cargo:rustc-link-search=native={}", path.display());
        }
        for lib in library.libs {
            let name = if lib == "stdc++"
                && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
            {
                "c++"
            } else {
                &lib
            };
            println!("cargo:rustc-link-lib={name}");
        }
    }
}
