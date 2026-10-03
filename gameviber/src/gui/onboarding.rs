//! First-launch setup: Intiface Central, toys, gamepad and capture method,
//! then a first mode. Every step can be skipped; it can be run again from
//! the Connection page.

use eframe::egui::{self, Margin, RichText, Vec2};

use super::connection::{capture_methods, INTIFACE_DOWNLOAD};
use super::play::mode_tiles;
use super::theme::*;
use super::toys::waiting_for_toys;
use super::{App, Page};
use crate::engine::{Command, Shared, SourceHealth, TEST_LEVEL};

const STEPS: [&str; 4] = ["Intiface Central", "Your toys", "Your gamepad", "Pick a mode"];

impl App {
    pub(super) fn onboarding_ui(&mut self, ui: &mut egui::Ui, s: &Shared, step: usize) {
        let frame = egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(20, 28));
        egui::Panel::left("setup-steps").frame(frame).exact_size(260.0).resizable(false).show(ui, |ui| {
            ui.label(RichText::new("〰 GameViber").size(17.0).strong().color(ACCENT));
            ui.add_space(24.0);
            eyebrow(ui, "Setup");
            ui.add_space(4.0);
            for (i, name) in STEPS.iter().enumerate() {
                let (mark, color) = match i.cmp(&step) {
                    std::cmp::Ordering::Less => ("✔".to_owned(), OK),
                    std::cmp::Ordering::Equal => ((i + 1).to_string(), ACCENT),
                    std::cmp::Ordering::Greater => ((i + 1).to_string(), MUTED),
                };
                let fill = if i == step { RAISED } else { SIDEBAR };
                egui::Frame::new().fill(fill).corner_radius(10).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(mark).strong().color(color));
                        ui.label(RichText::new(*name).color(if i == step { TEXT } else { MUTED }));
                    });
                });
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.label(muted("GameViber turns the vibration games send to your gamepad into sensations on your toys."));
            });
        });

        let frame = egui::Frame::new().fill(BG).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(40, 16));
        egui::Panel::bottom("setup-footer").frame(frame).exact_size(68.0).resizable(false).show(ui, |ui| {
            ui.horizontal_centered(|ui| self.setup_footer(ui, s, step));
        });

        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(40, 32));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                eyebrow(ui, &format!("Step {} of {}", step + 1, STEPS.len()));
                match step {
                    0 => self.setup_intiface(ui, s),
                    1 => self.setup_toys(ui, s),
                    2 => self.setup_gamepad(ui, s),
                    _ => self.setup_mode(ui, s),
                }
            });
        });
    }

    fn setup_footer(&mut self, ui: &mut egui::Ui, s: &Shared, step: usize) {
        if step == 0 {
            if ui.button("Skip setup").on_hover_text("You can run it again from Connection").clicked() {
                self.finish_setup();
            }
        } else if ui.button("Back").clicked() {
            self.onboarding = Some(step - 1);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let last = step + 1 == STEPS.len();
            let (label, ready) = match step {
                0 => ("Next", s.intiface.connected),
                1 => ("Next", !s.intiface.toys.is_empty()),
                2 => ("Next: pick a mode", true),
                _ => ("Start playing", true),
            };
            if ui.add_enabled(ready, primary(label).min_size(Vec2::new(140.0, 38.0))).clicked() {
                if last {
                    self.finish_setup();
                } else {
                    self.onboarding = Some(step + 1);
                }
            }
            let skip = match step {
                0 if !ready => Some("Continue without Intiface"),
                1 if !ready => Some("Continue without toys"),
                _ => None,
            };
            if let Some(skip) = skip {
                if ui.button(skip).clicked() {
                    self.onboarding = Some(step + 1);
                }
            }
        });
    }

    fn finish_setup(&mut self) {
        self.onboarding = None;
        self.page = Page::Play;
        self.send(Command::SetOnboarded(true));
    }

    fn setup_intiface(&mut self, ui: &mut egui::Ui, s: &Shared) {
        title(ui, "GameViber needs Intiface Central");
        ui.label(muted(
            "Intiface Central is a free app that connects to hundreds of toys over Bluetooth or USB. GameViber does \
             not talk to toys itself: it sends its vibrations to Intiface Central, which passes them on.",
        ));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let boxes = [
                ("🎮", "Your game", "sends rumble to the gamepad"),
                ("〰", "GameViber", "turns it into a feeling"),
                ("🔌", "Intiface Central", "talks to your toys"),
                ("📳", "Your toy", "Bluetooth or USB"),
            ];
            for (i, (icon, name, sub)) in boxes.iter().enumerate() {
                let highlight = i == 2;
                card(RAISED)
                    .stroke(egui::Stroke::new(if highlight { 2.0 } else { 1.0 }, if highlight { ACCENT } else { LINE }))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(118.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(*icon).size(18.0).color(ACCENT));
                            ui.label(RichText::new(*name).strong());
                            ui.label(muted(*sub).size(11.5));
                        });
                    });
                if i + 1 < boxes.len() {
                    ui.label(muted("›").size(20.0));
                }
            }
        });
        ui.add_space(16.0);
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if s.intiface.connected {
                    dot(ui, OK);
                    ui.label(RichText::new("Connected to Intiface Central").strong());
                } else {
                    ui.spinner();
                    ui.label(RichText::new("Looking for Intiface Central...").strong());
                    ui.label(muted("not running yet, checking again every few seconds"));
                }
            });
            if !s.intiface.connected {
                ui.add_space(8.0);
                ui.columns(2, |columns| {
                    card(RAISED).show(&mut columns[0], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("1. Install it").strong());
                        ui.label(muted("Available for Linux as an AppImage or a Flatpak."));
                        ui.hyperlink_to("Get Intiface Central ↗", INTIFACE_DOWNLOAD);
                    });
                    card(RAISED).show(&mut columns[1], |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("2. Open it and press Start").strong());
                        ui.label(muted("Leave it running while you play. GameViber connects on its own."));
                    });
                });
            }
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Intiface runs on another computer or port?").show(ui, |ui| {
                self.intiface_address(ui, s);
            });
        });
    }

    fn setup_toys(&mut self, ui: &mut egui::Ui, s: &Shared) {
        title(ui, "Connect your toys");
        ui.label(muted(
            "Turn your toy on, then press Start Scanning in Intiface Central's Devices tab. Toys show up here as \
             soon as Intiface finds them.",
        ));
        ui.add_space(12.0);
        if !s.intiface.connected {
            ui.horizontal(|ui| {
                dot(ui, WARN);
                ui.label("Intiface Central is not connected: go back to the previous step to set it up.");
            });
            return;
        }
        for toy in &s.intiface.toys {
            card(PANEL).inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("📳").size(18.0).color(ACCENT));
                    ui.label(RichText::new(&toy.name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚡ Buzz to test").clicked() {
                            self.send(Command::TestToy(toy.name.clone(), TEST_LEVEL));
                        }
                        pill(ui, "Ready", OK, RAISED);
                    });
                });
            });
        }
        waiting_for_toys(ui);
    }

    fn setup_gamepad(&mut self, ui: &mut egui::Ui, s: &Shared) {
        title(ui, "Plug in your gamepad");
        ui.label(muted(
            "GameViber listens to the vibration your games send to the gamepad. Pick how it listens: if you are \
             not sure, keep the recommended one.",
        ));
        ui.add_space(12.0);
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("🎮").size(18.0).color(ACCENT));
                match (&s.source_health, s.gamepads.first()) {
                    (SourceHealth::Waiting(why), _) => {
                        ui.spinner();
                        ui.label(format!("Starting: {why}"));
                    }
                    (SourceHealth::Failed(e), _) => {
                        dot(ui, DANGER);
                        ui.label(RichText::new("No gamepad found").strong());
                        ui.label(muted(e).size(12.0));
                    }
                    (_, Some(name)) => {
                        ui.label(RichText::new(name).strong());
                        if s.buttons_seen {
                            pill(ui, "✔ Buttons work", OK, RAISED);
                        } else {
                            ui.label(muted("press a button to check"));
                        }
                    }
                    (_, None) => {
                        ui.spinner();
                        ui.label("Looking for a gamepad...");
                    }
                }
            });
            let level = s.history.back().map(|x| x.strong.max(x.weak)).unwrap_or(0.0);
            ui.horizontal(|ui| {
                ui.label(muted("Rumble check"));
                meter(ui, 220.0, level, GAME);
                match s.last_rumble {
                    Some(_) => pill(ui, "✔ Rumble received", OK, RAISED),
                    None => {
                        ui.label(muted("start a game and get hit: the bar moves"));
                    }
                }
            });
        });
        ui.add_space(8.0);
        if let Some(command) = capture_methods(ui, s) {
            self.send(command);
        }
        ui.label(muted("You can switch any time in Connection."));
    }

    fn setup_mode(&mut self, ui: &mut egui::Ui, s: &Shared) {
        title(ui, "What are you playing?");
        ui.label(muted("Each mode turns the game's rumble into a different feeling. You can change it any time."));
        ui.add_space(12.0);
        if let Some(id) = mode_tiles(ui, s, false) {
            self.send(Command::SelectMode(id));
        }
        ui.add_space(12.0);
        ui.label(muted("These modes suit a whole genre. Later, from the Play page, an AI assistant can make one \
                        tailored to your game."));
    }
}

fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(26.0).strong().color(TEXT));
}
