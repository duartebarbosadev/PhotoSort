//! Shared calendar-date extraction. File information comes from the catalog;
//! grouping never reopens files or initializes another metadata pipeline.
use super::model::{Media, Metadata};
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};

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

/// The year a date in a file name is checked against.
pub fn current_year() -> i32 {
    Local::now().year()
}

pub fn label(media: &Media, metadata: &Metadata) -> Option<String> {
    metadata
        .grouping_date
        .as_deref()
        .and_then(parse)
        .or_else(|| parse(&metadata.captured))
        .or_else(|| filename(&media.name(), Local::now().year()))
        .or_else(|| {
            Local
                .timestamp_millis_opt(file_millis(media, Some(metadata))?)
                .single()
                .map(|time| time.format("%Y-%m-%d").to_string())
        })
}

/// The time of a file that carries no date of its own: the earlier of its
/// creation and modified times. A copy keeps the modified time and gets a new
/// creation time; an edit keeps the creation time and gets a new modified time.
pub fn file_millis(media: &Media, metadata: Option<&Metadata>) -> Option<i64> {
    let modified = i64::try_from(media.fingerprint.modified_ns / 1_000_000).ok();
    [metadata.and_then(|m| m.created_ms), modified]
        .into_iter()
        .flatten()
        .filter(|ms| *ms > 0)
        .min()
}

/// What the clock at the place of capture showed. A written offset says where
/// that clock was, so the written time is kept as it is; a naive camera time is
/// that clock already. A time given in UTC ("Z") names no place and is read on
/// this computer's clock.
pub fn wall_clock(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    // Most files without a capture time have no text for one either.
    if value.is_empty() {
        return None;
    }
    if let Ok(time) = DateTime::parse_from_rfc3339(value)
        .or_else(|_| DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z"))
    {
        return Some(if value.ends_with(['Z', 'z']) {
            time.with_timezone(&Local).naive_local()
        } else {
            time.naive_local()
        });
    }
    for format in [
        "%Y:%m:%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
    ] {
        if let Ok(time) = NaiveDateTime::parse_from_str(value, format) {
            return Some(time);
        }
    }
    NaiveDate::parse_from_str(&parse(value)?, "%Y-%m-%d")
        .ok()?
        .and_hms_opt(0, 0, 0)
}

/// Where a capture clock time sits on the one time line every list sorts by:
/// this computer's. Photos and videos taken at one place keep the order they
/// were taken in, whatever zone they are reviewed in.
pub fn local_millis(time: NaiveDateTime) -> Option<i64> {
    resolve(&Local, time)
}

/// The clock time a place on the capture time line stands for: what the
/// capture clock showed. Every view that prints a capture time reads it
/// here, so a card and the Details panel show the same time for one photo.
pub fn clock(millis: i64) -> Option<NaiveDateTime> {
    Local
        .timestamp_millis_opt(millis)
        .single()
        .map(|time| time.naive_local())
}

/// A clock time repeated when clocks go back is its first occurrence. One
/// skipped when clocks go forward is the instant the skipped stretch ends, so
/// it still sorts between the times around it.
fn resolve<Tz: TimeZone>(zone: &Tz, time: NaiveDateTime) -> Option<i64> {
    if let Some(found) = zone.from_local_datetime(&time).earliest() {
        return Some(found.timestamp_millis());
    }
    // The longest skip on record is one calendar day.
    let mut next = time.with_second(0)?.with_nanosecond(0)?;
    for _ in 0..25 * 60 {
        next += Duration::minutes(1);
        if let Some(found) = zone.from_local_datetime(&next).earliest() {
            return Some(found.timestamp_millis());
        }
    }
    None
}

/// A written date's place on the capture time line, time of day included.
pub fn timestamp(value: &str) -> Option<i64> {
    local_millis(wall_clock(value)?)
}
pub fn capture_millis(media: &Media, metadata: Option<&Metadata>) -> Option<i64> {
    capture_millis_or(media, metadata, || {
        name_millis(&media.name(), Local::now().year())
    })
}

/// The day a file name carries ("IMG_20240501_101500.jpg"), on the capture
/// time line.
pub fn name_millis(name: &str, current_year: i32) -> Option<i64> {
    filename(name, current_year).as_deref().and_then(timestamp)
}

/// [`capture_millis`] for a caller that keeps each file name's day
/// ([`name_millis`]) at hand: `named` is asked only when the metadata
/// carries no capture time.
pub fn capture_millis_or(
    media: &Media,
    metadata: Option<&Metadata>,
    named: impl FnOnce() -> Option<i64>,
) -> Option<i64> {
    metadata
        .and_then(|m| m.captured_ms.or_else(|| timestamp(&m.captured)))
        .or_else(|| {
            metadata
                .and_then(|m| m.grouping_date.as_deref())
                .and_then(timestamp)
        })
        .or_else(named)
        .or_else(|| file_millis(media, metadata))
}

/// Sort key shared by every list that shows photos in the order they were taken:
/// capture time (with the same fallbacks as `capture_millis`), undated photos
/// last, ties by file name so the order is stable.
pub fn capture_order(media: &Media, metadata: Option<&Metadata>) -> (bool, i64, String) {
    let millis = capture_millis(media, metadata);
    (millis.is_none(), millis.unwrap_or_default(), media.name())
}

/// Calendar browsing shows the day at the place of capture: the written camera
/// date, else the day `captured` (from `capture_millis`) falls on here.
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, MappedLocalTime};

    /// Clocks jump from 01:00 to 02:00 on 2024-03-31, as in Lisbon.
    #[derive(Clone)]
    struct Spring;
    fn jump() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2024, 3, 31)
            .unwrap()
            .and_hms_opt(1, 0, 0)
            .unwrap()
    }
    impl TimeZone for Spring {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Self {
            Spring
        }
        fn offset_from_local_date(&self, _: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            MappedLocalTime::None
        }
        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            if *local < jump() {
                MappedLocalTime::Single(FixedOffset::east_opt(0).unwrap())
            } else if *local < jump() + Duration::hours(1) {
                MappedLocalTime::None
            } else {
                MappedLocalTime::Single(FixedOffset::east_opt(3600).unwrap())
            }
        }
        fn offset_from_utc_date(&self, utc: &NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(0, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            FixedOffset::east_opt(if *utc < jump() { 0 } else { 3600 }).unwrap()
        }
    }

    #[test]
    fn a_time_in_the_skipped_hour_keeps_its_place_between_its_neighbors() {
        let at = |text: &str| resolve(&Spring, wall_clock(text).unwrap()).unwrap();
        let end = jump().and_utc().timestamp_millis();
        assert_eq!(at("2024:03:31 00:59:59"), end - 1000);
        // 01:30 never happened on this clock: it sorts where the skipped hour ends.
        assert_eq!(at("2024:03:31 01:30:00"), end);
        assert_eq!(at("2024:03:31 01:59:59.5"), end);
        assert_eq!(at("2024:03:31 02:00:00"), end);
        assert_eq!(at("2024:03:31 02:00:01"), end + 1000);
    }
}
