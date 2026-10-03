//! Connection page: gamepad, rumble capture and Intiface status, the capture
//! method in plain words, and help when the rumble is not detected.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::{capture_status, gamepad_status, intiface_status, App, RECENT_RUMBLE_SECS};
use crate::config::SourceChoice;
use crate::engine::{Command, Shared, SourceHealth};

pub const INTIFACE_DOWNLOAD: &str = "https://intiface.com/central/";

#[derive(Default)]
pub struct State {
    /// Intiface address being edited (None: show the saved one).
    url: Option<String>,
}

impl App {
    pub(super) fn connection_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Connection");
                ui.add_space(4.0);
                let (pad_color, pad_text) = gamepad_status(s);
                let (rumble_color, rumble_text) = capture_status(s);
                let (toy_color, toy_text) = intiface_status(s);
                let last = match s.last_rumble {
                    Some(t) => format!("Last rumble {:.0} s ago", s.time - t),
                    None => "No rumble received yet".to_owned(),
                };
                let cards = [
                    ("🎮 Gamepad", pad_color, pad_text, if s.buttons_seen { "Buttons received" } else { "Press a button to check" }.to_owned()),
                    ("📳 Rumble capture", rumble_color, rumble_text, last),
                    ("🔌 Intiface Central", toy_color, toy_text, s.settings.url.clone()),
                ];
                tile_grid(ui, cards.len(), 220.0, 96.0, |ui, i, size| {
                    let (title, color, state, detail) = &cards[i];
                    card(PANEL).show(ui, |ui| {
                        ui.set_width(size.x - 32.0);
                        ui.set_min_height(size.y - 32.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(*title).strong());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| dot(ui, *color));
                        });
                        ui.label(state);
                        ui.label(muted(detail).size(12.0));
                    });
                });
                if let SourceHealth::Failed(e) = &s.source_health {
                    card(PANEL).stroke(egui::Stroke::new(1.0, DANGER)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("The rumble cannot be captured").strong().color(DANGER_TEXT));
                        ui.label(RichText::new(e).monospace().size(12.0));
                        ui.label(muted("Check the gamepad is plugged in, or try the other method below."));
                    });
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("How GameViber listens to the rumble").strong().size(15.0));
                    ui.label(muted("Only change this if the rumble is not detected."));
                });
                if let Some(command) = capture_methods(ui, s) {
                    self.send(command);
                }
                card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Rumble not detected?").strong());
                    ui.label(muted(
                        "Start a game and get hit: the Rumble capture card should say \"receiving\". Nothing after a \
                         few hits? Switch to the other method, then restart the game. With the standard method, \
                         start GameViber before the game.",
                    ));
                    let recent = s.last_rumble.is_some_and(|t| s.time - t < RECENT_RUMBLE_SECS);
                    let level = s.history.back().map(|x| x.strong.max(x.weak)).unwrap_or(0.0);
                    ui.horizontal(|ui| {
                        ui.label(if recent { "Game rumble now" } else { "Game rumble" });
                        meter(ui, 240.0, level, GAME);
                    });
                });

                ui.add_space(12.0);
                ui.label(RichText::new("Intiface Central").strong().size(15.0));
                self.intiface_address(ui, s);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Run the setup guide again").clicked() {
                        self.onboarding = Some(0);
                    }
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new("Technical details").show(ui, |ui| {
                    ui.label(RichText::new(&s.source).monospace().size(12.0));
                    if let Some(e) = &s.intiface.error {
                        ui.label(RichText::new(format!("intiface: {e}")).monospace().size(12.0));
                    }
                });
            });
        });
    }

    /// Server address editor (Intiface on another machine or port).
    pub(super) fn intiface_address(&mut self, ui: &mut egui::Ui, s: &Shared) {
        ui.horizontal(|ui| {
            ui.label("Server address");
            let url = self.connection.url.get_or_insert_with(|| s.settings.url.clone());
            ui.add(egui::TextEdit::singleline(url).desired_width(260.0).font(egui::TextStyle::Monospace));
            let changed = *url != s.settings.url;
            if ui.add_enabled(changed, egui::Button::new("Apply")).clicked() {
                let url = url.clone();
                self.send(Command::SetUrl(url));
            }
            if !changed {
                // Follow the saved value until the user edits it.
                self.connection.url = None;
            }
        });
        ui.label(muted("Intiface Central shows it on its main screen. The default is ws://127.0.0.1:12345.").size(12.0));
    }
}

/// The two capture methods as tiles with their pros and cons; returns the
/// command to apply when the user picks one or toggles hiding.
pub(super) fn capture_methods(ui: &mut egui::Ui, s: &Shared) -> Option<Command> {
    let (source, hide) = (s.settings.source, s.settings.hide);
    let mut command = None;
    let methods = [
        (
            SourceChoice::Proxy,
            "Standard",
            "Recommended",
            "GameViber shows games a virtual copy of your gamepad and listens to what they send it.",
            &["Works with nearly every game", "No password needed", "Your gamepad can still vibrate too"][..],
            &["Start GameViber before the game", "Games may see two controllers: hide the real one below (asks for your password)"][..],
        ),
        (
            SourceChoice::Ebpf,
            "Kernel probe",
            "🔒 Password",
            "Games keep using your real gamepad; GameViber quietly listens in the background.",
            &["No second controller in games", "Can be turned on while a game is running", "Helps when a game ignores the virtual gamepad"][..],
            &["Asks for your password once per session", "Needs a recent Linux kernel"][..],
        ),
    ];
    tile_grid(ui, methods.len(), 300.0, 220.0, |ui, i, size| {
        let (choice, name, badge, summary, pros, cons) = methods[i];
        let selected = source == choice;
        let frame = card(if selected { SELECTED_BG } else { PANEL })
            .stroke(egui::Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { ACCENT } else { LINE }));
        // The whole tile picks the method; registered first so the checkbox inside stays clickable.
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let background = ui.interact(rect, ui.id().with(("capture-method", i)), egui::Sense::click());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
        frame.show(&mut child, |ui| {
            ui.set_width(size.x - 32.0);
                            ui.set_min_height(size.y - 32.0);
            let mut picked = false;
            ui.horizontal(|ui| {
                picked = ui.radio(selected, RichText::new(name).strong().size(15.0)).clicked();
                pill(ui, badge, if selected { ACCENT_TEXT } else { MUTED }, RAISED);
            });
            ui.label(summary);
            for pro in pros {
                ui.label(RichText::new(format!("✔ {pro}")).size(12.5).color(OK));
            }
            for con in cons {
                ui.label(RichText::new(format!("! {con}")).size(12.5).color(MUTED));
            }
            if choice == SourceChoice::Proxy && selected {
                let mut new_hide = hide;
                ui.checkbox(&mut new_hide, "Hide the real gamepad from games 🔒")
                    .on_hover_text("Games only see the virtual copy (asks for your password)");
                if new_hide != hide {
                    command = Some(Command::SetSource { source, hide: new_hide });
                }
            }
            if !selected && (picked || background.clicked()) {
                command = Some(Command::SetSource { source: choice, hide });
            }
        });
        background.on_hover_cursor(egui::CursorIcon::PointingHand);
    });
    if source == SourceChoice::None {
        ui.label(muted("Capture is off: only the simulator in Creator drives the mode."));
    }
    command
}
