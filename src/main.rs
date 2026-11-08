// Windows release builds open no console window beside the app; debug and
// test builds keep theirs. `attach_parent_console` gives command-line flags
// the terminal they were typed in.
#![cfg_attr(
    all(windows, not(debug_assertions), not(test)),
    windows_subsystem = "windows"
)]
mod diagnostic_output;
mod session_logging;
#[cfg(test)]
mod test_support;

fn main() -> eframe::Result {
    // Debug builds are console programs already.
    if !cfg!(debug_assertions) {
        attach_parent_console();
    }
    // The bundled ONNX Runtime uploader can race its own C++ static teardown.
    // Suppress it before runtime loading; the session API is too late to prevent
    // its initialization event and background uploader from being created.
    // SAFETY: this is the first application action, before native UI, model
    // loading or any application worker can start or read the environment.
    unsafe {
        std::env::set_var("ORT_DISABLE_TELEMETRY", "1");
    }
    let options =
        photosort::core::startup::Options::parse(std::env::args_os().skip(1)).map_err(|e| {
            eframe::Error::AppCreation(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                e,
            )))
        })?;
    if options.help {
        println!("{}", photosort::core::startup::HELP);
        return Ok(());
    }
    if options.version {
        println!("PhotoSort {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let _logging = session_logging::start(options.log_file.as_deref(), options.log_level)
        .map_err(|e| eframe::Error::AppCreation(e.into()))?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "application started");
    photosort::core::crash::install(photosort::core::crash::directory());
    let result = run(options);
    match &result {
        Ok(()) => tracing::info!("application stopped"),
        Err(error) => tracing::error!(%error, "application failed"),
    }
    result
}

fn run(options: photosort::core::startup::Options) -> eframe::Result {
    if options.check_runtime {
        use std::io::Write;
        let mut output =
            diagnostic_output::writer().map_err(|e| eframe::Error::AppCreation(e.into()))?;
        let report = photosort::core::diagnostics::check_runtime_with_backend(
            options
                .runtime_backend
                .unwrap_or(photosort::core::processing::Backend::Cpu),
            &|| false,
        )
        .map_err(|e| eframe::Error::AppCreation(e.into()))?;
        serde_json::to_writer_pretty(&mut output, &report)
            .map_err(|e| eframe::Error::AppCreation(e.into()))?;
        writeln!(output)
            .and_then(|_| output.flush())
            .map_err(|e| eframe::Error::AppCreation(e.into()))?;
        if !report.passed {
            return Err(eframe::Error::AppCreation(
                std::io::Error::other("Native runtime check did not pass; see JSON report").into(),
            ));
        }
        return Ok(());
    }
    if options.diagnostics {
        let report = photosort::core::diagnostics::collect()
            .map_err(|e| eframe::Error::AppCreation(e.into()))?;
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .map_err(|e| eframe::Error::AppCreation(e.into()))?
        );
        if !report.files_ready {
            return Err(eframe::Error::AppCreation(
                std::io::Error::other(
                    "Required model/runtime files are missing or invalid; see diagnostics JSON",
                )
                .into(),
            ));
        }
        return Ok(());
    }
    if options.clear_models {
        let report = photosort::core::model_assets::clear_managed(
            &photosort::core::session::data_dir(),
            &|| false,
        )
        .map_err(|e| eframe::Error::AppCreation(e.into()))?;
        println!(
            "Removed {} managed model copies ({} bytes); bundled resources and files outside the managed model set are preserved.",
            report.removed.len(),
            report.bytes
        );
        if report.cancelled || !report.errors.is_empty() {
            return Err(eframe::Error::AppCreation(
                std::io::Error::other(format!(
                    "Model clearing did not complete: {}",
                    report.errors.join("; ")
                ))
                .into(),
            ));
        }
    }
    if options.clear_cache {
        photosort::core::cache::maintain(&photosort::core::session::cache_dir(), Some(0), || false)
            .map_err(|e| eframe::Error::AppCreation(e.into()))?;
    }
    let smoke_delay = options.smoke_delay();
    let folder = if options.last_folder {
        use photosort::core::preferences::Preferences;
        Preferences::last_folder(&Preferences::path())
    } else {
        options.folder
    };
    photosort::ui::initialize_native_application()
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("PhotoSort")
        .with_inner_size([1440.0, 960.0])
        .with_min_inner_size([640.0, 600.0]);
    // macOS: one bar. The window draws behind a transparent title bar and the
    // close, minimise and zoom buttons sit in PhotoSort's own top bar. Other
    // systems keep their native title bar.
    if cfg!(target_os = "macos") {
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    match photosort::ui::app_icon() {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(error) => tracing::warn!(%error, "app icon unavailable"),
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "PhotoSort",
        options,
        Box::new(move |cc| {
            let mut app = photosort::ui::PhotoSortApp::new(cc, folder);
            if let Some(delay) = smoke_delay {
                app = app.with_smoke_test(delay);
            }
            Ok(Box::new(app))
        }),
    )
}

/// A GUI-subsystem program has no console of its own. Started from a
/// terminal, it writes `--help`, `--diagnostics` and its log there; started
/// from Explorer there is none to attach and nothing changes. Output the
/// caller redirected to a file or pipe keeps its handles.
#[cfg(windows)]
fn attach_parent_console() {
    use std::{ffi::c_void, ptr::null_mut};
    type Handle = *mut c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn GetStdHandle(which: u32) -> Handle;
        fn SetStdHandle(which: u32, handle: Handle) -> i32;
        fn GetFileType(handle: Handle) -> u32;
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;
    const FILE_TYPE_UNKNOWN: u32 = 0;
    const GENERIC_READ_WRITE: u32 = 0x8000_0000 | 0x4000_0000;
    const FILE_SHARE_READ_WRITE: u32 = 1 | 2;
    const OPEN_EXISTING: u32 = 3;
    let invalid = usize::MAX as Handle;
    let console: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
    // SAFETY: plain Win32 and C runtime calls on this process's own standard
    // handles and descriptors, made first in main before any other thread
    // exists; `console` outlives the call that reads it. Handles opened here
    // are kept for the life of the process.
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for (which, descriptor) in [(STD_OUTPUT_HANDLE, 1), (STD_ERROR_HANDLE, 2)] {
            let mut handle = GetStdHandle(which);
            if handle.is_null() || handle == invalid || GetFileType(handle) == FILE_TYPE_UNKNOWN {
                handle = CreateFileW(
                    console.as_ptr(),
                    GENERIC_READ_WRITE,
                    FILE_SHARE_READ_WRITE,
                    null_mut(),
                    OPEN_EXISTING,
                    0,
                    null_mut(),
                );
                if handle == invalid {
                    continue;
                }
                SetStdHandle(which, handle);
            }
            // The C runtime started without these descriptors; native
            // libraries and `diagnostic_output` write through them.
            if libc::get_osfhandle(descriptor) < 0 {
                let opened = libc::open_osfhandle(handle as libc::intptr_t, 0);
                if opened >= 0 && opened != descriptor {
                    libc::dup2(opened, descriptor);
                }
            }
        }
    }
}
#[cfg(not(windows))]
fn attach_parent_console() {}
