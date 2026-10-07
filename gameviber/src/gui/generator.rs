//! The Creator's Script tab: the active mode's script, asked of an AI
//! assistant (a request built from the game's name and what the mode reads,
//! the answer pasted back or a .luau file dropped, checked before it replaces
//! the script), started from a built-in mode, or written by hand.

use eframe::egui::{self, Margin, RichText};

use super::creator::{is_draft, with_name, Way};
use super::theme::*;
use super::{App, Page, Route};
use crate::config::{self, ModeEntry};
use crate::engine::{Command, Shared};
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
    /// The mode the request was copied for.
    copied: Option<String>,
    answer: String,
    error: Option<String>,
    fix_copied: bool,
    /// What the last script applied did.
    note: Option<String>,
}

impl App {
    pub(super) fn script_tab(&mut self, ui: &mut egui::Ui, s: &Shared, game: Option<&Game>) {
        let entry = ModeEntry::from_id(&s.mode.id);
        if entry.builtin {
            self.script_editor(ui, s);
            return;
        }
        let draft = is_draft(&self.creator.editor.text);
        let way = self.creator.way.unwrap_or(if draft { Way::Ai } else { Way::Write });
        ui.horizontal(|ui| {
            for (option, label) in [(Way::Ai, "✨ Ask an AI assistant"), (Way::BuiltIn, "Start from a built-in mode"), (Way::Write, "Write it yourself")] {
                if ui.selectable_label(way == option, RichText::new(label).size(14.0)).clicked() {
                    self.creator.way = Some(option);
                }
            }
        });
        if let Some(note) = &self.generator.note {
            ui.label(RichText::new(note).color(OK));
        }
        ui.add_space(6.0);
        match way {
            Way::Ai => {
                egui::ScrollArea::vertical().show(ui, |ui| self.ask_assistant(ui, s, game, draft));
            }
            Way::BuiltIn => {
                egui::ScrollArea::vertical().show(ui, |ui| self.builtin_script(ui, s, draft));
            }
            Way::Write => self.script_editor(ui, s),
        }
    }

