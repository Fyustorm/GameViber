//! The active mode's Inputs (Games › game › Inputs): what GameViber reads from the
//! game for the active mode (the inputs in its package), as guided steps —
//! what inputs are, naming the phases, showing what each looks like
//! (captures, `screen.rs`), reading exact values on screen (indicators), the
//! game's sound, and for experts the values other programs send — with, on
//! the side, what the mode knows right now.

use std::collections::HashMap;

use eframe::egui::{self, Margin, RichText, Vec2};

use super::audio::model_card;
use super::theme::*;
use super::{App, GameView, Route};
use crate::config::AudioSource;
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::package::{valid_name, ExternalInput, ExternalKind, Inputs, PhaseDef, MAX_PHASES};
use crate::mode::prompt::Depth;
use crate::mode::IndicatorValue;
use crate::models::Model;

const SURE_SIGN_HELP: &str = "An indicator shown only in this phase (the battle menu, drawn on the captures page): \
    while it is shown the phase is certain, right away, and the phase is only entered through it.";
const HOLD_HELP: &str = "How long the phase is kept after its last sign (its indicator gone, or another phase sounding \
    or looking more likely). Longer for signs that come and go, like a battle menu hidden during attacks.";

/// Captures per phase that make its recognition reliable.
const CAPTURES_WANTED: usize = 5;
/// Below this width the side column goes under the steps.
const TWO_COLUMNS_WIDTH: f32 = 900.0;
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
    pub(super) fn inputs_page(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let Some(inputs) = self.inputs_of(ui, s, game) else { return };
        let playing = s.game.as_ref().is_some_and(|g| g.id == game.id);
        let steps = |app: &mut Self, ui: &mut egui::Ui| {
            app.inputs_intro(ui);
            ui.add_space(10.0);
            app.phases_step(ui, s, game, inputs);
            ui.add_space(10.0);
            app.looks_step(ui, s, game, inputs);
            ui.add_space(10.0);
            app.indicators_step(ui, s, game, inputs, playing);
            ui.add_space(10.0);
            app.sound_step(ui, s, game);
            ui.add_space(10.0);
            app.experts_step(ui, s, inputs);
        };
        if ui.available_width() >= TWO_COLUMNS_WIDTH {
            let side = 320.0;
            let gap = 16.0;
            let main = ui.available_width() - side - gap;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let layout = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(Vec2::new(main, 0.0), layout, |ui| {
                    ui.set_width(main);
                    steps(self, ui);
                });
                ui.allocate_ui_with_layout(Vec2::new(side, 0.0), layout, |ui| {
                    ui.set_width(side);
                    self.modes_know(ui, s, game, inputs, playing);
                });
            });
        } else {
            steps(self, ui);
            ui.add_space(10.0);
            self.modes_know(ui, s, game, inputs, playing);
        }
    }

    /// The inputs of the active mode, when it is one of `game`'s and has a
    /// package; otherwise says why there are none to set up.
    pub(super) fn inputs_of<'a>(&mut self, ui: &mut egui::Ui, s: &'a Shared, game: &Game) -> Option<&'a Inputs> {
        let name = s.mode.info.as_ref().map_or_else(|| s.mode.id.clone(), |i| i.name.clone());
        let why = if !game.modes.contains(&super::main_of(&s.mode.id)) {
            "Inputs belong to a mode: open one of this game's modes to set up what it reads."
        } else if s.mode_inputs.is_none() {
            "Built-in modes read no inputs set up for a game: duplicate it to set some up for this game."
        } else {
            ui.label(muted(format!("Inputs of the mode {name}.")));
            return s.mode_inputs.as_ref();
        };
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(why);
            if ui.button("Open its modes").clicked() {
                self.route = Route::Game { id: game.id.clone(), view: GameView::Modes };
            }
        });
        None
    }

    fn inputs_intro(&mut self, ui: &mut egui::Ui) {
        card(SELECTED_BG).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("What are inputs?").strong().size(16.0));
            ui.label(
                "By default a mode only knows that the gamepad rumbles and which buttons you press: it cannot tell a \
                 fight from a dialogue. Inputs tell your modes what is going on in the game, so they can keep a tense \
                 vibration through a battle and calm down during a cutscene.",
            );
            ui.label(muted(
                "Everything here is optional, and belongs to this mode: a new mode made from this game's page starts \
                 with it. Steps 1 and 2 give the most for the least effort.",
            ));
        });
    }

    fn phases_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let status = if inputs.phases.len() >= 2 {
            Status::Done(format!("Done · {} phases", inputs.phases.len()))
        } else {
            Status::Next("Start here".to_owned())
        };
        let mut changed: Option<Inputs> = None;
        step_card(ui, 1, "Name the phases of the game", status, |ui| {
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
                egui::Grid::new("game-phases").num_columns(6).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(muted("Phase").size(12.0));
                    ui.label(muted("How it sounds (optional, for the sound model)").size(12.0));
                    ui.label(muted("Sure sign").size(12.0)).on_hover_text(SURE_SIGN_HELP);
                    ui.label(muted("Kept").size(12.0)).on_hover_text(HOLD_HELP);
                    ui.label(muted("Right now").size(12.0));
                    ui.label("");
                    ui.end_row();
                    for (i, phase) in inputs.phases.iter().enumerate() {
                        let current = s.phases.phase.as_deref() == Some(phase.name.as_str()) && !live.is_empty();
                        let name = RichText::new(&phase.name).strong();
                        ui.label(if current { name.color(ACCENT_TEXT) } else { name });
                        let text = self.inputs.sounds.entry(phase.name.clone()).or_insert_with(|| phase.sound.clone().unwrap_or_default());
                        let edit = ui.add(egui::TextEdit::singleline(text).hint_text("e.g. aggressive battle music with heavy drums").desired_width(260.0));
                        let typed = text.trim().to_owned();
                        if edit.lost_focus() && Some(typed.as_str()) != phase.sound.as_deref().or(Some("")) {
                            let mut g = inputs.clone();
                            g.phases[i].sound = (!typed.is_empty()).then_some(typed);
                            changed = Some(g);
                        }
                        // An indicator shown only in this phase.
                        let mut indicator = phase.indicator.clone().filter(|z| indicator_names.contains(&z.as_str()));
                        let label = indicator.clone().unwrap_or_else(|| if indicator_names.is_empty() { "no indicator yet".to_owned() } else { "none".to_owned() });
                        ui.add_enabled_ui(!indicator_names.is_empty(), |ui| {
                            egui::ComboBox::from_id_salt(("phase-indicator", i)).selected_text(label).width(120.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut indicator, None, "none");
                                for name in &indicator_names {
                                    ui.selectable_value(&mut indicator, Some((*name).to_owned()), *name);
                                }
                            })
                            .response
                            .on_hover_text(SURE_SIGN_HELP);
                        });
                        if indicator != phase.indicator.clone().filter(|z| indicator_names.contains(&z.as_str())) {
                            let mut g = inputs.clone();
                            g.phases[i].indicator = indicator;
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

    fn looks_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let counts: Vec<(String, usize)> =
            inputs.phases.iter().map(|sc| (sc.name.clone(), inputs.captures.iter().filter(|c| c.phase == sc.name).count())).collect();
        let enough = !counts.is_empty() && counts.iter().all(|(_, n)| *n >= CAPTURES_WANTED);
        let status = match (inputs.phases.is_empty(), enough) {
            (true, _) => Status::Optional("After step 1".to_owned()),
            (false, true) => Status::Done("Done".to_owned()),
            (false, false) => Status::Next("Recommended · next step".to_owned()),
        };
        let mut open = false;
        let mut command = None;
        step_card(ui, 2, "Show GameViber what each phase looks like", status, |ui| {
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
                    "Describe how a phase sounds in step 1 and the music is compared with it as well: useful when the \
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
            self.route = Route::Game { id: game.id.clone(), view: GameView::Screen };
        }
    }

    fn indicators_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, playing: bool) {
        let mut names: Vec<&str> = inputs.zones.iter().map(|z| z.indicator.as_str()).collect();
        names.sort();
        names.dedup();
        let status = Status::Optional(if names.is_empty() { "Optional".to_owned() } else { format!("Optional · {} indicators", names.len()) });
        let mut open = false;
        step_card(ui, 3, "Read exact values on screen", status, |ui| {
            ui.label(muted(
                "An indicator is a part of the screen GameViber reads ten times a second: how full the health bar is, \
                 whether the battle menu is shown. Phases are a good guess a few seconds late; indicators are exact and \
                 instant. A mode can then beat faster when health drops below 30%.",
            ));
            ui.horizontal_wrapped(|ui| {
                for name in &names {
                    let value = if playing { s.screen.indicators.iter().find(|(n, _, _)| n == name).and_then(|(_, _, v)| *v) } else { None };
                    let text = match value {
                        Some(IndicatorValue::Visibility(true)) => format!("{name} · shown"),
                        Some(IndicatorValue::Visibility(false)) => format!("{name} · hidden"),
                        Some(IndicatorValue::Gauge(v)) => format!("{name} · {:.0}%", v * 100.0),
                        Some(IndicatorValue::Unknown) => format!("{name} · unknown"),
                        None => (*name).to_owned(),
                    };
                    pill(ui, &text, TEXT, RAISED);
                }
                let label = if inputs.captures.is_empty() { "Capture images first, then draw indicators on them ›" } else { "Draw indicators on the captures ›" };
                open = ui.link(label).clicked();
            });
        });
        if open {
            self.route = Route::Game { id: game.id.clone(), view: GameView::Screen };
        }
    }

    fn sound_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let mut chosen: Option<Option<AudioSource>> = None;
        step_card(ui, 4, "Sound", Status::Done(if game.audio.is_none() { "Automatic".to_owned() } else { "Chosen".to_owned() }), |ui| {
            ui.label(muted(
                "Explosions and hits make the toys react (impacts), and the loudness adds to the intensity. By default \
                 GameViber listens to the game alone, found through the in-game overlay.",
            ));
            let label = |source: &Option<AudioSource>| match source {
                None => "Default (Setup › Sound)".to_owned(),
                Some(AudioSource::Auto) => "The game showing the overlay".to_owned(),
                Some(AudioSource::Everything) => "Everything the computer plays".to_owned(),
                Some(AudioSource::App(app)) => app.clone(),
                Some(AudioSource::Off) => "Off".to_owned(),
            };
            ui.horizontal(|ui| {
                ui.label("Listen to");
                egui::ComboBox::from_id_salt("game-audio").selected_text(label(&game.audio)).width(260.0).show_ui(ui, |ui| {
                    let mut options = vec![None, Some(AudioSource::Auto), Some(AudioSource::Everything), Some(AudioSource::Off)];
                    options.extend(s.audio.status.streams.iter().map(|app| Some(AudioSource::App(app.clone()))));
                    for option in options {
                        if ui.selectable_label(option == game.audio, label(&option)).clicked() {
                            chosen = Some(option);
                        }
                    }
                });
            });
            if s.game.as_ref().is_some_and(|g| g.id == game.id) {
                ui.horizontal(|ui| match (&s.audio.status.target, &s.audio.status.error) {
                    (_, Some(error)) => {
                        dot(ui, DANGER);
                        ui.label(error);
                    }
                    (Some(target), None) => {
                        dot(ui, OK);
                        ui.label(format!("Listening to {target}"));
                    }
                    (None, None) => {
                        dot(ui, WARN);
                        ui.label("Waiting for sound");
                    }
                });
            }
        });
        if let Some(audio) = chosen {
            self.send(Command::SaveGame(Game { audio, ..game.clone() }));
        }
    }

    fn experts_step(&mut self, ui: &mut egui::Ui, s: &Shared, inputs: &Inputs) {
        let mut changed = None;
        card(PANEL).inner_margin(Margin::symmetric(18, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(RichText::new("For experts: values sent by other programs").strong()).id_salt("experts").show(ui, |ui| {
                ui.label(muted(format!(
                    "A game's mod or a script reading its API can send exact values (health, ammo) and events (a kill) \
                     to {} (Setup › Other programs). Declare them here so that AI assistants know them.",
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
            });
        });
        if let Some(g) = changed {
            self.send(Command::SaveInputs(g));
        }
    }

    /// What the modes know right now, in plain words.
    fn modes_know(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, playing: bool) {
        let mut advanced = false;
        card(PANEL).inner_margin(Margin::symmetric(16, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("What the mode knows right now").strong().size(15.0));
            if !playing {
                ui.label(muted(format!("Live while {} is the game being played.", game.name)));
            } else {
                ui.label(muted("Live, while the game runs."));
                ui.add_space(4.0);
                match (&s.phases.phase, s.phases.phases.iter().find(|(n, _)| Some(n) == s.phases.phase.as_ref())) {
                    (Some(phase), Some((_, p))) => known(ui, &capitalized(phase), &format!("phase, {:.0}% sure", p * 100.0), ACCENT_TEXT),
                    _ if !inputs.phases.is_empty() => known(ui, "No phase yet", "it takes a few seconds of the game", MUTED),
                    _ => {}
                }
                for (name, _, value) in &s.screen.indicators {
                    let text = match value {
                        Some(IndicatorValue::Visibility(true)) => format!("{name}: shown"),
                        Some(IndicatorValue::Visibility(false)) => format!("{name}: hidden"),
                        Some(IndicatorValue::Gauge(v)) => format!("{name}: {:.0}%", v * 100.0),
                        Some(IndicatorValue::Unknown) | None => format!("{name}: unknown"),
                    };
                    known(ui, &text, "indicator", TEXT);
                }
                let busy: Vec<f64> = [s.audio.levels.map(|l| l.intensity), s.screen.levels.map(|l| l.action as f64)].into_iter().flatten().collect();
                if !busy.is_empty() {
                    let intensity = busy.iter().sum::<f64>() / busy.len() as f64;
                    known(ui, &format!("Intensity {intensity:.2}"), "sound and motion", TEXT);
                }
            }
            ui.add_space(8.0);
            ui.label(muted("Ask for an Advanced mode to get one written around these inputs."));
            advanced = ui.button("✨ New advanced mode").clicked();
        });
        if advanced {
            self.open_generator_for(game, Depth::Advanced);
        }
    }
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn known(ui: &mut egui::Ui, what: &str, from: &str, color: egui::Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(what).strong().color(color));
        ui.label(muted(from).size(12.0));
    });
}

/// A numbered step: title, status, then its content.
fn step_card(ui: &mut egui::Ui, n: usize, title: &str, status: Status, content: impl FnOnce(&mut egui::Ui)) {
    let next = matches!(status, Status::Next(_));
    card(PANEL).stroke(egui::Stroke::new(1.0, if next { ACCENT } else { PANEL })).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (badge, badge_fg, badge_bg, text, text_fg) = match &status {
                Status::Done(t) => ("✔".to_owned(), BG, OK, t, OK),
                Status::Next(t) => (n.to_string(), ON_ACCENT, ACCENT, t, ACCENT_TEXT),
                Status::Optional(t) => (n.to_string(), TEXT, RAISED, t, MUTED),
            };
            pill(ui, &badge, badge_fg, badge_bg);
            ui.label(RichText::new(format!("{n} · {title}")).strong().size(15.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(text).color(text_fg).size(12.5));
            });
        });
        ui.add_space(4.0);
        content(ui);
    });
}
