use super::model::Metadata;
use anyhow::{Context, Result};
use quick_xml::{
    Writer, XmlVersion,
    events::{BytesStart, Event},
    name::ResolveResult,
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

mod fallback;
mod movie;
#[cfg(feature = "metadata-native")]
mod native;

const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/";
const TIFF: &[u8] = b"http://ns.adobe.com/tiff/1.0/";
const RDF: &[u8] = b"http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// Candidate capture dates by precedence: EXIF original, XMP create, EXIF modify,
/// Photoshop created. The first one present wins.
#[derive(Default)]
struct Dates {
    day: [Option<String>; 4],
    time: [Option<i64>; 4],
}
impl Dates {
    fn assign(&mut self, slot: usize, raw: Option<&str>) {
        self.day[slot] = raw.and_then(super::dates::parse);
        self.time[slot] = raw.and_then(super::dates::timestamp);
    }
    /// A sidecar value replaces an embedded one only when it parses.
    fn overlay(&mut self, slot: usize, raw: Option<&str>) {
        if let Some(day) = raw.and_then(super::dates::parse) {
            self.day[slot] = Some(day);
        }
        if let Some(time) = raw.and_then(super::dates::timestamp) {
            self.time[slot] = Some(time);
        }
    }
}

/// EXIF SubSecTime digits are a decimal fraction of a second: "4" is 400 ms.
fn subsec_ms(raw: &str) -> Option<u16> {
    let digits = raw.trim().trim_end_matches('\0').trim();
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ms = 0;
    for (place, digit) in digits
        .bytes()
        .chain(std::iter::repeat(b'0'))
        .take(3)
        .enumerate()
    {
        ms += u16::from(digit - b'0') * [100, 10, 1][place];
    }
    Some(ms)
}

pub fn sidecar(path: &Path) -> PathBuf {
    let upper = path.with_extension("XMP");
    if upper.is_file() {
        upper
    } else {
        path.with_extension("xmp")
    }
}

pub fn read(path: &Path) -> Metadata {
    read_cancellable(path, &|| false).expect("An uncancelled metadata read returns a snapshot")
}

pub fn read_cancellable(path: &Path, cancelled: &impl Fn() -> bool) -> Result<Metadata> {
    anyhow::ensure!(!cancelled(), "Metadata read cancelled");
    let mut dates = Dates::default();
    let mut result = Metadata {
        created_ms: fs::metadata(path)
            .ok()
            .and_then(|m| m.created().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok()),
        orientation: 1,
        ..Default::default()
    };
    #[cfg(feature = "metadata-native")]
    let native_loaded = if let Some(snapshot) = native::read(path) {
        snapshot.apply(&mut result, &mut dates);
        true
    } else {
        false
    };
    #[cfg(not(feature = "metadata-native"))]
    let native_loaded = false;
    if !native_loaded {
        fallback::read(path, &mut result, &mut dates);
    }
    #[cfg(all(feature = "heif", target_os = "macos"))]
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
    {
        result.orientation = super::apple_image::orientation(path);
    }
    #[cfg(all(feature = "heif", not(target_os = "macos")))]
    if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("heic") || e.eq_ignore_ascii_case("heif"))
    {
        result.container_orientation = super::heif::orientation(path);
        if let Some(orientation) = result.container_orientation {
            result.orientation = orientation;
        }
    }
    if let Ok(xml) = fs::read_to_string(sidecar(path)) {
        if let Ok((rating, orientation)) = read_xmp(&xml) {
            if let Some(v) = rating {
                result.rating = v;
            }
            if let Some(v) = orientation {
                result.orientation = v;
            }
        }
        if let Ok([created, photoshop]) = read_xmp_date_values(&xml) {
            dates.overlay(1, created.as_deref());
            dates.overlay(3, photoshop.as_deref());
        }
    }
    anyhow::ensure!(!cancelled(), "Metadata read cancelled");
    result.grouping_date = dates.day.into_iter().flatten().next();
    result.captured_ms = dates.time.into_iter().flatten().next();
    // One bounded header read supplies video geometry and fallback capture time,
    // without starting a frame decoder. Explicit EXIF/XMP dates retain precedence.
    if let Some(movie) = movie::read_path(path, cancelled) {
        if let Some((width, height)) = movie.dimensions {
            result.width = width;
            result.height = height;
        }
        if result.captured_ms.is_none()
            && result.grouping_date.is_none()
            && let Some(created) = movie.created
        {
            result.captured = created.to_rfc3339();
            result.captured_ms = Some(created.timestamp_millis());
            result.grouping_date = Some(created.format("%Y-%m-%d").to_string());
        }
    }
    anyhow::ensure!(!cancelled(), "Metadata read cancelled");
    Ok(result)
}
/// Read date fields with namespace resolution, for both attribute and element XMP.
pub fn read_xmp_dates(xml: &str) -> Result<[Option<String>; 2]> {
    Ok(read_xmp_date_values(xml)?.map(|v| v.as_deref().and_then(super::dates::parse)))
}
fn read_xmp_date_values(xml: &str) -> Result<[Option<String>; 2]> {
    fn date_field(ns: ResolveResult<'_>, local: &[u8]) -> Option<usize> {
        match ns {
            ResolveResult::Bound(ns) if ns.as_ref() == XMP && local == b"CreateDate" => Some(0),
            ResolveResult::Bound(ns)
                if ns.as_ref() == b"http://ns.adobe.com/photoshop/1.0/"
                    && local == b"DateCreated" =>
            {
                Some(1)
            }
            _ => None,
        }
    }
    let mut reader = quick_xml::NsReader::from_str(xml);
    let mut result = [None, None];
    let mut field = None;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let (ns, local) = reader.resolver().resolve_element(e.name());
                field = date_field(ns, local.as_ref());
                for attribute in e.attributes() {
                    let attribute = attribute?;
                    let (ns, local) = reader.resolver().resolve_attribute(attribute.key);
                    if let Some(index) = date_field(ns, local.as_ref()) {
                        result[index] = Some(
                            attribute
                                .decoded_and_normalized_value(
                                    XmlVersion::Implicit1_0,
                                    reader.decoder(),
                                )?
                                .into_owned(),
                        );
                    }
                }
            }
            Event::Text(e) => {
                if let Some(index) = field {
                    result[index] = Some(e.decode()?.into_owned());
                }
            }
            Event::End(_) => field = None,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}
