//! A game's Signals (Games › game › Signals): what GameViber reads from the
//! game for the active mode (the inputs in its package), as guided steps —
//! what signals are, naming the scenes, showing what each looks like
//! (captures, `screen.rs`), reading exact values on screen (zones), the
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
use crate::package::{valid_name, InputDecl, InputKind, Inputs, SceneDef, MAX_SCENES};
use crate::mode::prompt::Depth;
use crate::mode::ZoneValue;
use crate::models::Model;

const SURE_SIGN_HELP: &str = "A zone shown only in this scene (the battle menu, drawn on the captures page): \
    while it is shown the scene is certain, right away, and the scene is only entered through it.";
const HOLD_HELP: &str = "How long the scene is kept after its last sign (its zone gone, or another scene sounding \
    or looking more likely). Longer for signs that come and go, like a battle menu hidden during attacks.";

/// Captures per scene that make its recognition reliable.
const CAPTURES_WANTED: usize = 5;
/// Below this width the side column goes under the steps.
const TWO_COLUMNS_WIDTH: f32 = 900.0;
const SUGGESTED_SCENES: [&str; 5] = ["battle", "exploration", "story", "menu", "boss"];

#[derive(Default)]
pub struct State {
    new_scene: String,
    /// Sound descriptions being typed, by scene.
    sounds: HashMap<String, String>,
    /// Holds being dragged, by scene.
    holds: HashMap<String, f64>,
    new_input: InputDecl,
}

enum Status {
    Done(String),
    Next(String),
    Optional(String),
}

impl App {
    pub(super) fn signals_page(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let Some(inputs) = self.inputs_of(ui, s, game) else { return };
        let playing = s.game.as_ref().is_some_and(|g| g.id == game.id);
        let steps = |app: &mut Self, ui: &mut egui::Ui| {
            app.signals_intro(ui);
            ui.add_space(10.0);
            app.scenes_step(ui, s, game, inputs);
            ui.add_space(10.0);
            app.looks_step(ui, s, game, inputs);
            ui.add_space(10.0);
            app.zones_step(ui, s, game, inputs, playing);
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
        let why = if !game.modes.contains(&s.mode.id) {
            "Signals belong to a mode: open one of this game's modes to set up what it reads."
        } else if s.mode_inputs.is_none() {
            "Built-in modes read no signals set up for a game: duplicate it to set some up for this game."
        } else {
            ui.label(muted(format!("Signals of the mode {name}.")));
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

    fn signals_intro(&mut self, ui: &mut egui::Ui) {
        card(SELECTED_BG).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("What are signals?").strong().size(16.0));
            ui.label(
                "By default a mode only knows that the gamepad rumbles and which buttons you press: it cannot tell a \
                 fight from a dialogue. Signals tell your modes what is going on in the game, so they can keep a tense \
                 vibration through a battle and calm down during a cutscene.",
            );
            ui.label(muted(
                "Everything here is optional, and belongs to this mode: a new mode made from this game's page starts \
                 with it. Steps 1 and 2 give the most for the least effort.",
            ));
        });
    }

