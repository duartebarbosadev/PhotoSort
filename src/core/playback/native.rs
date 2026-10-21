use anyhow::{Context, Result};
use image::RgbaImage;
use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::Path,
};
#[repr(C)]
#[derive(Default)]
pub struct Status {
    pub position: f64,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub ready: i32,
    pub playing: i32,
    pub ended: i32,
    pub seeking: i32,
}
unsafe extern "C" {
    fn ps_video_thumbnail(
        path: *const c_char,
        cancelled: extern "C" fn(*mut c_void) -> i32,
        context: *mut c_void,
        pixels: *mut u8,
        capacity: usize,
        width: *mut u32,
        height: *mut u32,
        error: *mut c_char,
        size: usize,
    ) -> i32;

    fn ps_video_open(
        path: *const c_char,
        cancelled: extern "C" fn(*mut c_void) -> i32,
        context: *mut c_void,
        error: *mut c_char,
        size: usize,
    ) -> *mut c_void;
    fn ps_video_close(handle: *mut c_void);
    fn ps_video_control(handle: *mut c_void, play: i32, seek: f64, volume: f32);
    fn ps_video_poll(
        handle: *mut c_void,
        status: *mut Status,
        pixels: *mut u8,
        capacity: usize,
        error: *mut c_char,
        size: usize,
    ) -> i32;
}
/// Created, controlled, polled and dropped only on the playback owner thread.
pub struct Player {
    handle: *mut c_void,
    pixels: Vec<u8>,
}
impl Player {
    pub fn open(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Self> {
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_os_str().as_bytes())?;
        let mut error = [0i8; 1024];
        // Native ownership transfers to this value; the input/error buffers live through the call.
        let handle = unsafe {
            ps_video_open(
                path.as_ptr(),
                check,
                &cancelled as *const _ as *mut c_void,
                error.as_mut_ptr(),
                error.len(),
            )
        };
        anyhow::ensure!(
            !handle.is_null(),
            "{}",
            unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy()
        );
        Ok(Self {
            handle,
            pixels: vec![0; 1280 * 1280 * 4],
        })
    }
    pub fn control(&mut self, playing: bool, seek: Option<f64>, volume: f32) {
        unsafe {
            ps_video_control(
                self.handle,
                i32::from(playing),
                seek.unwrap_or(-1.0),
                volume,
            )
        };
    }
    pub fn poll(&mut self) -> Result<(Status, Option<RgbaImage>)> {
        let mut status = Status::default();
        let mut error = [0i8; 1024];
        let result = unsafe {
            ps_video_poll(
                self.handle,
                &mut status,
                self.pixels.as_mut_ptr(),
                self.pixels.len(),
                error.as_mut_ptr(),
                error.len(),
            )
        };
        anyhow::ensure!(
            result >= 0,
            "{}",
            unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy()
        );
        let image = if result == 1 {
            let len = status.width as usize * status.height as usize * 4;
            anyhow::ensure!(len <= self.pixels.len(), "Invalid native frame size");
            Some(
                RgbaImage::from_raw(status.width, status.height, self.pixels[..len].to_vec())
                    .context("Invalid frame geometry")?,
            )
        } else {
            None
        };
        Ok((status, image))
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        unsafe { ps_video_close(self.handle) }
    }
}

impl super::Backend for Player {
    fn control(&mut self, playing: bool, seek: Option<f64>, volume: f32) {
        self.control(playing, seek, volume)
    }
    fn poll(&mut self) -> Result<super::Decoded> {
        let (s, frame) = self.poll()?;
        Ok(super::Decoded {
            position: s.position,
            duration: s.duration,
            ready: s.ready != 0,
            playing: s.playing != 0,
            ended: s.ended != 0,
            seeking: s.seeking != 0,
            frame,
        })
    }
}

/// Poster extraction runs on the existing image owner, never the UI/playback queue.
pub fn thumbnail(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<image::DynamicImage> {
    use std::os::unix::ffi::OsStrExt;
    let path = CString::new(path.as_os_str().as_bytes())?;
    let mut pixels = vec![0u8; 1280 * 1280 * 4];
    let mut error = [0i8; 1024];
    let (mut width, mut height) = (0, 0);
    let result = unsafe {
        ps_video_thumbnail(
            path.as_ptr(),
            check,
            &cancelled as *const _ as *mut c_void,
            pixels.as_mut_ptr(),
            pixels.len(),
            &mut width,
            &mut height,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    anyhow::ensure!(
        result == 0,
        "{}",
        unsafe { CStr::from_ptr(error.as_ptr()) }.to_string_lossy()
    );
    let len = width as usize * height as usize * 4;
    anyhow::ensure!(len <= pixels.len(), "Invalid native thumbnail size");
    pixels.truncate(len);
    Ok(image::DynamicImage::ImageRgba8(
        RgbaImage::from_raw(width, height, pixels).context("Invalid thumbnail geometry")?,
    ))
}

extern "C" fn check(context: *mut c_void) -> i32 {
    // This is a synchronous callback; the borrowed predicate lives until open returns.
    let predicate = unsafe { &*(context as *const &dyn Fn() -> bool) };
    i32::from(predicate())
}