fn target(ns: ResolveResult<'_>, local: &[u8]) -> Option<bool> {
    match ns {
        ResolveResult::Bound(ns) if ns.as_ref() == XMP && local == b"Rating" => Some(true),
        ResolveResult::Bound(ns) if ns.as_ref() == TIFF && local == b"Orientation" => Some(false),
        _ => None,
    }
}
pub fn read_xmp(xml: &str) -> Result<(Option<u8>, Option<u8>)> {
    let mut reader = quick_xml::NsReader::from_str(xml);
    let (mut rating, mut orientation) = (None, None);
    let mut field = None;
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let (ns, local) = reader.resolver().resolve_element(e.name());
                field = target(ns, local.as_ref());
                for attr in e.attributes() {
                    let attr = attr?;
                    let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                    if let Some(is_rating) = target(ns, local.as_ref()) {
                        let value = attr
                            .decoded_and_normalized_value(
                                XmlVersion::Implicit1_0,
                                reader.decoder(),
                            )?
                            .parse::<u8>()
                            .ok();
                        if is_rating {
                            rating = value.filter(|v| *v <= 5);
                        } else {
                            orientation = value.filter(|v| (1..=8).contains(v));
                        }
                    }
                }
            }
            Event::Text(e) => {
                if let Some(is_rating) = field {
                    let value = e.decode()?.parse::<u8>().ok();
                    if is_rating {
                        rating = value.filter(|v| *v <= 5);
                    } else {
                        orientation = value.filter(|v| (1..=8).contains(v));
                    }
                }
            }
            Event::End(_) => field = None,
            Event::Eof => break,
            _ => {}
        }
    }
    Ok((rating, orientation))
}
/// Preserve unrelated XML, including namespaces, edit history and application data.
pub fn update_xmp(xml: &str, rating: Option<u8>, orientation: Option<u8>) -> Result<String> {
    anyhow::ensure!(rating.is_none_or(|r| r <= 5), "Rating must be 0–5");
    anyhow::ensure!(
        orientation.is_none_or(|o| (1..=8).contains(&o)),
        "Invalid orientation"
    );
    let mut reader = quick_xml::NsReader::from_str(xml);
    let mut writer = Writer::new(Vec::new());
    let mut skip = 0usize;
    let mut inserted = false;
    loop {
        let event = reader.read_event()?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let start = matches!(event, Event::Start(_));
                if skip > 0 {
                    if start {
                        skip += 1;
                    }
                    continue;
                }
                let (ns, local) = reader.resolver().resolve_element(e.name());
                if target(ns, local.as_ref()).is_some_and(|r| {
                    if r {
                        rating.is_some()
                    } else {
                        orientation.is_some()
                    }
                }) {
                    if start {
                        skip = 1;
                    }
                    continue;
                }
                let name = String::from_utf8(e.name().as_ref().to_vec())?;
                let mut replacement = BytesStart::new(name);
                for attr in e.attributes() {
                    let attr = attr?;
                    let (ns, local) = reader.resolver().resolve_attribute(attr.key);
                    if !target(ns, local.as_ref()).is_some_and(|r| {
                        if r {
                            rating.is_some()
                        } else {
                            orientation.is_some()
                        }
                    }) {
                        replacement.push_attribute(attr);
                    }
                }
                writer.write_event(if start {
                    Event::Start(replacement)
                } else {
                    Event::Empty(replacement)
                })?;
            }
            Event::End(e) => {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let (ns, local) = reader.resolver().resolve_element(e.name());
                if !inserted
                    && matches!(ns,ResolveResult::Bound(ns) if ns.as_ref()==RDF)
                    && local.as_ref() == b"RDF"
                {
                    let mut desc = BytesStart::new("rdf:Description");
                    desc.push_attribute(("xmlns:rdf", std::str::from_utf8(RDF)?));
                    desc.push_attribute(("xmlns:xmp", std::str::from_utf8(XMP)?));
                    desc.push_attribute(("xmlns:tiff", std::str::from_utf8(TIFF)?));
                    desc.push_attribute(("rdf:about", ""));
                    let r = rating.map(|v| v.to_string());
                    let o = orientation.map(|v| v.to_string());
                    if let Some(r) = &r {
                        desc.push_attribute(("xmp:Rating", r.as_str()));
                    }
                    if let Some(o) = &o {
                        desc.push_attribute(("tiff:Orientation", o.as_str()));
                    }
                    writer.write_event(Event::Empty(desc))?;
                    inserted = true;
                }
                writer.write_event(event)?;
            }
            Event::Eof => break,
            _ => {
                if skip == 0 {
                    writer.write_event(event)?;
                }
            }
        }
    }
    anyhow::ensure!(
        inserted,
        "Existing XMP has no rdf:RDF container; no file was changed"
    );
    Ok(String::from_utf8(writer.into_inner())?)
}

