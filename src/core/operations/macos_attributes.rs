//! macOS metadata for the shared exclusive move fallback.
use super::{MoveCancelled, check_move_cancelled};
use anyhow::{Context, Result};
use std::{ffi::c_void, fs::File, os::fd::AsRawFd};

unsafe extern "C" {
    fn photosort_copy_xattrs(
        source: i32,
        destination: i32,
        cancelled: extern "C" fn(*mut c_void) -> i32,
        context: *mut c_void,
    ) -> i32;
}
struct Cancellation<'a> {
    check: &'a dyn Fn() -> bool,
    panicked: bool,
}
extern "C" fn cancelled(opaque: *mut c_void) -> i32 {
    // The synchronous C call borrows this stack value and never retains it.
    let context = unsafe { &mut *opaque.cast::<Cancellation<'_>>() };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(context.check)) {
        Ok(stopped) => i32::from(stopped),
        Err(_) => {
            // Do not unwind through C or bypass the caller's rollback path.
            context.panicked = true;
            1
        }
    }
}
pub(super) fn copy(input: &File, output: &File, check: &dyn Fn() -> bool) -> Result<()> {
    check_move_cancelled(check)?;
    let mut context = Cancellation {
        check,
        panicked: false,
    };
    // Both descriptors remain open throughout this synchronous call.
    let status = unsafe {
        photosort_copy_xattrs(
            input.as_raw_fd(),
            output.as_raw_fd(),
            cancelled,
            (&mut context as *mut Cancellation<'_>).cast(),
        )
    };
    anyhow::ensure!(
        !context.panicked,
        "Extended-attribute cancellation callback panicked"
    );
    if status == -1 {
        return Err(MoveCancelled.into());
    }
    if status != 0 {
        return Err(std::io::Error::from_raw_os_error(status))
            .context("Could not preserve file extended attributes");
    }
    check_move_cancelled(check)
}