    fn scenes_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let status = if inputs.scenes.len() >= 2 {
            Status::Done(format!("Done · {} scenes", inputs.scenes.len()))
        } else {
            Status::Next("Start here".to_owned())
        };
        let mut changed: Option<Inputs> = None;
        step_card(ui, 1, "Name the scenes of the game", status, |ui| {
            ui.label(muted(
                "A scene is a phase of the game that should not feel the same: a battle, exploring, a dialogue, a \
                 menu. Modes vibrate differently in each one. Name 2 to 4 of them, the ones that feel the most \
                 different.",
            ));
            ui.add_space(4.0);
            if !inputs.scenes.is_empty() {
                let live: HashMap<&str, f64> = if s.game.as_ref().is_some_and(|g| g.id == game.id) {
                    s.scenes.scenes.iter().map(|(n, p)| (n.as_str(), *p)).collect()
                } else {
                    HashMap::new()
                };
                let mut zone_names: Vec<&str> = inputs.zones.iter().map(|z| z.name.as_str()).collect();
                zone_names.dedup();
                zone_names.sort();
                zone_names.dedup();
                egui::Grid::new("game-scenes").num_columns(6).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label(muted("Scene").size(12.0));
                    ui.label(muted("How it sounds (optional, for the sound model)").size(12.0));
                    ui.label(muted("Sure sign").size(12.0)).on_hover_text(SURE_SIGN_HELP);
                    ui.label(muted("Kept").size(12.0)).on_hover_text(HOLD_HELP);
                    ui.label(muted("Right now").size(12.0));
                    ui.label("");
                    ui.end_row();
                    for (i, scene) in inputs.scenes.iter().enumerate() {
                        let current = s.scenes.scene.as_deref() == Some(scene.name.as_str()) && !live.is_empty();
                        let name = RichText::new(&scene.name).strong();
                        ui.label(if current { name.color(ACCENT_TEXT) } else { name });
                        let text = self.signals.sounds.entry(scene.name.clone()).or_insert_with(|| scene.sound.clone().unwrap_or_default());
                        let edit = ui.add(egui::TextEdit::singleline(text).hint_text("e.g. aggressive battle music with heavy drums").desired_width(260.0));
                        let typed = text.trim().to_owned();
                        if edit.lost_focus() && Some(typed.as_str()) != scene.sound.as_deref().or(Some("")) {
                            let mut g = inputs.clone();
                            g.scenes[i].sound = (!typed.is_empty()).then_some(typed);
                            changed = Some(g);
                        }
                        // A zone shown only in this scene.
                        let mut zone = scene.zone.clone().filter(|z| zone_names.contains(&z.as_str()));
                        let label = zone.clone().unwrap_or_else(|| if zone_names.is_empty() { "no zone yet".to_owned() } else { "none".to_owned() });
                        ui.add_enabled_ui(!zone_names.is_empty(), |ui| {
                            egui::ComboBox::from_id_salt(("scene-zone", i)).selected_text(label).width(120.0).show_ui(ui, |ui| {
                                ui.selectable_value(&mut zone, None, "none");
                                for name in &zone_names {
                                    ui.selectable_value(&mut zone, Some((*name).to_owned()), *name);
                                }
                            })
                            .response
                            .on_hover_text(SURE_SIGN_HELP);
                        });
                        if zone != scene.zone.clone().filter(|z| zone_names.contains(&z.as_str())) {
                            let mut g = inputs.clone();
                            g.scenes[i].zone = zone;
                            changed = Some(g);
                        }
                        let hold = self.signals.holds.entry(scene.name.clone()).or_insert(scene.hold);
                        let drag = ui.add(egui::DragValue::new(hold).range(0.0..=60.0).speed(0.2).max_decimals(1).suffix(" s")).on_hover_text(HOLD_HELP);
                        // Saved once let go (or typed).
                        let done = drag.drag_stopped() || drag.lost_focus() || (drag.changed() && !drag.dragged() && !drag.has_focus());
                        if done && *hold != scene.hold && changed.is_none() {
                            let mut g = inputs.clone();
                            g.scenes[i].hold = *hold;
                            changed = Some(g);
                        }
                        if !drag.dragged() && !drag.has_focus() && !done {
                            *hold = scene.hold;
                        }
                        match live.get(scene.name.as_str()) {
                            Some(p) => {
                                meter(ui, 110.0, *p, if current { ACCENT } else { GAME });
                            }
                            None => {
                                ui.label(muted("-"));
                            }
                        }
                        if ui.small_button("Remove").on_hover_text("Forget this scene (its captures stay, to sort)").clicked() {
                            let mut g = inputs.clone();
                            g.scenes.remove(i);
                            changed = Some(g);
                        }
                        ui.end_row();
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                let room = inputs.scenes.len() < MAX_SCENES;
                for suggestion in SUGGESTED_SCENES.iter().filter(|n| !inputs.scenes.iter().any(|s| s.name == **n)) {
                    if ui.add_enabled(room, egui::Button::new(format!("+ {suggestion}"))).clicked() {
                        let mut g = inputs.clone();
                        g.scenes.push(SceneDef { name: (*suggestion).to_owned(), ..SceneDef::default() });
                        changed = Some(g);
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut self.signals.new_scene).hint_text("another scene").desired_width(120.0));
                let name = self.signals.new_scene.trim().to_owned();
                let ok = room && valid_name(&name) && !inputs.scenes.iter().any(|s| s.name == name);
                if ui.add_enabled(ok, egui::Button::new("Add")).clicked() {
                    let mut g = inputs.clone();
                    g.scenes.push(SceneDef { name, ..SceneDef::default() });
                    changed = Some(g);
                    self.signals.new_scene.clear();
                }
            });
        });
        if let Some(g) = changed {
            self.send(Command::SaveInputs(g));
        }
    }

    fn looks_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let counts: Vec<(String, usize)> =
            inputs.scenes.iter().map(|sc| (sc.name.clone(), inputs.captures.iter().filter(|c| c.scene == sc.name).count())).collect();
        let enough = !counts.is_empty() && counts.iter().all(|(_, n)| *n >= CAPTURES_WANTED);
        let status = match (inputs.scenes.is_empty(), enough) {
            (true, _) => Status::Optional("After step 1".to_owned()),
            (false, true) => Status::Done("Done".to_owned()),
            (false, false) => Status::Next("Recommended · next step".to_owned()),
        };
        let mut open = false;
        let mut command = None;
        step_card(ui, 2, "Show GameViber what each scene looks like", status, |ui| {
            ui.label(muted(
                "GameViber recognizes a scene from images of it: capture a few screens of each scene, in different \
                 places. Where the music is the same in two scenes (a dungeon and its battles), the images tell them \
                 apart.",
            ));
            if !counts.is_empty() {
                egui::Grid::new("scene-captures").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (scene, n) in &counts {
                        ui.label(scene);
                        let color = if *n >= CAPTURES_WANTED { OK } else { WARN };
                        meter(ui, 220.0, (*n as f64 / CAPTURES_WANTED as f64).min(1.0), color);
                        ui.label(muted(format!("{n} of {CAPTURES_WANTED}")));
                        ui.end_row();
                    }
                });
            }
            let to_sort = inputs.captures.iter().filter(|c| c.scene.is_empty()).count();
            if to_sort > 0 {
                ui.label(RichText::new(format!("{to_sort} captures to sort")).color(WARN));
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "While playing, hold {} when the game shows a scene (pick which one on the captures page).",
                    crate::gamepad::combo_text(&s.settings.capture_combo)
                ));
                open = ui.add(primary("See the captures")).clicked();
            });
            if s.screen.model != crate::models::ModelState::Ready {
                command = model_card(ui, Model::Image, &s.screen.model);
            }
            egui::CollapsingHeader::new("The sound helps too").id_salt("sound-helps").show(ui, |ui| {
                ui.label(muted(
                    "Describe how a scene sounds in step 1 and the music is compared with it as well: useful when the \
                     music changes between scenes, useless when it does not.",
                ));
                if inputs.scenes.iter().any(|sc| sc.sound.is_some()) && s.audio.model != crate::models::ModelState::Ready {
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

    fn zones_step(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, playing: bool) {
        let mut names: Vec<&str> = inputs.zones.iter().map(|z| z.name.as_str()).collect();
        names.sort();
        names.dedup();
        let status = Status::Optional(if names.is_empty() { "Optional".to_owned() } else { format!("Optional · {} zones", names.len()) });
        let mut open = false;
        step_card(ui, 3, "Read exact values on screen", status, |ui| {
            ui.label(muted(
                "A zone is a part of the screen GameViber reads ten times a second: how full the health bar is, \
                 whether the battle menu is shown. Scenes are a good guess a few seconds late; zones are exact and \
                 instant. A mode can then beat faster when health drops below 30%.",
            ));
            ui.horizontal_wrapped(|ui| {
                for name in &names {
                    let value = if playing { s.screen.zones.iter().find(|(n, _, _)| n == name).and_then(|(_, _, v)| *v) } else { None };
                    let text = match value {
                        Some(ZoneValue::Visible(true)) => format!("{name} · shown"),
                        Some(ZoneValue::Visible(false)) => format!("{name} · hidden"),
                        Some(ZoneValue::Bar(v)) => format!("{name} · {:.0}%", v * 100.0),
                        Some(ZoneValue::Unknown) => format!("{name} · unknown"),
                        None => (*name).to_owned(),
                    };
                    pill(ui, &text, TEXT, RAISED);
                }
                let label = if inputs.captures.is_empty() { "Capture images first, then draw zones on them ›" } else { "Draw zones on the captures ›" };
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
                    s.inputs.address.as_deref().unwrap_or("GameViber")
                )));
                for (i, input) in inputs.inputs.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let kind = match input.kind {
                            InputKind::Value => format!("input.custom.{}", input.name),
                            InputKind::Event => format!("event {}", input.name),
                        };
                        ui.label(RichText::new(kind).monospace());
                        ui.label(&input.description);
                        if ui.small_button("Delete").clicked() {
                            let mut g = inputs.clone();
                            g.inputs.remove(i);
                            changed = Some(g);
                        }
                    });
                }
                let new = &mut self.signals.new_input;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("input-kind")
                        .selected_text(if new.kind == InputKind::Value { "Value" } else { "Event" })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut new.kind, InputKind::Value, "Value");
                            ui.selectable_value(&mut new.kind, InputKind::Event, "Event");
                        });
                    ui.add(egui::TextEdit::singleline(&mut new.name).hint_text("hp").desired_width(100.0));
                    ui.add(egui::TextEdit::singleline(&mut new.description).hint_text("health, 0 to 100").desired_width(220.0));
                    let ok = valid_name(&new.name) && !new.description.trim().is_empty() && !inputs.inputs.iter().any(|i| i.name == new.name);
                    if ui.add_enabled(ok, egui::Button::new("Declare")).clicked() {
                        let mut g = inputs.clone();
                        g.inputs.push(std::mem::take(new));
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
            ui.label(RichText::new("What your modes know right now").strong().size(15.0));
            if !playing {
                ui.label(muted(format!("Live while {} is the game being played.", game.name)));
            } else {
                ui.label(muted("Live, while the game runs."));
                ui.add_space(4.0);
                match (&s.scenes.scene, s.scenes.scenes.iter().find(|(n, _)| Some(n) == s.scenes.scene.as_ref())) {
                    (Some(scene), Some((_, p))) => known(ui, &capitalized(scene), &format!("scene, {:.0}% sure", p * 100.0), ACCENT_TEXT),
                    _ if !inputs.scenes.is_empty() => known(ui, "No scene yet", "it takes a few seconds of the game", MUTED),
                    _ => {}
                }
                for (name, _, value) in &s.screen.zones {
                    let text = match value {
                        Some(ZoneValue::Visible(true)) => format!("{name}: shown"),
                        Some(ZoneValue::Visible(false)) => format!("{name}: hidden"),
                        Some(ZoneValue::Bar(v)) => format!("{name}: {:.0}%", v * 100.0),
                        Some(ZoneValue::Unknown) | None => format!("{name}: unknown"),
                    };
                    known(ui, &text, "zone", TEXT);
                }
                let busy: Vec<f64> = [s.audio.levels.map(|l| l.intensity), s.screen.levels.map(|l| l.action as f64)].into_iter().flatten().collect();
                if !busy.is_empty() {
                    let intensity = busy.iter().sum::<f64>() / busy.len() as f64;
                    known(ui, &format!("Intensity {intensity:.2}"), "sound and motion", TEXT);
                }
            }
            ui.add_space(8.0);
            ui.label(muted("Ask for an Advanced mode to get one written around these signals."));
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
