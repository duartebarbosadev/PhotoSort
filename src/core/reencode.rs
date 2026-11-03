//! Lossless pixel rotation for PNG, TIFF and BMP files whose orientation cannot be
//! stored in their own metadata. It is offered only after explicit confirmation;
//! sidecar orientation remains the default. The file's own metadata (EXIF dates,
//! GPS, XMP, text) travels with the new pixels, with its orientation set upright.
use super::{metadata, model::Fingerprint};
use anyhow::{Context, Result, bail, ensure};
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
    codecs::{bmp::BmpEncoder, png::PngDecoder, png::PngEncoder},
    metadata::Orientation,
};
use std::{fs, io::Cursor, path::Path};
use tiff::{
    decoder::Decoder,
    encoder::{Compression, colortype, compression::DeflateLevel},
    tags::{ByteOrder, Tag, Type, ValueBuffer},
};

/// Formats whose pixels can be rewritten without loss.
pub fn supported(path: &Path) -> bool {
    format(path).is_some()
}

fn format(path: &Path) -> Option<ImageFormat> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some(ImageFormat::Png),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        "bmp" => Some(ImageFormat::Bmp),
        _ => None,
    }
}

/// Rewrite `path` with `orientation` (EXIF 2–8) applied to its pixels. The new
/// file is decoded again and must match the rotated pixels exactly before it
/// replaces the original; any unsupported detail leaves the source untouched.
pub fn rotate(path: &Path, expected: Fingerprint, orientation: u8) -> Result<()> {
    let format = format(path).context("Pixel rotation supports PNG, TIFF and BMP only")?;
    let orientation = Orientation::from_exif(orientation)
        .filter(|o| *o != Orientation::NoTransforms)
        .context("Invalid pixel rotation")?;
    metadata::replace_verified(path, expected, |staged| {
        let bytes = fs::read(staged)?;
        let (mut image, icc) = decode(&bytes, format)?;
        image.apply_orientation(orientation);
        let encoded = encode(&image, format, icc, &bytes, orientation.to_exif())?;
        let (check, _) = decode(&encoded, format)?;
        ensure!(
            check.color() == image.color()
                && (check.width(), check.height()) == (image.width(), image.height())
                && check.as_bytes() == image.as_bytes(),
            "Re-encoded pixels did not match; the file was left unchanged"
        );
        // Writing into the copied file keeps its permissions and attributes.
        fs::write(staged, &encoded)?;
        Ok(())
    })
}

fn decode(bytes: &[u8], format: ImageFormat) -> Result<(DynamicImage, Option<Vec<u8>>)> {
    // `image` decodes only the first frame or page; refuse anything with more.
    match format {
        ImageFormat::Png => ensure!(
            !PngDecoder::new(Cursor::new(bytes))?.is_apng()?,
            "Animated PNG cannot be rotated by re-encoding"
        ),
        ImageFormat::Tiff => ensure!(
            !tiff::decoder::Decoder::new(Cursor::new(bytes))?.more_images(),
            "Multi-page TIFF cannot be rotated by re-encoding"
        ),
        _ => {}
    }
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format).into_decoder()?;
    let icc = decoder.icc_profile()?;
    Ok((DynamicImage::from_decoder(decoder)?, icc))
}

fn encode(
    image: &DynamicImage,
    format: ImageFormat,
    icc: Option<Vec<u8>>,
    source: &[u8],
    orientation: u8,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let (width, height) = (image.width(), image.height());
    let color = ExtendedColorType::from(image.color());
    // A dropped colour profile would visibly shift colours, so it must survive.
    fn with_profile(encoder: &mut impl ImageEncoder, icc: Option<Vec<u8>>) -> Result<()> {
        if let Some(icc) = icc {
            encoder
                .set_icc_profile(icc)
                .map_err(|_| anyhow::anyhow!("The colour profile cannot be preserved"))?;
        }
        Ok(())
    }
    match format {
        ImageFormat::Png => {
            let mut encoder = PngEncoder::new(&mut out);
            with_profile(&mut encoder, icc)?;
            encoder.write_image(image.as_bytes(), width, height, color)?;
            out = carry_png_metadata(source, &out, orientation)?;
        }
        ImageFormat::Tiff => return encode_tiff(image, icc, source, orientation),
        ImageFormat::Bmp => {
            let mut encoder = BmpEncoder::new(&mut out);
            with_profile(&mut encoder, icc)?;
            encoder.write_image(image.as_bytes(), width, height, color)?;
        }
        _ => bail!("Unsupported pixel rotation format"),
    }
    Ok(out)
}

/// Orientations 5 to 8 swap width and height.
fn transposed(orientation: u8) -> bool {
    orientation >= 5
}

