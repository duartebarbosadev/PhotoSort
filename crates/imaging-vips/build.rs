fn main() {
    println!("cargo:rerun-if-changed=native/imaging.c");
    let target = std::env::var("TARGET").expect("Cargo build target");
    let mut build = cc::Build::new();
    build.file("native/imaging.c").std("c11").warnings(true);
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        msvc::build(build, &target);
        return;
    }
    let dependencies = [("vips", "8.18"), ("lcms2", "2.16")];
    // A libvips named by PHOTOSORT_VIPS_PREFIX, else one pkg-config already finds (such
    // as PKG_CONFIG_PATH naming a private build), else the private build in .tools/vips,
    // built by scripts/build_vips.py the first time.
    println!("cargo:rerun-if-env-changed=PHOTOSORT_VIPS_PREFIX");
    let explicit = std::env::var_os("PHOTOSORT_VIPS_PREFIX").map(std::path::PathBuf::from);
    let found = explicit.is_none()
        && pkg_config::Config::new()
            .cargo_metadata(false)
            .atleast_version("8.18")
            .probe("vips")
            .is_ok();
    if !found {
        let prefix = explicit.unwrap_or_else(private::default_prefix);
        private::ensure_built(&prefix, &[]);
        let pkgconfig = prefix.join("lib").join("pkgconfig");
        let path = std::env::var_os("PKG_CONFIG_PATH").unwrap_or_default();
        let joined =
            std::env::join_paths(std::iter::once(pkgconfig).chain(std::env::split_paths(&path)))
                .expect("PKG_CONFIG_PATH");
        // SAFETY: build scripts are single-threaded; pkg-config reads it below.
        unsafe { std::env::set_var("PKG_CONFIG_PATH", joined) };
    }
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
    assert!(is_recipe(&recipe), "Invalid native image build recipe");
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

