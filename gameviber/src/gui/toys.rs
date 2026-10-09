//! Toys page: the connection to Intiface Central, what it found, a test buzz,
//! which output channel of the active mode each toy plays, and per-toy
//! intensity settings (and what a stroker can do).

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{intiface_status, App};
use crate::config::ToySettings;
use crate::engine::{Command, Shared, TEST_LEVEL};
use crate::intiface::{Control, Toy};
use crate::stroke::{StrokeSettings, StrokeStyle};

pub const INTIFACE_DOWNLOAD: &str = "https://intiface.com/central/";

#[derive(Default)]
pub struct State {
    /// Intiface address being edited (None: show the saved one).
    url: Option<String>,
}

impl App {
    pub(super) fn toys_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Your toys");
                ui.label(muted(
                    "Found by Intiface Central. Turn a toy on while it scans to add it.",
                ));
                ui.add_space(8.0);
                self.intiface_card(ui, s);
                ui.add_space(8.0);
                if !s.intiface.connected {
                    return;
                }
                if s.intiface.toys.is_empty() {
                    waiting_for_toys(ui);
                    return;
                }
                let channels = s.mode.info.as_ref().map(|i| i.channels.clone()).unwrap_or_else(|| vec!["main".into()]);
                let height = if s.intiface.toys.iter().any(|t| t.stroker) { 440.0 } else { 270.0 };
                tile_grid(ui, s.intiface.toys.len(), 360.0, height, |ui, i, size| {
                    let toy = &s.intiface.toys[i];
                    card(PANEL).show(ui, |ui| {
                        ui.set_width(size.x - 32.0);
                        ui.set_min_height(size.y - 32.0);
                        self.toy_card(ui, s, toy, &channels);
                    });
                });
                ui.add_space(8.0);
                if s.intiface.toys.iter().any(|t| t.numbered) {
                    card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            "ℹ Several toys have the same name, so GameViber numbers them in the order they \
                             connect. Give each one its own name in Intiface Central to keep their settings \
                             attached to the right toy.",
                        );
                    });
                    ui.add_space(8.0);
                }
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

    /// Connection to Intiface Central: its state, how to get it running, its
    /// address, and the buttons to disconnect, reconnect and scan for toys.
    fn intiface_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let status = &s.intiface;
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (color, text) = intiface_status(s);
            ui.horizontal(|ui| {
                dot(ui, color);
                ui.label(RichText::new("🔌 Intiface Central").strong());
                ui.label(text);
                if status.connected && !status.server.is_empty() {
                    ui.label(muted(&status.server).size(12.0));
                }
                if !s.intiface_enabled {
                    return;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if status.paused {
                        if ui.add(primary("Connect")).clicked() {
                            self.send(Command::Intiface(Control::Connect));
                        }
                    } else if status.connected {
                        if ui.button("Disconnect").on_hover_text("Stops your toys and leaves Intiface Central").clicked() {
                            self.send(Command::Intiface(Control::Disconnect));
                        }
                        if ui.button("⟳ Reconnect").on_hover_text("Connects again, for a toy or a server that misbehaves").clicked() {
                            self.send(Command::Intiface(Control::Connect));
                        }
                    } else {
                        if ui.button("Stop trying").on_hover_text("Stays disconnected until you press Connect").clicked() {
                            self.send(Command::Intiface(Control::Disconnect));
                        }
                        if ui.button("⟳ Retry now").clicked() {
                            self.send(Command::Intiface(Control::Connect));
                        }
                    }
                });
            });
            if status.paused {
                ui.label(muted("Disconnected: your toys get nothing from GameViber until you connect again."));
            } else if s.intiface_enabled && !status.connected {
                ui.label(muted(
                    "GameViber reaches your toys through Intiface Central, a free app: open it and press its \
                     Start button. GameViber connects on its own.",
                ));
                ui.hyperlink_to("Get Intiface Central ↗", INTIFACE_DOWNLOAD);
            }
            if status.connected {
                ui.horizontal(|ui| {
                    if status.scanning {
                        ui.spinner();
                        ui.label("Looking for new toys");
                        if ui.button("Stop scanning").on_hover_text("Toys already found stay connected").clicked() {
                            self.send(Command::Intiface(Control::StopScanning));
                        }
                    } else {
                        dot(ui, IDLE);
                        ui.label("Not looking for new toys");
                        if ui.button("🔍 Start scanning").on_hover_text("Turn the toy on first").clicked() {
                            self.send(Command::Intiface(Control::StartScanning));
                        }
                    }
                });
                if let Some(e) = &status.error {
                    ui.label(RichText::new(e).monospace().size(12.0).color(MUTED));
                }
            }
            egui::CollapsingHeader::new("Server address").id_salt("intiface-address").show(ui, |ui| {
                self.intiface_address(ui, s);
                if let (false, Some(e)) = (status.connected, &status.error) {
                    ui.label(RichText::new(e).monospace().size(12.0).color(MUTED));
                }
            });
        });
    }

    /// Server address editor (Intiface on another machine or port).
    pub(super) fn intiface_address(&mut self, ui: &mut egui::Ui, s: &Shared) {
        ui.horizontal(|ui| {
            ui.label("Server address");
            let url = self.toys.url.get_or_insert_with(|| s.settings.url.clone());
            ui.add(egui::TextEdit::singleline(url).desired_width(260.0).font(egui::TextStyle::Monospace));
            let changed = *url != s.settings.url;
            if ui.add_enabled(changed, egui::Button::new("Apply")).clicked() {
                let url = url.clone();
                self.send(Command::SetUrl(url));
            }
            if !changed {
                // Follow the saved value until the user edits it.
                self.toys.url = None;
            }
        });
        ui.label(muted("Intiface Central shows it on its main screen. The default is ws://127.0.0.1:12345.").size(12.0));
    }

    fn toy_card(&self, ui: &mut egui::Ui, s: &Shared, toy: &Toy, channels: &[String]) {
        let name = toy.name.as_str();
        ui.horizontal(|ui| {
            ui.label(RichText::new(if toy.stroker { "↕" } else { "📳" }).size(20.0).color(ACCENT));
            ui.label(RichText::new(name).size(15.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, hint) = if toy.stroker {
                    ("↕ Stroke", "A few strokes to find which toy this is (it first gets in place slowly)")
                } else {
                    ("⚡ Buzz", "Short vibration to find which toy this is")
                };
                if ui.button(label).on_hover_text(hint).clicked() {
                    self.send(Command::TestToy(name.to_owned(), TEST_LEVEL));
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
        ui.separator();
        let settings = s.settings.toys.get(name).copied().unwrap_or_default();
        if let Some(settings) = toy_settings(ui, name, settings) {
            self.send(Command::SetToySettings { toy: name.to_owned(), settings });
        }
        if toy.stroker {
            ui.separator();
            if let Some(stroke) = stroke_settings(ui, settings.stroke) {
                self.send(Command::SetToySettings { toy: name.to_owned(), settings: ToySettings { stroke, ..settings } });
            }
        }
        let hints = if toy.stroker {
            ["The slowest, shortest strokes a mode can ask for", "Strokes at half strength", "The fastest, longest strokes a mode can ask for (within Strongest)"]
        } else {
            ["The gentlest vibration a mode can ask for", "A vibration at half strength", "The strongest vibration a mode can ask for (within Strongest)"]
        };
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Feel")));
            for ((label, level), hint) in [("Weakest", ToySettings::SILENT), ("Medium", TEST_LEVEL), ("Strongest", 1.0)].into_iter().zip(hints) {
                if ui.button(label).on_hover_text(hint).clicked() {
                    self.send(Command::TestToy(name.to_owned(), level));
                }
            }
            if settings != ToySettings::default() && ui.button("Defaults").clicked() {
                self.send(Command::SetToySettings { toy: name.to_owned(), settings: ToySettings::default() });
            }
        });
    }
}

/// Sliders for a toy's weakest and strongest vibration and its curve, next to a
/// preview of the curve. Returns the new settings when one changed.
fn toy_settings(ui: &mut egui::Ui, name: &str, settings: ToySettings) -> Option<ToySettings> {
    let mut new = settings;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 190.0).max(80.0);
            let mut min = new.min * 100.0;
            let mut max = new.max * 100.0;
            labeled(ui, "Weakest", |ui| ui.add(egui::Slider::new(&mut min, 0.0..=100.0).suffix("%").integer()))
                .on_hover_text("Raise it until the gentlest vibrations are felt: many toys do nothing below 10-20%");
            labeled(ui, "Strongest", |ui| ui.add(egui::Slider::new(&mut max, 0.0..=100.0).suffix("%").integer()))
                .on_hover_text("Lower it if this toy is too strong compared to the others");
            labeled(ui, "Curve", |ui| {
                ui.add(egui::Slider::new(&mut new.curve, ToySettings::CURVE_RANGE).step_by(0.05).fixed_decimals(2))
            })
            .on_hover_text("Below 1: gentle vibrations feel stronger. Above 1: they feel softer and peaks stand out");
            // Weakest and strongest push each other rather than crossing.
            if min != new.min * 100.0 {
                new.min = min / 100.0;
                new.max = new.max.max(new.min);
            } else if max != new.max * 100.0 {
                new.max = max / 100.0;
                new.min = new.min.min(new.max);
            }
        });
        curve_preview(ui, name, &new);
    });
    (new != settings).then_some(new)
}