/// PNG chunks are copied from the source after the encoder's own, before the
/// first IDAT: safe-to-copy ancillary chunks (text, eXIf, XMP), plus the colour
/// chunks whose meaning does not depend on the pixel layout. Chunks tied to the
/// old sample layout (PLTE, tRNS, sBIT, bKGD...) are not. Legacy "Raw profile"
/// text can hold an orientation that cannot be reset, so it is dropped rather
/// than turn the new pixels a second time.
fn carry_png_metadata(source: &[u8], encoded: &[u8], orientation: u8) -> Result<Vec<u8>> {
    let source = png_chunks(source).context("Cannot read the PNG's metadata")?;
    let encoded_chunks = png_chunks(encoded).context("Re-encoded PNG is malformed")?;
    let written: Vec<[u8; 4]> = encoded_chunks.iter().map(|(kind, _)| *kind).collect();
    let mut carried = Vec::new();
    for (kind, data) in source {
        if written.contains(&kind) || matches!(&kind, b"acTL" | b"fcTL" | b"fdAT") {
            continue;
        }
        let data = match &kind {
            b"gAMA" | b"cHRM" | b"sRGB" | b"cICP" => data.to_vec(),
            b"pHYs" if data.len() == 9 && transposed(orientation) => {
                [&data[4..8], &data[..4], &data[8..]].concat()
            }
            b"eXIf" => {
                let mut exif = data.to_vec();
                upright_exif(&mut exif);
                exif
            }
            b"iTXt" if data.starts_with(b"XML:com.adobe.xmp\0") => {
                // Keyword, compression flag; compressed XMP cannot be reset.
                if data.get(18) != Some(&0) {
                    continue;
                }
                upright_xmp(data)
            }
            b"tEXt" | b"zTXt" | b"iTXt" if data.starts_with(b"Raw profile type") => continue,
            // Ancillary (lowercase first letter) and safe to copy (lowercase last).
            _ if kind[0].is_ascii_lowercase() && kind[3].is_ascii_lowercase() => data.to_vec(),
            _ => continue,
        };
        carried.push((kind, data));
    }
    let mut out = encoded[..8].to_vec();
    let mut inserted = false;
    for (kind, data) in &encoded_chunks {
        if kind == b"IDAT" && !inserted {
            for (kind, data) in &carried {
                write_png_chunk(&mut out, kind, data);
            }
            inserted = true;
        }
        write_png_chunk(&mut out, kind, data);
    }
    ensure!(inserted, "Re-encoded PNG has no image data");
    Ok(out)
}

fn png_chunks(bytes: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut chunks = Vec::new();
    let mut at = 8;
    while at < bytes.len() {
        let length = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind: [u8; 4] = bytes.get(at + 4..at + 8)?.try_into().ok()?;
        let data = bytes.get(at + 8..(at + 8).checked_add(length)?)?;
        chunks.push((kind, data));
        at += 12 + length;
        if &kind == b"IEND" {
            break;
        }
    }
    Some(chunks)
}

fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// Set Orientation (0x0112) in IFD0 of an EXIF TIFF block to 1, in place.
fn upright_exif(exif: &mut [u8]) {
    let little = match exif.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return,
    };
    let u16_at = |b: &[u8], at: usize| {
        b.get(at..at + 2).map(|v| {
            let v = [v[0], v[1]];
            if little {
                u16::from_le_bytes(v)
            } else {
                u16::from_be_bytes(v)
            }
        })
    };
    let Some(ifd) = exif.get(4..8).map(|v| {
        let v = [v[0], v[1], v[2], v[3]];
        (if little {
            u32::from_le_bytes(v)
        } else {
            u32::from_be_bytes(v)
        }) as usize
    }) else {
        return;
    };
    let Some(count) = u16_at(exif, ifd) else {
        return;
    };
    for entry in 0..usize::from(count) {
        let at = ifd + 2 + entry * 12;
        // Tag 0x0112, type SHORT, count 1: the value sits in the entry itself.
        if u16_at(exif, at) == Some(0x0112) && u16_at(exif, at + 2) == Some(3) {
            let one = if little {
                1u16.to_le_bytes()
            } else {
                1u16.to_be_bytes()
            };
            if let Some(value) = exif.get_mut(at + 8..at + 10) {
                value.copy_from_slice(&one);
            }
        }
    }
}

/// Set `tiff:Orientation` in an XMP packet to 1, as attribute or element.
fn upright_xmp(xmp: &[u8]) -> Vec<u8> {
    let mut out = xmp.to_vec();
    for prefix in [
        &b"tiff:Orientation=\""[..],
        b"tiff:Orientation='",
        b"<tiff:Orientation>",
    ] {
        let mut from = 0;
        while let Some(found) = out[from..].windows(prefix.len()).position(|w| w == prefix) {
            let value = from + found + prefix.len();
            if out.get(value).is_some_and(u8::is_ascii_digit) {
                out[value] = b'1';
            }
            from = value;
        }
    }
    out
}

/// Descriptive IFD0 tags carried to the rewritten TIFF. Structure tags are the
/// encoder's own, and Orientation is left out: absent means upright.
const TIFF_TAGS: [u16; 15] = [
    269,   // DocumentName
    270,   // ImageDescription
    271,   // Make
    272,   // Model
    285,   // PageName
    305,   // Software
    306,   // DateTime
    315,   // Artist
    316,   // HostComputer
    318,   // WhitePoint
    319,   // PrimaryChromaticities
    700,   // XMP
    33432, // Copyright
    33723, // IPTC
    34377, // Photoshop
];

