//! Keybindings page: the gamepad combos GameViber listens to while playing
//! (panic stop, mark a moment).

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared, RECENT_SECS};
use crate::gamepad::{combo_text, parse_combo, BUTTONS, PANIC_COMBO_MIN};

impl App {
    pub(super) fn keybindings_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Keybindings");
                ui.label(muted(
                    "Button combos GameViber reacts to while you play. Pick buttons the game does not use together.",
                ));
                ui.add_space(8.0);
                let (panic, mark) = (&s.settings.panic_combo, &s.settings.mark_combo);
                let text = format!(
                    "Hold {} on the gamepad for half a second to stop every toy. At least {PANIC_COMBO_MIN} buttons.",
                    combo_text(panic)
                );
                if let Some(combo) = combo_card(ui, "⛔ Panic stop", &text, panic, mark) {
                    self.send(Command::SetPanicCombo(combo));
                }
                let text = format!(
                    "Something felt wrong? Hold {} for a moment: GameViber notes when, and saves the last \
                     {:.0} minutes a few seconds later, to fix the mode from \"Doesn't feel right?\".",
                    combo_text(mark),
                    RECENT_SECS / 60.0
                );
                if let Some(combo) = combo_card(ui, "⚑ Mark a moment", &text, mark, panic) {
                    self.send(Command::SetMarkCombo(combo));
                }
            });
        });
    }
}

/// Card with a picker of the buttons of a combo, which must differ from
/// `other`; returns the new combo.
fn combo_card(ui: &mut egui::Ui, title: &str, text: &str, combo: &[String], other: &[String]) -> Option<Vec<String>> {
    let mut picked = None;
    card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().size(15.0));
            pill(ui, &combo_text(combo), ACCENT_TEXT, RAISED);
        });
        ui.label(muted(text));
        ui.add_space(4.0);
        let same = |a: &[String]| parse_combo(a).is_some() && parse_combo(a) == parse_combo(other);
        ui.horizontal_wrapped(|ui| {
            for button in BUTTONS {
                let on = combo.iter().any(|b| b == button);
                let mut new: Vec<String> = combo.iter().filter(|b| *b != button).cloned().collect();
                if !on {
                    new.push(button.to_owned());
                }
                let (allowed, why) = if new.len() < PANIC_COMBO_MIN {
                    (false, format!("A combo needs at least {PANIC_COMBO_MIN} buttons"))
                } else if same(&new) {
                    (false, "The panic stop and the mark need different combos".to_owned())
                } else {
                    (true, String::new())
                };
                let response = ui.add_enabled(allowed, egui::Button::selectable(on, button)).on_disabled_hover_text(why);
                if response.clicked() {
                    picked = Some(new);
                }
            }
        });
    });
    ui.add_space(8.0);
    picked
}
