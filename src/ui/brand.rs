//! The app icon as an in-window logo (title bar, About, welcome).
//!
//! The 1024 px artwork is trimmed to its visible bounds and resampled once
//! per on-screen pixel height, so the texture maps 1:1 onto screen pixels and
//! stays sharp at any display scale instead of being stretched by the GPU.
use eframe::egui::{self, TextureHandle, emath::GuiRounding};
use image::RgbaImage;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The artwork cropped to its non-transparent bounds, or `None` if it failed
/// to decode (the logo is then left out rather than taking the app down).
fn artwork() -> Option<&'static RgbaImage> {
    static ARTWORK: OnceLock<Option<RgbaImage>> = OnceLock::new();
    ARTWORK
        .get_or_init(|| {
            let image = image::load_from_memory_with_format(
                super::platform::APP_ICON_PNG,
                image::ImageFormat::Png,
            )
            .inspect_err(|error| tracing::warn!(%error, "app icon unavailable"))
            .ok()?
            .into_rgba8();
            Some(trim(&image))
        })
        .as_ref()
}

fn trim(image: &RgbaImage) -> RgbaImage {
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] > 0 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
    }
    if left > right {
        return image.clone();
    }
    image::imageops::crop_imm(image, left, top, right - left + 1, bottom - top + 1).to_image()
}

/// The texture for an icon `height_px` screen pixels tall, made on first use.
fn texture(ctx: &egui::Context, height_px: u32) -> Option<TextureHandle> {
    let id = egui::Id::new("app-icon-textures");
    if let Some(handle) = ctx.data(|d| {
        d.get_temp::<HashMap<u32, TextureHandle>>(id)
            .and_then(|cache| cache.get(&height_px).cloned())
    }) {
        return Some(handle);
    }
    let art = artwork()?;
    let width_px = (height_px as f32 * art.width() as f32 / art.height() as f32)
        .round()
        .max(1.0) as u32;
    let scaled = image::imageops::resize(
        art,
        width_px,
        height_px,
        image::imageops::FilterType::Lanczos3,
    );
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [width_px as usize, height_px as usize],
        scaled.as_raw(),
    );
    let handle = ctx.load_texture("app-icon", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashMap<u32, TextureHandle>>(id)
            .insert(height_px, handle.clone());
    });
    Some(handle)
}

/// Shows the app icon `height` points tall, as wide as its artwork needs.
pub(crate) fn app_logo(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let ppp = ui.ctx().pixels_per_point();
    let height_px = (height * ppp).round().max(1.0) as u32;
    let Some(texture) = texture(ui.ctx(), height_px) else {
        return ui.allocate_response(egui::Vec2::ZERO, egui::Sense::hover());
    };
    let size = egui::vec2(texture.size()[0] as f32, texture.size()[1] as f32) / ppp;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        // Snapped to whole pixels so each texel lands on exactly one pixel.
        let rect = egui::Rect::from_min_size(rect.min.round_to_pixels(ppp), size);
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter()
            .image(texture.id(), rect, uv, egui::Color32::WHITE);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_artwork_decodes_trimmed_and_scales_to_exact_pixels() {
        let art = artwork().expect("embedded icon decodes");
        // Trimmed to the visible artwork: no fully transparent edge rows.
        assert!(art.width() < 1024 && art.height() < 1024);
        assert!((0..art.width()).any(|x| art.get_pixel(x, 0)[3] > 0));
        assert!((0..art.height()).any(|y| art.get_pixel(0, y)[3] > 0));

        let ctx = egui::Context::default();
        let small = texture(&ctx, 40).unwrap();
        assert_eq!(small.size()[1], 40);
        // Cached per pixel height.
        assert_eq!(texture(&ctx, 40).unwrap().id(), small.id());
        assert_ne!(texture(&ctx, 112).unwrap().id(), small.id());
    }
}