/// Re-encode with the tiff crate so the EXIF and GPS directories (capture date,
/// camera, location) come along byte for byte, with the source compression.
fn encode_tiff(
    image: &DynamicImage,
    icc: Option<Vec<u8>>,
    source: &[u8],
    orientation: u8,
) -> Result<Vec<u8>> {
    let swap = transposed(orientation);
    let mut decoder = Decoder::new(Cursor::new(source))?;
    let compression = match decoder.find_tag_unsigned::<u16>(Tag::Compression)? {
        Some(5) => Compression::Lzw,
        Some(8 | 32946) => Compression::Deflate(DeflateLevel::Balanced),
        Some(32773) => Compression::Packbits,
        _ => Compression::Uncompressed,
    };
    let mut top = Vec::new();
    {
        let mut ifd = decoder.image_ifd();
        for code in TIFF_TAGS.into_iter().chain([282, 283, 296]) {
            // X and Y resolution trade places when width and height do.
            let target = match code {
                282 if swap => 283,
                283 if swap => 282,
                other => other,
            };
            if let Some(mut value) = tiff_value(&mut ifd, code)? {
                if code == 700 {
                    let patched = upright_xmp(value.as_bytes());
                    value = ValueBuffer::from_value(&patched.as_slice());
                }
                top.push((target, value));
            }
        }
    }
    let mut directories = Vec::new();
    for pointer in [Tag::ExifDirectory, Tag::GpsDirectory] {
        let Some(at) = decoder.find_tag(pointer)? else {
            continue;
        };
        let directory = decoder.read_directory(at.into_ifd_pointer()?)?;
        let mut ifd = decoder.read_directory_tags(&directory);
        let mut entries = Vec::new();
        for (tag, _) in directory.iter() {
            let code = tag.to_u16();
            // The interoperability directory is a further offset; leave it out.
            if code == 0xA005 {
                continue;
            }
            let target = match code {
                0xA002 if swap => 0xA003, // PixelXDimension
                0xA003 if swap => 0xA002, // PixelYDimension
                other => other,
            };
            if let Some(value) = tiff_value(&mut ifd, code)? {
                entries.push((target, value));
            }
        }
        directories.push((pointer, entries));
    }

    let mut out = Cursor::new(Vec::new());
    let mut encoder = tiff::encoder::TiffEncoder::new(&mut out)?.with_compression(compression);
    let mut pointers = Vec::new();
    for (pointer, entries) in directories {
        let mut directory = encoder.extra_directory()?;
        for (code, value) in &entries {
            directory.write_tag_buf(Tag::from_u16_exhaustive(*code), value)?;
        }
        pointers.push((pointer, directory.finish_with_offsets()?.offset));
    }
    let (width, height) = (image.width(), image.height());
    macro_rules! write {
        ($color:ty, $buffer:expr) => {{
            let mut encoded = encoder.new_image::<$color>(width, height)?;
            let tags = encoded.encoder();
            for (code, value) in &top {
                tags.write_tag_buf(Tag::from_u16_exhaustive(*code), value)?;
            }
            for (pointer, offset) in &pointers {
                tags.write_tag(*pointer, *offset)?;
            }
            if let Some(icc) = &icc {
                tags.write_tag(Tag::IccProfile, icc.as_slice())?;
            }
            encoded.write_data($buffer.as_raw())?;
        }};
    }
    match image {
        DynamicImage::ImageLuma8(b) => write!(colortype::Gray8, b),
        DynamicImage::ImageRgb8(b) => write!(colortype::RGB8, b),
        DynamicImage::ImageRgba8(b) => write!(colortype::RGBA8, b),
        DynamicImage::ImageLuma16(b) => write!(colortype::Gray16, b),
        DynamicImage::ImageRgb16(b) => write!(colortype::RGB16, b),
        DynamicImage::ImageRgba16(b) => write!(colortype::RGBA16, b),
        DynamicImage::ImageRgb32F(b) => write!(colortype::RGB32Float, b),
        DynamicImage::ImageRgba32F(b) => write!(colortype::RGBA32Float, b),
        _ => bail!("This TIFF's sample format cannot be rotated by re-encoding"),
    }
    Ok(out.into_inner())
}

/// One tag's value as raw bytes in this machine's byte order, which the encoder
/// writes, so it can be stored again unchanged.
fn tiff_value(ifd: &mut tiff::decoder::IfdDecoder<'_>, code: u16) -> Result<Option<ValueBuffer>> {
    let mut value = ValueBuffer::empty(Type::BYTE);
    if ifd
        .find_tag_buf(Tag::from_u16_exhaustive(code), &mut value)?
        .is_none()
    {
        return Ok(None);
    }
    value.set_byte_order(ByteOrder::native());
    Ok(Some(value))
}
