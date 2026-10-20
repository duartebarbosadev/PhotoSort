#[cfg(target_os = "macos")]
mod allocator;
pub mod core;
pub mod ui;

#[cfg(target_os = "macos")]
#[global_allocator]
static ALLOCATOR: allocator::Allocator = allocator::Allocator;
