//! Live page: what is going on while playing, to keep on a second screen —
//! the mode (to switch it, its preset and main settings, what it tracks),
//! what goes to the toys, what the mode's inputs say right now, and the
//! gamepad with its combos.

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{gamepad_inputs, App, Page, Route};
use crate::engine::{Command, Shared, HISTORY_SECS};
use crate::gamepad::combo_text;
use crate::mode::{ParamDef, IndicatorValue};
use crate::shortcuts::{Action, Status as ShortcutStatus};

/// From this width the page shows three columns, else two.
const THREE_COLUMNS_WIDTH: f32 = 1100.0;
/// Width of the names before meters and curves.
const LABEL_WIDTH: f32 = 110.0;

impl App {
    pub(super) fn live_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal(|ui| {
                    heading(ui, "Live");
                    let game = s.game.as_ref().map_or("No game".to_owned(), |g| g.name.clone());
                    ui.label(muted(format!("· {game}")).size(16.0));
                    // Straight to the mode's pages.
                    if !s.mode.id.is_empty() {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("In the Creator ›").clicked() {
                                self.page = Page::Creator;
                            }
                            if ui.button("Its page ›").clicked() {
                                self.page = Page::Library;
                                self.route = Route::Mode;
                            }
                        });
                    }
                });
                ui.label(muted("What happens while you play: keep it on a second screen."));
                ui.add_space(8.0);
                let three = ui.available_width() >= THREE_COLUMNS_WIDTH;
                ui.columns(if three { 3 } else { 2 }, |columns| {
                    self.mode_card(&mut columns[0], s);
                    if three {
                        output_card(&mut columns[1], self, s);
                        inputs_card(&mut columns[2], s);
                    } else {
                        output_card(&mut columns[1], self, s);
                        columns[1].add_space(12.0);
                        inputs_card(&mut columns[1], s);
                    }
                });
                ui.add_space(12.0);
                gamepad_card(ui, s);
            });
        });
    }

    /// The active mode: switch it, its preset, its main settings, what it tracks.
    fn mode_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mode = &s.mode;
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Mode");
            // The game's modes first, then the built-in ones.
            let name_of = |id: &str| match s.catalog.get(id) {
                Some(Ok(info)) => info.name.clone(),
                _ => s.modes.iter().find(|e| e.id == id).map_or(id.to_owned(), |e| e.key.clone()),
            };
            let game_modes: Vec<&String> = s.game.iter().flat_map(|g| &g.modes).collect();
            let current = if mode.id.is_empty() { "No mode".to_owned() } else { name_of(&mode.id) };
            egui::ComboBox::from_id_salt("live-mode")
                .selected_text(RichText::new(current).strong().size(16.0))
                .width(ui.available_width() - 8.0)
                .show_ui(ui, |ui| {
                    if let Some(game) = &s.game {
                        ui.label(muted(format!("{}'s modes", game.name)).size(12.0));
                        if game_modes.is_empty() {
                            ui.label(muted("none yet").size(12.0));
                        }
                    }
                    for id in &game_modes {
                        if ui.selectable_label(**id == mode.id, name_of(id)).clicked() {
                            self.send(Command::SelectMode((*id).clone()));
                        }
                    }
                    ui.separator();
                    ui.label(muted("Built-in modes").size(12.0));
                    for entry in s.modes.iter().filter(|e| e.builtin) {
                        if ui.selectable_label(entry.id == mode.id, name_of(&entry.id)).clicked() {
                            self.send(Command::SelectMode(entry.id.clone()));
                        }
                    }
                });
            if let Some(error) = &mode.error {
                ui.label(RichText::new(error).color(DANGER_TEXT).monospace().size(12.0));
                if mode.suspended && ui.button("Resume").clicked() {
                    self.send(Command::Resume);
                }
            }
            let Some(info) = &mode.info else { return };
            ui.add_space(4.0);
            self.preset_picker(ui, info, mode);
            let main: Vec<&ParamDef> = if info.main_params.is_empty() {
                info.params.iter().take(3).collect()
            } else {
                info.params.iter().filter(|d| info.main_params.contains(&d.name)).collect()
            };
            for def in main {
                self.param(ui, def, mode);
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("All settings ›").clicked() {
                    self.page = Page::Library;
                    self.route = Route::Mode;
                }
                if ui.add(primary("Doesn't feel right?")).on_hover_text("Get the mode fixed by an AI assistant").clicked() {
                    self.page = Page::Library;
                    self.route = Route::Mode;
                    self.open_feedback(s);
                }
            });
            // The mode's plot() values, each scaled to its own range.
            if !s.plots.is_empty() {
                ui.add_space(6.0);
                eyebrow(ui, "What the mode tracks");
                for (name, points) in &s.plots {
                    let top = points.iter().map(|p| p[1].abs()).fold(0.0, f64::max).max(1e-9);
                    let line: Vec<(f64, f64)> = points.iter().map(|p| (p[0] - s.time, p[1] / top)).collect();
                    let last = points.back().map_or(0.0, |p| p[1]);
                    row(ui, name, |ui| {
                        let width = (ui.available_width() - 52.0).max(40.0);
                        sparkline(ui, Vec2::new(width, 22.0), HISTORY_SECS, &line, GAME);
                        ui.label(RichText::new(format!("{last:.2}")).monospace().size(12.0));
                    });
                }
            }
        });
    }
}

