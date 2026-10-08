//! The Creator's Script tab: the active mode's script, asked of an AI
//! assistant (a request built from the game's name, what the mode reads and
//! the player's own instructions, in one of three styles: the script at once,
//! after questions to the player, or after an analysis of the game whose
//! proposed phases and indicators are set up from its answer; the answer pasted
//! back or a .luau file dropped, checked before it replaces the script),
//! started from a built-in mode, or written by hand.

use eframe::egui::{self, Margin, RichText};

use super::creator::{is_draft, with_name, Tab, Way};
use super::theme::*;
use super::{App, Page, Route};
use crate::config::{self, ModeEntry};
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::mode::prompt::{self, Style};
use crate::mode::ModeRuntime;
use crate::package::Inputs;

const ASSISTANTS: [(&str, &str); 4] = [
    ("ChatGPT", "https://chatgpt.com"),
    ("Claude", "https://claude.ai"),
    ("Gemini", "https://gemini.google.com"),
    ("Le Chat", "https://chat.mistral.ai"),
];

/// The styles of request offered, with their label and what they do.
const STYLES: [(Style, &str, &str); 3] = [
    (Style::Direct, "Direct", "The script in one answer. Quick, for a first try or a simple game."),
    (
        Style::Analysis,
        "Analysis first",
        "The assistant proposes the game's phases and the indicators to draw; pasting its answer sets the phases up. \
         Then a short request, in the same conversation, asks for the script with what you set up.",
    ),
    (
        Style::Conversation,
        "Conversation",
        "The assistant first asks what you want (questions with choices) and proposes 2 or 3 designs to pick from, \
         then writes the script.",
    ),
];

