mod diagnostic_output;
mod session_logging;
#[cfg(test)]
mod test_support;

fn main() -> eframe::Result {
    // The bundled ONNX Runtime uploader can race its own C++ static teardown.
    // Suppress it before runtime loading; the session API is too late to prevent
    // its initialization event and background uploader from being created.
    // SAFETY: this is the first application action, before native UI, model
    // loading or any application worker can start or read the environment.
    #[cfg(feature = "onnx")]
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
            options.runtime_backend.unwrap_or_default(),
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
        photosort::core::preferences::Preferences::load(
            &photosort::core::preferences::Preferences::path(),
        )
        .map_err(|e| eframe::Error::AppCreation(e.into()))?
        .last_existing_folder()
    } else {
        options.folder
    };
    photosort::ui::initialize_native_application()
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?;
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("PhotoSort — Keep what matters")
        .with_inner_size([1440.0, 960.0])
        .with_min_inner_size([640.0, 600.0]);
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
