//! What this computer can afford. Full-size decodes are checked against the
//! installed memory rather than a fixed pixel count, and cache defaults scale
//! with memory and free disk. A header claiming an impossible size is still
//! refused: that file is corrupt or a decompression bomb, not a photo.
use std::{
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
/// Above any camera, scanner or stitched panorama: about 4.3 gigapixels.
pub const MAX_PIXELS: u64 = 1 << 32;
/// The longest edge a decoder is asked for. JPEG and LibRaw stop at 65535
/// themselves; TIFF and PNG panoramas can be longer.
pub const MAX_EDGE: u32 = 1 << 20;
/// Assumed when the platform does not say how much memory it has.
const FALLBACK_RAM: u64 = 8 * GIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Machine {
    pub ram: u64,
    /// Free space where the cache lives, when the platform says.
    pub free_disk: Option<u64>,
}
/// How a decode of some size suits this computer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// At most half the memory: it opens without asking.
    Fits,
    /// Up to 85%: other apps may slow down while it loads.
    Large,
    /// Up to one and a half times the memory: the system has to swap.
    Exceeds,
    /// More than that: only the preview is shown.
    Impossible,
}
impl Machine {
    /// Probed once per process, so defaults and limits stay steady while it runs.
    pub fn current() -> Self {
        static MACHINE: OnceLock<Machine> = OnceLock::new();
        *MACHINE.get_or_init(|| Machine {
            ram: total_ram()
                .filter(|&ram| ram >= GIB)
                .unwrap_or(FALLBACK_RAM),
            free_disk: free_disk(&super::session::cache_dir()),
        })
    }
    /// The largest decode that goes ahead without asking.
    pub fn quiet_allowance(self) -> u64 {
        self.ram / 2
    }
    /// The largest decode once the user chose to open a photo anyway.
    pub fn forced_allowance(self) -> u64 {
        self.ram / 2 * 3
    }
    pub fn fit(self, needed: u64) -> Fit {
        if needed <= self.quiet_allowance() {
            Fit::Fits
        } else if needed <= self.ram / 100 * 85 {
            Fit::Large
        } else if needed <= self.forced_allowance() {
            Fit::Exceeds
        } else {
            Fit::Impossible
        }
    }
}

/// How short the whole system is of memory right now, as the platform says.
/// Caches shrink while it is high and grow back once it clears.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    #[default]
    Normal,
    /// The system is compressing or swapping: hold less.
    Warn,
    /// Close to running out: hold only what is on screen.
    Critical,
}
static PRESSURE: AtomicU8 = AtomicU8::new(0);
impl Pressure {
    /// The current level. The first call starts a thread that checks the
    /// system every two seconds; a check costs one system call.
    pub fn current() -> Self {
        static MONITOR: OnceLock<()> = OnceLock::new();
        MONITOR.get_or_init(|| {
            let started = std::thread::Builder::new()
                .name("memory-pressure".into())
                .spawn(|| {
                    loop {
                        let level = system_pressure().unwrap_or_default();
                        let previous = PRESSURE.swap(level as u8, Ordering::Relaxed);
                        if previous != level as u8 {
                            tracing::info!(?level, "memory pressure changed");
                        }
                        std::thread::sleep(Duration::from_secs(2));
                    }
                });
            if let Err(error) = started {
                tracing::warn!(%error, "memory pressure is not watched");
            }
        });
        match PRESSURE.load(Ordering::Relaxed) {
            0 => Self::Normal,
            1 => Self::Warn,
            _ => Self::Critical,
        }
    }
    /// What memory caches keep of their budgets: all, half or a quarter.
    pub fn share(self, budget: usize) -> usize {
        match self {
            Self::Normal => budget,
            Self::Warn => budget / 2,
            Self::Critical => budget / 4,
        }
    }
    /// macOS `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warn, 4 critical.
    #[cfg(any(target_os = "macos", test))]
    fn from_darwin(level: i32) -> Self {
        match level {
            ..=1 => Self::Normal,
            2 | 3 => Self::Warn,
            _ => Self::Critical,
        }
    }
    /// From the share of memory still available (Linux, Windows).
    #[cfg(any(not(target_os = "macos"), test))]
    fn from_available(available: u64, total: u64) -> Self {
        if total == 0 || available.saturating_mul(20) >= total.saturating_mul(2) {
            Self::Normal
        } else if available.saturating_mul(20) >= total {
            Self::Warn
        } else {
            Self::Critical
        }
    }
}
#[cfg(target_os = "macos")]
fn system_pressure() -> Option<Pressure> {
    let mut level = 0i32;
    let mut size = std::mem::size_of::<i32>();
    // SAFETY: the value is an int; the out pointers live for the call.
    let result = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            (&raw mut level).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then(|| Pressure::from_darwin(level))
}
#[cfg(target_os = "linux")]
fn system_pressure() -> Option<Pressure> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<u64> {
        let line = info.lines().find(|line| line.starts_with(name))?;
        line.split_whitespace().nth(1)?.parse().ok()
    };
    Some(Pressure::from_available(
        field("MemAvailable:")?,
        field("MemTotal:")?,
    ))
}
#[cfg(target_os = "windows")]
fn system_pressure() -> Option<Pressure> {
    let (total, available) = windows_memory()?;
    Some(Pressure::from_available(available, total))
}
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn system_pressure() -> Option<Pressure> {
    None
}

