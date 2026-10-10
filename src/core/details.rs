//! Details panel content. One table builder serves a single photo and a
//! side-by-side comparison, so both read the already-indexed metadata and never
//! reopen a file. Raw EXIF codes are formatted here, once, for every backend.
use super::model::{Details, Media, Metadata};
use chrono::{Local, TimeZone};
use std::fmt::Write as _;

pub struct Section {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

pub struct Row {
    pub label: &'static str,
    /// One cell per requested photo; empty when that photo has no value.
    pub values: Vec<String>,
}

/// Parse an EXIF rational (`28/10`) or plain decimal. Zero denominators and
/// non-finite results are rejected rather than displayed.
pub fn ratio(text: &str) -> Option<f64> {
    let text = text.trim();
    let value = match text.split_once('/') {
        Some((numerator, denominator)) => {
            let denominator: f64 = denominator.trim().parse().ok()?;
            if denominator == 0.0 {
                return None;
            }
            numerator.trim().parse::<f64>().ok()? / denominator
        }
        None => text.parse().ok()?,
    };
    value.is_finite().then_some(value)
}

/// Lens name from EXIF LensSpecification (min/max focal length, min/max
/// aperture), e.g. `24–70 mm f/2.8`. Unknown parts (0/0) are omitted.
pub fn lens_specification(values: &[f64]) -> Option<String> {
    let known = |v: &f64| v.is_finite() && *v > 0.0;
    let (&low, &high) = (values.first()?, values.get(1)?);
    if !known(&low) {
        return None;
    }
    let mut name = if known(&high) && high > low {
        format!("{}–{} mm", trim(low), trim(high))
    } else {
        format!("{} mm", trim(low))
    };
    if let Some(aperture) = values.get(2).filter(|v| known(v)) {
        name.push_str(&format!(" f/{}", trim(*aperture)));
        if let Some(widest) = values.get(3).filter(|v| known(v) && **v > *aperture + 0.05) {
            name.push_str(&format!("–{}", trim(*widest)));
        }
    }
    Some(name)
}

/// Tables for one or more photos, omitting rows that nobody has a value for.
pub fn sections(items: &[(&Media, &Metadata)]) -> Vec<Section> {
    let row = |label: &'static str, cell: &dyn Fn(&Media, &Metadata) -> Option<String>| Row {
        label,
        values: items
            .iter()
            .map(|(media, meta)| cell(media, meta).unwrap_or_default())
            .collect(),
    };
    let detail = |label, format: fn(&Details) -> Option<String>| {
        row(label, &|_, meta| format(&meta.details))
    };
    let sections = [
        Section {
            title: "File",
            rows: vec![
                row("Format", &|media, _| extension(media)),
                row("Size", &|media, _| Some(size(media.fingerprint.bytes))),
                row("Created", &|_, meta| created(meta)),
            ],
        },
        Section {
            title: "Camera & settings",
            rows: vec![
                row("Source", &|media, meta| source(media, meta)),
                row("Lens", &|_, meta| text(&meta.lens)),
                detail("Focal length", |d| {
                    d.focal_length_mm
                        .map(|mm| format!("{} mm", trim(mm.into())))
                }),
                row("Aperture", &|_, meta| text(&meta.aperture)),
                row("Shutter", &|_, meta| shutter(&meta.exposure)),
                row("ISO", &|_, meta| {
                    text(&meta.iso).map(|iso| format!("ISO {iso}"))
                }),
                detail("Flash", |d| d.flash.map(flash)),
            ],
        },
        Section {
            title: "Image",
            rows: vec![
                row("Dimensions", &|_, meta| {
                    (meta.width > 0 && meta.height > 0)
                        .then(|| format!("{} × {}", meta.width, meta.height))
                }),
                row("Megapixels", &|_, meta| {
                    (meta.width > 0 && meta.height > 0).then(|| {
                        format!(
                            "{:.1} MP",
                            f64::from(meta.width) * f64::from(meta.height) / 1_000_000.0
                        )
                    })
                }),
                row("Orientation", &|_, meta| {
                    (1..=8)
                        .contains(&meta.orientation)
                        .then(|| orientation(meta.orientation))
                }),
                detail("Bit depth", |d| {
                    d.bit_depth.map(|bits| format!("{bits} bit"))
                }),
            ],
        },
        Section {
            title: "Technical",
            rows: vec![
                detail("Color space", |d| d.color_space.map(color_space)),
                detail("White balance", |d| {
                    d.white_balance
                        .map(|v| label(v, &[(0, "Auto"), (1, "Manual")]))
                }),
                detail("Metering", |d| {
                    d.metering.map(|v| {
                        label(
                            v,
                            &[
                                (0, "Unknown"),
                                (1, "Average"),
                                (2, "Center-weighted"),
                                (3, "Spot"),
                                (4, "Multi-spot"),
                                (5, "Multi-segment"),
                                (6, "Partial"),
                            ],
                        )
                    })
                }),
                detail("Exposure mode", |d| {
                    d.exposure_mode
                        .map(|v| label(v, &[(0, "Auto"), (1, "Manual"), (2, "Auto bracket")]))
                }),
                detail("Exposure comp.", |d| d.exposure_bias_ev.map(bias)),
                detail("Scene", |d| {
                    d.scene.map(|v| {
                        label(
                            v,
                            &[
                                (0, "Standard"),
                                (1, "Landscape"),
                                (2, "Portrait"),
                                (3, "Night"),
                            ],
                        )
                    })
                }),
                detail("Software", |d| d.software.clone()),
            ],
        },
    ];
    sections
        .into_iter()
        .map(|mut section| {
            section.rows.retain(|row| {
                row.values.iter().any(|value| !value.is_empty())
                    && !(row.label == "Orientation"
                        && row
                            .values
                            .iter()
                            .all(|value| value.is_empty() || value == "Normal"))
            });
            section
        })
        .filter(|section| !section.rows.is_empty())
        .collect()
}

