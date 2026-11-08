fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // The app, its tests and examples all pass UTF-8 paths to native libraries.
        let manifest = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("native")
            .join("photosort.manifest");
        println!("cargo:rerun-if-changed=native/photosort.manifest");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        // GStreamer is not on PATH after its installer runs, and a missing DLL must not stop
        // PhotoSort from starting. Its DLLs load on first use instead, after
        // core::playback::portable has pointed Windows at the installation.
        for dll in [
            "gstreamer-1.0-0.dll",
            "gstbase-1.0-0.dll",
            "gstapp-1.0-0.dll",
            "gstvideo-1.0-0.dll",
            "gobject-2.0-0.dll",
            "glib-2.0-0.dll",
            "gio-2.0-0.dll",
        ] {
            println!("cargo:rustc-link-arg=/DELAYLOAD:{dll}");
        }
        println!("cargo:rustc-link-lib=delayimp");
    }

    println!("cargo:rerun-if-changed=native/metadata.cpp");
    let library = Native::find(
        "exiv2",
        "exiv2",
        "0.28",
        "Install Exiv2 (macOS: brew install exiv2; Ubuntu: libexiv2-dev; Windows: cargo vcpkg build)",
    );
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("native/metadata.cpp");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Without /EHsc, MSVC does not run destructors while an Exiv2 exception
        // unwinds to the bridge's catch: the metadata mutex's lock_guard would
        // stay locked after the first unreadable file, and every later read would hang.
        build.flag("/EHsc");
    }
    library.configure(&mut build);
    build.compile("photosort_metadata");
    // ELF linkers may discard shared libraries seen before their static callers.
    library.link();
    if library.vcpkg {
        // Exiv2 reports file and process details through psapi; its .pc names it, vcpkg does not.
        println!("cargo:rustc-link-lib=psapi");
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
        println!("cargo:rerun-if-changed=native/clipboard.m");
        cc::Build::new()
            .file("native/clipboard.m")
            .flag("-fobjc-arc")
            .compile("photosort_clipboard");
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

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=native/apple_image.m");
        println!("cargo:rerun-if-changed=native/apple_image.h");
        cc::Build::new()
            .file("native/apple_image.m")
            .compile("photosort_apple_image");
        for framework in ["Foundation", "CoreFoundation", "CoreGraphics", "ImageIO"] {
            println!("cargo:rustc-link-lib=framework={framework}");
        }
    }

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        println!("cargo:rerun-if-changed=native/heif_metadata.c");
        let mut library = Native::find(
            "libheif",
            "libheif",
            "1.23",
            "Install libheif >= 1.23 (Ubuntu: libheif-dev; Windows: cargo vcpkg build)",
        );
        if library.vcpkg {
            // vcpkg builds static libraries; their headers otherwise declare DLL imports.
            library.defines.push(("LIBHEIF_STATIC_BUILD".into(), None));
            library.defines.push(("LIBDE265_STATIC_BUILD".into(), None));
        }
        println!("cargo:rustc-env=PHOTOSORT_HEIF_VERSION={}", library.version);
        let mut build = cc::Build::new();
        build.file("native/heif_metadata.c");
        library.configure(&mut build);
        build.compile("photosort_heif_metadata");
        // With MSVC, libheif-sys links libheif itself. Its vcpkg request (libheif[aom]) also
        // compiles libheif's optional codec plugins into heif.lib, and the vcpkg crate does not
        // follow feature dependencies, so link whichever codec libraries vcpkg installed.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
            library.link();
        } else if library.vcpkg {
            link_vcpkg_optional(&["libde265", "aom", "x265", "dav1d", "kvazaar", "openjpeg"]);
        }
    }

    println!("cargo:rerun-if-changed=native/raw.c");
    let library = Native::find(
        "libraw",
        "libraw",
        "0.21",
        "Install LibRaw (macOS: brew install libraw; Ubuntu: libraw-dev; Windows: cargo vcpkg build)",
    );
    let mut build = cc::Build::new();
    build.file("native/raw.c");
    library.configure(&mut build);
    build.compile("photosort_raw");
    if library.vcpkg {
        library.link();
    } else {
        let library = pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version("0.21")
            .probe("libraw")
            .expect("LibRaw link configuration");
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

/// Headers and compile definitions of one native library, from vcpkg on Windows (MSVC)
/// when `cargo vcpkg build` has installed it, otherwise from pkg-config.
struct Native {
    pkg_config: &'static str,
    port: &'static str,
    version: String,
    minimum: &'static str,
    include_paths: Vec<std::path::PathBuf>,
    defines: Vec<(String, Option<String>)>,
    vcpkg: bool,
}

impl Native {
    fn find(
        pkg_config: &'static str,
        port: &'static str,
        minimum: &'static str,
        hint: &str,
    ) -> Self {
        if let Some(found) = Self::find_vcpkg(pkg_config, port, minimum) {
            return found;
        }
        let library = pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version(minimum)
            .probe(pkg_config)
            .expect(hint);
        Self {
            pkg_config,
            port,
            version: library.version,
            minimum,
            include_paths: library.include_paths,
            defines: library.defines.into_iter().collect(),
            vcpkg: false,
        }
    }

    #[cfg(windows)]
    fn find_vcpkg(
        pkg_config: &'static str,
        port: &'static str,
        minimum: &'static str,
    ) -> Option<Self> {
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
            return None;
        }
        // The pinned vcpkg revision in Cargo.toml decides the version; read it from the
        // headers only to report it and to keep the minimum honest.
        let library = vcpkg::Config::new()
            .cargo_metadata(false)
            .find_package(port)
            .ok()?;
        let version = header_version(&library.include_paths, port).unwrap_or_default();
        if !version.is_empty() && !at_least(&version, minimum) {
            panic!("{port} {version} from vcpkg is older than {minimum}; run cargo vcpkg build");
        }
        Some(Self {
            pkg_config,
            port,
            version,
            minimum,
            include_paths: library.include_paths,
            defines: Vec::new(),
            vcpkg: true,
        })
    }

    #[cfg(not(windows))]
    fn find_vcpkg(_: &'static str, _: &'static str, _: &'static str) -> Option<Self> {
        None
    }

    fn configure(&self, build: &mut cc::Build) {
        for include in &self.include_paths {
            build.include(include);
        }
        for (name, value) in &self.defines {
            build.define(name, value.as_deref());
        }
    }

    /// Emits the link lines for this library and everything it depends on.
    fn link(&self) {
        if self.vcpkg {
            self.link_vcpkg();
            return;
        }
        pkg_config::Config::new()
            .atleast_version(self.minimum)
            .probe(self.pkg_config)
            .unwrap_or_else(|e| panic!("{} link configuration: {e}", self.pkg_config));
    }

    #[cfg(windows)]
    fn link_vcpkg(&self) {
        vcpkg::Config::new()
            .find_package(self.port)
            .unwrap_or_else(|e| panic!("{} link configuration: {e}", self.port));
    }

    #[cfg(not(windows))]
    fn link_vcpkg(&self) {
        unreachable!("{} comes from vcpkg only on Windows", self.port)
    }
}