/// What goes to the toys: now, over the last seconds, per toy, and the ceiling.
fn output_card(ui: &mut egui::Ui, app: &mut App, s: &Shared) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Output");
        let now = s.history.back().map_or(0.0, |x| x.channels.values().copied().fold(0.0, f64::max));
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{:.0}%", now * 100.0)).size(34.0).strong().color(if s.panic { DANGER_TEXT } else { ACCENT_TEXT }));
            ui.vertical(|ui| {
                if s.panic {
                    ui.label(RichText::new("Stopped (panic)").color(DANGER_TEXT));
                    if ui.button("Start again").clicked() {
                        app.send(Command::Rearm);
                    }
                } else {
                    ui.label(muted("now, after the safety ceiling"));
                }
            });
        });
        let game: Vec<(f64, f64)> = s.history.iter().map(|x| (x.t - s.time, x.strong.max(x.weak))).collect();
        let output: Vec<(f64, f64)> = s.history.iter().map(|x| (x.t - s.time, x.channels.values().copied().fold(0.0, f64::max))).collect();
        let width = ui.available_width();
        ui.label(RichText::new("Game rumble").size(12.0).color(GAME));
        sparkline(ui, Vec2::new(width, 44.0), HISTORY_SECS, &game, GAME);
        ui.label(RichText::new("Sent to the toys").size(12.0).color(ACCENT_TEXT));
        sparkline(ui, Vec2::new(width, 64.0), HISTORY_SECS, &output, ACCENT);
        ui.label(muted(format!("last {HISTORY_SECS:.0} s")).size(11.0));
        ui.add_space(4.0);
        if s.toy_levels.is_empty() {
            ui.label(muted("No toy connected."));
        }
        for (toy, level) in &s.toy_levels {
            row(ui, toy, |ui| {
                meter(ui, (ui.available_width() - 50.0).max(40.0), *level, ACCENT);
                ui.label(RichText::new(format!("{:.0}%", level * 100.0)).size(12.0));
            });
            if let Some(position) = s.toy_positions.get(toy) {
                row(ui, "", |ui| position_meter(ui, (ui.available_width() - 50.0).max(40.0), *position, super::toys::range(s, toy)));
            }
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Max");
            let mut cap = s.settings.global_cap * 100.0;
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(60.0);
            if ui.add(egui::Slider::new(&mut cap, 0.0..=100.0).suffix("%").integer()).on_hover_text("No toy ever goes above this intensity").changed() {
                app.send(Command::SetCap(cap / 100.0));
            }
        });
        if let Some(seconds) = s.recording {
            ui.horizontal(|ui| {
                dot(ui, DANGER);
                ui.label(format!("Recording · {seconds:.0} s"));
            });
        }
    });
}