pub fn write_sidecar(path: &Path, rating: Option<u8>, orientation: Option<u8>) -> Result<()> {
    let dest = sidecar(path);
    anyhow::ensure!(
        !fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()),
        "Refusing a symlink sidecar"
    );
    let original=match fs::read_to_string(&dest) {Ok(s)=>s,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"></rdf:RDF></x:xmpmeta>".into(),Err(e)=>return Err(e.into())};
    let updated = update_xmp(&original, rating, orientation)?;
    atomic_write(&dest, updated.as_bytes())
}

pub fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<()> {
    let parent = dest.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    // Clock values can repeat across threads. Own an exclusively created random
    // temporary file so failure cleanup can never remove another writer's file.
    let mut temp = tempfile::Builder::new()
        .prefix(".photosort-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    super::replace::persist(temp, dest)?;
    Ok(())
}
pub fn rotate_orientation(mut orientation: u8, turns: u8) -> u8 {
    const CLOCKWISE: [u8; 9] = [1, 6, 5, 8, 7, 4, 3, 2, 1];
    for _ in 0..turns % 4 {
        orientation = CLOCKWISE[usize::from(orientation.clamp(1, 8))];
    }
    orientation
}

/// Destination policy is shared by every workflow and captured in each reviewed plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WritePolicy {
    #[default]
    EmbedWhenSupported,
    SidecarOnly,
    /// Embed where supported; a rotation-only edit of a file that cannot store
    /// orientation rotates its pixels losslessly instead of using a sidecar.
    EmbedOrRotatePixels,
}
impl WritePolicy {
    pub fn embeds(self) -> bool {
        self != Self::SidecarOnly
    }
}
/// Old serialized plans remain sidecar-only; preferences never rewrite a retry plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WriteTarget {
    #[default]
    Sidecar,
    Embedded,
    /// Lossless pixel rotation for files that cannot store orientation; only
    /// chosen by the `EmbedOrRotatePixels` policy, never by `write_target`.
    Pixels,
}
pub fn write_target(path: &Path, metadata: &Metadata, policy: WritePolicy) -> WriteTarget {
    if policy.embeds() && metadata.embedded_metadata_writable && !sidecar(path).exists() {
        WriteTarget::Embedded
    } else {
        WriteTarget::Sidecar
    }
}
/// Stage a complete independent copy, then verify the source again before replacing
/// it. Never use the move service's hard-link optimization for metadata editing.
pub fn write_embedded(
    path: &Path,
    expected: super::model::Fingerprint,
    rating: Option<u8>,
    orientation: Option<u8>,
) -> Result<()> {
    anyhow::ensure!(
        rating.is_none_or(|r| r <= 5) && orientation.is_none_or(|o| (1..=8).contains(&o)),
        "Invalid metadata values"
    );
    #[cfg(not(feature = "metadata-native"))]
    {
        let _ = (path, expected);
        anyhow::bail!("Embedded metadata needs a full native build");
    }
    #[cfg(feature = "metadata-native")]
    replace_verified(path, expected, |staged| {
        native::write_copy(staged, rating, orientation)
    })
}

