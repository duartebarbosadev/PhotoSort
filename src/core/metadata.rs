use super::model::Metadata;
use anyhow::{Context, Result};
use image::ImageDecoder;
use quick_xml::{
    Writer,
    events::{BytesStart, Event},
    name::ResolveResult,
};
use std::{
    fs::{self, File},
    io::{BufReader, Write},
    path::{Path, PathBuf},
};

mod movie;
#[cfg(feature = "metadata-native")]
mod native;

const XMP: &[u8] = b"http://ns.adobe.com/xap/1.0/";
const TIFF: &[u8] = b"http://ns.adobe.com/tiff/1.0/";
const RDF: &[u8] = b"http://www.w3.org/1999/02/22-rdf-syntax-ns#";

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
    let mut date_tags: [Option<String>; 4] = Default::default();
    let mut time_tags: [Option<i64>; 4] = Default::default();
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
        snapshot.apply(&mut result, &mut date_tags, &mut time_tags);
        true
    } else {
        false
    };
    #[cfg(not(feature = "metadata-native"))]
    let native_loaded = false;
    if !native_loaded {
        if let Ok(file) = File::open(path)
            && let Ok(exif) = exif::Reader::new().read_from_container(&mut BufReader::new(file))
        {
            let text = |tag| {
                exif.get_field(tag, exif::In::PRIMARY)
                    .map(|f| {
                        f.display_value()
                            .with_unit(&exif)
                            .to_string()
                            .trim_matches('"')
                            .to_owned()
                    })
                    .unwrap_or_default()
            };
            let uint = |tag| {
                exif.get_field(tag, exif::In::PRIMARY)
                    .and_then(|f| f.value.get_uint(0))
                    .unwrap_or(0)
            };
            result.metadata_fields = exif
                .fields()
                .filter(|f| !f.value.display_as(f.tag).to_string().is_empty())
                .count();
            result.camera = text(exif::Tag::Model);
            result.lens = text(exif::Tag::LensModel);
            result.captured = text(exif::Tag::DateTimeOriginal);
            time_tags[0] = super::dates::timestamp(&result.captured);
            time_tags[2] = super::dates::timestamp(&text(exif::Tag::DateTime));
            date_tags[0] = super::dates::parse(&result.captured);
            date_tags[2] = super::dates::parse(&text(exif::Tag::DateTime));
            result.exposure = text(exif::Tag::ExposureTime);
            result.aperture = text(exif::Tag::FNumber);
            result.iso = text(exif::Tag::PhotographicSensitivity);
            result.orientation = uint(exif::Tag::Orientation).clamp(1, 8) as u8;
            result.width = uint(exif::Tag::PixelXDimension);
            result.height = uint(exif::Tag::PixelYDimension);
            let coordinate = |tag, ref_tag, negative: &str| -> Option<f64> {
                let f = exif.get_field(tag, exif::In::PRIMARY)?;
                let exif::Value::Rational(v) = &f.value else {
                    return None;
                };
                if v.len() < 3 {
                    return None;
                }
                let sign = if text(ref_tag).contains(negative) {
                    -1.0
                } else {
                    1.0
                };
                Some(sign * (v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0))
            };
            result.gps = coordinate(exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef, "S").zip(
                coordinate(exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef, "W"),
            );
        }
        if let Ok(reader) = image::ImageReader::open(path).and_then(|r| r.with_guessed_format())
            && let Ok(mut decoder) = reader.into_decoder()
            && let Ok(Some(bytes)) = decoder.xmp_metadata()
            && let Ok(xml) = std::str::from_utf8(&bytes)
        {
            if let Ok((Some(rating), _)) = read_xmp(xml) {
                result.rating = rating;
            }
            if let Ok([created, photoshop]) = read_xmp_date_values(xml) {
                date_tags[1] = created.as_deref().and_then(super::dates::parse);
                date_tags[3] = photoshop.as_deref().and_then(super::dates::parse);
                time_tags[1] = created.as_deref().and_then(super::dates::timestamp);
                time_tags[3] = photoshop.as_deref().and_then(super::dates::timestamp);
            }
        }
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
            date_tags[1] = created
                .as_deref()
                .and_then(super::dates::parse)
                .or(date_tags[1].take());
            date_tags[3] = photoshop
                .as_deref()
                .and_then(super::dates::parse)
                .or(date_tags[3].take());
            time_tags[1] = created
                .as_deref()
                .and_then(super::dates::timestamp)
                .or(time_tags[1]);
            time_tags[3] = photoshop
                .as_deref()
                .and_then(super::dates::timestamp)
                .or(time_tags[3]);
        }
    }
    anyhow::ensure!(!cancelled(), "Metadata read cancelled");
    result.grouping_date = date_tags.into_iter().flatten().next();
    result.captured_ms = time_tags.into_iter().flatten().next();
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
                let (ns, local) = reader.resolve_element(e.name());
                field = date_field(ns, local.as_ref());
                for attribute in e.attributes() {
                    let attribute = attribute?;
                    let (ns, local) = reader.resolve_attribute(attribute.key);
                    if let Some(index) = date_field(ns, local.as_ref()) {
                        result[index] = Some(
                            attribute
                                .decode_and_unescape_value(reader.decoder())?
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
                let (ns, local) = reader.resolve_element(e.name());
                field = target(ns, local.as_ref());
                for attr in e.attributes() {
                    let attr = attr?;
                    let (ns, local) = reader.resolve_attribute(attr.key);
                    if let Some(is_rating) = target(ns, local.as_ref()) {
                        let value = attr
                            .decode_and_unescape_value(reader.decoder())?
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
                let (ns, local) = reader.resolve_element(e.name());
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
                    let (ns, local) = reader.resolve_attribute(attr.key);
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
                let (ns, local) = reader.resolve_element(e.name());
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
    temp.persist(dest).map_err(|error| error.error)?;
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
}
/// Old serialized plans remain sidecar-only; preferences never rewrite a retry plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum WriteTarget {
    #[default]
    Sidecar,
    Embedded,
}
pub fn write_target(path: &Path, metadata: &Metadata, policy: WritePolicy) -> WriteTarget {
    if policy == WritePolicy::EmbedWhenSupported
        && metadata.embedded_metadata_writable
        && !sidecar(path).exists()
    {
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
    {
        let verify = || -> Result<()> {
            anyhow::ensure!(
                !fs::symlink_metadata(path)?.file_type().is_symlink(),
                "Refusing a symlink image"
            );
            anyhow::ensure!(
                super::model::Fingerprint::read(path)? == expected,
                "File changed since metadata review"
            );
            anyhow::ensure!(
                !sidecar(path).exists(),
                "An XMP sidecar appeared during embedded metadata editing"
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
        native::write_copy(staged.path(), rating, orientation)?;
        // Exiv2 may replace its input while rewriting the container. Reopen the
        // path so we sync the verified result rather than an obsolete file handle.
        File::open(staged.path())?.sync_all()?;
        verify()?;
        staged.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}
