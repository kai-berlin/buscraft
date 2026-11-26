//! The app icon (assets/buscraft.svg), drawn at the sizes the window and the header need.

use egui::{Color32, ColorImage, Sense, TextureHandle, TextureOptions, Ui};

const SVG: &[u8] = include_bytes!("../assets/buscraft.svg");

/// Render the icon as RGBA pixels (straight alpha), `size` × `size`.
pub fn render(size: u32) -> Option<Vec<u8>> {
    let tree = resvg::usvg::Tree::from_data(SVG, &resvg::usvg::Options::default()).ok()?;
    let mut pix = resvg::tiny_skia::Pixmap::new(size, size)?;
    let s = tree.size();
    let scale = size as f32 / s.width().max(s.height());
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pix.as_mut());
    // tiny-skia keeps premultiplied colours; windows and egui's from_rgba_unmultiplied want straight ones.
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for p in pix.pixels() {
        let c = p.demultiply();
        out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(out)
}

/// Window / taskbar icon.
pub fn window_icon() -> Option<egui::IconData> {
    let size = 256;
    Some(egui::IconData { rgba: render(size)?, width: size, height: size })
}

fn texture(ui: &Ui) -> Option<TextureHandle> {
    // One window, one context: the texture is made once and kept.
    static TEX: std::sync::OnceLock<Option<TextureHandle>> = std::sync::OnceLock::new();
    TEX.get_or_init(|| {
        let size = 96;
        let img = ColorImage::from_rgba_unmultiplied([size as usize, size as usize], &render(size)?);
        Some(ui.ctx().load_texture("buscraft-app-icon", img, TextureOptions::LINEAR))
    })
    .clone()
}

/// The icon at `side` points, e.g. in the title bar.
pub fn show(ui: &mut Ui, side: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(side, side), Sense::hover());
    if let Some(t) = texture(ui) {
        ui.painter().image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_renders() {
        let px = super::render(64).unwrap();
        assert_eq!(px.len(), 64 * 64 * 4);
        // The middle of the icon is drawn (not transparent).
        let mid = (32 * 64 + 32) * 4;
        assert!(px[mid + 3] > 200);
        // BPS_ICON_OUT=file.png saves a 256 px picture of the icon for checking.
        if let Ok(out) = std::env::var("BPS_ICON_OUT") {
            image::RgbaImage::from_raw(256, 256, super::render(256).unwrap()).unwrap().save(out).unwrap();
        }
    }
}
