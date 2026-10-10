//! The Creator's Phases and Other programs tabs: what GameViber reads from
//! the game for the active mode (the inputs in its package) — naming the
//! phases, how they sound, how many captures show each, the game's sound —
//! and, for experts, the values other programs send.

use std::collections::HashMap;

use eframe::egui::{self, Margin, RichText};

use super::audio::model_card;
use super::theme::*;
use super::creator::Tab;
use super::App;
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::package::{valid_name, ExternalInput, ExternalKind, Inputs, PhaseDef, MAX_PHASES};
use crate::models::Model;

const SURE_SIGN_HELP: &str = "Indicators shown only in this phase (the battle menu, drawn on the captures page): \
    while they are all shown the phase is certain, right away, and the phase is only entered through them. \
    Pick several when one is shared by phases: battle shows the health gauge and its menu, exploration the gauge \
    alone; the sign of the most indicators shown wins.";
const OTHERWISE_HELP: &str = "The phase while no other phase's sign is shown (story: neither a menu nor a gauge on \
    screen), unless the sound or the captures clearly say another phase without a sign. One phase per mode.";
const HOLD_HELP: &str = "How long the phase is kept after its last sign (its indicator gone, or another phase sounding \
    or looking more likely). Longer for signs that come and go, like a battle menu hidden during attacks.";
const IGNORE_HELP: &str = "Guessed events the mode does not get in this phase: the hits of the sound (on_audio_hit, and \
    on_impact from the sound), loud clicks and music in a menu; the flashes of the image (on_impact from the screen).";
const IGNORE_SURE: &str = "Right away: the phase comes from its indicators.";
const IGNORE_LATE: &str = "Without a sure sign, the phase is guessed a few seconds late and can be wrong: a few events \
    still pass when it starts, and some are lost when it is wrongly recognized. Tie it to an indicator to make it exact.";

/// Captures per phase that make its recognition reliable.
const CAPTURES_WANTED: usize = 5;
const SUGGESTED_PHASES: [&str; 5] = ["battle", "exploration", "story", "menu", "boss"];

#[derive(Default)]
pub struct State {
    new_phase: String,
    /// Sound descriptions being typed, by phase.
    sounds: HashMap<String, String>,
    /// Holds being dragged, by phase.
    holds: HashMap<String, f64>,
    new_input: ExternalInput,
}

enum Status {
    Done(String),
    Next(String),
    Optional(String),
}

impl App {
    pub(super) fn phases_tab(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let Some(inputs) = self.inputs_of(ui, s, game) else { return };
        self.phases_why(ui);
        if inputs.phases.is_empty() {
            ui.horizontal(|ui| {
                ui.label(muted("Not sure which ones?"));
                if ui.link("✨ The AI assistant proposes them, with the indicators to draw ›").clicked() {
                    self.creator.tab = Tab::Assistant;
                }
            });
        }
        ui.add_space(10.0);
        self.phases_step(ui, s, game, inputs);
        ui.add_space(10.0);
        self.looks_step(ui, s, inputs);
        ui.add_space(10.0);
        self.sound_step(ui, s, game);
    }

    pub(super) fn programs_tab(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let Some(inputs) = self.inputs_of(ui, s, game) else { return };
        self.experts_step(ui, s, inputs);
    }

