//! Process-wide routing for the terminal-only runtime diagnostic, before native loading.
use filedescriptor::{FileDescriptor, StdioDescriptor};
use std::io::Write;

pub fn writer() -> anyhow::Result<FileDescriptor> {
    std::io::stdout().flush()?;
    #[cfg(not(windows))]
    let output = FileDescriptor::redirect_stdio(&std::io::stderr(), StdioDescriptor::Stdout)?;
    #[cfg(windows)]
    let output = {
        use std::os::windows::io::IntoRawHandle;
        // Windows has both Win32 standard handles and CRT file descriptors.
        // Keep an independent report handle before changing either namespace.
        let output = FileDescriptor::dup(&std::io::stdout())?;
        let previous = FileDescriptor::redirect_stdio(&std::io::stderr(), StdioDescriptor::Stdout)?;
        // The original handle belongs to CRT stdout; _dup2 closes it. Do not
        // let the descriptor wrapper close that same handle a second time.
        let _ = previous.into_raw_handle();
        // SAFETY: this runs before model loading/threads in a terminal-only
        // command. 1 and 2 are the process stdout/stderr CRT descriptors.
        anyhow::ensure!(
            unsafe { libc::dup2(2, 1) } >= 0,
            "Cannot route native stdout to stderr: {}",
            std::io::Error::last_os_error()
        );
        output
    };
    // Intentionally never restore stdout: native buffers can flush at process
    // exit, after the report and its models have been dropped. The returned
    // descriptor writes only the JSON to the caller's original stdout.
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_json_keeps_native_and_late_output_on_stderr() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diagnostic_output::tests::runtime_json_child",
                "--ignored",
                "--nocapture",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        // Ignore the test harness prelude; production emits no prelude.
        let report = &stdout[stdout.find('{').unwrap()..];
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(report).unwrap(),
            serde_json::json!({"ready":true})
        );
        assert!(stderr.contains("runtime output") && stderr.contains("after writer drop"));
        #[cfg(unix)]
        assert!(stderr.contains("native exit output"));
    }
    #[test]
    #[ignore = "Runs only in an isolated child; changes process stdio and exits"]
    fn runtime_json_child() {
        let mut output = writer().unwrap();
        println!("runtime output");
        output.write_all(b"{\"ready\":true}\n").unwrap();
        output.flush().unwrap();
        drop(output);
        println!("after writer drop");
        #[cfg(unix)]
        {
            unsafe extern "C" {
                fn puts(text: *const std::ffi::c_char) -> std::ffi::c_int;
                fn atexit(callback: extern "C" fn()) -> std::ffi::c_int;
            }
            extern "C" fn late() {
                // SAFETY: a valid static NUL-terminated string; libc owns stdout.
                unsafe {
                    puts(c"native exit output".as_ptr());
                }
            }
            // SAFETY: static no-argument callback valid through process shutdown.
            assert_eq!(unsafe { atexit(late) }, 0);
        }
        std::process::exit(0);
    }
}
