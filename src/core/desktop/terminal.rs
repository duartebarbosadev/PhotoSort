//! Small platform adapters for opening an interactive terminal without a command string.
use anyhow::{Context, Result, ensure};
use std::{
    path::Path,
    process::{Child, Command},
    sync::Mutex,
};

#[cfg(any(not(target_os = "windows"), test))]
use std::process::Stdio;

#[derive(Default)]
pub(super) struct Children(Mutex<Vec<Child>>);
impl Children {
    pub fn open(&self, folder: &Path) -> Result<()> {
        let mut command = command(folder)?;
        #[cfg(target_os = "macos")]
        {
            // LaunchServices returns after handing the directory to Terminal.
            let status = command.status().context("Could not launch Terminal")?;
            ensure!(
                status.success(),
                "Terminal could not open this folder ({status})"
            );
        }
        #[cfg(not(target_os = "macos"))]
        {
            #[cfg(target_os = "linux")]
            {
                // Validate the user's desktop entry before starting a possibly
                // long-lived terminal process. No arbitrary command is requested.
                let status = Command::new("xdg-terminal-exec")
                    .arg("--print-id")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .context("Install xdg-terminal-exec and configure a preferred terminal")?;
                ensure!(
                    status.success(),
                    "Configure a preferred terminal for xdg-terminal-exec"
                );
            }
            let child = command
                .spawn()
                .context("Could not open a terminal in this folder")?;
            self.0.lock().unwrap().push(child);
        }
        Ok(())
    }
    pub fn poll(&self) -> bool {
        let Ok(mut children) = self.0.try_lock() else {
            return true;
        };
        children.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
        !children.is_empty()
    }
    // Dropping Child closes its handle, without terminating the user's terminal.
    // At app exit the OS adopts any live Unix children; no reaper thread is needed.
}

fn command(folder: &Path) -> Result<Command> {
    ensure!(
        folder.is_absolute(),
        "Terminal requires an absolute folder path"
    );
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("/usr/bin/open");
        command.args(["-b", "com.apple.Terminal", "--"]).arg(folder);
        command
    };
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut argument = std::ffi::OsString::from("--dir=");
        argument.push(folder);
        let mut command = Command::new("xdg-terminal-exec");
        command.arg(argument);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        use std::os::windows::process::CommandExt;
        let system =
            std::env::var_os("SystemRoot").context("Windows system directory is unavailable")?;
        let system = std::path::PathBuf::from(system);
        ensure!(
            system.is_absolute(),
            "Windows system directory must be absolute"
        );
        let mut command =
            Command::new(system.join("System32/WindowsPowerShell/v1.0/powershell.exe"));
        // A new console uses the configured Windows console host. The path is
        // process state, never a PowerShell expression or a cmd /K command.
        command
            .args(["-NoLogo", "-NoExit"])
            .creation_flags(0x00000010);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    anyhow::bail!("Terminal integration is unavailable on this platform");
    command.current_dir(folder);
    #[cfg(not(target_os = "windows"))]
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::worker_patience;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn terminal_path_is_literal_process_data() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("photos ' café $(touch SENTINEL) & %PATH%;");
        let command = command(&folder).unwrap();
        assert_eq!(command.get_current_dir(), Some(folder.as_path()));
        let args: Vec<_> = command.get_args().collect();
        #[cfg(target_os = "macos")]
        {
            assert_eq!(command.get_program(), "/usr/bin/open");
            assert_eq!(
                args,
                [
                    std::ffi::OsStr::new("-b"),
                    std::ffi::OsStr::new("com.apple.Terminal"),
                    std::ffi::OsStr::new("--"),
                    folder.as_os_str()
                ]
            );
        }
        #[cfg(target_os = "linux")]
        {
            assert_eq!(command.get_program(), "xdg-terminal-exec");
            let mut expected = std::ffi::OsString::from("--dir=");
            expected.push(&folder);
            assert_eq!(args, [expected.as_os_str()]);
        }
        #[cfg(target_os = "windows")]
        {
            assert_eq!(args, ["-NoLogo", "-NoExit"]);
            assert!(Path::new(command.get_program()).is_absolute());
        }
        assert!(!dir.path().join("SENTINEL").exists());
        assert!(super::command(Path::new("relative")).is_err());
    }

    #[test]
    fn child_probe() {
        if std::env::var_os("PHOTOSORT_TERMINAL_CHILD_PROBE").is_some() {
            std::fs::write("probe-ready", "ready").unwrap();
            let deadline = Instant::now() + worker_patience();
            while !Path::new("probe-release").exists() {
                assert!(
                    Instant::now() < deadline,
                    "Parent did not release child probe"
                );
                thread::sleep(Duration::from_millis(5));
            }
        }
    }

    #[test]
    fn live_children_do_not_block_and_finished_children_are_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "core::desktop::terminal::tests::child_probe"])
            .env("PHOTOSORT_TERMINAL_CHILD_PROBE", "1")
            .current_dir(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let children = Children(Mutex::new(vec![child]));
        let end = Instant::now() + worker_patience();
        while !dir.path().join("probe-ready").exists() {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(children.poll());
        // Contention must not wait for a concurrent spawn to release the pool.
        let guard = children.0.lock().unwrap();
        assert!(children.poll());
        drop(guard);
        std::fs::write(dir.path().join("probe-release"), "release").unwrap();
        while children.poll() {
            assert!(Instant::now() < end);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(children.0.lock().unwrap().is_empty());
    }
}