fn text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn extension(media: &Media) -> Option<String> {
    Some(media.path.extension()?.to_string_lossy().to_uppercase())
}

fn size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

/// Capture time, falling back to filesystem creation, then to the raw EXIF text.
fn created(meta: &Metadata) -> Option<String> {
    meta.captured_ms
        .or(meta.created_ms)
        .and_then(|ms| Local.timestamp_millis_opt(ms).single())
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .or_else(|| text(&meta.captured))
}

fn source(media: &Media, meta: &Metadata) -> Option<String> {
    let model = meta.camera.trim();
    let make = meta.details.make.as_deref().unwrap_or_default().trim();
    // Models usually repeat the maker ("Canon EOS R"), so do not print it twice.
    let source = if model.to_lowercase().starts_with(&make.to_lowercase()) {
        model.to_owned()
    } else {
        format!("{make} {model}").trim().to_owned()
    };
    if !source.is_empty() {
        return Some(source);
    }
    let name = media.name().to_lowercase();
    ["screenshot", "captura"]
        .iter()
        .any(|marker| name.contains(marker))
        .then(|| "Screen capture".to_owned())
}

/// Photographers read short exposures as `1/N s`; cameras may store them as any
/// rational (Samsung writes 249783/100000000) or a decimal.
fn shutter(exposure: &str) -> Option<String> {
    let text = text(exposure)?;
    let Some(seconds) = ratio(text.trim_end_matches('s').trim()).filter(|s| *s > 0.0) else {
        return Some(text);
    };
    Some(if seconds < 0.3 {
        format!("1/{} s", (1.0 / seconds).round())
    } else {
        format!("{} s", trim(seconds))
    })
}

fn trim(value: f64) -> String {
    let mut text = format!("{value:.1}");
    if text.ends_with(".0") {
        text.truncate(text.len() - 2);
    }
    text
}

fn bias(ev: f32) -> String {
    let mut text = String::new();
    let _ = write!(
        text,
        "{}{} EV",
        if ev > 0.0 { "+" } else { "" },
        trim(ev.into())
    );
    text
}

fn label(code: u16, names: &[(u16, &str)]) -> String {
    names
        .iter()
        .find(|(known, _)| *known == code)
        .map_or_else(|| code.to_string(), |(_, name)| (*name).to_owned())
}

fn color_space(code: u16) -> String {
    label(
        code,
        &[(1, "sRGB"), (2, "Adobe RGB"), (0xFFFF, "Uncalibrated")],
    )
}

fn orientation(code: u8) -> String {
    match code {
        1 => "Normal",
        2 => "Mirrored",
        3 => "Rotated 180°",
        4 => "Mirrored vertically",
        5 => "Mirrored, rotated 90° CCW",
        6 => "Rotated 90° CW",
        7 => "Mirrored, rotated 90° CW",
        8 => "Rotated 90° CCW",
        other => return other.to_string(),
    }
    .to_owned()
}

/// EXIF Flash is a bit field: bit 0 fired, bits 3–4 mode, bit 5 no function,
/// bit 6 red-eye reduction.
fn flash(code: u16) -> String {
    if code & 0b10_0000 != 0 {
        return "No flash function".to_owned();
    }
    let fired = code & 1 != 0;
    let mut text = match ((code >> 3) & 0b11, fired) {
        (1, true) => "Fired (Forced)",
        (3, true) => "Fired (Auto)",
        (_, true) => "Fired",
        (3, false) => "Off (Auto)",
        _ => "Off",
    }
    .to_owned();
    if code & 0b100_0000 != 0 {
        text.push_str(", Red-eye");
    }
    text
}
