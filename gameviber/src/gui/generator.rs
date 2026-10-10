//! The Creator's AI assistant and Script tabs: the game's phases and the
//! mode's script asked of an AI assistant (a request built from the game's name, what the mode reads, what
//! the player ticked — their toys, what to feel, an analysis of the game first
//! whose proposed phases and indicators are set up from its answer, questions
//! before the script — and their own instructions; the answer pasted back or a
//! .luau file dropped, checked before it replaces the script); or the script
//! started from a built-in mode, or written by hand. The Create page asks the
//! assistant the same for a new mode.

use eframe::egui::{self, Margin, RichText};

use super::creator::{is_draft, with_name, Tab, Way};
use super::theme::*;
use super::{App, Page, Route};
use crate::config::{self, ModeEntry};
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::mode::prompt::{self, FeelKind, Step, Wishes, FEELS};
use crate::mode::ModeRuntime;
use crate::package::Inputs;

const ASSISTANTS: [(&str, &str); 3] = [
    ("Gemini", "https://gemini.google.com"),
    ("Claude", "https://claude.ai"),
    ("ChatGPT", "https://chatgpt.com"),
];

/// The widest the AI assistant's steps get.
const STEPS_WIDTH: f32 = 900.0;

#[derive(Default)]
pub struct State {
    /// The mode and the request copied for it.
    copied: Option<(String, Step)>,
    /// The player's instructions being typed, for the mode (by id).
    instructions: Option<(String, String)>,
    /// What the player ticked, for the mode (by id), until the engine has it.
    wishes: Option<(String, Wishes)>,
    /// The mode whose request is being changed (its wishes shown in full).
    changing: Option<String>,
    /// What the Create page asks of the next mode: the player's wishes and instructions.
    pub(super) new_wishes: Wishes,
    pub(super) new_instructions: String,
    /// The analysis's answer and the script's, pasted.
    analysis_answer: String,
    answer: String,
    /// The mode whose analysis or script is asked again (its step open once done).
    redo_analysis: Option<String>,
    redo_script: Option<String>,
    /// What the last analysis set up, for the mode (by id).
    setup_note: Option<(String, String)>,
    error: Option<String>,
    fix_copied: bool,
    /// What the last script applied did.
    note: Option<String>,
}

impl App {
    /// The Script tab: started from a built-in mode, or written by hand (the AI assistant has its own tab).
    pub(super) fn script_tab(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let entry = ModeEntry::from_id(&s.mode.id);
        if entry.builtin {
            self.script_editor(ui, s);
            return;
        }
        let draft = is_draft(&self.creator.editor.text);
        let way = self.creator.way.unwrap_or(if draft { Way::BuiltIn } else { Way::Write });
        ui.horizontal(|ui| {
            for (option, label) in [(Way::BuiltIn, "Start from a built-in mode"), (Way::Write, "Write it yourself")] {
                if ui.selectable_label(way == option, RichText::new(label).size(14.0)).clicked() {
                    self.creator.way = Some(option);
                }
            }
            ui.label(muted("|"));
            if ui.link("✨ Or ask the AI assistant ›").clicked() {
                self.creator.tab = Tab::Assistant;
            }
        });
        if let Some(note) = &self.generator.note {
            ui.label(RichText::new(note).color(OK));
        }
        ui.add_space(6.0);
        match way {
            Way::BuiltIn | Way::Ai => {
                egui::ScrollArea::vertical().show(ui, |ui| self.builtin_script(ui, s, draft));
            }
            Way::Write => self.script_editor(ui, s),
        }
    }

    /// The AI assistant's tab: what it is asked, the game's phases it proposes, the script it writes.
    pub(super) fn assistant_tab(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let draft = is_draft(&self.creator.editor.text);
        egui::ScrollArea::vertical().show(ui, |ui| {
            // Steps read in a column: on a wide window their buttons stay near their text.
            ui.set_max_width(STEPS_WIDTH);
            self.ask_assistant(ui, s, game, draft);
        });
    }

