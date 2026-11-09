mod brand;
#[cfg(test)]
mod test_frame;
mod theme;
mod workspace;
pub use workspace::PhotoSortApp;

mod platform;
pub use platform::{app_icon, initialize_native_application};

/// The window's narrowest width, at which the top bar takes its tightest
/// fit. On Windows the window has no frame and its own buttons take the
/// bar's right end, so the window starts that much wider.
pub const MIN_WINDOW_WIDTH: f32 = if cfg!(windows) {
    FRAMELESS_MIN_WIDTH
} else {
    640.0
};
/// 640 points plus room for the three 46 point window buttons.
const FRAMELESS_MIN_WIDTH: f32 = 780.0;
