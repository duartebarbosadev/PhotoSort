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
        Arc, Mutex,
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
pub struct Copier {
    generation: Arc<AtomicU64>,
    images: Latest<Render>,
}
/// One copy's image, rendered after its file references are on the clipboard.
struct Render {
    media: Media,
    token: platform::Token,
    generation: u64,
    cache: PathBuf,
    limits: super::cache::Limits,
}
/// The copy's pixels, decoded within the user's cache limits: with no disk
/// cache allowed, copying writes none.
fn render(media: &Media, cache: PathBuf, limits: super::cache::Limits) -> Result<Arc<RgbaImage>> {
    let mut pipeline = Pipeline::new(64 * 1024 * 1024, cache);
    pipeline.configure_cache(super::cache::Limits {
        previews_mib: 64,
        ..limits
    });
    pipeline.render_whole(media, IMAGE_EDGE)
}
impl Default for Copier {
    fn default() -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        let images = Latest::new(move |job: Render| {
            let stale = || current.load(Ordering::Acquire) != job.generation;
            if stale() {
                return;
            }
            let image = render(&job.media, job.cache, job.limits);
            if stale() {
                return;
            }
            if let Err(error) = image.and_then(|image| platform::add_image(job.token, &image)) {
                tracing::warn!("Clipboard image for {:?} failed: {error:#}", job.media.path);
            }
        });
        Self { generation, images }
    }
}
impl Copier {
    /// Puts the file references on the clipboard now and adds the image data
    /// from a background thread once it is rendered, unless something else
    /// was copied in the meantime.
    pub fn copy(
        &self,
        payload: Payload,
        cache: PathBuf,
        limits: super::cache::Limits,
    ) -> Result<()> {
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
        self.images.submit(Render {
            media,
            token,
            generation,
            cache,
            limits,
        })
    }
}

