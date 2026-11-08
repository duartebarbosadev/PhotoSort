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

/// Windows (MSVC): libvips and everything it needs come from `cargo vcpkg build` as static
/// libraries in their own triplet (native/vcpkg-triplets/x64-windows-static-md-vips.cmake),
/// and are linked with this adapter into `photosort_vips.dll`, which exports only the
/// adapter's functions. The application links GStreamer's GLib, whose import library has the same
/// name as vcpkg's static GLib, so linking libvips into the executable would mix the two.
#[cfg(windows)]
mod msvc {
    use std::{
        collections::{BTreeMap, BTreeSet},
        fs,
        path::{Path, PathBuf},
        process::Command,
    };

    const DLL: &str = "photosort_vips";
    /// Cargo.toml installs libvips for this triplet.
    const TRIPLET: &str = "x64-windows-static-md-vips";
    const EXPORTS: [&str; 3] = [
        "photosort_vips_init",
        "photosort_vips_free",
        "photosort_vips_render",
    ];
    /// Libraries named in pkg-config files that MSVC does not have or need.
    const UNIX_ONLY: [&str; 4] = ["m", "pthread", "dl", "rt"];
    /// Windows libraries static GLib and GIO call that their pkg-config files leave out.
    const WINDOWS: [&str; 4] = ["advapi32", "ole32", "shell32", "user32"];

    pub fn build(mut build: cc::Build, target: &str) {
        println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
        let library = vcpkg::Config::new()
            .cargo_metadata(false)
            .target_triplet(TRIPLET)
            .find_package("libvips")
            .unwrap_or_else(|e| {
                panic!("Build libvips with `cargo vcpkg build` (see CONTRIBUTING.md): {e}")
            });
        let root = vcpkg::find_vcpkg_root(&vcpkg::Config::new()).expect("vcpkg root");
        let installed = root.join("installed").join(&library.vcpkg_triplet);
        let pkgconfig = installed.join("lib").join("pkgconfig");
        let modules = Modules::load(&pkgconfig, "vips");
        for (name, minimum, key) in [
            ("vips", "8.18", "PHOTOSORT_VIPS_VERSION"),
            ("lcms2", "2.16", "PHOTOSORT_LCMS_VERSION"),
        ] {
            let version = modules.version(name);
            assert!(
                at_least(version, minimum),
                "{name} {version} from vcpkg is older than {minimum}; run cargo vcpkg build"
            );
            println!("cargo:rustc-env={key}={version}");
        }
        // vcpkg's package ABI hashes the port, its features, the triplet, the compiler and
        // the ABI of every dependency: the whole recipe, as build_vips.py's is elsewhere.
        let status = root.join("installed").join("vcpkg");
        println!("cargo:rerun-if-changed={}", status.join("status").display());
        println!(
            "cargo:rerun-if-changed={}",
            status.join("updates").display()
        );
        let recipe = installed_abi(&status, "libvips", &library.vcpkg_triplet)
            .expect("vcpkg records no ABI hash for libvips; run cargo vcpkg build");
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
        let objects = build.compile_intermediates();

        let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let dll = out.join(format!("{DLL}.dll"));
        let mut link = cc::windows_registry::find_tool(target, "link.exe")
            .expect("MSVC link.exe")
            .to_command();
        link.args(["/NOLOGO", "/DLL", "/OPT:REF", "/OPT:ICF"])
            .arg(format!("/OUT:{}", dll.display()))
            .arg(format!(
                "/IMPLIB:{}",
                out.join(format!("{DLL}.lib")).display()
            ))
            .args(EXPORTS.iter().map(|name| format!("/EXPORT:{name}")))
            .args(&objects)
            .args(modules.libraries(&installed.join("lib")))
            .args(WINDOWS.iter().map(|name| format!("{name}.lib")));
        run(link);
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=dylib={DLL}");
        // Cargo puts OUT_DIR on PATH for `cargo run` and tests; a copy beside the
        // profile's executables serves everything else, including the Windows packager.
        if let Some(profile) = out.ancestors().nth(3) {
            copy_if_changed(&dll, &profile.join(format!("{DLL}.dll")));
        }
    }

