//! ONNX Runtime packages PhotoSort installs only when the user asks: the CPU
//! runtime on every platform, and NVIDIA GPU support (CUDA) on Windows. Every
//! archive comes from its publisher (Microsoft, NVIDIA), is pinned by size and
//! SHA-256, and only the libraries PhotoSort loads are kept. Packages land in
//! the app's data folder, which `models::runtime_path` already searches.
use super::*;

pub const ORT_VERSION: &str = "1.29.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Package {
    /// ONNX Runtime for the CPU.
    Cpu,
    /// ONNX Runtime's CUDA build with the NVIDIA libraries it loads. Its
    /// runtime also runs every model on the CPU, so it replaces `Cpu`.
    Cuda,
}

/// One pinned archive and the libraries to keep from it.
pub struct Archive {
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// Archive members to keep, as (path in the archive, installed file name).
    /// A path ending in `/` keeps every library directly inside that folder.
    pub members: &'static [(&'static str, &'static str)],
}

pub struct Info {
    pub package: Package,
    pub title: &'static str,
    pub publisher: &'static str,
    /// The installed folder's name inside the data folder.
    pub directory: &'static str,
    pub archives: &'static [Archive],
}

impl Info {
    pub fn download_bytes(&self) -> u64 {
        self.archives.iter().map(|a| a.bytes).sum()
    }
}

#[cfg(any(feature = "model-downloads", test))]
const LIBRARY_SUFFIXES: [&str; 3] = [".dll", ".so", ".dylib"];

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
const CATALOG: &[Info] = &[
    Info {
        package: Package::Cpu,
        title: "ONNX Runtime 1.29.0",
        publisher: "Microsoft",
        directory: "runtime",
        archives: &[Archive {
            url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-win-x64-1.29.0.zip",
            bytes: 79_645_520,
            sha256: "c9b4b7086b529ad814f428c1bad028e20a25d7dc0699836775faace4ab5b78b2",
            members: &[
                (
                    "onnxruntime-win-x64-1.29.0/lib/onnxruntime.dll",
                    "onnxruntime.dll",
                ),
                (
                    "onnxruntime-win-x64-1.29.0/lib/onnxruntime_providers_shared.dll",
                    "onnxruntime_providers_shared.dll",
                ),
            ],
        }],
    },
    Info {
        package: Package::Cuda,
        title: "NVIDIA GPU support (CUDA 12, cuDNN 9)",
        publisher: "Microsoft and NVIDIA",
        directory: "runtime-cuda",
        archives: &[
            Archive {
                url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-win-x64-gpu_cuda12-1.29.0.zip",
                bytes: 362_974_227,
                sha256: "3526773ba1fef5849a3e9d18558d87bbbb75b0db3ddcde4f08694a746ac3169f",
                members: &[
                    (
                        "onnxruntime-win-x64-gpu_cuda12-1.29.0/lib/onnxruntime.dll",
                        "onnxruntime.dll",
                    ),
                    (
                        "onnxruntime-win-x64-gpu_cuda12-1.29.0/lib/onnxruntime_providers_shared.dll",
                        "onnxruntime_providers_shared.dll",
                    ),
                    (
                        "onnxruntime-win-x64-gpu_cuda12-1.29.0/lib/onnxruntime_providers_cuda.dll",
                        "onnxruntime_providers_cuda.dll",
                    ),
                ],
            },
            Archive {
                url: "https://developer.download.nvidia.com/compute/cuda/redist/cuda_cudart/windows-x86_64/cuda_cudart-windows-x86_64-12.9.79-archive.zip",
                bytes: 3_521_238,
                sha256: "179e9c43b0735ffe67207b3da556eb5a0c50f3047961882b7657d3b822d34ef8",
                members: &[(
                    "cuda_cudart-windows-x86_64-12.9.79-archive/bin/cudart64_12.dll",
                    "cudart64_12.dll",
                )],
            },
            Archive {
                url: "https://developer.download.nvidia.com/compute/cuda/redist/libcublas/windows-x86_64/libcublas-windows-x86_64-12.9.2.10-archive.zip",
                bytes: 549_731_131,
                sha256: "62782c354e1536118caab8fe5fd295ac5fd9076e4814aba0bdaf032fe2b833ae",
                members: &[
                    (
                        "libcublas-windows-x86_64-12.9.2.10-archive/bin/cublas64_12.dll",
                        "cublas64_12.dll",
                    ),
                    (
                        "libcublas-windows-x86_64-12.9.2.10-archive/bin/cublasLt64_12.dll",
                        "cublasLt64_12.dll",
                    ),
                ],
            },
            Archive {
                url: "https://developer.download.nvidia.com/compute/cuda/redist/libcufft/windows-x86_64/libcufft-windows-x86_64-11.4.1.4-archive.zip",
                bytes: 198_361_265,
                sha256: "f26f80bb9abff3269c548e1559e8c2b4ba58ccb8acc6095bbc6404fc962d4b80",
                members: &[(
                    "libcufft-windows-x86_64-11.4.1.4-archive/bin/cufft64_11.dll",
                    "cufft64_11.dll",
                )],
            },
            Archive {
                url: "https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.28.1.4_cuda12-archive.zip",
                bytes: 1_885_727_510,
                sha256: "75bf8c4201adfe994213ac080b97ecba0e04ee86be10c1774982e837bee3ee8f",
                members: &[("cudnn-windows-x86_64-9.28.1.4_cuda12-archive/bin/x64/", "")],
            },
        ],
    },
];

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const CATALOG: &[Info] = &[Info {
    package: Package::Cpu,
    title: "ONNX Runtime 1.29.0",
    publisher: "Microsoft",
    directory: "runtime",
    archives: &[Archive {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-osx-arm64-1.29.0.tgz",
        bytes: 41_578_864,
        sha256: "d0706fc34f315d8c88639d0a8c81f2e09e815f282cabed3493c06a054352cf92",
        members: &[(
            "onnxruntime-osx-arm64-1.29.0/lib/libonnxruntime.dylib",
            "libonnxruntime.dylib",
        )],
    }],
}];

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const CATALOG: &[Info] = &[Info {
    package: Package::Cpu,
    title: "ONNX Runtime 1.29.0",
    publisher: "Microsoft",
    directory: "runtime",
    archives: &[Archive {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/onnxruntime-linux-x64-1.29.0.tgz",
        bytes: 11_082_880,
        sha256: "c3fddc4f139a045b0c4902c57410f0694f1c2fdf9b6939fbe38b1aeae7cd14ba",
        // The versioned file is the library itself; the unversioned names are links.
        members: &[(
            "onnxruntime-linux-x64-1.29.0/lib/libonnxruntime.so.1.29.0",
            "libonnxruntime.so",
        )],
    }],
}];

