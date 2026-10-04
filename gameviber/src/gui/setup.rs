//! Setup page: what does not depend on the game, under tabs — the gamepad,
//! the gamepad combos, the in-game overlay, the default sound and scene
//! model, and the local server other programs send values to.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Gamepad,
    Combos,
    Overlay,
    Sound,
    Programs,
}

impl Tab {
    const ALL: [(Tab, &'static str); 5] = [
        (Tab::Gamepad, "Gamepad"),
        (Tab::Combos, "Gamepad combos"),
        (Tab::Overlay, "In-game overlay"),
        (Tab::Sound, "Sound"),
        (Tab::Programs, "Other programs"),
    ];
}

impl App {
    pub(super) fn setup_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin { left: 24, right: 24, top: 18, bottom: 0 });
        egui::Panel::top("setup-tabs").frame(frame).show(ui, |ui| {
            heading(ui, "Setup");
            ui.label(muted("Set once, for every game."));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (tab, label) in Tab::ALL {
                    if ui.selectable_label(self.setup_tab == tab, RichText::new(label).size(14.0)).clicked() {
                        if tab == Tab::Overlay && self.setup_tab != tab {
                            self.overlay.forget_install_state();
                        }
                        self.setup_tab = tab;
                    }
                }
            });
            ui.add_space(4.0);
        });
        match self.setup_tab {
            Tab::Gamepad => self.gamepad_ui(ui, s),
            Tab::Combos => self.keybindings_ui(ui, s),
            Tab::Overlay => self.overlay_ui(ui, s),
            Tab::Sound => self.audio_ui(ui, s),
            Tab::Programs => self.programs_ui(ui, s),
        }
    }

    /// The local server other programs send values and events to, and what came lately.
    fn programs_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let view = &s.inputs;
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    eyebrow(ui, "Values from other programs");
                    ui.label(muted(
                        "A game's mod, or a script reading a game's API, can send values and events to modes \
                         (input.custom, on_event) as JSON: {\"set\": {\"hp\": 0.4}} or {\"event\": \"kill\"}. Declare \
                         what it sends in the game's Signals, so that AI assistants know it.",
                    ));
                    ui.horizontal(|ui| match (&view.address, &view.error) {
                        (Some(address), _) => {
                            dot(ui, OK);
                            ui.label(format!("WebSocket: {address}"));
                            if view.clients > 0 {
                                pill(ui, &format!("{} connected", view.clients), ON_ACCENT, ACCENT);
                            }
                        }
                        (None, Some(error)) => {
                            dot(ui, DANGER);
                            ui.label(error);
                        }
                        (None, None) => {
                            dot(ui, IDLE);
                            ui.label("WebSocket off");
                        }
                    });
                    if let Some(pipe) = &view.pipe {
                        ui.label(muted(format!("Or one message per line to the pipe {}", pipe.display())));
                    }
                    let mut apply = None;
                    let port = self.screen.port.get_or_insert_with(|| s.settings.inputs_port.to_string());
                    ui.horizontal(|ui| {
                        ui.label("Port");
                        ui.add(egui::TextEdit::singleline(port).desired_width(60.0));
                        let parsed = port.trim().parse::<u16>().ok();
                        if ui.add_enabled(parsed.is_some_and(|p| p != s.settings.inputs_port), egui::Button::new("Apply")).clicked() {
                            apply = parsed;
                        }
                        ui.label(muted("0 turns the WebSocket off"));
                    });
                    if let Some(port) = apply {
                        self.send(Command::SetInputsPort(port));
                    }
                    if let Some(rejected) = &view.rejected {
                        ui.label(RichText::new(format!("Last message refused: {rejected}")).color(DANGER_TEXT).size(12.0));
                    }
                    received(ui, s);
                });
            });
        });
    }
}

/// The values and events received lately.
pub(super) fn received(ui: &mut egui::Ui, s: &Shared) {
    let view = &s.inputs;
    if view.values.is_empty() && view.events.is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::Grid::new("custom-values").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        for (name, value) in &view.values {
            ui.label(RichText::new(format!("input.custom.{name}")).monospace());
            ui.label(value);
            ui.end_row();
        }
        for (t, name) in view.events.iter().rev() {
            ui.label(RichText::new(format!("event {name}")).monospace());
            ui.label(muted(format!("{:.0} s ago", s.time - t)));
            ui.end_row();
        }
    });
}