#[derive(Default)]
pub struct State {
    style: Style,
    /// The mode and the style the request was copied for.
    copied: Option<(String, Style)>,
    /// The player's instructions being typed, for the mode (by id).
    instructions: Option<(String, String)>,
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
        let inputs = s.mode_inputs.clone().unwrap_or_default();
        self.request_card(ui, s, game, &inputs);
        ui.add_space(10.0);
        self.answer_card(ui, s, &inputs, draft);
        if self.generator.style == Style::Analysis {
            ui.add_space(10.0);
            self.script_request_card(ui, s, game, &inputs);
        }
        if !draft {
            ui.add_space(6.0);
            if ui.link("The mode works but does not feel right? Ask for a fix instead ›").clicked() {
                self.page = Page::Library;
                self.route = Route::Mode;
                self.open_feedback(s);
            }
        }
    }

    /// The request for `style`, with what the mode reads and the player's instructions.
    fn request(&self, s: &Shared, game: &Game, inputs: &Inputs, style: Style) -> String {
        // Indicators and values from other programs make an advanced request.
        let depth = if !inputs.zones.is_empty() || !inputs.external.is_empty() { prompt::Depth::Advanced } else { prompt::Depth::Quick };
        let phases: Vec<String> = inputs.phases.iter().map(|p| p.name.clone()).collect();
        let described = Some(inputs.describe()).filter(|d| !d.is_empty());
        let instructions = self.instructions(s, inputs);
        prompt::new_mode_prompt(
            &prompt::Templates::load(),
            &prompt::NewMode {
                game: &game.name,
                language: &s.settings.language,
                style,
                depth,
                described: described.as_deref(),
                phases: &phases,
                instructions: &instructions,
            },
        )
    }

    /// The player's instructions for the mode, as typed.
    fn instructions(&self, s: &Shared, inputs: &Inputs) -> String {
        match &self.generator.instructions {
            Some((mode, text)) if *mode == s.mode.id => text.clone(),
            _ => inputs.instructions.clone(),
        }
    }

    /// Copies the request for `style` when clicked.
    fn copy_button(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, label: &str, style: Style) {
        ui.horizontal(|ui| {
            if ui.add(primary(label)).clicked() {
                ui.ctx().copy_text(self.request(s, game, inputs, style));
                self.generator.copied = Some((s.mode.id.clone(), style));
            }
            if self.generator.copied.as_ref().is_some_and(|(mode, copied)| *mode == s.mode.id && *copied == style) {
                ui.label(RichText::new("✔ Copied").color(OK));
            }
        });
    }

    fn request_card(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let phases: Vec<&str> = inputs.phases.iter().map(|p| p.name.as_str()).collect();
        let advanced = !inputs.zones.is_empty() || !inputs.external.is_empty();
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let style = self.generator.style;
            step(ui, 1, if style == Style::Analysis { "Send the analysis request to an AI assistant" } else { "Send the request to an AI assistant" });
            ui.horizontal(|ui| {
                ui.label(muted("How"));
                for (option, label, help) in STYLES {
                    if ui.selectable_label(style == option, label).on_hover_text(help).clicked() {
                        self.generator.style = option;
                    }
                }
            });
            let help = STYLES.iter().find(|(option, _, _)| *option == style).map_or("", |(_, _, help)| help);
            ui.label(muted(help).size(12.5));
            ui.add_space(4.0);
            let what = match (style, phases.is_empty(), advanced) {
                (Style::Analysis, true, _) => format!("For {}.", game.name),
                (Style::Analysis, false, _) => format!("For {}, keeping its phases {}.", game.name, phases.join(", ")),
                (_, _, true) => format!("For {}, with what the mode reads: its phases, indicators and values from other programs.", game.name),
                (_, true, _) => format!("For {}. Name its phases first (Phases tab), or ask for an analysis: the assistant then gives each its own feel.", game.name),
                (_, false, _) => format!("For {}, with its phases {}.", game.name, phases.join(", ")),
            };
            ui.label(muted(what));
            ui.add_space(4.0);
            self.instructions_field(ui, s, inputs);
            ui.add_space(4.0);
            let label = if style == Style::Analysis { "📋 Copy the analysis request" } else { "📋 Copy the request" };
            self.copy_button(ui, s, game, inputs, label, style);
            ui.horizontal(|ui| {
                ui.label(muted("Paste it in a new conversation:"));
                for (name, url) in ASSISTANTS {
                    ui.hyperlink_to(format!("{name} ↗"), url);
                }
            });
            ui.label(muted("Tip: turn web search on if the assistant has it, so it checks the game's controls.").size(12.0));
        });
    }

    /// The player's own instructions, written at the end of every request for the
    /// mode's script, saved with the mode once typed.
    fn instructions_field(&mut self, ui: &mut egui::Ui, s: &Shared, inputs: &Inputs) {
        ui.label(RichText::new("Your instructions").strong()).on_hover_text("Written at the end of the request, and kept with the mode for its next requests");
        let mut text = self.instructions(s, inputs);
        let edit = ui.add(
            egui::TextEdit::multiline(&mut text)
                .hint_text("e.g. stronger on critical hits, nothing in menus, a slow wave while exploring")
                .desired_width(f32::INFINITY)
                .desired_rows(2),
        );
        if edit.changed() {
            self.generator.instructions = Some((s.mode.id.clone(), text.clone()));
        }
        if edit.lost_focus() && text.trim() != inputs.instructions && inputs.has_package() {
            self.send(Command::SaveInputs(Inputs { instructions: text.trim().to_owned(), ..inputs.clone() }));
        }
        ui.label(
            muted("They come last, and the assistant follows them first. You can also write more at the end of the request \
                   once pasted, or answer the assistant afterwards.")
            .size(12.0),
        );
    }

    /// The answer pasted back: a script, or the setup an analysis proposes.
    fn answer_card(&mut self, ui: &mut egui::Ui, s: &Shared, inputs: &Inputs, draft: bool) {
        let analysis = self.generator.style == Style::Analysis;
        let mut apply = false;
        let mut set_up = None;
        let g = &mut self.generator;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            step(ui, 2, if analysis { "Paste the answers" } else { "Paste the answer" });
            ui.label(muted(if analysis {
                "First the analysis's: GameViber sets up the phases it proposes and lists the indicators to draw. Then, \
                 after step 3, the script's (or drop a .luau file on this window)."
            } else {
                "The whole answer or just the code (Ctrl+V), or drop a .luau file on this window. It is checked before it is used."
            }));
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
            // An analysis's answer holds a setup and no script.
            let setup = (!g.answer.trim().is_empty() && !prompt::has_script(&g.answer)).then(|| prompt::extract_setup(&g.answer)).flatten();
            ui.horizontal(|ui| match setup {
                Some(setup) => {
                    let what = format!("{} phase(s), {} indicator(s) to draw", setup.phases.len(), setup.indicators.len());
                    if ui.add_enabled(inputs.has_package(), primary("Set up what it proposes")).on_hover_text(what).clicked() {
                        set_up = Some(setup);
                    }
                    ui.label(muted("Phases already set up keep what you chose; they only get what they lack.").size(12.0));
                }
                None => {
                    apply = ui.add_enabled(!g.answer.trim().is_empty(), primary("Use this script")).clicked();
                    if !draft {
                        ui.label(muted("It replaces the current script, kept as a .bak copy.").size(12.0));
                    }
                }
            });
        });
        if let Some(setup) = set_up {
            let mut g = inputs.clone();
            let applied = g.apply_setup(&setup);
            self.send(Command::SaveInputs(g));
            self.generator.answer.clear();
            self.generator.note = Some(format!(
                "✔ Set up: {} phase(s) added, {} completed, {} indicator(s) to draw. Check them, then ask for the script (step 3).",
                applied.phases_added, applied.phases_completed, applied.indicators_to_draw
            ));
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

    /// After an analysis: what is left to set up, and the request for the script.
    fn script_request_card(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            step(ui, 3, "Set up the rest, then ask for the script");
            let to_draw: Vec<_> = inputs.to_draw().collect();
            if !to_draw.is_empty() {
                ui.label(RichText::new("Indicators to draw").strong());
                for planned in &to_draw {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&planned.name).monospace());
                        ui.label(muted(planned.kind.label()).size(12.0));
                        ui.label(muted(&planned.place).size(12.5));
                    });
                }
            }
            ui.horizontal(|ui| {
                if ui.link("Captures & indicators ›").on_hover_text("Take a few captures of each phase, and draw the indicators on them").clicked() {
                    self.creator.tab = Tab::Screen;
                }
                if ui.link("Phases ›").clicked() {
                    self.creator.tab = Tab::Phases;
                }
            });
            ui.add_space(4.0);
            ui.label(muted("Send it in the same conversation as the analysis: it lists what you set up, and is short since the \
                            assistant already has the rest."));
            self.copy_button(ui, s, game, inputs, "📋 Copy the script request", Style::AfterAnalysis);
        });
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
