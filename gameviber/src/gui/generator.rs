//! "A mode for this game" dialog, opened from a game's modes: builds a
//! request for an AI assistant from the game's name (and, for an advanced
//! mode, its inputs), copies it to the clipboard, then creates a mode from
//! the answer pasted back (or a dropped .luau file), checking that it loads,
//! and adds it to the game.

use eframe::egui::{self, RichText};

use super::theme::*;
use super::{App, GameView, Route};
use crate::engine::Shared;
use crate::game::Game;
use crate::mode::{prompt, ModeRuntime};

const ASSISTANTS: [(&str, &str); 4] = [
    ("ChatGPT", "https://chatgpt.com"),
    ("Claude", "https://claude.ai"),
    ("Gemini", "https://gemini.google.com"),
    ("Le Chat", "https://chat.mistral.ai"),
];

#[derive(Default)]
pub struct State {
    pub open: bool,
    game: String,
    /// The game the mode is for, by id; None: a game typed in the dialog.
    game_id: Option<String>,
    depth: prompt::Depth,
    /// Game name the request was copied for.
    copied: Option<String>,
    answer: String,
    error: Option<String>,
    fix_copied: bool,
}

impl App {
    pub(super) fn open_generator(&mut self) {
        self.generator.open = true;
    }

    /// The dialog for a mode of `game`.
    pub(super) fn open_generator_for(&mut self, game: &Game, depth: prompt::Depth) {
        self.generator = State { open: true, game: game.name.clone(), game_id: Some(game.id.clone()), depth, ..State::default() };
    }

