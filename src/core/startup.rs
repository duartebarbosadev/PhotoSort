//! Parse startup arguments before initializing graphics, workers or models.
use std::{ffi::OsString, path::PathBuf};
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub folder: Option<PathBuf>,
    pub last_folder: bool,
    pub clear_cache: bool,
    pub clear_models: bool,
    pub diagnostics: bool,
    pub check_runtime: bool,
    pub smoke_test: bool,
    pub smoke_delay_ms: Option<u64>,
    pub help: bool,
    pub version: bool,
}
impl Options {
    pub fn smoke_delay(&self) -> Option<std::time::Duration> {
        self.smoke_test
            .then(|| std::time::Duration::from_millis(self.smoke_delay_ms.unwrap_or(2000)))
    }
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut result = Self::default();
        let mut args = args.into_iter();
        let mut positional = false;
        while let Some(arg) = args.next() {
            if !positional && arg == "--" {
                positional = true;
                continue;
            }
            if !positional && (arg == "--help" || arg == "-h") {
                result.help = true;
                return Ok(result);
            }
            if !positional && (arg == "--version" || arg == "-V") {
                result.version = true;
                return Ok(result);
            }
            if !positional && arg == "--smoke-test" {
                result.smoke_test = true;
                continue;
            }
            if !positional && arg == "--smoke-delay-ms" {
                if result.smoke_delay_ms.is_some() {
                    return Err("Specify --smoke-delay-ms only once.".into());
                }
                let delay = args
                    .next()
                    .and_then(|value| value.to_str().and_then(|s| s.parse::<u64>().ok()));
                result.smoke_delay_ms = Some(
                    delay
                        .filter(|value| (200..=60_000).contains(value))
                        .ok_or("--smoke-delay-ms requires an integer from 200 to 60000.")?,
                );
                continue;
            }
            if !positional && arg == "--clear-models" {
                result.clear_models = true;
                continue;
            }
            if !positional && arg == "--check-runtime" {
                result.check_runtime = true;
                continue;
            }
            if !positional && arg == "--diagnostics" {
                result.diagnostics = true;
                continue;
            }
            if !positional && arg == "--clear-cache" {
                result.clear_cache = true;
                continue;
            }
            if !positional && arg == "--last-folder" {
                if result.last_folder || result.folder.is_some() {
                    return Err("Choose one folder or --last-folder.".into());
                }
                result.last_folder = true;
                continue;
            }
            let folder = if !positional && arg == "--folder" {
                let path = args.next().ok_or("--folder requires a path.")?;
                if path.is_empty() || path.to_string_lossy().starts_with("--") {
                    return Err("--folder requires a path.".into());
                }
                path
            } else {
                if !positional && arg.to_string_lossy().starts_with('-') {
                    return Err(format!(
                        "Unknown option: {}. Use --help.",
                        arg.to_string_lossy()
                    ));
                }
                arg
            };
            if result.folder.is_some() || result.last_folder {
                return Err("Choose one folder or --last-folder.".into());
            }
            result.folder = Some(folder.into());
        }
        if result.smoke_delay_ms.is_some() && !result.smoke_test {
            return Err("--smoke-delay-ms requires --smoke-test.".into());
        }
        if (result.diagnostics && result.check_runtime)
            || ((result.diagnostics || result.check_runtime)
                && (result.folder.is_some()
                    || result.last_folder
                    || result.clear_cache
                    || result.clear_models
                    || result.smoke_test))
        {
            return Err(
                "Diagnostic commands run alone and never change files or open a folder.".into(),
            );
        }
        Ok(result)
    }
}
pub const HELP: &str = "PhotoSort Rust\n\nUsage: photosort [--folder PATH | --last-folder]\n       photosort [PATH]\n\n  --folder PATH   Open a photo folder\n  --last-folder   Open the most recent folder that still exists\n  --clear-cache   Clear regenerable previews and analysis evidence before startup\n  --clear-models  Remove managed model copies before startup; keep bundled models\n  --diagnostics   Print JSON file/readiness diagnostics and exit without a window\n  --check-runtime Run native inference checks, print JSON and exit without a window\n  --smoke-test    Request normal guarded quit after the first UI frame and delay\n  --smoke-delay-ms N  Delay of 200–60000 ms (default 2000; requires --smoke-test)\n  --help, -h      Show this help\n  --version, -V   Show the version\n\nAnalysis stays local. File changes require a reviewed plan.";