/// A decode refused because it needs more memory than it was allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooLarge {
    /// Estimated peak bytes of the decode.
    pub needed: u64,
    pub width: u32,
    pub height: u32,
}
impl std::fmt::Display for TooLarge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Opening this photo at full size needs about {} of memory ({} x {})",
            size_text(self.needed),
            self.width,
            self.height
        )
    }
}
impl std::error::Error for TooLarge {}

/// Refuses sizes no real photo has. These are never overridden.
pub fn check_size(width: u32, height: u32) -> anyhow::Result<()> {
    anyhow::ensure!(
        width > 0
            && height > 0
            && width <= MAX_EDGE
            && height <= MAX_EDGE
            && u64::from(width) * u64::from(height) <= MAX_PIXELS,
        "This file reports an impossible size ({width} x {height})"
    );
    Ok(())
}
/// Refuses a decode of `needed` bytes over `allowance`, saying how much it needs.
pub fn require(needed: u64, allowance: u64, width: u32, height: u32) -> anyhow::Result<()> {
    if needed > allowance {
        return Err(TooLarge {
            needed,
            width,
            height,
        }
        .into());
    }
    Ok(())
}

/// "3.8 GB", "450 MB".
pub fn size_text(bytes: u64) -> String {
    if bytes >= GIB {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    } else {
        format!("{} MB", bytes.div_ceil(MIB))
    }
}

#[cfg(target_os = "macos")]
fn total_ram() -> Option<u64> {
    let mut ram = 0u64;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: hw.memsize is a u64; the out pointers live for the call.
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            (&raw mut ram).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0).then_some(ram)
}
#[cfg(target_os = "linux")]
fn total_ram() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}
#[cfg(target_os = "windows")]
fn total_ram() -> Option<u64> {
    windows_memory().map(|(total, _)| total)
}
/// Total and available physical memory.
#[cfg(target_os = "windows")]
fn windows_memory() -> Option<(u64, u64)> {
    #[repr(C)]
    struct MemoryStatusEx {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }
    unsafe extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatusEx) -> i32;
    }
    // SAFETY: plain-old-data out parameter whose length field is set first.
    let mut status: MemoryStatusEx = unsafe { std::mem::zeroed() };
    status.length = std::mem::size_of::<MemoryStatusEx>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0)
        .then_some((status.total_physical, status.available_physical))
}
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn total_ram() -> Option<u64> {
    None
}

/// The cache folder may not exist yet: measure its nearest existing parent.
fn existing(path: &Path) -> Option<&Path> {
    path.ancestors().find(|path| path.exists())
}
#[cfg(unix)]
fn free_disk(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(existing(path)?.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs fills the zeroed struct; the path outlives the call.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    (unsafe { libc::statvfs(path.as_ptr(), &mut stats) } == 0)
        .then(|| (stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}
#[cfg(windows)]
fn free_disk(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let wide: Vec<u16> = existing(path)?
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut available = 0u64;
    // SAFETY: a NUL-terminated path; unused outputs may be null.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}
#[cfg(not(any(unix, windows)))]
fn free_disk(_: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_open_quietly_up_to_half_the_memory_and_never_past_one_and_a_half() {
        let machine = Machine {
            ram: 16 * GIB,
            free_disk: None,
        };
        assert_eq!(machine.fit(8 * GIB), Fit::Fits);
        assert_eq!(machine.fit(8 * GIB + 1), Fit::Large);
        assert_eq!(machine.fit(14 * GIB), Fit::Exceeds);
        assert_eq!(machine.fit(24 * GIB), Fit::Exceeds);
        assert_eq!(machine.fit(24 * GIB + 1), Fit::Impossible);
        let refused = require(9 * GIB, machine.quiet_allowance(), 24_000, 16_000).unwrap_err();
        assert_eq!(
            refused.downcast_ref::<TooLarge>().map(|t| t.needed),
            Some(9 * GIB)
        );
        assert!(require(9 * GIB, machine.forced_allowance(), 24_000, 16_000).is_ok());
        // A 400 MP pixel-shift file is a photo; a 70000 px square JPEG
        // header past 4.3 GP is not.
        assert!(check_size(23_296, 17_472).is_ok());
        assert!(check_size(70_000, 70_000).is_err());
        assert!(check_size(0, 10).is_err());
        assert!(Machine::current().ram >= GIB);
    }
    #[test]
    fn pressure_levels_follow_the_system_and_shrink_budgets() {
        assert_eq!(Pressure::from_darwin(1), Pressure::Normal);
        assert_eq!(Pressure::from_darwin(2), Pressure::Warn);
        assert_eq!(Pressure::from_darwin(4), Pressure::Critical);
        assert_eq!(
            Pressure::from_available(4 * GIB, 16 * GIB),
            Pressure::Normal
        );
        assert_eq!(Pressure::from_available(GIB, 16 * GIB), Pressure::Warn);
        assert_eq!(
            Pressure::from_available(GIB / 2, 16 * GIB),
            Pressure::Critical
        );
        assert_eq!(Pressure::Warn.share(1024), 512);
        assert_eq!(Pressure::Critical.share(1024), 256);
    }
}
