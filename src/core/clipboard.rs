//! Photos on the system clipboard: file references that paste as copies in a
//! file manager and, for one photo, image data that pastes into other apps.
use super::{
    model::{Media, MediaKind},
    pipeline::Pipeline,
};
use anyhow::{Context, Result};
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
};

/// Image data stays small enough to encode quickly and paste anywhere.
pub const IMAGE_EDGE: u32 = 4096;

/// What one copy puts on the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub struct Payload {
    pub paths: Vec<PathBuf>,
    /// The one still photo whose pixels go beside its file reference.
    pub image: Option<Media>,
}
impl Payload {
    pub fn new(media: Vec<Media>) -> Option<Self> {
        let image = match media.as_slice() {
            [one] if one.kind != MediaKind::Video => Some(one.clone()),
            _ => None,
        };
        (!media.is_empty()).then(|| Self {
            paths: media.into_iter().map(|m| m.path).collect(),
            image,
        })
    }
}

/// Copies run one at a time: a newer copy drops an older one's image.
#[derive(Default)]
pub struct Copier {
    generation: Arc<AtomicU64>,
}
impl Copier {
    /// Puts the file references on the clipboard now and adds the image data
    /// from a background thread once it is rendered, unless something else
    /// was copied in the meantime.
    pub fn copy(&self, payload: Payload, cache: PathBuf) -> Result<()> {
        if let Some(missing) = payload.paths.iter().find(|path| !path.is_file()) {
            anyhow::bail!(
                "{} is no longer on disk",
                missing
                    .file_name()
                    .unwrap_or(missing.as_os_str())
                    .to_string_lossy()
            );
        }
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        let token = platform::write_files(&payload.paths)?;
        let Some(media) = payload.image else {
            return Ok(());
        };
        if !platform::IMAGES {
            return Ok(());
        }
        let current = self.generation.clone();
        thread::Builder::new()
            .name("photosort-clipboard".into())
            .spawn(move || {
                let image = Pipeline::new(64 * 1024 * 1024, cache).render_whole(&media, IMAGE_EDGE);
                if current.load(Ordering::Acquire) != generation {
                    return;
                }
                if let Err(error) = image.and_then(|image| platform::add_image(token, &image)) {
                    tracing::warn!("Clipboard image for {:?} failed: {error:#}", media.path);
                }
            })
            .context("Cannot start copying the photo")?;
        Ok(())
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn png(image: &RgbaImage) -> Result<Vec<u8>> {
    use image::{
        ImageEncoder,
        codecs::png::{CompressionType, FilterType, PngEncoder},
    };
    let mut bytes = Vec::new();
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )?;
    Ok(bytes)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn path_text(path: &std::path::Path) -> Result<&str> {
    path.to_str()
        .with_context(|| format!("Cannot copy {path:?}: the path is not valid text"))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{CString, c_char, c_long};
    unsafe extern "C" {
        fn photosort_clipboard_write_files(paths: *const *const c_char, count: usize) -> c_long;
        fn photosort_clipboard_add_png(change: c_long, bytes: *const u8, len: usize) -> i32;
    }
    pub const IMAGES: bool = true;
    pub fn write_files(paths: &[PathBuf]) -> Result<i64> {
        let owned = paths
            .iter()
            .map(|path| Ok(CString::new(path_text(path)?)?))
            .collect::<Result<Vec<_>>>()?;
        let pointers: Vec<_> = owned.iter().map(|path| path.as_ptr()).collect();
        // SAFETY: the pointers borrow `owned`, which outlives this synchronous call.
        let change = unsafe { photosort_clipboard_write_files(pointers.as_ptr(), pointers.len()) };
        anyhow::ensure!(change >= 0, "The clipboard did not accept the photos");
        Ok(change as i64)
    }
    pub fn add_image(token: i64, image: &RgbaImage) -> Result<()> {
        let png = png(image)?;
        // SAFETY: `png` outlives this synchronous call, which copies the bytes.
        let added =
            unsafe { photosort_clipboard_add_png(token as c_long, png.as_ptr(), png.len()) };
        anyhow::ensure!(added >= 0, "The clipboard did not accept the image");
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use clipboard_win::{Clipboard, options::NoClear, raw};
    pub const IMAGES: bool = true;
    fn open() -> Result<Clipboard> {
        Clipboard::new_attempts(10)
            .map_err(|error| anyhow::anyhow!("Cannot open the clipboard: {error}"))
    }
    fn sequence() -> i64 {
        raw::seq_num().map_or(0, |n| i64::from(n.get()))
    }
    pub fn write_files(paths: &[PathBuf]) -> Result<i64> {
        let paths = paths
            .iter()
            .map(|p| path_text(p))
            .collect::<Result<Vec<_>>>()?;
        let _open = open()?;
        raw::empty().map_err(|error| anyhow::anyhow!("Cannot clear the clipboard: {error}"))?;
        raw::set_file_list_with(&paths, NoClear)
            .map_err(|error| anyhow::anyhow!("The clipboard did not accept the photos: {error}"))?;
        Ok(sequence())
    }
    pub fn add_image(token: i64, image: &RgbaImage) -> Result<()> {
        let png = png(image)?;
        // Most Windows apps read CF_DIB; it carries no alpha, which photos lack anyway.
        let mut bmp = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image.clone())
            .into_rgb8()
            .write_to(&mut bmp, image::ImageFormat::Bmp)?;
        let _open = open()?;
        if sequence() != token {
            return Ok(());
        }
        raw::set_bitmap_with(bmp.get_ref(), NoClear)
            .map_err(|error| anyhow::anyhow!("The clipboard did not accept the image: {error}"))?;
        if let Some(format) = raw::register_format("PNG") {
            raw::set_without_clear(format.get(), &png).map_err(|error| {
                anyhow::anyhow!("The clipboard did not accept the PNG: {error}")
            })?;
        }
        Ok(())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    /// arboard replaces the clipboard on every write, so files and pixels
    /// cannot share it; file managers and upload fields both read the files.
    pub const IMAGES: bool = false;
    /// X11 serves the clipboard from the copying process: keep one open.
    static CLIPBOARD: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    pub fn write_files(paths: &[PathBuf]) -> Result<i64> {
        let mut clipboard = CLIPBOARD.get_or_init(Default::default).lock().unwrap();
        if clipboard.is_none() {
            *clipboard = Some(arboard::Clipboard::new().context("Cannot open the clipboard")?);
        }
        clipboard
            .as_mut()
            .unwrap()
            .set()
            .file_list(paths)
            .context("The clipboard did not accept the photos")?;
        Ok(0)
    }
    pub fn add_image(_token: i64, _image: &RgbaImage) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Fingerprint, MediaId};
    fn media(id: u64, name: &str, kind: MediaKind) -> Media {
        Media {
            id: MediaId(id),
            path: PathBuf::from("/photos").join(name),
            kind,
            fingerprint: Fingerprint::default(),
        }
    }
    #[test]
    fn one_photo_carries_its_image_and_several_carry_only_their_files() {
        let one = Payload::new(vec![media(1, "a.heic", MediaKind::Photo)]).unwrap();
        assert_eq!(one.paths, [PathBuf::from("/photos/a.heic")]);
        assert_eq!(one.image.map(|m| m.id), Some(MediaId(1)));
        let video = Payload::new(vec![media(2, "b.mov", MediaKind::Video)]).unwrap();
        assert!(video.image.is_none());
        let several = Payload::new(vec![
            media(3, "c.arw", MediaKind::Raw),
            media(1, "a.heic", MediaKind::Photo),
        ])
        .unwrap();
        assert_eq!(
            several.paths,
            [
                PathBuf::from("/photos/c.arw"),
                PathBuf::from("/photos/a.heic")
            ]
        );
        assert!(several.image.is_none());
        assert!(Payload::new(Vec::new()).is_none());
    }
}
