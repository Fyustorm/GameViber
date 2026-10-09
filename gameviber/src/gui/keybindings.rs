//! Shortcuts page: the gamepad combos GameViber listens to while playing
//! (panic stop, mark a moment, capture the screen), and the same actions on
//! keyboard keys the desktop holds back from the game.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared, RECENT_SECS};
use crate::shortcuts::Status as ShortcutStatus;
use crate::gamepad::{combo_text, parse_combo, BUTTONS, PANIC_COMBO_MIN};

impl App {
    pub(super) fn keybindings_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Shortcuts");
                ui.label(muted(
                    "Keys and button combos GameViber reacts to while you play. A combo is also seen by the game: \
                     keyboard shortcuts and the gamepad's back paddles are not.",
                ));
                ui.add_space(8.0);
                self.keyboard_card(ui, s);
                ui.label(muted(
                    "Back paddles (P1 to P4) and SHARE: kept from the game when a combo uses them. They work if your \
                     gamepad's driver reports them: press one, it lights up in Setup › Gamepad. Many gamepads in Xbox \
                     360 mode only copy their paddles onto other buttons (set in the gamepad's own app).",
                ).size(12.5));
                ui.add_space(8.0);
                let (panic, mark, capture) = (&s.settings.panic_combo, &s.settings.mark_combo, &s.settings.capture_combo);
                let text = format!(
                    "Hold {} on the gamepad for half a second to stop every toy. At least {PANIC_COMBO_MIN} buttons, or a back paddle alone.",
                    combo_text(panic)
                );
                if let Some(combo) = combo_card(ui, "⛔ Panic stop", &text, panic, &[mark, capture]) {
                    self.send(Command::SetPanicCombo(combo));
                }
                let text = format!(
                    "Something felt wrong? Hold {} for a moment: GameViber notes when, and saves the last \
                     {:.0} minutes a few seconds later, to fix the mode from \"Doesn't feel right?\".",
                    combo_text(mark),
                    RECENT_SECS / 60.0
                );
                if let Some(combo) = combo_card(ui, "⚑ Mark a moment", &text, mark, &[panic, capture]) {
                    self.send(Command::SetMarkCombo(combo));
                }
                let target = if s.capture_phase.is_empty() { "to sort later".to_owned() } else { format!("as {}", s.capture_phase) };
                let text = format!(
                    "Hold {} in game to capture its image into the game being played ({target}; change it on its captures \
                     page). The in-game overlay confirms. Captures teach GameViber the game's phases, and indicators are \
                     drawn on them.",
                    combo_text(capture)
                );
                if let Some(combo) = combo_card(ui, "📸 Capture the screen", &text, capture, &[panic, mark]) {
                    self.send(Command::SetCaptureCombo(combo));
                }
            });
        });
    }
}

impl App {
    /// Keyboard shortcuts through the desktop: on or off, their keys.
    fn keyboard_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("⌨ Keyboard shortcuts").strong().size(15.0));
                let mut on = s.settings.keyboard_shortcuts;
                if ui.checkbox(&mut on, "Use them").changed() {
                    self.send(Command::SetKeyboardShortcuts(on));
                }
            });
            ui.label(muted(
                "The desktop keeps these keys for GameViber, so the game never sees them: capture a dialogue without \
                 opening the game's menu over it. The desktop asks you to confirm them the first time.",
            ));
            match &s.shortcuts {
                ShortcutStatus::Off => {}
                ShortcutStatus::Connecting => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(muted("Waiting for the desktop (confirm the keys in its window)..."));
                    });
                }
                ShortcutStatus::Ready(keys) => {
                    egui::Grid::new("keyboard-shortcuts").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                        for (action, keys) in keys {
                            let text = if keys.is_empty() { RichText::new("no key").color(WARN) } else { RichText::new(keys).color(ACCENT_TEXT) };
                            ui.label(text);
                            ui.label(action.description().trim_start_matches("GameViber: "));
                            ui.end_row();
                        }
                    });
                    if crate::shortcuts::CONFIGURABLE
                        && ui.button("Change the keys").on_hover_text("Opens the desktop's shortcut settings").clicked()
                    {
                        self.send(Command::ConfigureShortcuts);
                    }
                }
                ShortcutStatus::Failed(error) => {
                    ui.label(RichText::new(format!("The desktop does not offer global shortcuts: {error}")).color(DANGER_TEXT).size(12.0));
                }
            }
        });
        ui.add_space(8.0);
    }
}

/// Card with a picker of the buttons of a combo, which must differ from the
/// `others`; returns the new combo.
fn combo_card(ui: &mut egui::Ui, title: &str, text: &str, combo: &[String], others: &[&Vec<String>]) -> Option<Vec<String>> {
    let mut picked = None;
    card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).strong().size(15.0));
            pill(ui, &combo_text(combo), ACCENT_TEXT, RAISED);
        });
        ui.label(muted(text));
        ui.add_space(4.0);
        let same = |a: &[String]| parse_combo(a).is_some() && others.iter().any(|o| parse_combo(a) == parse_combo(o));
        ui.horizontal_wrapped(|ui| {
            for button in BUTTONS {
                let on = combo.iter().any(|b| b == button);
                let mut new: Vec<String> = combo.iter().filter(|b| *b != button).cloned().collect();
                if !on {
                    new.push(button.to_owned());
                }
                let (allowed, why) = if parse_combo(&new).is_none() {
                    (false, format!("A combo needs at least {PANIC_COMBO_MIN} buttons, or one back paddle (P1 to P4) or SHARE"))
                } else if same(&new) {
                    (false, "Each combo needs other buttons".to_owned())
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
