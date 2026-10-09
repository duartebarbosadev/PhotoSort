//! Lossless pixel rotation for PNG, TIFF and BMP files whose orientation cannot be
//! stored in their own metadata. It is offered only after explicit confirmation;
//! sidecar orientation remains the default.
use super::{metadata, model::Fingerprint};
use anyhow::{Context, Result, bail, ensure};
use image::{
    DynamicImage, ExtendedColorType, ImageDecoder, ImageEncoder, ImageFormat, ImageReader,
    codecs::{bmp::BmpEncoder, png::PngDecoder, png::PngEncoder, tiff::TiffEncoder},
    metadata::Orientation,
};
use std::{fs, io::Cursor, path::Path};

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
        let encoded = encode(&image, format, icc)?;
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

fn encode(image: &DynamicImage, format: ImageFormat, icc: Option<Vec<u8>>) -> Result<Vec<u8>> {
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
        }
        ImageFormat::Tiff => {
            let mut encoder = TiffEncoder::new(Cursor::new(&mut out));
            with_profile(&mut encoder, icc)?;
            encoder.write_image(image.as_bytes(), width, height, color)?;
        }
        ImageFormat::Bmp => {
            let mut encoder = BmpEncoder::new(&mut out);
            with_profile(&mut encoder, icc)?;
            encoder.write_image(image.as_bytes(), width, height, color)?;
        }
        _ => bail!("Unsupported pixel rotation format"),
    }
    Ok(out)
}
