//! Toys page: what Intiface Central found, a test buzz, and which output
//! channel of the active mode each toy plays.

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{App, Page};
use crate::engine::{Command, Shared};

impl App {
    pub(super) fn toys_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Your toys");
                ui.label(muted(
                    "Found by Intiface Central. Turn a toy on and press Start Scanning in Intiface to add it.",
                ));
                ui.add_space(8.0);
                if !s.intiface.connected {
                    card(PANEL).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            dot(ui, WARN);
                            ui.label(RichText::new("Intiface Central is not running").strong());
                        });
                        ui.label(muted(
                            "GameViber reaches your toys through Intiface Central. Open it and press its Start button.",
                        ));
                        if ui.button("Connection settings").clicked() {
                            self.page = Page::Connection;
                        }
                    });
                    return;
                }
                if s.intiface.toys.is_empty() {
                    waiting_for_toys(ui);
                    return;
                }
                let channels = s.mode.info.as_ref().map(|i| i.channels.clone()).unwrap_or_else(|| vec!["main".into()]);
                tile_grid(ui, s.intiface.toys.len(), 360.0, 150.0, |ui, i, size| {
                    let toy = &s.intiface.toys[i];
                    card(PANEL).show(ui, |ui| {
                        ui.set_width(size.x - 32.0);
                        ui.set_min_height(size.y - 32.0);
                        self.toy_card(ui, s, &toy.name, &channels);
                    });
                });
                ui.add_space(8.0);
                if channels.len() > 1 {
                    card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(format!(
                            "ℹ This mode sends {} different feelings ({}). Pick which ones each toy plays.",
                            channels.len(),
                            channels.join(", ")
                        ));
                    });
                }
            });
        });
    }

    fn toy_card(&self, ui: &mut egui::Ui, s: &Shared, name: &str, channels: &[String]) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("📳").size(20.0).color(ACCENT));
            ui.label(RichText::new(name).size(15.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚡ Buzz").on_hover_text("Short vibration to find which toy this is").clicked() {
                    self.send(Command::TestToy(name.to_owned()));
                }
            });
        });
        let level = s.toy_levels.get(name).copied().unwrap_or(0.0);
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Now")));
            meter(ui, (ui.available_width() - 50.0).max(40.0), level, ACCENT);
            ui.label(RichText::new(format!("{:.0}%", level * 100.0)).monospace().size(12.0));
        });
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Plays")));
            for channel in channels {
                let mut toys: Vec<String> = match s.settings.routing.get(channel) {
                    Some(list) => list.clone(),
                    None if channel == "main" => s.intiface.toys.iter().map(|t| t.name.clone()).collect(),
                    None => Vec::new(),
                };
                let on = toys.iter().any(|t| t == name);
                let label = if channels.len() == 1 { if on { "On" } else { "Off" }.to_owned() } else { channel.clone() };
                if ui.selectable_label(on, label).clicked() {
                    toys.retain(|t| t != name);
                    if !on {
                        toys.push(name.to_owned());
                    }
                    self.send(Command::SetRouting { channel: channel.clone(), toys });
                }
            }
        });
    }
}

pub(super) fn waiting_for_toys(ui: &mut egui::Ui) {
    card(BG).stroke(egui::Stroke::new(1.5, LINE)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Waiting for toys...");
        });
        ui.label(muted("Not showing up? Check it is charged, switched on and not connected to another app."));
    });
}
