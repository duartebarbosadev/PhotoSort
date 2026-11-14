//! Native file pickers, and what they leave behind in egui's keyboard state.
use eframe::egui;
use std::path::PathBuf;

/// Shows a native picker and returns what was chosen there.
///
/// The picker runs inside the frame that asked for it and has the keyboard
/// until it closes, so the key that opened it (the O of Cmd+O) is let go
/// without egui hearing of it. egui forgets held keys only when told the
/// window lost focus, and on macOS that notice is delivered after the picker
/// has closed, when the window reads as focused again. Left alone, egui takes
/// the next press of that key for a held key repeating, which shortcuts
/// ignore: every second press was lost. Nothing is held once a picker closes.
///
/// A menu the picker was chosen from closes too, whether a folder was picked
/// or the picker was cancelled: an open menu keeps the shortcuts to itself,
/// and one that stays open on clicks inside would be left over the window.
pub(super) fn pick(
    ctx: &egui::Context,
    dialog: rfd::FileDialog,
    choose: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
) -> Option<PathBuf> {
    let chosen = show(dialog, choose);
    ctx.input_mut(|input| input.keys_down.clear());
    egui::Popup::close_all(ctx);
    chosen
}

#[cfg(not(test))]
fn show(
    dialog: rfd::FileDialog,
    choose: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
) -> Option<PathBuf> {
    choose(dialog)
}

/// Tests open no window: the picker counts as shown and then cancelled.
#[cfg(test)]
fn show(
    _dialog: rfd::FileDialog,
    _choose: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>,
) -> Option<PathBuf> {
    SHOWN.with(|shown| shown.set(shown.get() + 1));
    None
}

#[cfg(test)]
thread_local! {
    static SHOWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many pickers this test's thread has shown.
#[cfg(test)]
pub(super) fn shown() -> usize {
    SHOWN.with(std::cell::Cell::get)
}
