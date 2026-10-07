//! Shared calendar-date extraction. File information comes from the catalog;
//! grouping never reopens files or initializes another metadata pipeline.
use super::model::{Media, Metadata};
use chrono::{Datelike, Local, NaiveDate, TimeZone};

pub fn parse(value: &str) -> Option<String> {
    let part = value.trim().split(['T', ' ']).next()?;
    ["%Y:%m:%d", "%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(part, format).ok())
        .map(|date| date.format("%Y-%m-%d").to_string())
}

pub fn filename(name: &str, current_year: i32) -> Option<String> {
    let bytes = name.as_bytes();
    let compact = bytes.windows(8).enumerate().find_map(|(i, part)| {
        (part.iter().all(u8::is_ascii_digit)
            && bytes.get(i + 8).is_none_or(|c| b"_ -T".contains(c)))
        .then_some(part)
    });
    let separated = bytes.windows(10).find(|part| {
        part[..4].iter().all(u8::is_ascii_digit)
            && b"-_.".contains(&part[4])
            && part[5..7].iter().all(u8::is_ascii_digit)
            && b"-_.".contains(&part[7])
            && part[8..].iter().all(u8::is_ascii_digit)
    });
    [compact, separated].into_iter().flatten().find_map(|part| {
        let digits: String = part
            .iter()
            .filter(|c| c.is_ascii_digit())
            .map(|c| *c as char)
            .collect();
        let date = NaiveDate::parse_from_str(&digits, "%Y%m%d").ok()?;
        (1900..=current_year + 10)
            .contains(&date.year())
            .then(|| date.format("%Y-%m-%d").to_string())
    })
}

pub fn label(media: &Media, metadata: &Metadata) -> Option<String> {
    metadata
        .grouping_date
        .as_deref()
        .and_then(parse)
        .or_else(|| parse(&metadata.captured))
        .or_else(|| filename(&media.name(), Local::now().year()))
        .or_else(|| {
            let seconds = i64::try_from(media.fingerprint.modified_ns / 1_000_000_000).ok()?;
            Local
                .timestamp_opt(seconds, 0)
                .single()
                .map(|time| time.format("%Y-%m-%d").to_string())
        })
}

/// Preserve time-of-day and explicit timezone offsets for chronological browsing.
pub fn timestamp(value: &str) -> Option<i64> {
    use chrono::{DateTime, NaiveDateTime};
    if let Ok(time) = DateTime::parse_from_rfc3339(value.trim()) {
        return Some(time.timestamp_millis());
    }
    for format in [
        "%Y:%m:%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(time) = NaiveDateTime::parse_from_str(value.trim(), format) {
            return Local
                .from_local_datetime(&time)
                .earliest()
                .map(|time| time.timestamp_millis());
        }
    }
    let date = NaiveDate::parse_from_str(&parse(value)?, "%Y-%m-%d").ok()?;
    Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|time| time.timestamp_millis())
}
pub fn capture_millis(media: &Media, metadata: Option<&Metadata>) -> Option<i64> {
    metadata
        .and_then(|m| m.captured_ms.or_else(|| timestamp(&m.captured)))
        .or_else(|| {
            metadata
                .and_then(|m| m.grouping_date.as_deref())
                .and_then(timestamp)
        })
        .or_else(|| {
            filename(&media.name(), Local::now().year())
                .as_deref()
                .and_then(timestamp)
        })
        .or_else(|| metadata.and_then(|m| m.created_ms))
        .or_else(|| {
            (media.fingerprint.modified_ns > 0)
                .then(|| i64::try_from(media.fingerprint.modified_ns / 1_000_000).ok())
                .flatten()
        })
}

/// Calendar browsing preserves the written camera date, including an explicit
/// offset. Absolute timestamps still determine chronological ordering.
pub fn calendar(captured: Option<i64>, metadata: Option<&Metadata>) -> Option<String> {
    metadata
        .and_then(|m| parse(&m.captured).or_else(|| m.grouping_date.as_deref().and_then(parse)))
        .or_else(|| {
            Local
                .timestamp_millis_opt(captured?)
                .single()
                .map(|t| t.format("%Y-%m-%d").to_string())
        })
}