#[cfg(not(any(
    all(target_os = "windows", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
)))]
const CATALOG: &[Info] = &[];

/// The packages this platform can download.
pub fn catalog() -> &'static [Info] {
    CATALOG
}

pub fn info(package: Package) -> Option<&'static Info> {
    CATALOG.iter().find(|info| info.package == package)
}

/// Marks a complete install; written last, so a folder without it is not trusted.
const MARKER: &str = "photosort-runtime.txt";

/// The installed folder of `package` under the data folder `root`, when complete.
pub fn installed(root: &Path, package: Package) -> Option<PathBuf> {
    let info = info(package)?;
    let directory = root.join(info.directory);
    let marker = std::fs::read_to_string(directory.join(MARKER)).ok()?;
    (marker.trim() == marker_text(info)).then_some(directory)
}

fn marker_text(info: &Info) -> String {
    let sums: Vec<_> = info.archives.iter().map(|a| a.sha256).collect();
    format!("{} {}", info.title, sums.join(" "))
}

/// Whether this computer has an NVIDIA driver, which CUDA needs and no
/// download can provide.
pub fn nvidia_driver_present() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("nvcuda.dll")
            .is_file()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Downloads, verifies and installs `package` into `root`, replacing an older
/// copy only once the new one is complete.
pub(super) fn install(
    root: &Path,
    package: Package,
    cancelled: &impl Fn() -> bool,
    progress: &impl Fn(u64, u64),
) -> Result<PathBuf> {
    #[cfg(not(feature = "model-downloads"))]
    {
        let _ = (root, cancelled, progress);
        let info = info(package).context("This package is not available on this platform")?;
        anyhow::bail!("This build cannot download {}", info.title);
    }
    #[cfg(feature = "model-downloads")]
    install_from(
        root,
        package,
        cancelled,
        progress,
        &|archive, file, cancelled, progress| {
            super::download::fetch(
                archive.url,
                archive.bytes,
                archive.sha256,
                file,
                &cancelled,
                &progress,
            )
        },
    )
}

/// Places one archive at the given path; `install_from` verifies it afterwards.
#[cfg(feature = "model-downloads")]
type Obtain<'a> = dyn Fn(&Archive, &Path, &dyn Fn() -> bool, &dyn Fn(u64)) -> Result<()> + 'a;