fn labeled(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> egui::Response) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted(label)));
        add(ui)
    })
    .inner
}

/// What a stroker can do and how far it goes: its range, its fastest and slowest
/// moves, how often it may turn, and what the intensity changes. Returns the new
/// settings when one changed.
fn stroke_settings(ui: &mut egui::Ui, settings: StrokeSettings) -> Option<StrokeSettings> {
    let mut new = settings;
    ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(80.0);
    let mut bottom = new.bottom * 100.0;
    let mut top = new.top * 100.0;
    labeled(ui, "Lowest", |ui| ui.add(egui::Slider::new(&mut bottom, 0.0..=100.0).suffix("%").integer()))
        .on_hover_text("The lowest position the toy goes to, from its whole length");
    labeled(ui, "Highest", |ui| ui.add(egui::Slider::new(&mut top, 0.0..=100.0).suffix("%").integer()))
        .on_hover_text("The highest position the toy goes to, from its whole length");
    // Lowest and highest push each other rather than crossing.
    if bottom != new.bottom * 100.0 {
        new.bottom = bottom / 100.0;
        new.top = new.top.max(new.bottom);
    } else if top != new.top * 100.0 {
        new.top = top / 100.0;
        new.bottom = new.bottom.min(new.top);
    }
    labeled(ui, "Fastest", |ui| {
        ui.add(egui::Slider::new(&mut new.fastest, StrokeSettings::FASTEST_RANGE).suffix(" s").step_by(0.05).fixed_decimals(2))
    })
    .on_hover_text(
        "Time of the fastest move over the toy's whole length. GameViber never asks for faster: \
         raise it if the toy stops following (shorter strokes, stutters)",
    );
    labeled(ui, "Slowest", |ui| {
        ui.add(egui::Slider::new(&mut new.slowest, StrokeSettings::SLOWEST_RANGE).suffix(" s").step_by(0.5).fixed_decimals(1))
    })
    .on_hover_text("Time of the slowest move over the toy's whole length: lower it if slow strokes jerk");
    labeled(ui, "Turns", |ui| {
        ui.add(egui::Slider::new(&mut new.min_turn, StrokeSettings::TURN_RANGE).suffix(" s").step_by(0.05).fixed_decimals(2))
    })
    .on_hover_text("Shortest time between two changes of direction: raise it if quick turns get lost");
    new.slowest = new.slowest.max(new.fastest);
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Stronger")));
        for style in StrokeStyle::ALL {
            let hint = match style {
                StrokeStyle::Both => "Faster and longer strokes",
                StrokeStyle::Speed => "Faster strokes, always over the whole range",
                StrokeStyle::Depth => "Longer strokes, at a medium speed",
            };
            if ui.selectable_label(new.style == style, style.label()).on_hover_text(hint).clicked() {
                new.style = style;
            }
        }
    })
    .response
    .on_hover_text("What a stronger feeling changes in the strokes");
    (new != settings).then_some(new)
}

/// Requested intensity (x) -> toy intensity (y).
fn curve_preview(ui: &mut egui::Ui, name: &str, settings: &ToySettings) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(72.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, RAISED);
    painter.line_segment([rect.left_bottom(), rect.right_top()], egui::Stroke::new(1.0, LINE));
    let points: Vec<egui::Pos2> = (0..=40)
        .map(|i| {
            let x = i as f64 / 40.0;
            let y = settings.shape(x);
            egui::pos2(rect.left() + rect.width() * x as f32, rect.bottom() - rect.height() * y as f32)
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, ACCENT)));
    response.on_hover_text(format!("How {name} answers: what the mode asks for (left to right) and what the toy plays (up)"));
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
