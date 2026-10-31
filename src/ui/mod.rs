mod brand;
#[cfg(test)]
mod test_frame;
mod theme;
mod workspace;
pub use workspace::PhotoSortApp;

mod platform;
pub use platform::{app_icon, initialize_native_application};