/// What the mode's inputs say right now.
fn inputs_card(ui: &mut egui::Ui, s: &Shared) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Inputs");
        // The phase, and how likely each is.
        match &s.phases.phase {
            Some(phase) => ui.label(RichText::new(phase).size(22.0).strong().color(ACCENT_TEXT)),
            None if s.phases.phases.is_empty() => ui.label(muted("No phases: name them in the Creator.")),
            None => ui.label(RichText::new("No phase yet").size(18.0).color(MUTED)),
        };
        for (name, p) in &s.phases.phases {
            let current = s.phases.phase.as_ref() == Some(name);
            row(ui, name, |ui| {
                meter(ui, (ui.available_width() - 44.0).max(40.0), *p, if current { ACCENT } else { GAME });
                ui.label(RichText::new(format!("{:.0}%", p * 100.0)).size(12.0));
            });
        }
        // Indicators on screen.
        if !s.screen.indicators.is_empty() {
            ui.add_space(6.0);
            eyebrow(ui, "Indicators");
            if s.screen.frame.is_none() {
                ui.label(muted("Read on the game's image, through the in-game overlay.").size(12.0));
            }
            for (name, _, value) in &s.screen.indicators {
                row(ui, name, |ui| match value {
                    Some(IndicatorValue::Gauge(x)) => {
                        meter(ui, (ui.available_width() - 44.0).max(40.0), *x, OK);
                        ui.label(RichText::new(format!("{:.0}%", x * 100.0)).size(12.0));
                    }
                    Some(IndicatorValue::Visibility(true)) => {
                        ui.label(RichText::new("shown").color(ACCENT_TEXT));
                    }
                    Some(IndicatorValue::Visibility(false)) => {
                        ui.label(muted("hidden"));
                    }
                    Some(IndicatorValue::Unknown) => {
                        ui.label(RichText::new("unknown").color(WARN));
                    }
                    None => {
                        ui.label(muted("-"));
                    }
                });
            }
        }
        // The game's sound.
        ui.add_space(6.0);
        eyebrow(ui, "Sound");
        match &s.audio.levels {
            Some(levels) => {
                row(ui, "level", |ui| meter(ui, ui.available_width().max(40.0), levels.level, GAME));
                row(ui, "action", |ui| meter(ui, ui.available_width().max(40.0), levels.intensity, GAME))
                    .on_hover_text("Loudness and density of hits over the last seconds");
                if let Some((t, hit)) = &s.audio.last_hit {
                    ui.label(muted(format!("last hit {:.0} s ago ({:.0}%)", s.time - t, hit.strength * 100.0)).size(12.0));
                }
            }
            None => {
                ui.label(muted("Not listening.").size(12.0));
            }
        }
        // Values from other programs.
        if !s.external.values.is_empty() || !s.external.events.is_empty() {
            ui.add_space(6.0);
            eyebrow(ui, "From other programs");
            super::setup::received(ui, s);
        }
    });
}

/// A label of fixed width, then `content` on the same line.
pub(super) fn row<R>(ui: &mut egui::Ui, label: &str, content: impl FnOnce(&mut egui::Ui) -> R) -> egui::Response {
    ui.horizontal(|ui| {
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(Vec2::new(LABEL_WIDTH, 18.0), layout, |ui| {
            ui.set_width(LABEL_WIDTH);
            ui.add(egui::Label::new(RichText::new(label).size(12.5)).truncate()).on_hover_text(label);
        });
        content(ui);
    })
    .response
}

/// The gamepad right now, and its combos.
fn gamepad_card(ui: &mut egui::Ui, s: &Shared) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Gamepad");
        ui.horizontal_wrapped(|ui| gamepad_inputs(ui, s));
        ui.add_space(6.0);
        let target = if s.capture_phase.is_empty() { "to sort later".to_owned() } else { format!("into {}", s.capture_phase) };
        let capture = format!("Capture the screen ({target})");
        let combos = [
            ("⛔", "Stop every toy", combo_text(&s.settings.panic_combo), Action::Panic),
            ("⚑", "Mark a moment that felt wrong", combo_text(&s.settings.mark_combo), Action::Mark),
            ("📸", capture.as_str(), combo_text(&s.settings.capture_combo), Action::Capture),
        ];
        // The same actions on the keyboard, when the desktop gave keys.
        let keys = |action: Action| match &s.shortcuts {
            ShortcutStatus::Ready(keys) => keys.iter().find(|(a, k)| *a == action && !k.is_empty()).map(|(_, k)| k.clone()),
            _ => None,
        };
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(18.0, 6.0);
            for (icon, what, combo, action) in combos {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.label(RichText::new(icon).size(15.0));
                    pill(ui, &combo, TEXT, RAISED);
                    if let Some(keys) = keys(action) {
                        pill(ui, &format!("⌨ {keys}"), TEXT, RAISED);
                    }
                    ui.label(muted(what));
                });
            }
        });
    });
}