    /// The inputs of the active mode, when it is one of `game`'s and has a
    /// package; otherwise says why there are none to set up.
    pub(super) fn inputs_of<'a>(&mut self, ui: &mut egui::Ui, s: &'a Shared, game: &Game) -> Option<&'a Inputs> {
        if game.modes.contains(&super::main_of(&s.mode.id)) {
            if let Some(inputs) = s.mode_inputs.as_ref() {
                return Some(inputs);
            }
        }
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label("Built-in modes read nothing set up for a game: duplicate it to set some up for this game.");
        });
        None
    }

    /// Why phases, how they are told apart: open until hidden.
    fn phases_why(&mut self, ui: &mut egui::Ui) {
        let id = egui::Id::new("phases-why");
        let mut open = ui.data_mut(|d| *d.get_persisted_mut_or(id, true));
        ui.horizontal(|ui| {
            ui.label(muted("Parts of the game that should not feel the same. Told apart by the sound, and better with captures."));
            let label = if open { "Hide the explanation" } else { "Why phases?" };
            if ui.small_button(label).clicked() {
                open = !open;
            }
        });
        ui.data_mut(|d| d.insert_persisted(id, open));
        if !open {
            return;
        }
        ui.add_space(6.0);
        ui.columns(3, |columns| {
            let texts = [
                ("Why", "A fight and a cutscene should not feel the same. In each phase the script keeps its own background: a slow wave through a boss fight, calm in the hub. The rumble and your buttons make the peaks on top."),
                ("How GameViber tells them apart", "The sound: from a few words saying how each phase sounds. The image: from captures, about five per phase. An indicator on screen (Captures & indicators): the surest."),
                ("Good to know", "Phases come a few seconds late and can be wrong: they set the mood, never the timing. Two or three phases that feel very different beat many close ones."),
            ];
            for (column, (title, text)) in columns.iter_mut().zip(texts) {
                card(PANEL).inner_margin(Margin::symmetric(14, 12)).show(column, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.label(RichText::new(title).strong());
                        ui.label(muted(text).size(12.5));
                    });
                });
            }
        });
    }

    fn phases_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let status = if inputs.phases.len() >= 2 {
            Status::Done(format!("Done · {} phases", inputs.phases.len()))
        } else {
            Status::Next("Recommended".to_owned())
        };
        let mut changed: Option<Inputs> = None;
        step_card(ui, "The phases of the game", status, |ui| {
            ui.label(muted(
                "A phase is a part of the game that should not feel the same: a battle, exploring, a dialogue, a \
                 menu. Modes vibrate differently in each one. Name 2 to 4 of them, the ones that feel the most \
                 different.",
            ));
            ui.add_space(4.0);
            if !inputs.phases.is_empty() {
                let live: HashMap<&str, f64> = if s.game.as_ref().is_some_and(|g| g.id == game.id) {
                    s.phases.phases.iter().map(|(n, p)| (n.as_str(), *p)).collect()
                } else {
                    HashMap::new()
                };
                let mut indicator_names: Vec<&str> = inputs.zones.iter().map(|z| z.indicator.as_str()).collect();
                indicator_names.dedup();
                indicator_names.sort();
                indicator_names.dedup();
                egui::Grid::new("game-phases").num_columns(7).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(muted("Phase").size(12.0));
                    ui.label(muted("How it sounds (optional, for the sound model)").size(12.0));
                    ui.label(muted("Sure sign").size(12.0)).on_hover_text(SURE_SIGN_HELP);
                    ui.label(muted("Kept").size(12.0)).on_hover_text(HOLD_HELP);
                    ui.label(muted("Ignored").size(12.0)).on_hover_text(IGNORE_HELP);
                    ui.label(muted("Right now").size(12.0));
                    ui.label("");
                    ui.end_row();
                    for (i, phase) in inputs.phases.iter().enumerate() {
                        let current = s.phases.phase.as_deref() == Some(phase.name.as_str()) && !live.is_empty();
                        let name = RichText::new(&phase.name).strong();
                        ui.label(if current { name.color(ACCENT_TEXT) } else { name });
                        let text = self.inputs.sounds.entry(phase.name.clone()).or_insert_with(|| phase.sound.clone().unwrap_or_default());
                        let edit = ui.add(egui::TextEdit::singleline(text).hint_text("e.g. aggressive battle music with heavy drums").desired_width(220.0));
                        let typed = text.trim().to_owned();
                        if edit.lost_focus() && Some(typed.as_str()) != phase.sound.as_deref().or(Some("")) {
                            let mut g = inputs.clone();
                            g.phases[i].sound = (!typed.is_empty()).then_some(typed);
                            changed = Some(g);
                        }
                        // Indicators shown only in this phase, all together; or none of the others' signs.
                        let known: Vec<String> = phase.indicators.iter().filter(|z| indicator_names.contains(&z.as_str())).cloned().collect();
                        let (mut sign, mut otherwise) = (known.clone(), phase.otherwise);
                        let label = if otherwise {
                            "none of the others".to_owned()
                        } else if !sign.is_empty() {
                            sign.join(" + ")
                        } else if indicator_names.is_empty() {
                            "no indicator yet".to_owned()
                        } else {
                            "none".to_owned()
                        };
                        ui.add_enabled_ui(!indicator_names.is_empty(), |ui| {
                            egui::ComboBox::from_id_salt(("phase-indicator", i))
                                .selected_text(label)
                                .width(140.0)
                                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                .show_ui(ui, |ui| {
                                    for name in &indicator_names {
                                        let mut on = sign.iter().any(|n| n == name);
                                        if ui.checkbox(&mut on, *name).changed() {
                                            if on {
                                                sign.push((*name).to_owned());
                                                otherwise = false;
                                            } else {
                                                sign.retain(|n| n != name);
                                            }
                                        }
                                    }
                                    ui.separator();
                                    if ui.checkbox(&mut otherwise, "None of the others").on_hover_text(OTHERWISE_HELP).changed() && otherwise {
                                        sign.clear();
                                    }
                                })
                                .response
                                .on_hover_text(SURE_SIGN_HELP);
                        });
                        let sure = !sign.is_empty() || otherwise;
                        if sign != known || otherwise != phase.otherwise {
                            let mut g = inputs.clone();
                            g.phases[i].indicators = sign;
                            g.phases[i].otherwise = otherwise;
                            if otherwise {
                                // One phase of no sign.
                                g.phases.iter_mut().enumerate().filter(|(k, _)| *k != i).for_each(|(_, p)| p.otherwise = false);
                            }
                            changed = Some(g);
                        }
                        let hold = self.inputs.holds.entry(phase.name.clone()).or_insert(phase.hold);
                        let drag = ui.add(egui::DragValue::new(hold).range(0.0..=60.0).speed(0.2).max_decimals(1).suffix(" s")).on_hover_text(HOLD_HELP);
                        // Saved once let go (or typed).
                        let done = drag.drag_stopped() || drag.lost_focus() || (drag.changed() && !drag.dragged() && !drag.has_focus());
                        if done && *hold != phase.hold && changed.is_none() {
                            let mut g = inputs.clone();
                            g.phases[i].hold = *hold;
                            changed = Some(g);
                        }
                        if !drag.dragged() && !drag.has_focus() && !done {
                            *hold = phase.hold;
                        }
                        let mut ignore = phase.ignore;
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut ignore.sound_hits, "Sound hits");
                            ui.checkbox(&mut ignore.flashes, "Flashes");
                            if !ignore.is_none() && !sure {
                                ui.label(RichText::new("⚠").color(WARN)).on_hover_text(IGNORE_LATE);
                            }
                        })
                        .response
                        .on_hover_text(format!("{IGNORE_HELP}\n\n{}", if sure { IGNORE_SURE } else { IGNORE_LATE }));
                        if ignore != phase.ignore && changed.is_none() {
                            let mut g = inputs.clone();
                            g.phases[i].ignore = ignore;
                            changed = Some(g);
                        }
                        match live.get(phase.name.as_str()) {
                            Some(p) => {
                                meter(ui, 110.0, *p, if current { ACCENT } else { GAME });
                            }
                            None => {
                                ui.label(muted("-"));
                            }
                        }
                        if ui.small_button("Remove").on_hover_text("Forget this phase (its captures stay, to sort)").clicked() {
                            let mut g = inputs.clone();
                            g.phases.remove(i);
                            changed = Some(g);
                        }
                        ui.end_row();
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                let room = inputs.phases.len() < MAX_PHASES;
                for suggestion in SUGGESTED_PHASES.iter().filter(|n| !inputs.phases.iter().any(|s| s.name == **n)) {
                    if ui.add_enabled(room, egui::Button::new(format!("+ {suggestion}"))).clicked() {
                        let mut g = inputs.clone();
                        g.phases.push(PhaseDef { name: (*suggestion).to_owned(), ..PhaseDef::default() });
                        changed = Some(g);
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut self.inputs.new_phase).hint_text("another phase").desired_width(120.0));
                let name = self.inputs.new_phase.trim().to_owned();
                let ok = room && valid_name(&name) && !inputs.phases.iter().any(|s| s.name == name);
                if ui.add_enabled(ok, egui::Button::new("Add")).clicked() {
                    let mut g = inputs.clone();
                    g.phases.push(PhaseDef { name, ..PhaseDef::default() });
                    changed = Some(g);
                    self.inputs.new_phase.clear();
                }
            });
        });
        if let Some(g) = changed {
            self.send(Command::SaveInputs(g));
        }
    }

    fn looks_step(&mut self, ui: &mut egui::Ui, s: &Shared, inputs: &Inputs) {
        let counts: Vec<(String, usize)> =
            inputs.phases.iter().map(|sc| (sc.name.clone(), inputs.captures.iter().filter(|c| c.phase == sc.name).count())).collect();
        let enough = !counts.is_empty() && counts.iter().all(|(_, n)| *n >= CAPTURES_WANTED);
        let status = match (inputs.phases.is_empty(), enough) {
            (true, _) => Status::Optional("Once phases are named".to_owned()),
            (false, true) => Status::Done("Done".to_owned()),
            (false, false) => Status::Optional("Recommended".to_owned()),
        };
        let mut open = false;
        let mut command = None;
        step_card(ui, "What each phase looks like", status, |ui| {
            ui.label(muted(
                "GameViber recognizes a phase from images of it: capture a few screens of each phase, in different \
                 spots. Where the music is the same in two phases (a dungeon and its battles), the images tell them \
                 apart.",
            ));
            if !counts.is_empty() {
                egui::Grid::new("phase-captures").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (phase, n) in &counts {
                        ui.label(phase);
                        let color = if *n >= CAPTURES_WANTED { OK } else { WARN };
                        meter(ui, 220.0, (*n as f64 / CAPTURES_WANTED as f64).min(1.0), color);
                        ui.label(muted(format!("{n} of {CAPTURES_WANTED}")));
                        ui.end_row();
                    }
                });
            }
            let to_sort = inputs.captures.iter().filter(|c| c.phase.is_empty()).count();
            if to_sort > 0 {
                ui.label(RichText::new(format!("{to_sort} captures to sort")).color(WARN));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "While playing, hold {} when the game shows a phase (pick which one on the captures page).",
                    crate::gamepad::combo_text(&s.settings.capture_combo)
                ));
                open = ui.add(primary("See the captures")).clicked();
            });
            if s.screen.model != crate::models::ModelState::Ready {
                command = model_card(ui, Model::Image, &s.screen.model);
            }
            egui::CollapsingHeader::new("The sound helps too").id_salt("sound-helps").show(ui, |ui| {
                ui.label(muted(
                    "Describe how a phase sounds above and the music is compared with it as well: useful when the \
                     music changes between phases, useless when it does not.",
                ));
                if inputs.phases.iter().any(|sc| sc.sound.is_some()) && s.audio.model != crate::models::ModelState::Ready {
                    if let Some(c) = model_card(ui, Model::Sound, &s.audio.model) {
                        command = Some(c);
                    }
                }
            });
        });
        if let Some(c) = command {
            self.send(c);
        }
        if open {
            self.creator.tab = Tab::Screen;
        }
    }

    fn sound_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let mut chosen = None;
        let status = Status::Done(if game.audio.is_none() { "Automatic".to_owned() } else { "Chosen".to_owned() });
        step_card(ui, "The game's sound", status, |ui| {
            ui.label(muted(
                "Explosions and hits make the toys react (impacts), and the loudness adds to the intensity. By default \
                 GameViber listens to the game alone, found through the in-game overlay. Every mode of the game listens \
                 to the same.",
            ));
            chosen = super::library::game_sound(ui, s, game);
        });
        if let Some(audio) = chosen {
            self.send(Command::SaveGame(Game { audio, ..game.clone() }));
        }
    }

    fn experts_step(&mut self, ui: &mut egui::Ui, s: &Shared, inputs: &Inputs) {
        let mut changed = None;
        card(PANEL).inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Values sent by other programs").strong().size(15.0));
            {
                ui.label(muted(format!(
                    "A game's mod or a script reading its API can send exact values (health, ammo) and events (a kill) \
                     to {} (Setup › Other programs). Declare them here so that AI assistants know them: a request \
                     with them is an advanced one.",
                    s.external.address.as_deref().unwrap_or("GameViber")
                )));
                for (i, input) in inputs.external.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let kind = match input.kind {
                            ExternalKind::Value => format!("input.external.{}", input.name),
                            ExternalKind::Event => format!("event {}", input.name),
                        };
                        ui.label(RichText::new(kind).monospace());
                        ui.label(&input.description);
                        if ui.small_button("Delete").clicked() {
                            let mut g = inputs.clone();
                            g.external.remove(i);
                            changed = Some(g);
                        }
                    });
                }
                let new = &mut self.inputs.new_input;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("input-kind")
                        .selected_text(if new.kind == ExternalKind::Value { "Value" } else { "Event" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut new.kind, ExternalKind::Value, "Value");
                            ui.selectable_value(&mut new.kind, ExternalKind::Event, "Event");
                        });
                    ui.add(egui::TextEdit::singleline(&mut new.name).hint_text("hp").desired_width(100.0));
                    ui.add(egui::TextEdit::singleline(&mut new.description).hint_text("health, 0 to 100").desired_width(220.0));
                    let ok = valid_name(&new.name) && !new.description.trim().is_empty() && !inputs.external.iter().any(|i| i.name == new.name);
                    if ui.add_enabled(ok, egui::Button::new("Declare")).clicked() {
                        let mut g = inputs.clone();
                        g.external.push(std::mem::take(new));
                        changed = Some(g);
                    }
                });
                super::setup::received(ui, s);
            }
        });
        if let Some(g) = changed {
            self.send(Command::SaveInputs(g));
        }
    }
}

/// A part of the tab: title, status, then its content.
fn step_card(ui: &mut egui::Ui, title: &str, status: Status, content: impl FnOnce(&mut egui::Ui)) {
    let next = matches!(status, Status::Next(_));
    card(PANEL).stroke(egui::Stroke::new(1.0, if next { GAME } else { PANEL })).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (badge, badge_fg, badge_bg, text, text_fg) = match &status {
                Status::Done(t) => ("✔", BG, OK, t, OK),
                Status::Next(t) => ("•", BG, GAME, t, GAME),
                Status::Optional(t) => ("○", TEXT, RAISED, t, MUTED),
            };
            pill(ui, badge, badge_fg, badge_bg);
            ui.label(RichText::new(title).strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(text).color(text_fg).size(12.5));
            });
        });
        ui.add_space(4.0);
        content(ui);
    });
}