/// Replace `path` with an edited private copy made in the same folder, so the
/// result keeps permissions and extended attributes and is swapped in atomically.
/// The source must still match `expected` before, during and after the edit.
pub(crate) fn replace_verified(
    path: &Path,
    expected: super::model::Fingerprint,
    edit: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let verify = || -> Result<()> {
        anyhow::ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink(),
            "Refusing a symlink image"
        );
        anyhow::ensure!(
            super::model::Fingerprint::read(path)? == expected,
            "File changed since review"
        );
        anyhow::ensure!(
            !sidecar(path).exists(),
            "An XMP sidecar appeared while editing the file"
        );
        Ok(())
    };
    verify()?;
    let parent = path.parent().context("Missing image parent")?;
    let staged = tempfile::Builder::new()
        .prefix(".photosort-metadata-")
        .suffix(".tmp")
        .tempfile_in(parent)?;
    fs::copy(path, staged.path())?;
    verify()?;
    edit(staged.path())?;
    // The editor may replace its input while rewriting the container. Reopen the
    // path so we sync the verified result rather than an obsolete file handle.
    // Windows flushes only through a handle with write access.
    fs::OpenOptions::new()
        .write(true)
        .open(staged.path())?
        .sync_all()?;
    verify()?;
    super::replace::persist(staged, path)?;
    Ok(())
}

#[cfg(test)]
mod subsec_tests {
    use super::*;
    #[test]
    fn sub_second_digits_are_a_decimal_fraction() {
        assert_eq!(subsec_ms("4"), Some(400));
        assert_eq!(subsec_ms("45"), Some(450));
        assert_eq!(subsec_ms(" 123 "), Some(123));
        assert_eq!(subsec_ms("123456"), Some(123));
        assert_eq!(subsec_ms("007"), Some(7));
        assert_eq!(subsec_ms(""), None);
        assert_eq!(subsec_ms("1a"), None);
    }
    /// Prints what the app reads from each photo in PHOTOSORT_SAMPLE_DIR, to
    /// check the facts the same-shot format pairing depends on.
    #[test]
    #[ignore = "read-only diagnostic requiring PHOTOSORT_SAMPLE_DIR"]
    fn sample_capture_facts() {
        let root = PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("sample dir"));
        let mut paths: Vec<_> = walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(Result::ok)
            .map(|e| e.into_path())
            .filter(|p| {
                p.extension().is_some_and(|e| {
                    ["jpg", "jpeg", "heic"].contains(&e.to_ascii_lowercase().to_str().unwrap())
                })
            })
            .collect();
        paths.sort();
        for path in paths {
            let m = read(&path);
            println!(
                "{} | {} | {:?} | {:?} | {}x{} | o{}",
                path.file_name().unwrap().to_string_lossy(),
                m.camera,
                m.captured_ms,
                m.captured_subsec_ms,
                m.width,
                m.height,
                m.orientation
            );
        }
    }
}
