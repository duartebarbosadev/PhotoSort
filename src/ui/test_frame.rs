//! Test frames. egui 0.36 asserts that every frame's texture updates are
//! handled; headless tests never upload textures, so they clear them on
//! purpose and keep the rest of the output (platform and AccessKit trees).
use eframe::egui;

pub(crate) trait TestFrame {
    fn run_test(&self, input: egui::RawInput, ui: impl FnMut(&mut egui::Ui)) -> egui::FullOutput;
}

impl TestFrame for egui::Context {
    fn run_test(&self, input: egui::RawInput, ui: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut output = self.run_ui(input, ui);
        output.textures_delta.clear();
        output
    }
}

/// A rooted path that is absolute on every platform, for tests naming files
/// outside any real folder: `/library/a.jpg` on Unix, `C:\library\a.jpg` on Windows.
#[cfg(test)]
pub(crate) fn absolute(path: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        std::path::PathBuf::from(format!("C:{}", path.replace('/', "\\")))
    } else {
        std::path::PathBuf::from(path)
    }
}