    fn run(mut command: Command) {
        let output = command.output().expect("run link.exe");
        if !output.status.success() {
            panic!(
                "Linking {DLL}.dll failed:\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn copy_if_changed(from: &Path, to: &Path) {
        let built = fs::read(from).expect("read the built DLL");
        if fs::read(to).is_ok_and(|existing| existing == built) {
            return;
        }
        if let Err(error) = fs::write(to, &built) {
            // A running test or app holds the old copy open; Cargo's own runs use OUT_DIR.
            println!("cargo:warning=Could not update {}: {error}", to.display());
        }
    }

    /// The pkg-config graph vcpkg installs for one module: what it and everything it
    /// requires (publicly or privately, since all of it is static) compile and link with.
    struct Modules {
        files: BTreeMap<String, PcFile>,
    }

    struct PcFile {
        version: String,
        requires: Vec<String>,
        cflags: Vec<String>,
        libs: Vec<String>,
    }

    impl Modules {
        fn load(directory: &Path, root: &str) -> Self {
            let mut files = BTreeMap::new();
            let mut pending = vec![root.to_owned()];
            while let Some(name) = pending.pop() {
                if files.contains_key(&name) {
                    continue;
                }
                let path = directory.join(format!("{name}.pc"));
                println!("cargo:rerun-if-changed={}", path.display());
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("{}: {e}; run cargo vcpkg build", path.display()));
                let file = PcFile::parse(&text, directory);
                pending.extend(file.requires.iter().cloned());
                files.insert(name, file);
            }
            Self { files }
        }

        fn version(&self, name: &str) -> &str {
            &self
                .files
                .get(name)
                .unwrap_or_else(|| panic!("libvips from vcpkg does not use {name}"))
                .version
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

        /// Every vcpkg library by path, then the Windows libraries they name.
        fn libraries(&self, lib: &Path) -> Vec<PathBuf> {
            let mut vcpkg = BTreeSet::new();
            let mut system = BTreeSet::new();
            for flag in self.files.values().flat_map(|file| &file.libs) {
                let Some(name) = flag.strip_prefix("-l") else {
                    continue;
                };
                let name = name.trim_end_matches(".lib");
                let path = lib.join(format!("{name}.lib"));
                if path.is_file() {
                    vcpkg.insert(path);
                } else if !UNIX_ONLY.contains(&name) {
                    system.insert(PathBuf::from(format!("{name}.lib")));
                }
            }
            vcpkg.into_iter().chain(system).collect()
        }
    }

    impl PcFile {
        fn parse(text: &str, directory: &Path) -> Self {
            // pkg-config defines pcfiledir itself; vcpkg's files locate their prefix from it.
            let mut variables = BTreeMap::from([(
                "pcfiledir".to_owned(),
                directory.display().to_string().replace('\\', "/"),
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
            let mut requires = Vec::new();
            for list in [field("Requires"), field("Requires.private")] {
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
            let words = |text: &str| arguments(text);
            Self {
                version: field("Version").to_owned(),
                requires,
                cflags: words(field("Cflags")),
                libs: words(&format!("{} {}", field("Libs"), field("Libs.private"))),
            }
        }
    }

    /// Splits a pkg-config field into arguments, keeping quoted ones (paths with spaces) whole.
    fn arguments(text: &str) -> Vec<String> {
        let mut arguments = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        for c in text.chars() {
            match c {
                '"' => quoted = !quoted,
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

    /// The ABI hash vcpkg's status database records for an installed port.
    fn installed_abi(status: &Path, port: &str, triplet: &str) -> Option<String> {
        // The status file, then its incremental updates in order; later entries win.
        let mut databases = vec![status.join("status")];
        if let Ok(entries) = fs::read_dir(status.join("updates")) {
            let mut updates: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
            updates.sort();
            databases.extend(updates);
        }
        let mut abi = None;
        for database in databases {
            let Ok(text) = fs::read_to_string(&database) else {
                continue;
            };
            for paragraph in text.replace("\r\n", "\n").split("\n\n") {
                let field = |name: &str| {
                    paragraph
                        .lines()
                        .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                        .map(str::trim)
                };
                if field("Package") != Some(port)
                    || field("Architecture") != Some(triplet)
                    || field("Feature").is_some()
                {
                    continue;
                }
                abi = match field("Status") {
                    Some(status) if status.ends_with(" installed") => {
                        field("Abi").map(str::to_owned)
                    }
                    _ => None,
                };
            }
        }
        abi
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
        panic!("Building for MSVC needs a Windows host, where vcpkg provides libvips");
    }
}