    /// The request to copy, the answer to paste back.
    fn ask_assistant(&mut self, ui: &mut egui::Ui, s: &Shared, game: Option<&Game>, draft: bool) {
        self.take_dropped_file(ui.ctx());
        let Some(game) = game else {
            ui.label(muted("The request names the mode's game: give it one on its page first."));
            return;
        };
        let inputs = s.mode_inputs.as_ref();
        // Indicators and values from other programs make an advanced request.
        let depth = if inputs.is_some_and(|i| !i.zones.is_empty() || !i.external.is_empty()) { prompt::Depth::Advanced } else { prompt::Depth::Quick };
        let phases: Vec<String> = inputs.iter().flat_map(|i| i.phases.iter().map(|p| p.name.clone())).collect();
        let g = &mut self.generator;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            step(ui, 1, "Send the request to an AI assistant");
            let what = match (phases.is_empty(), depth) {
                (_, prompt::Depth::Advanced) => format!("For {}, with what the mode reads: its phases, indicators and values from other programs.", game.name),
                (true, _) => format!("For {}. Name its phases first (Phases tab): the assistant then gives each its own feel.", game.name),
                (false, _) => format!("For {}, with its phases {}.", game.name, phases.join(", ")),
            };
            ui.label(muted(what));
            ui.horizontal(|ui| {
                if ui.add(primary("📋 Copy the request")).clicked() {
                    let described = inputs.map(|i| i.describe()).filter(|d| !d.is_empty());
                    let request = prompt::new_mode_prompt(&prompt::Templates::load(), &game.name, &s.settings.language, depth, described.as_deref(), &phases);
                    ui.ctx().copy_text(request);
                    g.copied = Some(s.mode.id.clone());
                }
                if g.copied.as_deref() == Some(s.mode.id.as_str()) {
                    ui.label(RichText::new("✔ Copied").color(OK));
                }
            });
            ui.horizontal(|ui| {
                ui.label(muted("Paste it in a new conversation:"));
                for (name, url) in ASSISTANTS {
                    ui.hyperlink_to(format!("{name} ↗"), url);
                }
            });
            ui.label(muted("Tip: turn web search on if the assistant has it, so it checks the game's controls.").size(12.0));
        });
        ui.add_space(10.0);
        let mut apply = false;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            step(ui, 2, "Paste the answer");
            ui.label(muted("The whole answer or just the code (Ctrl+V), or drop a .luau file on this window. It is checked before it is used."));
            egui::ScrollArea::vertical().id_salt("answer").max_height(220.0).show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut g.answer).code_editor().hint_text("mode { api = 1, ... }").desired_width(f32::INFINITY).desired_rows(8);
                if ui.add(edit).changed() {
                    g.error = None;
                }
            });
            if let Some(error) = &g.error {
                ui.label(RichText::new("Does not load yet").strong().color(DANGER_TEXT));
                ui.label(RichText::new(error).color(DANGER_TEXT).monospace().size(12.0));
                ui.horizontal(|ui| {
                    if ui.add(primary("📋 Copy the fix request")).clicked() {
                        ui.ctx().copy_text(prompt::fix_prompt(error));
                        g.fix_copied = true;
                    }
                    let note = if g.fix_copied { "✔ Copied: send it to the assistant, then paste its new answer." } else { "Assistants make such slips: send them this." };
                    ui.label(muted(note));
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                apply = ui.add_enabled(!g.answer.trim().is_empty(), primary("Use this script")).clicked();
                if !draft {
                    ui.label(muted("It replaces the current script, kept as a .bak copy.").size(12.0));
                }
            });
        });
        if !draft {
            ui.add_space(6.0);
            if ui.link("The mode works but does not feel right? Ask for a fix instead ›").clicked() {
                self.page = Page::Library;
                self.route = Route::Mode;
                self.open_feedback(s);
            }
        }
        if apply {
            let script = prompt::extract_script(&self.generator.answer);
            match ModeRuntime::probe("generated", &script) {
                Ok(_) => {
                    if self.replace_script(s, &script, draft) {
                        self.generator.answer.clear();
                        self.generator.note = Some("✔ The script loads and runs now: play to try it, or use the simulator (Sessions).".to_owned());
                    }
                }
                Err(e) => {
                    self.generator.error = Some(e);
                    self.generator.fix_copied = false;
                }
            }
        }
    }

    /// The built-in modes, to start from one.
    fn builtin_script(&mut self, ui: &mut egui::Ui, s: &Shared, draft: bool) {
        ui.label(muted(if draft {
            "Each one suits a genre. Its script becomes this mode's: tune it, or ask an assistant to adapt it later."
        } else {
            "Its script replaces this mode's, kept as a .bak copy."
        }));
        ui.add_space(6.0);
        let mut chosen = None;
        for entry in s.modes.iter().filter(|e| e.builtin) {
            let Some(Ok(info)) = s.catalog.get(&entry.id) else { continue };
            card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&info.name).strong().size(15.0));
                            ui.label(muted(&info.category).size(12.0));
                        });
                        ui.label(muted(&info.description).size(12.5));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Start from it").clicked() {
                            chosen = Some(entry.clone());
                        }
                    });
                });
            });
        }
        if let Some(builtin) = chosen {
            match builtin.source() {
                Ok(source) => {
                    let name = s.mode.info.as_ref().map_or_else(|| ModeEntry::from_id(&s.mode.id).key, |i| i.name.clone());
                    if self.replace_script(s, &with_name(&source, &name), draft) {
                        self.generator.note = Some(format!("✔ Started from {}: tune it on its page or in Live.", builtin.key));
                    }
                }
                Err(e) => log::error!("cannot read {}: {e}", builtin.id),
            }
        }
    }

    /// Writes `script` as the active mode's (keeping the previous one unless it
    /// was the new mode's template), reloads it and shows it.
    fn replace_script(&mut self, s: &Shared, script: &str, draft: bool) -> bool {
        let entry = ModeEntry::from_id(&s.mode.id);
        let Some(path) = entry.path() else { return false };
        let backup = (!draft).then(|| path.with_extension("luau.bak"));
        let written = match &backup {
            Some(backup) => std::fs::copy(&path, backup).map(|_| ()).and_then(|_| config::write_file(&path, script)),
            None => config::write_file(&path, script),
        };
        if let Err(e) = written {
            self.generator.error = Some(format!("cannot save the mode: {e}"));
            return false;
        }
        log::info!("new script for {}", path.display());
        self.generator.error = None;
        // The editor reads it again.
        self.creator.editor = Default::default();
        self.creator.way = Some(super::creator::Way::Write);
        self.send(Command::ReloadMode);
        self.send(Command::RefreshModes);
        true
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
        ui.label(RichText::new(text).strong().size(15.0));
    });
}
