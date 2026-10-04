//! Screen page: the game's image as the in-game overlay copies it, and what
//! is measured on it. The overlay copies frames only while this page is open.

use eframe::egui::{self, Margin};

use super::theme::*;
use super::App;
use crate::engine::Shared;

/// The preview is never wider than this.
const PREVIEW_WIDTH: f32 = 640.0;

#[derive(Default)]
pub struct State {
    texture: Option<egui::TextureHandle>,
    /// Count of the frame in the texture.
    shown: Option<u32>,
}

impl App {
    pub(super) fn screen_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Game image");
                ui.label(muted(
                    "The in-game overlay can copy small images of the game for GameViber: to tell a battle from a \
                     cutscene, or read a health bar. They are analysed on your computer and never saved. For now \
                     they are only copied while this page is open.",
                ));
                ui.add_space(8.0);
                self.screen_preview(ui, s);
            });
        });
    }

    fn screen_preview(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let view = &s.screen;
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Seen right now");
            let Some(frame) = &view.frame else {
                self.screen.texture = None;
                self.screen.shown = None;
                let hint = if s.overlay_unavailable {
                    "Another GameViber holds the in-game overlay."
                } else if s.overlay_clients.is_empty() {
                    "Nothing: start a game with the in-game overlay (see the Overlay page)."
                } else {
                    "Waiting for the game's image..."
                };
                ui.horizontal(|ui| {
                    dot(ui, WARN);
                    ui.label(hint);
                });
                return;
            };
            ui.horizontal(|ui| {
                dot(ui, OK);
                ui.label(format!(
                    "{}: {}x{} copies of a {}x{} image, {:.0} per second",
                    view.game.as_deref().unwrap_or("game"),
                    frame.width,
                    frame.height,
                    frame.source_width,
                    frame.source_height,
                    view.rate
                ));
            });
            if self.screen.shown != Some(frame.count) {
                let image = egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.pixels);
                match &mut self.screen.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => self.screen.texture = Some(ui.ctx().load_texture("game-image", image, egui::TextureOptions::LINEAR)),
                }
                self.screen.shown = Some(frame.count);
            }
            if let Some(texture) = &self.screen.texture {
                let width = ui.available_width().min(PREVIEW_WIDTH);
                let size = egui::vec2(width, width * frame.height as f32 / frame.width.max(1) as f32);
                ui.add(egui::Image::new(texture).fit_to_exact_size(size).corner_radius(6.0));
            }
            if let Some(levels) = view.levels {
                ui.add_space(6.0);
                egui::Grid::new("screen-levels").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (label, value, hint) in [
                        ("Brightness", levels.brightness, "Average brightness of the image"),
                        ("Motion", levels.motion, "How much the image changes between two copies"),
                    ] {
                        ui.label(muted(label)).on_hover_text(hint);
                        meter(ui, 240.0, value as f64, GAME);
                        ui.end_row();
                    }
                });
            }
        });
    }
}