    pub(super) fn generator_ui(&mut self, ctx: &egui::Context, s: &Shared) {
        if !self.generator.open {
            return;
        }
        self.take_dropped_file(ctx);
        let mut create = false;
        let modal = egui::Modal::new(egui::Id::new("mode-generator")).show(ctx, |ui| {
            ui.set_width(560.0);
            let g = &mut self.generator;
            heading(ui, "A mode made for your game");
            let for_game = g.game_id.as_ref().and_then(|id| s.games.iter().find(|game| game.id == *id));
            ui.label(muted(
                "Built-in modes are generic. An AI assistant can write a mode tailored to your game, \
                 its controls and its mechanics in a minute. GameViber prepares the request; you paste \
                 the answer back.",
            ));
            ui.add_space(12.0);

            step(ui, 1, "Which game are you playing?");
            match for_game {
                Some(game) => {
                    ui.label(RichText::new(&game.name).size(15.0));
                }
                None => {
                    ui.add(egui::TextEdit::singleline(&mut g.game).hint_text("e.g. Hades II").desired_width(f32::INFINITY));
                }
            }
            ui.add_space(10.0);

            step(ui, 2, "Quick or advanced?");
            ui.radio_value(&mut g.depth, prompt::Depth::Quick, "Quick (a couple of minutes)").on_hover_text(
                "The rumble, the buttons, and the phases, impacts and intensity GameViber gets from the sound and image",
            );
            ui.radio_value(&mut g.depth, prompt::Depth::Advanced, "Advanced").on_hover_text(
                "Also the raw sound and image, and the game's indicators, captures and values from other programs (its \
                 Inputs)",
            );
            // The inputs of the active mode, when it is one of the game's: the new mode starts with them.
            let inputs = for_game.filter(|game| game.modes.contains(&s.mode.id)).and(s.mode_inputs.as_ref());
            let described = for_game.zip(inputs).map(|(game, inputs)| (game.name.clone(), inputs.describe()));
            match (&described, g.depth) {
                (Some((game, text)), prompt::Depth::Advanced) if !text.is_empty() => {
                    ui.label(muted(format!("The request includes the inputs set up for {game}.")).size(12.0));
                }
                (_, prompt::Depth::Advanced) => {
                    ui.label(muted("No inputs set up yet: the assistant may ask you to draw indicators in the mode's Inputs.").size(12.0));
                }
                (Some((_, text)), prompt::Depth::Quick) if text.contains("input.phase") => {
                    ui.label(muted("The request names the game's phases.").size(12.0));
                }
                _ => {}
            }
            ui.add_space(10.0);

            step(ui, 3, "Send the request to an AI assistant");
            ui.horizontal(|ui| {
                let game = g.game.trim().to_owned();
                if ui.add_enabled(!game.is_empty(), primary("📋 Copy the request")).clicked() {
                    let described = described.as_ref().map(|(_, text)| text.as_str());
                    let phases: Vec<String> = inputs.iter().flat_map(|i| i.phases.iter().map(|s| s.name.clone())).collect();
                    let request = prompt::new_mode_prompt(&prompt::Templates::load(), &game, &s.settings.language, g.depth, described, &phases);
                    ui.ctx().copy_text(request);
                    g.copied = Some(game.clone());
                }
                if g.copied.as_deref() == Some(game.as_str()) {
                    ui.label(RichText::new("✔ Copied").color(OK));
                }
            });
            ui.label(muted("Paste it in a new conversation with any assistant:"));
            ui.horizontal(|ui| {
                for (name, url) in ASSISTANTS {
                    ui.hyperlink_to(format!("{name} ↗"), url);
                }
            });
            ui.label(
                muted("Tip: enable web search if the assistant has it, so it checks the game's default controls.")
                    .size(12.0),
            );
            ui.add_space(10.0);

            step(ui, 4, "Paste the answer");
            ui.label(muted("The whole answer or just the code. You can also drop a .luau file on this window."));
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut g.answer)
                    .code_editor()
                    .hint_text("mode { api = 1, ... }")
                    .desired_width(f32::INFINITY)
                    .desired_rows(8);
                if ui.add(edit).changed() {
                    g.error = None;
                }
            });
            if let Some(error) = &g.error {
                ui.label(RichText::new(error).color(DANGER_TEXT).monospace().size(12.0));
                ui.horizontal(|ui| {
                    if ui.button("📋 Copy a fix request").clicked() {
                        ui.ctx().copy_text(prompt::fix_prompt(error));
                        g.fix_copied = true;
                    }
                    let note = if g.fix_copied { "✔ Copied: send it to the assistant, then paste its new answer." } else { "and send it to the assistant." };
                    ui.label(muted(note));
                });
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                create = ui.add_enabled(!g.answer.trim().is_empty(), primary("Create the mode")).clicked();
                if ui.button("Close").clicked() {
                    g.open = false;
                }
            });
        });
        if modal.should_close() {
            self.generator.open = false;
        }
        if create {
            self.create_generated_mode();
        }
    }

    fn create_generated_mode(&mut self) {
        let g = &mut self.generator;
        let script = prompt::extract_script(&g.answer);
        if let Err(e) = ModeRuntime::probe("generated", &script) {
            g.error = Some(e);
            g.fix_copied = false;
            return;
        }
        let stem = match prompt::file_stem(&g.game) {
            stem if stem.is_empty() => "my-game".to_owned(),
            stem => stem,
        };
        let game = g.game_id.clone();
        *g = State::default();
        // Created from the game's page, the mode is added to the game.
        if let Some(id) = &game {
            self.route = Route::Game { id: id.clone(), view: GameView::Modes };
        }
        self.create_mode(&stem, &script, None);
        self.page = super::Page::Games;
        if let Some(id) = game {
            self.route = Route::Game { id, view: GameView::Mode };
        }
    }

    /// Loads a .luau file dropped on the window into the answer field.
    fn take_dropped_file(&mut self, ctx: &egui::Context) {
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(file) = files.first() else { return };
        match file.bytes().map(|bytes| String::from_utf8_lossy(&bytes).into_owned()) {
            Ok(text) => {
                self.generator.answer = text;
                self.generator.error = None;
            }
            Err(e) => self.generator.error = Some(format!("cannot read {}: {e}", file.path().display())),
        }
    }
}

fn step(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(n.to_string()).strong().color(ACCENT));
        ui.label(RichText::new(text).strong());
    });
}