/// Runs jobs on one thread, newest first: while one runs, a newer job
/// replaces any still waiting. Holding Cmd/Ctrl+C repeats the copy many
/// times a second; each would otherwise decode the whole photo at once.
struct Latest<J> {
    /// The waiting job, and whether the thread is running.
    state: Arc<Mutex<(Option<J>, bool)>>,
    run: Arc<dyn Fn(J) + Send + Sync>,
}
impl<J: Send + 'static> Latest<J> {
    fn new(run: impl Fn(J) + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::new(Mutex::new((None, false))),
            run: Arc::new(run),
        }
    }
    fn submit(&self, job: J) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.0 = Some(job);
        if state.1 {
            return Ok(());
        }
        let (shared, run) = (self.state.clone(), self.run.clone());
        let spawned = thread::Builder::new()
            .name("photosort-clipboard".into())
            .spawn(move || {
                loop {
                    let job = {
                        let mut state = shared.lock().unwrap();
                        let job = state.0.take();
                        state.1 = job.is_some();
                        job
                    };
                    let Some(job) = job else { break };
                    // A photo that fails to decode is not a crash of the app.
                    if let Err(error) = super::crash::catch(|| run(job)) {
                        tracing::warn!("Clipboard image failed: {error}");
                    }
                }
            });
        match spawned {
            Ok(_) => {
                state.1 = true;
                Ok(())
            }
            Err(error) => {
                state.0 = None;
                Err(error).context("Cannot start copying the photo")
            }
        }
    }
}

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
    /// The pasteboard change count after the files were written.
    pub type Token = i64;
    pub fn write_files(paths: &[PathBuf]) -> Result<Token> {
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
    pub fn add_image(token: Token, image: &RgbaImage) -> Result<()> {
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
    /// The clipboard sequence number after the files were written.
    pub type Token = i64;
    fn open() -> Result<Clipboard> {
        Clipboard::new_attempts(10)
            .map_err(|error| anyhow::anyhow!("Cannot open the clipboard: {error}"))
    }
    fn sequence() -> i64 {
        raw::seq_num().map_or(0, |n| i64::from(n.get()))
    }
    pub fn write_files(paths: &[PathBuf]) -> Result<Token> {
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
    pub fn add_image(token: Token, image: &RgbaImage) -> Result<()> {
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

/// File lists as Linux file managers and browsers read them.
#[cfg(any(test, not(any(target_os = "macos", target_os = "windows"))))]
mod uris {
    use std::fmt::Write;
    /// `file://` URL for an absolute path's raw bytes, which need not be UTF-8.
    fn file_uri(path: &[u8]) -> String {
        let mut uri = String::from("file://");
        for &byte in path {
            if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
                uri.push(char::from(byte));
            } else {
                let _ = write!(uri, "%{byte:02X}");
            }
        }
        uri
    }
    /// `text/uri-list`: one URL per line, each ending in CRLF (RFC 2483).
    pub fn uri_list(paths: &[&[u8]]) -> String {
        paths.iter().map(|path| file_uri(path) + "\r\n").collect()
    }
    /// `x-special/gnome-copied-files`: "copy", then one URL per line, as
    /// GNOME Files and other GTK file managers write it.
    pub fn gnome_copied_files(paths: &[&[u8]]) -> String {
        std::iter::once("copy".to_owned())
            .chain(paths.iter().map(|path| file_uri(path)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    //! The copying process serves the clipboard, so PhotoSort keeps one offer
    //! alive with every format at once: Wayland data-control where the
    //! compositor has it, otherwise X11 (directly or through XWayland).
    use super::*;
    use std::{os::unix::ffi::OsStrExt, sync::atomic::AtomicBool};

    const URI_LIST: &str = "text/uri-list";
    const GNOME_COPIED_FILES: &str = "x-special/gnome-copied-files";
    const PNG: &str = "image/png";

    /// MIME type and bytes of each format on offer.
    type Offer = Vec<(&'static str, Arc<[u8]>)>;

    pub enum Token {
        /// The flag clears once another copy replaces the offer.
        Wayland { ours: Arc<AtomicBool>, files: Offer },
        /// The number of the X11 offer.
        X11(u64),
    }

    /// Set once the compositor turns out to lack data-control (GNOME), so
    /// later copies go straight to XWayland.
    static NO_DATA_CONTROL: AtomicBool = AtomicBool::new(false);

    fn file_offer(paths: &[PathBuf]) -> Result<Offer> {
        let paths = paths
            .iter()
            .map(std::path::absolute)
            .collect::<std::io::Result<Vec<_>>>()?;
        let bytes: Vec<&[u8]> = paths.iter().map(|p| p.as_os_str().as_bytes()).collect();
        Ok(vec![
            (URI_LIST, uris::uri_list(&bytes).into_bytes().into()),
            (
                GNOME_COPIED_FILES,
                uris::gnome_copied_files(&bytes).into_bytes().into(),
            ),
        ])
    }

    pub fn write_files(paths: &[PathBuf]) -> Result<Token> {
        let files = file_offer(paths)?;
        if std::env::var_os("WAYLAND_DISPLAY").is_some() && !NO_DATA_CONTROL.load(Ordering::Relaxed)
        {
            match wayland::offer(&files) {
                Ok(ours) => return Ok(Token::Wayland { ours, files }),
                Err(error) => {
                    if matches!(
                        error.downcast_ref(),
                        Some(wl_clipboard_rs::copy::Error::MissingProtocol { .. })
                    ) {
                        NO_DATA_CONTROL.store(true, Ordering::Relaxed);
                    }
                    if std::env::var_os("DISPLAY").is_none() {
                        return Err(error.context("Cannot reach the clipboard"));
                    }
                    tracing::debug!("Wayland clipboard unavailable, using X11: {error:#}");
                }
            }
        }
        x11::Server::get()?.offer(files).map(Token::X11)
    }

    pub fn add_image(token: Token, image: &RgbaImage) -> Result<()> {
        match token {
            Token::Wayland { ours, mut files } => {
                if !ours.load(Ordering::Acquire) {
                    return Ok(());
                }
                files.push((PNG, png(image)?.into()));
                if !ours.load(Ordering::Acquire) {
                    return Ok(());
                }
                wayland::offer(&files).map(drop)
            }
            Token::X11(number) => {
                let server = x11::Server::get()?;
                if !server.is_current(number) {
                    return Ok(());
                }
                server.add(number, PNG, png(image)?.into())
            }
        }
    }

    mod wayland {
        use super::*;
        use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

        /// Offers every format until another copy replaces them; the flag
        /// says whether this offer is still on the clipboard.
        pub fn offer(formats: &Offer) -> Result<Arc<AtomicBool>> {
            let mut options = Options::new();
            options
                .foreground(true)
                .omit_additional_text_mime_types(true);
            let sources = formats
                .iter()
                .map(|(mime, bytes)| MimeSource {
                    source: Source::Bytes(bytes.to_vec().into()),
                    mime_type: MimeType::Specific((*mime).to_owned()),
                })
                .collect();
            let prepared = options.prepare_copy_multi(sources)?;
            let ours = Arc::new(AtomicBool::new(true));
            let flag = ours.clone();
            thread::Builder::new()
                .name("photosort-clipboard-wayland".into())
                .spawn(move || {
                    if let Err(error) = prepared.serve() {
                        tracing::warn!("Serving the clipboard failed: {error:#}");
                    }
                    flag.store(false, Ordering::Release);
                })?;
            Ok(ours)
        }
    }

    mod x11 {
        use super::*;
        use std::{collections::HashMap, sync::Mutex};
        use x11rb::{
            connection::{Connection, RequestConnection as _},
            protocol::{
                Event,
                xproto::{
                    Atom, AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, CreateWindowAux,
                    EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
                    SelectionRequestEvent, Window, WindowClass,
                },
            },
            rust_connection::RustConnection,
            wrapper::ConnectionExt as _,
        };

        /// Room for the request header around property data.
        const HEADER: usize = 64;
        /// INCR pastes move at most this much per round trip.
        const CHUNK: usize = 1 << 20;

        /// An invisible window that owns CLIPBOARD and answers pastes.
        pub struct Server {
            conn: RustConnection,
            window: Window,
            clipboard: Atom,
            targets: Atom,
            incr: Atom,
            state: Mutex<State>,
        }
        #[derive(Default)]
        struct State {
            number: u64,
            /// What the clipboard holds while PhotoSort owns it, by target.
            offer: Option<Vec<(Atom, Arc<[u8]>)>>,
            /// Large pastes in progress, by requestor window and property.
            transfers: HashMap<(Window, Atom), Transfer>,
        }
        struct Transfer {
            target: Atom,
            data: Arc<[u8]>,
            sent: usize,
        }

        static SERVER: Mutex<Option<Arc<Server>>> = Mutex::new(None);

        impl Server {
            /// The running server, connecting on first use and again after
            /// the connection drops.
            pub fn get() -> Result<Arc<Self>> {
                let mut server = SERVER.lock().unwrap();
                if let Some(server) = &*server {
                    return Ok(server.clone());
                }
                let started = Arc::new(Self::connect().context("Cannot reach the clipboard")?);
                let serving = started.clone();
                thread::Builder::new()
                    .name("photosort-clipboard-x11".into())
                    .spawn(move || serving.serve())?;
                *server = Some(started.clone());
                Ok(started)
            }

            fn connect() -> Result<Self> {
                let (conn, screen) = x11rb::connect(None)?;
                let root = conn.setup().roots[screen].root;
                let window = conn.generate_id()?;
                conn.create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    window,
                    root,
                    0,
                    0,
                    1,
                    1,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x11rb::COPY_FROM_PARENT,
                    &CreateWindowAux::new(),
                )?;
                let atom = |name: &str| -> Result<Atom> {
                    Ok(conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
                };
                let (clipboard, targets, incr) =
                    (atom("CLIPBOARD")?, atom("TARGETS")?, atom("INCR")?);
                Ok(Self {
                    conn,
                    window,
                    clipboard,
                    targets,
                    incr,
                    state: Mutex::default(),
                })
            }

            fn atom(&self, name: &str) -> Result<Atom> {
                Ok(self.conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
            }

            /// Replaces the clipboard with `formats` and returns the offer's number.
            pub fn offer(&self, formats: Offer) -> Result<u64> {
                let formats = formats
                    .into_iter()
                    .map(|(mime, bytes)| Ok((self.atom(mime)?, bytes)))
                    .collect::<Result<Vec<_>>>()?;
                // Held while taking ownership, so a late SelectionClear for an
                // earlier offer cannot empty this one.
                let mut state = self.state.lock().unwrap();
                state.number += 1;
                state.offer = Some(formats);
                self.conn
                    .set_selection_owner(self.window, self.clipboard, x11rb::CURRENT_TIME)?;
                let owner = self
                    .conn
                    .get_selection_owner(self.clipboard)?
                    .reply()?
                    .owner;
                if owner != self.window {
                    state.offer = None;
                    anyhow::bail!("The clipboard did not accept the photos");
                }
                Ok(state.number)
            }

            pub fn is_current(&self, number: u64) -> bool {
                let state = self.state.lock().unwrap();
                state.number == number && state.offer.is_some()
            }

            /// Adds a format to offer `number` if it is still on the clipboard.
            pub fn add(&self, number: u64, mime: &str, bytes: Arc<[u8]>) -> Result<()> {
                let target = self.atom(mime)?;
                let mut state = self.state.lock().unwrap();
                if state.number == number
                    && let Some(offer) = &mut state.offer
                {
                    offer.push((target, bytes));
                }
                Ok(())
            }

            fn serve(&self) {
                loop {
                    let event = match self.conn.wait_for_event() {
                        Ok(event) => event,
                        Err(error) => {
                            tracing::warn!("The X11 clipboard connection closed: {error}");
                            break;
                        }
                    };
                    let handled = match event {
                        Event::SelectionRequest(request) => self.answer(request),
                        Event::SelectionClear(clear) if clear.selection == self.clipboard => {
                            self.lost()
                        }
                        Event::PropertyNotify(notify) if notify.state == Property::DELETE => {
                            self.next_chunk(notify.window, notify.atom)
                        }
                        _ => Ok(()),
                    };
                    if let Err(error) = handled {
                        tracing::warn!("A clipboard paste failed: {error:#}");
                    }
                }
                *SERVER.lock().unwrap() = None;
            }

            fn lost(&self) -> Result<()> {
                let mut state = self.state.lock().unwrap();
                if self
                    .conn
                    .get_selection_owner(self.clipboard)?
                    .reply()?
                    .owner
                    != self.window
                {
                    state.offer = None;
                }
                Ok(())
            }

            fn answer(&self, request: SelectionRequestEvent) -> Result<()> {
                // Obsolete clients leave the property unset and mean the target.
                let property = if request.property == x11rb::NONE {
                    request.target
                } else {
                    request.property
                };
                let served = request.selection == self.clipboard
                    && self.provide(request.requestor, property, request.target)?;
                self.conn.send_event(
                    false,
                    request.requestor,
                    EventMask::NO_EVENT,
                    SelectionNotifyEvent {
                        response_type: SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: request.time,
                        requestor: request.requestor,
                        selection: request.selection,
                        target: request.target,
                        property: if served { property } else { x11rb::NONE },
                    },
                )?;
                self.conn.flush()?;
                Ok(())
            }

            /// Writes `target` to the requestor's property, or starts an INCR
            /// transfer when it is too large for one request.
            fn provide(&self, window: Window, property: Atom, target: Atom) -> Result<bool> {
                let mut state = self.state.lock().unwrap();
                let Some(offer) = &state.offer else {
                    return Ok(false);
                };
                if target == self.targets {
                    let atoms: Vec<Atom> = std::iter::once(self.targets)
                        .chain(offer.iter().map(|(atom, _)| *atom))
                        .collect();
                    self.conn.change_property32(
                        PropMode::REPLACE,
                        window,
                        property,
                        AtomEnum::ATOM,
                        &atoms,
                    )?;
                    return Ok(true);
                }
                let Some(data) = offer
                    .iter()
                    .find(|(atom, _)| *atom == target)
                    .map(|(_, data)| data.clone())
                else {
                    return Ok(false);
                };
                if data.len() + HEADER <= self.conn.maximum_request_bytes() {
                    self.conn.change_property8(
                        PropMode::REPLACE,
                        window,
                        property,
                        target,
                        &data,
                    )?;
                    return Ok(true);
                }
                // Watch for the requestor deleting the property, then send chunks.
                self.conn.change_window_attributes(
                    window,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
                )?;
                let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
                self.conn.change_property32(
                    PropMode::REPLACE,
                    window,
                    property,
                    self.incr,
                    &[size],
                )?;
                state.transfers.insert(
                    (window, property),
                    Transfer {
                        target,
                        data,
                        sent: 0,
                    },
                );
                Ok(true)
            }

            /// Sends the next INCR chunk; an empty one ends the transfer.
            fn next_chunk(&self, window: Window, property: Atom) -> Result<()> {
                let mut state = self.state.lock().unwrap();
                let Some(transfer) = state.transfers.get_mut(&(window, property)) else {
                    return Ok(());
                };
                let chunk = CHUNK.min(self.conn.maximum_request_bytes() - HEADER);
                let end = (transfer.sent + chunk).min(transfer.data.len());
                self.conn.change_property8(
                    PropMode::REPLACE,
                    window,
                    property,
                    transfer.target,
                    &transfer.data[transfer.sent..end],
                )?;
                let finished = end == transfer.sent;
                transfer.sent = end;
                if finished {
                    state.transfers.remove(&(window, property));
                    if !state.transfers.keys().any(|(other, _)| *other == window) {
                        self.conn.change_window_attributes(
                            window,
                            &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
                        )?;
                    }
                }
                self.conn.flush()?;
                Ok(())
            }
        }
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
    fn repeated_copies_render_one_at_a_time_and_only_the_newest_waits() {
        let (started_tx, started) = std::sync::mpsc::channel();
        let (release, release_rx) = std::sync::mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let ran = Arc::new(Mutex::new(Vec::new()));
        let log = ran.clone();
        let latest = Latest::new(move |job: u32| {
            started_tx.send(job).unwrap();
            release_rx.lock().unwrap().recv().unwrap();
            log.lock().unwrap().push(job);
        });
        latest.submit(1).unwrap();
        assert_eq!(started.recv().unwrap(), 1);
        // A held shortcut: thirty more while the first still renders.
        for job in 2..=31 {
            latest.submit(job).unwrap();
        }
        release.send(()).unwrap();
        assert_eq!(started.recv().unwrap(), 31);
        release.send(()).unwrap();
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        while latest.state.lock().unwrap().1 {
            assert!(std::time::Instant::now() < until);
            thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(*ran.lock().unwrap(), [1, 31]);
        // A panicking render is reported, and later copies still render.
        let (done_tx, done) = std::sync::mpsc::channel();
        let latest = Latest::new(move |job: u32| {
            assert!(job != 1, "corrupt photo");
            done_tx.send(job).unwrap();
        });
        latest.submit(1).unwrap();
        latest.submit(2).unwrap();
        assert_eq!(done.recv().unwrap(), 2);
    }
    #[test]
    fn a_copy_writes_the_disk_cache_only_when_the_user_allows_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.png");
        image::RgbaImage::from_pixel(1200, 800, image::Rgba([90, 120, 150, 255]))
            .save(&path)
            .unwrap();
        let photo = Media {
            id: crate::core::model::MediaId::for_path(&path),
            fingerprint: crate::core::model::Fingerprint::read(&path).unwrap(),
            path,
            kind: MediaKind::Photo,
        };
        let files = |cache: &std::path::Path| {
            walkdir::WalkDir::new(cache)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .count()
        };
        let (allowed, none) = (dir.path().join("allowed"), dir.path().join("none"));
        render(&photo, allowed.clone(), Default::default()).unwrap();
        assert!(files(&allowed) > 0, "the default limits keep a disk copy");
        let off = crate::core::cache::Limits {
            disk_mib: 0,
            ..Default::default()
        };
        assert_eq!(render(&photo, none.clone(), off).unwrap().width(), 1200);
        assert_eq!(files(&none), 0);
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
    #[test]
    fn linux_file_lists_escape_each_path_as_a_file_url() {
        let paths: [&[u8]; 2] = [
            b"/photos/Trip #1/100%.jpg",
            "/photos/S\u{e3}o Paulo/pra\u{e7}a.heic".as_bytes(),
        ];
        assert_eq!(
            uris::uri_list(&paths),
            "file:///photos/Trip%20%231/100%25.jpg\r\n\
             file:///photos/S%C3%A3o%20Paulo/pra%C3%A7a.heic\r\n"
        );
        assert_eq!(
            uris::gnome_copied_files(&paths),
            "copy\nfile:///photos/Trip%20%231/100%25.jpg\n\
             file:///photos/S%C3%A3o%20Paulo/pra%C3%A7a.heic"
        );
    }
}
