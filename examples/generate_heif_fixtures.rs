//! Generate tiny codec fixtures into a new directory; requires a HEVC encoder.
#[cfg(all(feature = "heif", not(target_os = "macos")))]
fn main() -> anyhow::Result<()> {
    use libheif_rs::*;
    use std::path::PathBuf;
    let root = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    anyhow::ensure!(!root.exists(), "Choose a new output directory");
    std::fs::create_dir_all(&root)?;
    let library = LibHeif::new_checked()?;
    let variants = [
        ImageOrientation::Normal,
        ImageOrientation::FlipHorizontally,
        ImageOrientation::Rotate180,
        ImageOrientation::FlipVertically,
        ImageOrientation::Rotate90CwThenFlipHorizontally,
        ImageOrientation::Rotate90Cw,
        ImageOrientation::Rotate90CwThenFlipVertically,
        ImageOrientation::Rotate270Cw,
    ];
    for (name, orientation, profile, nclx, premultiplied) in variants
        .into_iter()
        .enumerate()
        .map(|(i, o)| (format!("orientation-{}.heic", i + 1), o, None, None, false))
        .chain([
            (
                "linear-icc.heic".into(),
                ImageOrientation::Normal,
                Some({
                    let mut p = moxcms::ColorProfile::new_srgb();
                    p.cicp = None;
                    let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
                    p.red_trc = Some(curve.clone());
                    p.green_trc = Some(curve.clone());
                    p.blue_trc = Some(curve);
                    p.encode()?
                }),
                None,
                false,
            ),
            (
                "linear-nclx.heic".into(),
                ImageOrientation::Normal,
                None,
                Some(TransferCharacteristics::Linear),
                false,
            ),
            (
                "invalid-icc.heic".into(),
                ImageOrientation::Normal,
                Some(vec![0; 128]),
                None,
                false,
            ),
            (
                "premultiplied.heic".into(),
                ImageOrientation::Normal,
                None,
                None,
                true,
            ),
        ])
    {
        let mut image = Image::new(64, 48, ColorSpace::Rgb(RgbChroma::Rgba))?;
        image.create_plane(Channel::Interleaved, 64, 48, 8)?;
        let plane = image.planes_mut().interleaved.unwrap();
        for y in 0..48usize {
            for x in 0..64usize {
                let (gray, alpha) = if name.starts_with("orientation") {
                    (32 + (x / 16) * 48 + (y / 24) * 24, 255)
                } else {
                    (128, 128)
                };
                let gray = if premultiplied {
                    (gray * alpha + 127) / 255
                } else {
                    gray
                } as u8;
                plane.data[y * plane.stride + x * 4..][..4].copy_from_slice(&[
                    gray,
                    gray,
                    gray,
                    alpha as u8,
                ]);
            }
        }
        if let Some(data) = profile {
            image.set_color_profile_raw(&ColorProfileRaw::new(color_profile_types::PROF, data))?;
        }
        if nclx.is_some() {
            let profile = ColorProfileNCLX::new().expect("NCLX allocation");
            image.set_color_profile_nclx(&profile)?;
        }
        image.set_premultiplied_alpha(premultiplied);
        let mut encoder = library.encoder_for_format(CompressionFormat::Hevc)?;
        encoder.set_quality(EncoderQuality::LossLess)?;
        encoder.set_parameter_value("x265:pools", EncoderParameterValue::String("none".into()))?;
        encoder.set_parameter_value(
            "x265:frame-threads",
            EncoderParameterValue::String("1".into()),
        )?;
        let mut options = EncodingOptions::new()?;
        options.set_image_orientation(orientation);
        options.set_mac_os_compatibility_workaround_no_nclx_profile(false);
        options.set_save_two_colr_boxes_when_icc_and_nclx_available(true);
        let mut context = HeifContext::new()?;
        context.encode_image(&image, &mut encoder, Some(options))?;
        let path = root.join(name);
        context.write_to_file(path.to_str().unwrap())?;
        if nclx.is_some() {
            // Fixture authoring only: label the unchanged encoded gray values as
            // linear light. The binding does not expose the NCLX transfer setter.
            let mut bytes = std::fs::read(&path)?;
            let offsets: Vec<_> = bytes
                .windows(8)
                .enumerate()
                .filter_map(|(i, w)| (w == b"colrnclx").then_some(i))
                .collect();
            anyhow::ensure!(offsets.len() == 1, "Expected one encoder-written NCLX box");
            bytes[offsets[0] + 8..offsets[0] + 12].copy_from_slice(&[0, 1, 0, 8]);
            std::fs::write(path, bytes)?;
        }
    }
    Ok(())
}
#[cfg(not(all(feature = "heif", not(target_os = "macos"))))]
fn main() {
    eprintln!("Generate these fixtures on a portable HEIF build with a HEVC encoder.");
}
