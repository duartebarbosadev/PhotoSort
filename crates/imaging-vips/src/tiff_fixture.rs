use std::{fs::File, path::Path};
use tiff::{
    encoder::{Compression, TiffEncoder, colortype},
    tags::Tag,
};

pub fn write(path: &Path, profile: Option<&[u8]>, bits: u16, planar: bool, compressed: bool) {
    let pixels: Vec<u8> = (0..32u8)
        .flat_map(|y| (0..64u8).flat_map(move |x| [x * 4, y * 8, (x + y) * 2, (x / 8) * 24]))
        .collect();
    let words: Vec<u16> = pixels.iter().map(|v| u16::from(*v) * 257).collect();
    let file = File::create(path).unwrap();
    let mut encoder = TiffEncoder::new(file)
        .unwrap()
        .with_compression(if compressed {
            Compression::Lzw
        } else {
            Compression::Uncompressed
        });
    if !planar {
        if bits == 8 {
            let mut image = encoder.new_image::<colortype::CMYK8>(64, 32).unwrap();
            image.rows_per_strip(7).unwrap();
            if let Some(profile) = profile {
                image.encoder().write_tag(Tag::IccProfile, profile).unwrap();
            }
            image.write_data(&pixels).unwrap();
        } else {
            let mut image = encoder.new_image::<colortype::CMYK16>(64, 32).unwrap();
            image.rows_per_strip(7).unwrap();
            if let Some(profile) = profile {
                image.encoder().write_tag(Tag::IccProfile, profile).unwrap();
            }
            image.write_data(&words).unwrap();
        }
        return;
    }
    assert!(!compressed);
    let mut directory = encoder.image_directory().unwrap();
    directory.write_tag(Tag::ImageWidth, 64u32).unwrap();
    directory.write_tag(Tag::ImageLength, 32u32).unwrap();
    directory
        .write_tag(Tag::BitsPerSample, &[bits; 4][..])
        .unwrap();
    directory.write_tag(Tag::Compression, 1u16).unwrap();
    directory
        .write_tag(Tag::PhotometricInterpretation, 5u16)
        .unwrap();
    directory.write_tag(Tag::SamplesPerPixel, 4u16).unwrap();
    directory
        .write_tag(Tag::SampleFormat, &[1u16; 4][..])
        .unwrap();
    directory.write_tag(Tag::PlanarConfiguration, 2u16).unwrap();
    directory.write_tag(Tag::RowsPerStrip, 32u32).unwrap();
    if let Some(profile) = profile {
        directory.write_tag(Tag::IccProfile, profile).unwrap();
    }
    let mut offsets = Vec::new();
    for channel in 0..4 {
        let offset = if bits == 8 {
            let plane: Vec<u8> = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[channel])
                .collect();
            directory.write_data(plane.as_slice()).unwrap()
        } else {
            let plane: Vec<u16> = words
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[channel])
                .collect();
            directory.write_data(plane.as_slice()).unwrap()
        };
        offsets.push(u32::try_from(offset).unwrap());
    }
    directory
        .write_tag(Tag::StripOffsets, offsets.as_slice())
        .unwrap();
    directory
        .write_tag(
            Tag::StripByteCounts,
            &[64 * 32 * u32::from(bits / 8); 4][..],
        )
        .unwrap();
    directory.finish().unwrap();
}
