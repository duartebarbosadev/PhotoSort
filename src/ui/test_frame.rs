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
