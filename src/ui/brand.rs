//! Shared public identity for the UI, native window and saved-scan preview.
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, vec2};

pub const NAME: &str = "Diskvale";
pub const WINDOW_TITLE: &str = "Diskvale — Disk Explorer";

pub fn window_icon() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../assets/brand/diskvale-app-256.png"))
        .expect("valid embedded Diskvale window icon")
}

pub(super) fn header_mark(ui: &mut egui::Ui) {
    let id = egui::Id::new("diskvale-brand-icon");
    let cached = ui.ctx().data_mut(|data| data.get_temp::<TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!(
            "../../assets/brand/diskvale-app-64.png"
        ))
        .expect("valid embedded Diskvale header icon");
        let texture = ui.ctx().load_texture(
            "Diskvale logo",
            ColorImage::from_rgba_unmultiplied(
                [icon.width as usize, icon.height as usize],
                &icon.rgba,
            ),
            TextureOptions::LINEAR,
        );
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    ui.add(
        egui::Image::new(&texture)
            .fit_to_exact_size(vec2(30.0, 30.0))
            .alt_text(NAME),
    );
}
