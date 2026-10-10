//! Gamepad page: the gamepad and rumble capture status, the inputs received
//! right now, its buttons (set up step by step when its driver does not give
//! the Xbox layout), the capture method in plain words, and help when the
//! rumble is not detected.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::{capture_status, gamepad_inputs, gamepad_status, App, RECENT_RUMBLE_SECS};
use crate::config::SourceChoice;
use crate::engine::{Command, Shared, SourceHealth};
use super::pad_setup::{Labels, PadSetup};
use crate::gamepad::mapping::Origin;
use crate::source::{Method, PadInfo, PadLayout};

impl App {
    pub(super) fn gamepad_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Gamepad");
                ui.label(muted("GameViber listens to the vibration your games send to the gamepad."));
                ui.add_space(8.0);
                let (pad_color, pad_text) = gamepad_status(s);
                let (rumble_color, rumble_text) = capture_status(s);
                let last = match s.last_rumble {
                    Some(t) => format!("Last rumble {:.0} s ago", s.time - t),
                    None => "No rumble received yet".to_owned(),
                };
                let cards = [
                    ("🎮 Gamepad", pad_color, pad_text, if s.buttons_seen { "Buttons received" } else { "Press a button to check" }.to_owned()),
                    ("📳 Rumble capture", rumble_color, rumble_text, last),
                ];
                tile_grid(ui, cards.len(), 260.0, 96.0, |ui, i, size| {
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
                card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    eyebrow(ui, "Received right now");
                    ui.horizontal_wrapped(|ui| gamepad_inputs(ui, s));
                    rumble_check(ui, s);
                });
                if let Some(hint) = &s.source_hint {
                    card(PANEL).stroke(egui::Stroke::new(1.0, WARN)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(hint).color(WARN));
                    });
                }
                if let Some(pad) = &s.pad {
                    card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.buttons_card(ui, pad);
                    });
                    self.pad_setup_ui(ui.ctx(), pad);
                } else {
                    self.pad_setup = None;
                }
                if let SourceHealth::Failed(e) = &s.source_health {
                    card(PANEL).stroke(egui::Stroke::new(1.0, DANGER)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("The rumble cannot be captured").strong().color(DANGER_TEXT));
                        ui.label(RichText::new(e).monospace().size(12.0));
                        let other = if crate::source::METHODS.len() > 1 { " Still nothing? Try the other method below." } else { "" };
                        ui.label(muted(if s.settings.source == SourceChoice::Proxy {
                            format!(
                                "Toys are stopped until it comes back: GameViber reconnects as soon as the gamepad is \
                                 plugged in or wakes up.{other}"
                            )
                        } else {
                            "Check the gamepad is plugged in, or try the other method below.".to_owned()
                        }));
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
                    ui.label(muted(if crate::source::METHODS.len() > 1 {
                        "Start a game and get hit: the Rumble capture card should say \"receiving\". Nothing after a \
                         few hits? Switch to the other method, then restart the game. With the standard method, \
                         start GameViber before the game."
                    } else {
                        "Start a game and get hit: the Rumble capture card should say \"receiving\". Nothing after a \
                         few hits? Start GameViber before the game, and check the game uses the virtual controller \
                         (hiding the real one helps)."
                    }));
                });
                ui.add_space(8.0);
                egui::CollapsingHeader::new("Technical details").show(ui, |ui| {
                    ui.label(RichText::new(&s.source).monospace().size(12.0));
                });
            });
        });
    }
}

impl App {
    /// The gamepad's layout, and its setup when one runs.
    fn buttons_card(&mut self, ui: &mut egui::Ui, pad: &PadInfo) {
        if self.pad_setup.as_ref().is_some_and(|setup| setup.guid != pad.guid) {
            self.pad_setup = None;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("🕹 Buttons").strong());
            let (text, color) = match pad.layout {
                PadLayout::Driver => ("Standard layout", OK),
                PadLayout::Mapped(Origin::User) => ("Set up by you", OK),
                PadLayout::Mapped(Origin::Community) => ("Known gamepad", OK),
                PadLayout::Missing => ("To set up", WARN),
            };
            pill(ui, text, color, RAISED);
        });
        ui.label(match pad.layout {
            PadLayout::Driver => format!("{}: its driver tells which button is which; games get a copy of it.", pad.name),
            PadLayout::Mapped(Origin::User) => {
                format!("{}: games get it as an Xbox 360 controller, with the buttons you set up.", pad.name)
            }
            PadLayout::Mapped(Origin::Community) => format!(
                "{}: its buttons come from SDL's community database; games get it as an Xbox 360 controller.",
                pad.name
            ),
            PadLayout::Missing => format!(
                "{} does not tell which button is which (a DInput mode, for instance). Show GameViber once, \
                 a button at a time: games then get it as an Xbox 360 controller.",
                pad.name
            ),
        });
        if !pad.rumble {
            ui.label(muted("It cannot vibrate itself: the game's rumble only goes to your toys."));
        }
        ui.horizontal(|ui| {
            let start = match pad.layout {
                PadLayout::Missing => ui.add(primary("Set up its buttons")).clicked(),
                PadLayout::Driver => ui.button("Buttons wrong in games? Set them up").clicked(),
                PadLayout::Mapped(_) => ui.button("Set up again").clicked(),
            };
            if start {
                self.pad_setup = Some(PadSetup::new(pad, Labels::default()));
            }
            if pad.layout == PadLayout::Mapped(Origin::User)
                && ui.button("Forget my setup").on_hover_text("Back to the gamepad's own layout").clicked()
            {
                self.send(Command::ForgetMapping(pad.guid.clone()));
            }
        });
    }
}

/// Level of the game's rumble right now, with a hint until some arrived.
pub(super) fn rumble_check(ui: &mut egui::Ui, s: &Shared) {
    let level = s.history.back().map(|x| x.strong.max(x.weak)).unwrap_or(0.0);
    let recent = s.last_rumble.is_some_and(|t| s.time - t < RECENT_RUMBLE_SECS);
    ui.horizontal(|ui| {
        ui.label(muted(if recent { "Game rumble now" } else { "Game rumble" }));
        meter(ui, 220.0, level, GAME);
        match s.last_rumble {
            Some(_) => pill(ui, "✔ Rumble received", OK, RAISED),
            None => {
                ui.label(muted("start a game and get hit: the bar moves"));
            }
        }
    });
}

/// The two capture methods as tiles with their pros and cons; returns the
/// command to apply when the user picks one or toggles hiding.
pub(super) fn capture_methods(ui: &mut egui::Ui, s: &Shared) -> Option<Command> {
    let (source, hide) = (s.settings.source, s.settings.hide);
    let mut command = None;
    let methods = crate::source::METHODS;
    tile_grid(ui, methods.len(), 300.0, 240.0, |ui, i, size| {
        let Method { choice, name, badge, summary, pros, cons } = methods[i];
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
                ui.checkbox(&mut new_hide, crate::source::HIDE.label).on_hover_text(crate::source::HIDE.hover);
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
        ui.label(muted("Capture is off: only replayed sessions drive the mode."));
    }
    command
}