fn is_recipe(recipe: &str) -> bool {
    recipe == "external" || (recipe.len() == 64 && recipe.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// PhotoSort's private libvips (scripts/build_vips.py), built on first use.
mod private {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    /// The repository root: this crate lives in crates/imaging-vips.
    fn repository() -> PathBuf {
        let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
        manifest
            .ancestors()
            .nth(2)
            .expect("repository root")
            .to_owned()
    }

    /// Where scripts/build_vips.py installs by default.
    pub fn default_prefix() -> PathBuf {
        repository().join(".tools").join("vips")
    }

    /// Builds libvips into `prefix` unless it is there. Takes a few minutes once, and
    /// needs Python 3.11+ and network access; later builds reuse the prefix (delete it
    /// to rebuild). A lock beside it keeps concurrent builds from building it twice.
    pub fn ensure_built(prefix: &Path, env: &[(&str, &Path)]) {
        let done = prefix.join("lib").join("pkgconfig").join("vips.pc");
        println!("cargo:rerun-if-changed={}", done.display());
        let script = repository().join("scripts").join("build_vips.py");
        println!("cargo:rerun-if-changed={}", script.display());
        if done.is_file() {
            return;
        }
        if prefix.exists() {
            panic!(
                "{} holds no complete libvips build; delete it so the build can make one",
                prefix.display()
            );
        }
        let parent = prefix.parent().expect("libvips prefix has a parent");
        fs::create_dir_all(parent).expect("create the libvips prefix's folder");
        let lock = prefix.with_extension("lock");
        let start = Instant::now();
        while fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
            .is_err()
        {
            // Another build is making it (or a crashed one left the lock behind).
            if done.is_file() {
                return;
            }
            if start.elapsed() > Duration::from_secs(30 * 60) {
                panic!(
                    "Waited 30 minutes for another libvips build; if none is running, delete {}",
                    lock.display()
                );
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if done.is_file() {
            let _ = fs::remove_file(&lock);
            return;
        }
        println!(
            "cargo:warning=Building libvips into {} (first build only, a few minutes; delete it to rebuild)",
            prefix.display()
        );
        let python = std::env::var_os("PYTHON")
            .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into());
        let mut command = Command::new(&python);
        command
            .arg("-B")
            .arg(&script)
            .arg("--prefix")
            .arg(prefix)
            // Cargo reads build-script stdout as instructions; keep the build's log apart.
            .stdout(Stdio::from(std::io::stderr()));
        for (name, value) in env {
            command.env(name, value);
        }
        let status = command.status();
        let _ = fs::remove_file(&lock);
        if !status.as_ref().is_ok_and(|status| status.success()) {
            // A failed attempt must not look like an unfinished one next time.
            if !done.is_file() && prefix.exists() {
                let _ = fs::remove_dir_all(prefix);
            }
            panic!(
                "Building libvips with {} failed ({}). See the log above, or run \
                 `{} scripts/build_vips.py` yourself (CONTRIBUTING.md lists what it needs).",
                script.display(),
                match status {
                    Ok(status) => status.to_string(),
                    Err(error) => format!("{} could not start: {error}", python.to_string_lossy()),
                },
                python.to_string_lossy()
            );
        }
    }
}

/// Windows (MSVC): scripts/build_vips.py builds libvips as a static library against the
/// GLib of the GStreamer installation PhotoSort uses for video, and the codecs from
/// `cargo vcpkg build`. This links them into the application: libvips and the codecs
/// statically, GLib through GStreamer's import libraries, so the process has one GLib.
/// The application delay-loads GStreamer's DLLs (see the root build.rs) and points
/// Windows at them before its first image is decoded.
#[cfg(windows)]
mod msvc {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::{Path, PathBuf},
    };

    /// Libraries named in pkg-config files that MSVC does not have or need.
    const UNIX_ONLY: [&str; 4] = ["m", "pthread", "dl", "rt"];
    const GSTREAMER: &str = r"C:\Program Files\gstreamer\1.0\msvc_x86_64";
    const TRIPLET: &str = "x64-windows-static-md";

    pub fn build(mut build: cc::Build, target: &str) {
        for name in [
            "PHOTOSORT_VIPS_PREFIX",
            "VCPKG_ROOT",
            "GSTREAMER_1_0_ROOT_MSVC_X86_64",
        ] {
            println!("cargo:rerun-if-env-changed={name}");
        }
        let prefix = std::env::var_os("PHOTOSORT_VIPS_PREFIX")
            .map(PathBuf::from)
            .unwrap_or_else(super::private::default_prefix);
        let vcpkg_root = vcpkg::find_vcpkg_root(&vcpkg::Config::new())
            .expect("vcpkg tree; run cargo vcpkg build");
        let vcpkg = vcpkg_root.join("installed").join(TRIPLET);
        let gstreamer = std::env::var_os("GSTREAMER_1_0_ROOT_MSVC_X86_64")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(GSTREAMER));
        let sources = [
            (prefix.join("lib/pkgconfig"), Linkage::Static),
            (vcpkg.join("lib/pkgconfig"), Linkage::Static),
            (gstreamer.join("lib/pkgconfig"), Linkage::Dynamic),
        ];
        // The build script finds the same vcpkg tree and GStreamer.
        let mut env = vec![("VCPKG_ROOT", vcpkg_root.as_path())];
        env.push(("GSTREAMER_1_0_ROOT_MSVC_X86_64", gstreamer.as_path()));
        super::private::ensure_built(&prefix, &env);
        let modules = Modules::load(&sources, "vips");
        for (name, minimum, key) in [
            ("vips", "8.18", "PHOTOSORT_VIPS_VERSION"),
            ("lcms2", "2.16", "PHOTOSORT_LCMS_VERSION"),
        ] {
            let version = &modules.get(name).version;
            assert!(
                at_least(version, minimum),
                "{name} {version} is older than {minimum}"
            );
            println!("cargo:rustc-env={key}={version}");
        }
        let recipe = modules
            .get("vips")
            .variables
            .get("photosort_recipe")
            .cloned()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "external".into());
        assert!(
            super::is_recipe(&recipe),
            "Invalid native image build recipe"
        );
        println!("cargo:rustc-env=PHOTOSORT_VIPS_BUILD_ID={target}:{recipe}");

        let (includes, defines) = modules.cflags();
        for include in includes {
            build.include(include);
        }
        for (name, value) in defines {
            build.define(&name, value.as_deref());
        }
        build.compile("photosort_imaging_vips");
        // GLib's variables libvips reads (see the file), linked whole.
        println!("cargo:rerun-if-changed=native/glib_data_win.c");
        cc::Build::new()
            .file("native/glib_data_win.c")
            .warnings(true)
            .cargo_metadata(false)
            .compile("photosort_glib_data");
        println!("cargo:rustc-link-lib=static:+whole-archive=photosort_glib_data");
        modules.link();
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Linkage {
        /// Static libraries: everything they need is linked too.
        Static,
        /// Import libraries of DLLs, which bring their own private dependencies.
        Dynamic,
    }

    /// The pkg-config graph of one module across several trees: the first tree with a
    /// module's file provides it.
    struct Modules {
        files: BTreeMap<String, PcFile>,
    }

    struct PcFile {
        linkage: Linkage,
        version: String,
        variables: BTreeMap<String, String>,
        requires: Vec<String>,
        cflags: Vec<String>,
        libs: Vec<String>,
    }

    impl Modules {
        fn load(sources: &[(PathBuf, Linkage)], root: &str) -> Self {
            let mut files = BTreeMap::new();
            let mut pending = vec![root.to_owned()];
            while let Some(name) = pending.pop() {
                if files.contains_key(&name) {
                    continue;
                }
                let (path, linkage) = sources
                    .iter()
                    .map(|(directory, linkage)| (directory.join(format!("{name}.pc")), *linkage))
                    .find(|(path, _)| path.is_file())
                    .unwrap_or_else(|| {
                        panic!("No pkg-config file for {name}; run cargo vcpkg build")
                    });
                println!("cargo:rerun-if-changed={}", path.display());
                let text =
                    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                let file = PcFile::parse(&text, path.parent().unwrap(), linkage);
                pending.extend(file.requires.iter().cloned());
                files.insert(name, file);
            }
            Self { files }
        }

        fn get(&self, name: &str) -> &PcFile {
            self.files
                .get(name)
                .unwrap_or_else(|| panic!("libvips does not use {name}"))
        }

        fn cflags(&self) -> (BTreeSet<PathBuf>, BTreeMap<String, Option<String>>) {
            let mut includes = BTreeSet::new();
            let mut defines = BTreeMap::new();
            for flag in self.files.values().flat_map(|file| &file.cflags) {
                if let Some(path) = flag.strip_prefix("-I") {
                    includes.insert(PathBuf::from(path));
                } else if let Some(define) = flag.strip_prefix("-D") {
                    let (name, value) = match define.split_once('=') {
                        Some((name, value)) => (name, Some(value.to_owned())),
                        None => (define, None),
                    };
                    // vips.pc carries libvips' own build flag; there is no config.h here.
                    if name != "HAVE_CONFIG_H" {
                        defines.insert(name.to_owned(), value);
                    }
                }
            }
            (includes, defines)
        }

        /// Static libraries under names of their own, so a GStreamer import library of
        /// the same name (jpeg.lib, tiff.lib) cannot stand in for one; import libraries
        /// and Windows libraries by name.
        fn link(&self) {
            let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
            println!("cargo:rustc-link-search=native={}", out.display());
            let mut searched = BTreeSet::new();
            let mut linked = BTreeSet::new();
            for file in self.files.values() {
                let directories: Vec<PathBuf> = file
                    .libs
                    .iter()
                    .filter_map(|flag| flag.strip_prefix("-L"))
                    .map(PathBuf::from)
                    .collect();
                for name in file.libs.iter().filter_map(|flag| flag.strip_prefix("-l")) {
                    if UNIX_ONLY.contains(&name) || !linked.insert(name.to_owned()) {
                        continue;
                    }
                    let found = directories.iter().find_map(|directory| {
                        [format!("{name}.lib"), format!("lib{name}.a")]
                            .into_iter()
                            .map(|file| directory.join(file))
                            .find(|path| path.is_file())
                    });
                    match (file.linkage, found) {
                        (Linkage::Static, Some(path)) => {
                            let copy = format!("photosort-vips-{name}.lib");
                            fs::copy(&path, out.join(&copy))
                                .unwrap_or_else(|e| panic!("Copying {}: {e}", path.display()));
                            println!("cargo:rustc-link-lib=static:-bundle,+verbatim={copy}");
                        }
                        (Linkage::Dynamic, Some(path)) => {
                            let directory = path.parent().unwrap().to_owned();
                            if searched.insert(directory.clone()) {
                                println!("cargo:rustc-link-search=native={}", directory.display());
                            }
                            println!("cargo:rustc-link-lib=dylib={name}");
                        }
                        // Windows libraries, such as ws2_32.
                        (_, None) => println!("cargo:rustc-link-lib=dylib={name}"),
                    }
                }
            }
        }
    }

    impl PcFile {
        fn parse(text: &str, directory: &Path, linkage: Linkage) -> Self {
            // pkg-config defines pcfiledir itself, with spaces escaped (C:/Program\ Files);
            // these files locate their prefix from it.
            let mut variables = BTreeMap::from([(
                "pcfiledir".to_owned(),
                directory
                    .display()
                    .to_string()
                    .replace('\\', "/")
                    .replace(' ', "\\ "),
            )]);
            let mut fields = BTreeMap::new();
            for line in text.lines().map(str::trim) {
                if line.starts_with('#') {
                    continue;
                }
                let field = line.find(':');
                let variable = line.find('=');
                match (field, variable) {
                    (Some(colon), equals) if equals.is_none_or(|equals| colon < equals) => {
                        let value = expand(line[colon + 1..].trim(), &variables);
                        fields.insert(line[..colon].trim().to_owned(), value);
                    }
                    (_, Some(equals)) => {
                        let value = expand(line[equals + 1..].trim(), &variables);
                        variables.insert(line[..equals].trim().to_owned(), value);
                    }
                    _ => {}
                }
            }
            let field = |name: &str| fields.get(name).map(String::as_str).unwrap_or("");
            // A DLL brings its own private dependencies; a static library does not.
            let private = |name: &str| {
                if linkage == Linkage::Static {
                    field(name)
                } else {
                    ""
                }
            };
            let mut requires = Vec::new();
            for list in [field("Requires"), private("Requires.private")] {
                // "glib-2.0 >= 2.52, gobject-2.0": names, each optionally followed by a constraint.
                let mut words = list
                    .split(|c: char| c == ',' || c.is_whitespace())
                    .filter(|w| !w.is_empty());
                while let Some(word) = words.next() {
                    if ["=", "<", ">", "<=", ">=", "!="].contains(&word) {
                        words.next();
                    } else {
                        requires.push(word.to_owned());
                    }
                }
            }
            Self {
                linkage,
                version: field("Version").to_owned(),
                requires,
                cflags: arguments(field("Cflags")),
                libs: arguments(&format!("{} {}", field("Libs"), private("Libs.private"))),
                variables,
            }
        }
    }

    /// Splits a pkg-config field into arguments, keeping quoted or backslash-escaped
    /// spaces (paths with spaces) inside one argument.
    fn arguments(text: &str) -> Vec<String> {
        let mut arguments = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => quoted = !quoted,
                '\\' => current.extend(chars.next()),
                c if c.is_whitespace() && !quoted => {
                    if !current.is_empty() {
                        arguments.push(std::mem::take(&mut current));
                    }
                }
                c => current.push(c),
            }
        }
        if !current.is_empty() {
            arguments.push(current);
        }
        arguments
    }

    fn expand(value: &str, variables: &BTreeMap<String, String>) -> String {
        let mut result = String::new();
        let mut rest = value;
        while let Some(start) = rest.find("${") {
            result.push_str(&rest[..start]);
            let Some(end) = rest[start..].find('}') else {
                break;
            };
            let name = &rest[start + 2..start + end];
            result.push_str(variables.get(name).map(String::as_str).unwrap_or(""));
            rest = &rest[start + end + 1..];
        }
        result.push_str(rest);
        result
    }

    fn at_least(version: &str, minimum: &str) -> bool {
        let parts =
            |v: &str| -> Vec<u32> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
        parts(version) >= parts(minimum)
    }
}

#[cfg(not(windows))]
mod msvc {
    pub fn build(_: cc::Build, _: &str) {
        panic!("Building for MSVC needs a Windows host");
    }
}