/// Installs `package` from archives `obtain` provides (downloads in the app,
/// local copies in tests). Each archive is checked by size and SHA-256 before
/// anything is unpacked, whatever its source.
#[cfg(feature = "model-downloads")]
fn install_from(
    root: &Path,
    package: Package,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
    obtain: &Obtain,
) -> Result<PathBuf> {
    let info = info(package).context("This package is not available on this platform")?;
    std::fs::create_dir_all(root)?;
    let staging = tempfile::Builder::new()
        .prefix(&format!(".{}-", info.directory))
        .tempdir_in(root)?;
    let total = info.download_bytes();
    let mut done = 0;
    for (index, archive) in info.archives.iter().enumerate() {
        ensure!(!cancelled(), "Cancelled");
        let name = archive.url.rsplit('/').next().unwrap_or("archive");
        let file = staging.path().join(format!("download-{index}-{name}"));
        obtain(archive, &file, cancelled, &|n| progress(done + n, total))
            .with_context(|| format!("Download {name}"))?;
        verify(&file, archive, cancelled).with_context(|| format!("Verify {name}"))?;
        done += archive.bytes;
        extract(&file, archive, staging.path(), cancelled)
            .with_context(|| format!("Unpack {name}"))?;
        std::fs::remove_file(&file)?;
    }
    ensure!(!cancelled(), "Cancelled");
    std::fs::write(staging.path().join(MARKER), marker_text(info))?;
    let destination = root.join(info.directory);
    replace_directory(staging.keep(), &destination)?;
    Ok(destination)
}

#[cfg(feature = "model-downloads")]
fn verify(file: &Path, archive: &Archive, cancelled: &dyn Fn() -> bool) -> Result<()> {
    let mut input = File::open(file)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    let mut bytes = 0u64;
    loop {
        ensure!(!cancelled(), "Cancelled");
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        bytes += n as u64;
    }
    ensure!(
        bytes == archive.bytes,
        "Size differs from the pinned {} bytes",
        archive.bytes
    );
    ensure!(
        hex(&hash.finalize()) == archive.sha256,
        "SHA-256 differs from the pinned value"
    );
    Ok(())
}

/// Unpacks only the listed members with the system `tar` (bsdtar on Windows 10+
/// and macOS reads .zip and .tgz; GNU tar on Linux reads .tgz).
#[cfg(feature = "model-downloads")]
fn extract(
    archive: &Path,
    spec: &Archive,
    into: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let scratch = tempfile::Builder::new()
        .prefix(".unpack-")
        .tempdir_in(into)?;
    let mut command = std::process::Command::new(tar_program());
    command
        .arg("-xf")
        .arg(archive)
        .arg("-C")
        .arg(scratch.path());
    for (member, _) in spec.members {
        command.arg(member.trim_end_matches('/'));
    }
    let output = command.output().context("Run tar to unpack the download")?;
    ensure!(
        output.status.success(),
        "tar failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    for (member, name) in spec.members {
        ensure!(!cancelled(), "Cancelled");
        let source = scratch.path().join(member.trim_end_matches('/'));
        if member.ends_with('/') {
            let mut kept = 0;
            for entry in std::fs::read_dir(&source)? {
                let entry = entry?;
                let file_name = entry.file_name();
                let text = file_name.to_string_lossy();
                if entry.file_type()?.is_file()
                    && LIBRARY_SUFFIXES.iter().any(|s| text.ends_with(s))
                {
                    std::fs::rename(entry.path(), into.join(&file_name))?;
                    kept += 1;
                }
            }
            ensure!(kept > 0, "{member} holds no libraries");
        } else {
            let metadata = std::fs::symlink_metadata(&source)
                .with_context(|| format!("{member} is missing from the download"))?;
            ensure!(
                metadata.file_type().is_file(),
                "{member} is not a regular file"
            );
            std::fs::rename(&source, into.join(name))?;
        }
    }
    Ok(())
}

/// Windows ships bsdtar as `%SystemRoot%\System32\tar.exe`; use it by full path so
/// another `tar` on PATH (Git's GNU tar cannot read .zip) is never picked.
#[cfg(feature = "model-downloads")]
fn tar_program() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let system = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
            .join("System32")
            .join("tar.exe");
        if system.is_file() {
            return system;
        }
    }
    PathBuf::from("tar")
}