/// Links each of `ports` that vcpkg installed, ignoring the rest.
#[cfg(windows)]
fn link_vcpkg_optional(ports: &[&str]) {
    for port in ports {
        let _ = vcpkg::Config::new().find_package(port);
    }
}

#[cfg(not(windows))]
fn link_vcpkg_optional(_: &[&str]) {}

/// The version a vcpkg port's headers declare, such as "1.23.7".
#[cfg(windows)]
fn header_version(include_paths: &[std::path::PathBuf], port: &str) -> Option<String> {
    let (header, macro_name) = match port {
        "exiv2" => ("exiv2/exv_conf.h", "EXV_PACKAGE_VERSION"),
        "libheif" => ("libheif/heif_version.h", "LIBHEIF_VERSION"),
        "libraw" => ("libraw/libraw_version.h", ""),
        _ => return None,
    };
    let text = include_paths
        .iter()
        .find_map(|dir| std::fs::read_to_string(dir.join(header)).ok())?;
    if port == "libraw" {
        let number = |name: &str| {
            text.lines().find_map(|line| {
                line.trim()
                    .strip_prefix(&format!("#define {name} "))?
                    .trim()
                    .parse::<u32>()
                    .ok()
            })
        };
        return Some(format!(
            "{}.{}.{}",
            number("LIBRAW_MAJOR_VERSION")?,
            number("LIBRAW_MINOR_VERSION")?,
            number("LIBRAW_PATCH_VERSION")?
        ));
    }
    text.lines().find_map(|line| {
        let rest = line
            .trim()
            .strip_prefix(&format!("#define {macro_name} "))?;
        Some(rest.trim().trim_matches('"').to_string())
    })
}

#[cfg(windows)]
fn at_least(version: &str, minimum: &str) -> bool {
    let parts = |v: &str| -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parts(version) >= parts(minimum)
}
