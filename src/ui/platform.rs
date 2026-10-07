//! Native application requests feed the same Rust departure handler as window close.
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};

/// Called on the main thread before eframe creates the shared NSApplication.
pub fn initialize_native_application() -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    // SAFETY: main calls this before creating the event loop; the native function
    // validates the main thread and refuses to replace an existing application.
    if !unsafe { photosort_application_initialize() } {
        return Err(std::io::Error::other(
            "Could not initialize PhotoSort's native Quit handling",
        ));
    }
    Ok(())
}
struct Pending {
    requested: AtomicBool,
    #[cfg(any(target_os = "macos", test))]
    ctx: egui::Context,
}
impl Pending {
    #[cfg(any(target_os = "macos", test))]
    fn request(&self) {
        self.requested.store(true, Ordering::Release);
        self.ctx.request_repaint();
    }
    fn take(&self) -> bool {
        self.requested.swap(false, Ordering::AcqRel)
    }
}
/// Boxed callback state stays alive until the native callback is unregistered.
/// Main-thread-only ownership also makes destruction safe against native actions.
pub(super) struct QuitHandler {
    pending: Box<Pending>,
    _main_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl QuitHandler {
    pub(super) fn install(ctx: &egui::Context) -> Result<Option<Self>, String> {
        #[cfg(target_os = "macos")]
        {
            let mut handler = Self {
                pending: Box::new(Pending {
                    requested: AtomicBool::new(false),
                    ctx: ctx.clone(),
                }),
                _main_thread: Default::default(),
            };
            // SAFETY: the Box address is stable; Drop unregisters before freeing
            // it. Native installation validates the main thread and sole owner.
            if !unsafe { photosort_quit_install(request_quit, handler.context()) } {
                return Err(
                    "Native Quit handling could not be attached. Use the window close button."
                        .into(),
                );
            }
            Ok(Some(handler))
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = ctx;
            Ok(None)
        }
    }
    pub(super) fn take_requested(&self) -> bool {
        self.pending.take()
    }
    #[cfg(target_os = "macos")]
    fn context(&mut self) -> *mut std::ffi::c_void {
        (&mut *self.pending as *mut Pending).cast()
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn photosort_application_initialize() -> bool;
    fn photosort_quit_install(
        callback: extern "C" fn(*mut std::ffi::c_void),
        context: *mut std::ffi::c_void,
    ) -> bool;
    fn photosort_quit_remove(context: *mut std::ffi::c_void);
}
#[cfg(target_os = "macos")]
extern "C" fn request_quit(context: *mut std::ffi::c_void) {
    // SAFETY: only the installed native callback invokes this, on the main
    // thread, while its QuitHandler owns this non-null boxed Pending state.
    unsafe { &*context.cast::<Pending>() }.request();
}
impl Drop for QuitHandler {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        // SAFETY: ownership cannot move off the main thread, and the native side
        // only removes this handler's context. The Box is freed after this call.
        unsafe {
            photosort_quit_remove(self.context());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_native_requests_coalesce_and_can_be_requested_again_after_cancel() {
        let pending = Pending {
            requested: AtomicBool::new(false),
            ctx: egui::Context::default(),
        };
        assert!(!pending.take());
        pending.request();
        pending.request();
        assert!(pending.take());
        assert!(!pending.take());
        pending.request();
        assert!(pending.take());
    }
}