/// Moves the completed `staging` folder to `destination`. An existing copy is
/// set aside first and removed afterwards; if it is in use (Windows keeps
/// loaded libraries locked), the error asks for a restart and nothing changes.
#[cfg(feature = "model-downloads")]
fn replace_directory(staging: PathBuf, destination: &Path) -> Result<()> {
    if destination.exists() {
        let old = destination.with_extension("old");
        let _ = std::fs::remove_dir_all(&old);
        if let Err(error) = std::fs::rename(destination, &old) {
            let _ = std::fs::remove_dir_all(&staging);
            anyhow::bail!(
                "The installed runtime is in use ({error}). Restart PhotoSort, then install again"
            );
        }
        if let Err(error) = std::fs::rename(&staging, destination) {
            let _ = std::fs::rename(&old, destination);
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        let _ = std::fs::remove_dir_all(&old);
    } else {
        std::fs::rename(&staging, destination)?;
    }
    Ok(())
}

/// Removes an installed package. Windows refuses while its libraries are
/// loaded, so the message asks for a restart.
pub(super) fn remove(root: &Path, package: Package) -> Result<()> {
    let info = info(package).context("This package is not available on this platform")?;
    let directory = root.join(info.directory);
    if directory.exists() {
        std::fs::remove_dir_all(&directory).map_err(|error| {
            anyhow::anyhow!(
                "{} is in use ({error}). Restart PhotoSort, then remove it again",
                info.title
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_archive_is_pinned_from_its_publisher_and_keeps_libraries_only() {
        for info in catalog() {
            assert!(!info.archives.is_empty(), "{}", info.title);
            for archive in info.archives {
                assert!(
                    archive.url.starts_with(
                        "https://github.com/microsoft/onnxruntime/releases/download/v1.29.0/"
                    ) || archive
                        .url
                        .starts_with("https://developer.download.nvidia.com/compute/"),
                    "{}",
                    archive.url
                );
                assert_eq!(archive.sha256.len(), 64);
                assert!(
                    archive
                        .sha256
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                );
                assert!(archive.bytes > 0);
                for (member, name) in archive.members {
                    assert!(
                        member.ends_with('/') || LIBRARY_SUFFIXES.iter().any(|s| name.ends_with(s)),
                        "{member}"
                    );
                }
            }
        }
        // Every platform PhotoSort releases on can install the CPU runtime.
        if cfg!(any(
            all(target_os = "windows", target_arch = "x86_64"),
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64"),
        )) {
            assert!(info(Package::Cpu).is_some());
        }
    }
    /// Installs every package from local copies of its pinned archives:
    /// `PHOTOSORT_RUNTIME_ARCHIVES=<folder> PHOTOSORT_RUNTIME_ROOT=<data folder>
    /// cargo test runtime_packages_install_from_local_archives -- --ignored`.
    /// Copies with any other content are refused like a bad download.
    #[cfg(feature = "model-downloads")]
    #[test]
    #[ignore = "needs the pinned archives on disk"]
    fn runtime_packages_install_from_local_archives() {
        let archives = PathBuf::from(std::env::var_os("PHOTOSORT_RUNTIME_ARCHIVES").unwrap());
        let root = PathBuf::from(std::env::var_os("PHOTOSORT_RUNTIME_ROOT").unwrap());
        for info in catalog() {
            let directory = install_from(
                &root,
                info.package,
                &|| false,
                &|_, _| {},
                &|archive, file, _, _| {
                    let name = archive.url.rsplit('/').next().unwrap();
                    std::fs::copy(archives.join(name), file)?;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(installed(&root, info.package), Some(directory.clone()));
            let files = std::fs::read_dir(&directory).unwrap().count();
            eprintln!("{}: {files} files in {}", info.title, directory.display());
        }
    }
    #[cfg(feature = "model-downloads")]
    #[test]
    fn a_download_with_other_content_is_refused_and_nothing_is_installed() {
        let Some(info) = info(Package::Cpu) else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        let result = install_from(
            root.path(),
            Package::Cpu,
            &|| false,
            &|_, _| {},
            &|_, file, _, _| {
                std::fs::write(file, b"not the pinned archive")?;
                Ok(())
            },
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("Size differs"));
        assert!(installed(root.path(), Package::Cpu).is_none());
        assert!(!root.path().join(info.directory).exists());
        // Only the emptied staging folder's removal is pending; nothing else was left.
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[test]
    fn a_folder_counts_as_installed_only_with_the_marker_for_these_pins() {
        let Some(info) = info(Package::Cpu) else {
            return;
        };
        let root = tempfile::tempdir().unwrap();
        assert!(installed(root.path(), Package::Cpu).is_none());
        let directory = root.path().join(info.directory);
        std::fs::create_dir_all(&directory).unwrap();
        assert!(installed(root.path(), Package::Cpu).is_none());
        std::fs::write(directory.join(MARKER), "an older package").unwrap();
        assert!(installed(root.path(), Package::Cpu).is_none());
        std::fs::write(directory.join(MARKER), marker_text(info)).unwrap();
        assert_eq!(installed(root.path(), Package::Cpu), Some(directory));
        remove(root.path(), Package::Cpu).unwrap();
        assert!(installed(root.path(), Package::Cpu).is_none());
    }
}
