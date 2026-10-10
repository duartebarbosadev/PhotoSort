//! Pure-Rust metadata reader used when the native exiv2 bridge is unavailable or
//! rejects a file. It produces the same fields as `native::Snapshot::apply`.
use super::{Dates, read_xmp, read_xmp_date_values};
use crate::core::model::{Details, Metadata};
use image::ImageDecoder;
use std::{fs::File, io::BufReader, path::Path};

pub(super) fn read(path: &Path, result: &mut Metadata, dates: &mut Dates) {
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
        if result.lens.trim().is_empty()
            && let Some(field) = exif.get_field(exif::Tag::LensSpecification, exif::In::PRIMARY)
            && let exif::Value::Rational(values) = &field.value
        {
            let values: Vec<f64> = values.iter().map(exif::Rational::to_f64).collect();
            result.lens = crate::core::details::lens_specification(&values).unwrap_or_default();
        }
        result.captured = text(exif::Tag::DateTimeOriginal);
        dates.assign(0, Some(&result.captured));
        dates.assign(2, Some(&text(exif::Tag::DateTime)));
        result.exposure = text(exif::Tag::ExposureTime);
        result.aperture = text(exif::Tag::FNumber);
        result.iso = text(exif::Tag::PhotographicSensitivity);
        result.orientation = uint(exif::Tag::Orientation).clamp(1, 8) as u8;
        let rational = |tag| {
            exif.get_field(tag, exif::In::PRIMARY)
                .and_then(|f| match &f.value {
                    exif::Value::Rational(v) => v.first().map(exif::Rational::to_f64),
                    exif::Value::SRational(v) => v.first().map(exif::SRational::to_f64),
                    _ => None,
                })
        };
        let code = |tag| {
            exif.get_field(tag, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
                .and_then(|v| u16::try_from(v).ok())
        };
        let owned = |tag| Some(text(tag).trim().to_owned()).filter(|v| !v.is_empty());
        result.details = Details {
            make: owned(exif::Tag::Make),
            software: owned(exif::Tag::Software),
            focal_length_mm: rational(exif::Tag::FocalLength).map(|v| v as f32),
            exposure_bias_ev: rational(exif::Tag::ExposureBiasValue).map(|v| v as f32),
            flash: code(exif::Tag::Flash),
            white_balance: code(exif::Tag::WhiteBalance),
            metering: code(exif::Tag::MeteringMode),
            exposure_mode: code(exif::Tag::ExposureMode),
            scene: code(exif::Tag::SceneCaptureType),
            color_space: code(exif::Tag::ColorSpace),
            bit_depth: code(exif::Tag::BitsPerSample),
        };
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
            dates.assign(1, created.as_deref());
            dates.assign(3, photoshop.as_deref());
        }
    }
}