    /// Where the player is, what they ask, then each step: its request, the answer pasted back.
    fn ask_assistant(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, draft: bool) {
        self.take_dropped_file(ui.ctx());
        let inputs = s.mode_inputs.clone().unwrap_or_default();
        let analysis = self.wishes(s, &inputs).phases_first;
        progress_bar(ui, s, &inputs, analysis, draft);
        ui.add_space(10.0);
        self.wishes_card(ui, s, &inputs);
        ui.add_space(10.0);
        if analysis {
            self.analysis_card(ui, s, game, &inputs);
            ui.add_space(10.0);
            self.rest_card(ui, &inputs);
            ui.add_space(10.0);
            self.script_card(ui, s, game, &inputs, draft, 3, Step::AfterAnalysis);
        } else {
            self.script_card(ui, s, game, &inputs, draft, 1, Step::Script);
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

    /// The request for `step`, with what the mode reads, the player's wishes and instructions.
    fn request(&self, s: &Shared, game: &Game, inputs: &Inputs, step: Step) -> String {
        // Indicators and values from other programs make an advanced request.
        let depth = if !inputs.zones.is_empty() || !inputs.external.is_empty() { prompt::Depth::Advanced } else { prompt::Depth::Quick };
        let phases: Vec<String> = inputs.phases.iter().map(|p| p.name.clone()).collect();
        let described = Some(inputs.describe()).filter(|d| !d.is_empty());
        let instructions = self.instructions(s, inputs);
        let wishes = self.wishes(s, inputs);
        prompt::new_mode_prompt(
            &prompt::Templates::load(),
            &prompt::NewMode {
                game: &game.name,
                language: &s.settings.language,
                step,
                wishes: &wishes,
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

    /// What the player ticked for the mode, as clicked.
    fn wishes(&self, s: &Shared, inputs: &Inputs) -> Wishes {
        match &self.generator.wishes {
            Some((mode, wishes)) if *mode == s.mode.id => wishes.clone(),
            _ => inputs.wishes.clone().unwrap_or_default(),
        }
    }

    /// Copies the request for `step` when clicked.
    fn copy_button(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, label: &str, step: Step) {
        ui.horizontal(|ui| {
            if ui.add(primary(label)).clicked() {
                ui.ctx().copy_text(self.request(s, game, inputs, step));
                self.generator.copied = Some((s.mode.id.clone(), step));
                // The request asked is the one kept with the mode.
                if inputs.wishes.is_none() {
                    self.save_request(s, inputs);
                }
            }
            if self.generator.copied.as_ref().is_some_and(|(mode, copied)| *mode == s.mode.id && *copied == step) {
                ui.label(RichText::new("✔ Copied").color(OK));
            }
        });
    }

    /// What the player asks of the mode: its summary, or the boxes to tick.
    fn wishes_card(&mut self, ui: &mut egui::Ui, s: &Shared, inputs: &Inputs) {
        let mut wishes = self.wishes(s, inputs);
        // A mode not asked for yet shows the boxes at once.
        let changing = inputs.wishes.is_none() || self.generator.changing.as_deref() == Some(s.mode.id.as_str());
        let mut changed = false;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Your request").strong().size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if inputs.wishes.is_some() && ui.selectable_label(changing, if changing { "Done ▴" } else { "Change ▾" }).clicked() {
                        self.generator.changing = if changing { None } else { Some(s.mode.id.clone()) };
                    }
                });
            });
            if changing {
                changed = wishes_ui(ui, &mut wishes);
                ui.add_space(6.0);
                self.instructions_field(ui, s, inputs);
            } else {
                ui.label(muted(summary(&wishes, &self.instructions(s, inputs))));
            }
        });
        if changed {
            self.generator.wishes = Some((s.mode.id.clone(), wishes));
            self.save_request(s, inputs);
        }
    }

    /// Keeps the player's wishes and instructions with the mode, as typed and ticked.
    fn save_request(&mut self, s: &Shared, inputs: &Inputs) {
        if inputs.has_package() {
            let (wishes, instructions) = (self.wishes(s, inputs), self.instructions(s, inputs));
            self.send(Command::SaveInputs(Inputs { wishes: Some(wishes), instructions: instructions.trim().to_owned(), ..inputs.clone() }));
        }
    }

    /// Where to paste a request.
    fn assistant_links(ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(muted("Paste it in a new conversation:"));
            for (name, url) in ASSISTANTS {
                ui.hyperlink_to(format!("{name} ↗"), url);
            }
        });
        ui.label(muted("Tip: turn web search on if the assistant has it, so it checks the game's controls.").size(12.0));
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
        if edit.lost_focus() && text.trim() != inputs.instructions {
            self.save_request(s, inputs);
        }
        ui.label(
            muted("They come last, and the assistant follows them first. You can also write more at the end of the request \
                   once pasted, or answer the assistant afterwards.")
            .size(12.0),
        );
    }

    /// Step 1 of an analysis: its request and its answer, which sets the phases up; done once set up.
    fn analysis_card(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let open = !inputs.analysed || self.generator.redo_analysis.as_deref() == Some(s.mode.id.as_str());
        let mut set_up = None;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                step(ui, 1, "Ask for the game's phases and indicators", inputs.analysed);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if inputs.analysed && ui.selectable_label(open, if open { "Cancel" } else { "↺ Ask again" }).clicked() {
                        self.generator.redo_analysis = if open { None } else { Some(s.mode.id.clone()) };
                    }
                });
            });
            if !open {
                if let Some((mode, note)) = &self.generator.setup_note {
                    if *mode == s.mode.id {
                        ui.label(RichText::new(note).color(OK));
                    }
                }
                let phases: Vec<&str> = inputs.phases.iter().map(|p| p.name.as_str()).collect();
                ui.label(muted(format!("Phases set up: {}.", if phases.is_empty() { "none".to_owned() } else { phases.join(", ") })));
                return;
            }
            let phases: Vec<&str> = inputs.phases.iter().map(|p| p.name.as_str()).collect();
            ui.label(muted(if phases.is_empty() {
                format!("For {}: the assistant proposes its phases and the indicators to draw.", game.name)
            } else {
                format!("For {}, keeping its phases {}: the assistant completes them and proposes the indicators to draw.", game.name, phases.join(", "))
            }));
            ui.add_space(4.0);
            self.copy_button(ui, s, game, inputs, "📋 Copy the analysis request", Step::Analysis);
            Self::assistant_links(ui);
            ui.add_space(8.0);
            ui.label(RichText::new("Paste its answer").strong());
            let g = &mut self.generator;
            egui::ScrollArea::vertical().id_salt("analysis-answer").max_height(160.0).show(ui, |ui| {
                let hint = "The whole answer: GameViber reads the json block it ends with";
                ui.add(egui::TextEdit::multiline(&mut g.analysis_answer).hint_text(hint).desired_width(f32::INFINITY).desired_rows(5));
            });
            let pasted = !g.analysis_answer.trim().is_empty();
            let setup = pasted.then(|| prompt::extract_setup(&g.analysis_answer)).flatten();
            ui.horizontal(|ui| match setup {
                Some(setup) => {
                    let what = format!("{} phase(s), {} indicator(s) to draw", setup.phases.len(), setup.indicators.len());
                    if ui.add_enabled(inputs.has_package(), primary("Set up what it proposes")).clicked() {
                        set_up = Some(setup);
                    }
                    ui.label(muted(format!("Found: {what}. Phases already set up keep what you chose.")).size(12.0));
                }
                None if pasted => {
                    ui.label(RichText::new("No setup found: paste the whole answer, with the json block it ends with.").color(WARN).size(12.5));
                }
                None => {}
            });
        });
        if let Some(setup) = set_up {
            let mut g = inputs.clone();
            let applied = g.apply_setup(&setup);
            self.send(Command::SaveInputs(g));
            self.generator.analysis_answer.clear();
            self.generator.redo_analysis = None;
            self.generator.setup_note = Some((
                s.mode.id.clone(),
                format!(
                    "✔ Set up: {} phase(s) added, {} completed, {} indicator(s) to draw.",
                    applied.phases_added, applied.phases_completed, applied.indicators_to_draw
                ),
            ));
        }
    }

    /// Step 2 of an analysis: the indicators it proposed, to draw; done once drawn.
    fn rest_card(&mut self, ui: &mut egui::Ui, inputs: &Inputs) {
        let to_draw: Vec<_> = inputs.to_draw().cloned().collect();
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            step(ui, 2, "Draw the indicators, take captures", inputs.analysed && to_draw.is_empty());
            if !inputs.analysed {
                ui.label(muted("Once the analysis is set up: the indicators it proposes are listed here."));
                return;
            }
            if to_draw.is_empty() {
                ui.label(muted("Every indicator proposed is drawn. A few captures of each phase make the image recognition more reliable."));
            } else {
                ui.label(muted("Take a capture where each one shows, then draw it on it."));
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
        });
    }

    /// The script's step: its request and the answer pasted back, checked before
    /// it replaces the script; done once the mode has one.
    #[allow(clippy::too_many_arguments)]
    fn script_card(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, draft: bool, n: usize, request: Step) {
        let open = draft || self.generator.redo_script.as_deref() == Some(s.mode.id.as_str());
        let mut apply = false;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                step(ui, n, "Ask for the script", !draft);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !draft && ui.selectable_label(open, if open { "Cancel" } else { "↺ Ask again" }).clicked() {
                        self.generator.redo_script = if open { None } else { Some(s.mode.id.clone()) };
                    }
                });
            });
            if !open {
                match (&s.mode.error, &self.generator.note) {
                    (Some(_), _) => ui.label(RichText::new("The script does not load: see Logs, or ask again.").color(DANGER_TEXT)),
                    (None, Some(note)) => ui.label(RichText::new(note).color(OK)),
                    (None, None) => ui.label(muted("The mode has its script: play to try it, or use the simulator (Sessions).")),
                };
                return;
            }
            if request == Step::AfterAnalysis {
                ui.label(muted("Send it in the same conversation as the analysis: it lists what you set up, and is short since the \
                                assistant already has the rest."));
                self.copy_button(ui, s, game, inputs, "📋 Copy the script request", request);
            } else {
                let phases: Vec<&str> = inputs.phases.iter().map(|p| p.name.as_str()).collect();
                let advanced = !inputs.zones.is_empty() || !inputs.external.is_empty();
                ui.label(muted(match (phases.is_empty(), advanced) {
                    (_, true) => format!("For {}, with what the mode reads: its phases, indicators and values from other programs.", game.name),
                    (true, _) => format!("For {}. Name its phases first (Phases tab), or tick \"Propose the game's phases first\": the assistant then gives each its own feel.", game.name),
                    (false, _) => format!("For {}, with its phases {}.", game.name, phases.join(", ")),
                }));
                ui.add_space(4.0);
                self.copy_button(ui, s, game, inputs, "📋 Copy the request", request);
                Self::assistant_links(ui);
            }
            ui.add_space(8.0);
            ui.label(RichText::new("Paste its answer").strong());
            ui.label(muted("The whole answer or just the code (Ctrl+V), or drop a .luau file on this window. It is checked before it is used.").size(12.0));
            let g = &mut self.generator;
            egui::ScrollArea::vertical().id_salt("answer").max_height(220.0).show(ui, |ui| {
                let edit = egui::TextEdit::multiline(&mut g.answer).code_editor().hint_text("mode { api = 1, ... }").desired_width(f32::INFINITY).desired_rows(6);
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
            ui.horizontal(|ui| {
                apply = ui.add_enabled(!g.answer.trim().is_empty(), primary("Use this script")).clicked();
                if !draft {
                    ui.label(muted("It replaces the current script, kept as a .bak copy.").size(12.0));
                }
            });
        });
        if apply {
            let script = prompt::extract_script(&self.generator.answer);
            match ModeRuntime::probe("generated", &script) {
                Ok(_) => {
                    if self.replace_script(s, &script, draft) {
                        self.generator.answer.clear();
                        self.generator.redo_script = None;
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

/// The boxes the player ticks to say what a mode should do and how the
/// assistant should work; whether anything changed.
pub(super) fn wishes_ui(ui: &mut egui::Ui, w: &mut Wishes) -> bool {
    let before = w.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Your toys").strong());
        ui.checkbox(&mut w.vibrators, "📳 Vibrators");
        ui.checkbox(&mut w.strokers, "↕ Strokers");
    });
    // One of them at least.
    if !w.vibrators && !w.strokers {
        if before.vibrators {
            w.strokers = true;
        } else {
            w.vibrators = true;
        }
    }
    ui.add_space(6.0);
    ui.label(RichText::new("What should you feel?").strong());
    let screen_before = w.needs_screen();
    ui.columns(3, |columns| {
        let groups = [
            (FeelKind::Moment, "When something happens", None),
            (FeelKind::Background, "All the time", None),
            (FeelKind::Genre, "Depending on the game", Some("Left out by the assistant where the game has none.")),
        ];
        for (ui, (kind, title, note)) in columns.iter_mut().zip(groups) {
            ui.label(muted(title).size(12.5));
            for feel in FEELS.iter().filter(|f| f.kind == kind) {
                let mut on = w.wants(feel.key);
                let label = if feel.screen { format!("{} 🖥", feel.label) } else { feel.label.to_owned() };
                if ui.checkbox(&mut on, label).on_hover_text(feel.help).changed() {
                    w.set(feel.key, on);
                }
            }
            if let Some(note) = note {
                ui.label(muted(note).size(11.5));
            }
        }
    });
    // What reads the screen needs an indicator: the analysis proposes it.
    if w.needs_screen() && !screen_before {
        w.phases_first = true;
    }
    ui.add_space(6.0);
    ui.label(RichText::new("How should the assistant work?").strong());
    ui.checkbox(&mut w.phases_first, "Propose the game's phases and screen indicators first")
        .on_hover_text("Pasting its answer sets the phases up and lists the indicators to draw; then a short request asks for the script.");
    match (w.phases_first, w.needs_screen()) {
        (true, true) => ui.label(muted("   🖥 needs an indicator: the assistant says which one to draw.").size(12.0)),
        (false, true) => ui.label(RichText::new("   🖥 needs an indicator: the request asks which one to draw, and the mode works without it.").size(12.0).color(WARN)),
        _ => ui.label(muted("   You set them up in GameViber, then ask for the script.").size(12.0)),
    };
    ui.checkbox(&mut w.ask_first, "Ask me questions and propose designs before writing the script");
    let steps = match (w.phases_first, w.ask_first) {
        (true, false) => "Copy the request › paste the answer: phases set up › draw the indicators › ask for the script › paste it",
        (true, true) => "Copy the request › paste the answer: phases set up › draw the indicators › ask for the script › answer its questions › paste it",
        (false, true) => "Copy the request › answer its questions › paste the script",
        (false, false) => "Copy the request › paste the script",
    };
    ui.horizontal_wrapped(|ui| {
        ui.label(muted("What happens:").size(12.5));
        ui.label(RichText::new(steps).size(12.5).color(GAME));
    });
    *w != before
}

/// What the player asked of a mode, in one line.
fn summary(w: &Wishes, instructions: &str) -> String {
    let toys = match (w.vibrators, w.strokers) {
        (true, true) => "Vibrators and strokers",
        (false, true) => "Strokers",
        _ => "Vibrators",
    };
    let mut parts = vec![toys.to_owned()];
    parts.extend(FEELS.iter().filter(|f| w.wants(f.key)).map(|f| f.label.to_owned()));
    if w.phases_first {
        parts.push("Phases first".to_owned());
    }
    if w.ask_first {
        parts.push("Questions first".to_owned());
    }
    if let Some(first) = instructions.trim().lines().next().filter(|l| !l.is_empty()) {
        let short: String = first.chars().take(60).collect();
        parts.push(format!("+ \"{short}{}\"", if short.len() < first.len() { "…" } else { "" }));
    }
    parts.join(" · ")
}

/// A step's number (✔ once done) and title.
fn step(ui: &mut egui::Ui, n: usize, text: &str, done: bool) {
    ui.horizontal(|ui| {
        if done {
            ui.label(RichText::new("✔").strong().color(OK));
        } else {
            ui.label(RichText::new(n.to_string()).strong().color(ACCENT));
        }
        ui.label(RichText::new(text).strong().size(15.0).color(if done { MUTED } else { TEXT }));
        if done {
            ui.label(RichText::new("Done").color(OK).size(12.5));
        }
    });
}

/// Where the player is: each step, done, current or to come, and what it gave.
fn progress_bar(ui: &mut egui::Ui, s: &Shared, inputs: &Inputs, analysis: bool, draft: bool) {
    let to_draw = inputs.to_draw().count();
    let script = match (draft, &s.mode.error) {
        (true, _) => (false, "to paste".to_owned()),
        (false, Some(_)) => (true, "does not load".to_owned()),
        (false, None) => (true, "loads".to_owned()),
    };
    let steps: Vec<(&str, bool, String)> = if analysis {
        let phases = match (inputs.analysed, inputs.phases.len()) {
            (false, _) => "to paste".to_owned(),
            (true, n) => format!("{n} phase(s) set up"),
        };
        let drawn = match (inputs.analysed, to_draw) {
            (false, _) => String::new(),
            (true, 0) => "all drawn".to_owned(),
            (true, n) => format!("{n} indicator(s) to draw"),
        };
        vec![
            ("Phases and indicators", inputs.analysed, phases),
            ("Draw the indicators", inputs.analysed && to_draw == 0, drawn),
            ("Script", script.0, script.1),
        ]
    } else {
        vec![("Script", script.0, script.1)]
    };
    let current = steps.iter().position(|(_, done, _)| !done);
    card(SIDEBAR).inner_margin(Margin::symmetric(18, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            for (i, (title, done, detail)) in steps.iter().enumerate() {
                if i > 0 {
                    ui.label(muted("────"));
                }
                let (mark, color) = match (*done, current == Some(i)) {
                    (true, _) => ("✔".to_owned(), OK),
                    (false, true) => (format!("{}", i + 1), ACCENT),
                    (false, false) => (format!("{}", i + 1), IDLE),
                };
                ui.label(RichText::new(mark).strong().size(16.0).color(color));
                ui.vertical(|ui| {
                    ui.label(RichText::new(*title).strong().color(if current == Some(i) { TEXT } else { MUTED }));
                    if !detail.is_empty() {
                        ui.label(RichText::new(detail).size(12.0).color(if *done { OK } else { MUTED }));
                    }
                });
            }
            if current.is_none() {
                ui.label(RichText::new("   Ready: play to try it").color(OK));
            }
        });
    });
}
