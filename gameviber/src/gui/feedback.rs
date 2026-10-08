//! "Doesn't feel right?" page: the player answers the mode's questions
//! (`ask()`, or common complaints when it has none), tries the quick fixes
//! they offer, and picks a recorded session; GameViber replays the session
//! into the mode and builds a request for an AI assistant with all of it and
//! the earlier rounds, then applies the corrected mode pasted back.

use std::collections::BTreeMap;
use std::path::PathBuf;

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{App, Page};
use crate::config::{self, FeedbackRound, ModeEntry};
use crate::engine::{Command, Shared, RECENT_SECS};
use crate::mode::{prompt, report, ModeInfo, ModeRuntime, ParamValue, Question};
use crate::session::Session;

/// Common complaints, ticked rather than typed (all of them when the mode asks
/// nothing, below its own questions otherwise).
const PROBLEMS: [&str; 7] = [
    "Too strong overall",
    "Too weak overall",
    "Vibrates when nothing is happening (menus, cutscenes, exploration)",
    "Misses moments that should be felt",
    "Parries, dodges or special moves are not detected",
    "Reacts too late, or lasts too long",
    "Monotonous: not enough variation",
];
/// Below this width the page shows its two columns one above the other.
const TWO_COLUMNS_WIDTH: f32 = 860.0;

#[derive(Default)]
pub struct State {
    pub open: bool,
    /// Mode the answers are about; they are reset for another mode.
    mode_id: String,
    /// Question id -> answer index (checkboxes: 1 when ticked).
    answers: BTreeMap<String, usize>,
    ticked: [bool; PROBLEMS.len()],
    words: String,
    game: String,
    /// Recording to replay into the mode (None: no session).
    session: Option<PathBuf>,
    /// The last minutes were just saved: select the new recording once listed
    /// (holds the number of recordings before).
    select_new: Option<usize>,
    /// The request goes to the conversation that wrote the mode, which already
    /// has the context, rules and API.
    short: bool,
    copied: bool,
    answer: String,
    error: Option<String>,
    fix_copied: bool,
    /// Outcome shown after applying an answer.
    note: Option<String>,
}

impl App {
    pub(super) fn open_feedback(&mut self, s: &Shared) {
        let f = &mut self.feedback;
        if f.mode_id != s.mode.id {
            *f = State { mode_id: s.mode.id.clone(), game: std::mem::take(&mut f.game), short: f.short, ..State::default() };
        }
        f.open = true;
        f.note = None;
        if f.session.as_ref().is_none_or(|p| !s.recordings.iter().any(|r| r.path == *p)) {
            f.session = s.recordings.first().map(|r| r.path.clone());
        }
        if f.game.is_empty() {
            // Proton games all run as wine64-preloader: not a game name.
            f.game = s.overlay_clients.iter().map(|c| c.exe.clone()).find(|exe| !exe.starts_with("wine")).unwrap_or_default();
        }
    }

    /// The page, shown on Play in place of the mode while open.
    pub(super) fn feedback_page(&mut self, ui: &mut egui::Ui, s: &Shared) {
        if let Some(before) = self.feedback.select_new {
            if s.recordings.len() > before {
                self.feedback.session = s.recordings.first().map(|r| r.path.clone());
                self.feedback.select_new = None;
            }
        }
        let Some(info) = s.mode.info.clone() else {
            self.feedback.open = false;
            return;
        };
        ui.horizontal(|ui| {
            if ui.button(format!("⏴ Back to {}", info.name)).clicked() {
                self.feedback.open = false;
            }
            ui.add_space(8.0);
            heading(ui, &format!("{} doesn't feel right?", info.name));
        });
        ui.label(muted(
            "Say what feels wrong with a few clicks. Some answers can be fixed right away; the rest goes into a \
             request for an AI assistant, with a session you recorded replayed into the mode.",
        ));
        ui.add_space(10.0);
        let mut quick_fix = None;
        let mut save_recent = false;
        let mut copy = false;
        let mut apply = false;
        let mut left = |app: &mut Self, ui: &mut egui::Ui| {
            card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                quick_fix = app.questions_ui(ui, s, &info);
            });
            ui.add_space(12.0);
            card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                save_recent = app.session_ui(ui, s);
            });
        };
        let mut right = |app: &mut Self, ui: &mut egui::Ui| {
            card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                copy = app.request_ui(ui, s, &info);
            });
            ui.add_space(12.0);
            card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                apply = app.answer_ui(ui);
            });
        };
        egui::ScrollArea::vertical().show(ui, |ui| {
            if ui.available_width() >= TWO_COLUMNS_WIDTH {
                let gap = 16.0;
                let width = (ui.available_width() - gap) / 2.0;
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    let layout = egui::Layout::top_down(egui::Align::Min);
                    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), layout, |ui| {
                        ui.set_width(width);
                        left(self, ui);
                    });
                    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), layout, |ui| {
                        ui.set_width(width);
                        right(self, ui);
                    });
                });
            } else {
                left(self, ui);
                ui.add_space(12.0);
                right(self, ui);
            }
        });

        if save_recent {
            self.send(Command::SaveRecent);
        }
        if let Some((question, answer, value)) = quick_fix {
            self.apply_quick_fix(s, &info, &question, answer, value);
        }
        if copy {
            match self.feel_request(s) {
                Ok(request) => {
                    ui.ctx().copy_text(request);
                    self.feedback.copied = true;
                    self.feedback.error = None;
                }
                Err(e) => self.feedback.error = Some(e),
            }
        }
        if apply {
            self.apply_feedback_answer(s);
        }
    }

    /// Step 1; returns a quick fix to apply.
    fn questions_ui(&mut self, ui: &mut egui::Ui, s: &Shared, info: &ModeInfo) -> Option<(Question, usize, f64)> {
        let f = &mut self.feedback;
        let mut quick_fix = None;
        step(ui, 1, "What feels wrong?");
        ui.label(
            muted("Any answer other than the one picked at first (\"Good\") goes into the request. When a setting \
                   fixes it, you can also apply that fix now instead.")
            .size(12.5),
        );
        ui.add_space(4.0);
        for q in &info.feedback {
            let answer = f.answers.entry(q.id.clone()).or_insert(q.default);
            if let Some(fix) = question_ui(ui, q, answer, info, &s.mode.values) {
                quick_fix = Some((q.clone(), *answer, fix));
            }
        }
        let generic = |ui: &mut egui::Ui, ticked: &mut [bool; PROBLEMS.len()]| {
            for (problem, ticked) in PROBLEMS.iter().zip(ticked.iter_mut()) {
                ui.checkbox(ticked, *problem);
            }
        };
        if info.feedback.is_empty() {
            generic(ui, &mut f.ticked);
        } else {
            egui::CollapsingHeader::new("Other problems").show(ui, |ui| generic(ui, &mut f.ticked));
        }
        ui.add(
            egui::TextEdit::multiline(&mut f.words)
                .hint_text("In your own words: when it happens, what you expected to feel...")
                .desired_width(f32::INFINITY)
                .desired_rows(3),
        );
        ui.horizontal(|ui| {
            ui.label("Game");
            ui.add(egui::TextEdit::singleline(&mut f.game).hint_text("optional").desired_width(260.0));
        });
        quick_fix
    }

    /// Step 2; returns true to save the last minutes of play.
    fn session_ui(&mut self, ui: &mut egui::Ui, s: &Shared) -> bool {
        let f = &mut self.feedback;
        let mut save_recent = false;
        step(ui, 2, "Show what happened (optional)");
        ui.label(muted(format!(
            "Pick a recorded session where it felt wrong (⚑: moments you marked with {} while playing). Just \
             played it? Save the last minutes.",
            crate::gamepad::combo_text(&s.settings.mark_combo)
        )));
        let label = |path: &Option<PathBuf>| match path {
            None => "No session".to_owned(),
            Some(p) => s.recordings.iter().find(|r| r.path == *p).map_or("?".to_owned(), |r| {
                let what = r.header.game.as_deref().unwrap_or(&r.header.mode);
                let marks = match r.header.marks {
                    0 => String::new(),
                    n => format!(" · ⚑ {n}"),
                };
                format!("{} · {what} · {:.0} s{marks}", r.header.started, r.header.duration)
            }),
        };
        egui::ComboBox::from_id_salt("feedback-session")
            .width(ui.available_width().min(380.0))
            .selected_text(label(&f.session))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.session, None, "No session");
                for r in &s.recordings {
                    let path = Some(r.path.clone());
                    let text = label(&path);
                    ui.selectable_value(&mut f.session, path, text);
                }
            });
        let save = ui
            .add_enabled(f.select_new.is_none(), egui::Button::new(format!("⏺ Save the last {:.0} min", RECENT_SECS / 60.0)))
            .on_hover_text("GameViber always keeps the last minutes of play in memory");
        if save.clicked() {
            f.select_new = Some(s.recordings.len());
            save_recent = true;
        }
        save_recent
    }

    /// Step 3: what the request holds, which conversation it is for; returns
    /// true to copy it.
    fn request_ui(&mut self, ui: &mut egui::Ui, s: &Shared, info: &ModeInfo) -> bool {
        let f = &mut self.feedback;
        step(ui, 3, "Send the request to an AI assistant");
        let (problems, _) = answers(f, info);
        let nothing_said = problems.is_empty() && f.words.trim().is_empty();
        if nothing_said {
            card(PANEL).stroke(egui::Stroke::new(1.0, WARN)).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("⚠ Say what feels wrong first (step 1)").color(WARN).strong());
                ui.label(muted("Pick an answer other than \"Good\", tick a problem or describe it in your own words."));
            });
        } else {
            card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                eyebrow(ui, "The request says");
                for problem in &problems {
                    ui.label(format!("• {}", problem.replace("**", "")));
                }
                if !f.words.trim().is_empty() {
                    ui.label("• What you wrote");
                }
                match f.session.as_ref().and_then(|p| s.recordings.iter().find(|r| r.path == *p)) {
                    Some(r) => {
                        ui.label(muted(format!("With the session of {}, replayed into the mode.", r.header.started)).size(12.5));
                    }
                    None => {
                        ui.label(
                            RichText::new("No session: the assistant can only guess from the code. Pick one in step 2 if you can.")
                                .color(WARN)
                                .size(12.5),
                        );
                    }
                }
            });
        }
        ui.add_space(6.0);
        ui.radio_value(&mut f.short, false, "New conversation: with the full context (GameViber, rules, API)");
        ui.radio_value(&mut f.short, true, "The conversation that wrote the mode: shorter, it already knows the rest");
        if f.short {
            ui.label(
                muted("Without the mode's code: if it changed since that conversation (edited, or fixed elsewhere), \
                       pick a new conversation.")
                .size(12.0),
            );
        }
        ui.add_space(4.0);
        let mut copy = false;
        ui.horizontal(|ui| {
            copy = ui.add_enabled(!nothing_said, primary("📋 Copy the request")).clicked();
            if f.copied {
                let note = if f.short { "✔ Copied: paste it in that conversation" } else { "✔ Copied: paste it in a new conversation" };
                ui.label(RichText::new(note).color(OK));
            }
        });
        ui.label(muted(format!("The assistant answers in {} (Settings).", s.settings.language)).size(12.0));
        copy
    }

    /// Step 4; returns true to apply the pasted answer.
    fn answer_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let f = &mut self.feedback;
        step(ui, 4, "Paste the answer");
        ui.label(muted("If it only suggests new settings, set them on the mode's page instead."));
        let edit = egui::TextEdit::multiline(&mut f.answer)
            .code_editor()
            .hint_text("mode { api = 1, ... }")
            .desired_width(f32::INFINITY)
            .desired_rows(10);
        egui::ScrollArea::vertical().id_salt("feedback-answer").max_height(260.0).show(ui, |ui| {
            if ui.add(edit).changed() {
                f.error = None;
                f.note = None;
            }
        });
        ui.add_space(4.0);
        let apply = ui.add_enabled(!f.answer.trim().is_empty(), primary("Apply the fix")).clicked();
        if let Some(error) = &f.error {
            card(PANEL).stroke(egui::Stroke::new(1.0, DANGER)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(error).color(DANGER_TEXT).monospace().size(12.0));
                ui.horizontal(|ui| {
                    if ui.button("📋 Copy a fix request").clicked() {
                        ui.ctx().copy_text(prompt::fix_prompt(error));
                        f.fix_copied = true;
                    }
                    let note = if f.fix_copied {
                        "✔ Copied: send it to the assistant, then paste its new answer."
                    } else {
                        "and send it to the assistant."
                    };
                    ui.label(muted(note));
                });
            });
        }
        if let Some(note) = &f.note {
            ui.label(RichText::new(note).color(OK));
        }
        apply
    }

    /// Sets the parameter a question is linked to, notes it in the mode's history
    /// and puts the question back to its "fine" answer.
    fn apply_quick_fix(&mut self, s: &Shared, info: &ModeInfo, q: &Question, answer: usize, value: f64) {
        let Some(name) = &q.param else { return };
        let Some(def) = info.params.iter().find(|p| p.name == *name) else { return };
        let from = match s.mode.values.get(name).unwrap_or(&def.default) {
            ParamValue::Number(n) => *n,
            _ => return,
        };
        self.send(Command::SetParam(name.clone(), ParamValue::Number(value)));
        log::info!("quick fix: {} {from} -> {value} ({}: {})", def.label, q.label, q.options[answer]);
        let entry = ModeEntry::from_id(&s.mode.id);
        let mut history = entry.load_feedback();
        history.push(FeedbackRound::QuickFix {
            date: crate::platform::local_time(),
            version: info.version.clone(),
            question: q.label.clone(),
            answer: q.options[answer].clone(),
            setting: def.label.clone(),
            from,
            to: value,
        });
        entry.save_feedback(&history);
        let f = &mut self.feedback;
        f.answers.insert(q.id.clone(), q.default);
        f.note = Some(format!("✔ {} set to {value}: play a bit, then come back if it still feels off.", def.label));
    }

    /// The request for the AI assistant, with the session replayed into the mode;
    /// records the round in the mode's history.
    fn feel_request(&self, s: &Shared) -> Result<String, String> {
        let f = &self.feedback;
        let info = s.mode.info.as_ref().ok_or("no mode loaded")?;
        let entry = ModeEntry::from_id(&s.mode.id);
        let source = entry.source().map_err(|e| format!("cannot read the mode: {e}"))?;
        let session = match &f.session {
            Some(path) => {
                let session = Session::open(path).map_err(|e| format!("cannot read the session: {e:#}"))?;
                let sim = report::simulate(&entry.chunk_name(), &source, &s.mode.values, &crate::package::Inputs::of(&entry), session, &Default::default())?;
                Some(sim.report())
            }
            None => None,
        };
        let (problems, settings) = answers(f, info);
        let answers = problems.clone();
        // The assistant also gets the setting each answer is about.
        let mut problems: Vec<String> = problems
            .into_iter()
            .enumerate()
            .map(|(i, p)| settings.get(&i).map_or(p.clone(), |setting| format!("{p} (setting `{setting}`)")))
            .collect();
        if !f.words.trim().is_empty() {
            problems.push(format!("In the player's words: {}", f.words.trim()));
        }
        let mut history = entry.load_feedback();
        let earlier = history.lines();
        let described = s.mode_inputs.as_ref().map(|i| i.describe());
        let request = prompt::feel_prompt(&prompt::Templates::load(), &prompt::FeelReport {
            name: &info.name,
            game: &f.game,
            problems: &problems,
            history: &earlier,
            params: &info.params,
            values: &s.mode.values,
            source: &source,
            session: session.as_deref(),
            full: !f.short,
            language: &s.settings.language,
            inputs: described.as_deref(),
        });
        history.push(FeedbackRound::Request {
            date: crate::platform::local_time(),
            version: info.version.clone(),
            answers: answers.iter().map(|a| a.replace("**", "")).collect(),
            words: f.words.trim().to_owned(),
        });
        entry.save_feedback(&history);
        Ok(request)
    }

    /// Replaces the active mode by the corrected one: in place for a user mode
    /// (keeping a .bak copy), as a new mode for a built-in one.
    fn apply_feedback_answer(&mut self, s: &Shared) {
        let f = &mut self.feedback;
        if !prompt::has_script(&f.answer) {
            f.error = None;
            f.note = Some("No mode code in this answer: set the values it suggests on the mode's page.".into());
            return;
        }
        let script = prompt::extract_script(&f.answer);
        let fixed = match ModeRuntime::probe("fixed", &script) {
            Ok(info) => info,
            Err(e) => {
                f.error = Some(e);
                f.fix_copied = false;
                return;
            }
        };
        let entry = ModeEntry::from_id(&s.mode.id);
        let mut history = entry.load_feedback();
        history.push(FeedbackRound::Fixed {
            date: crate::platform::local_time(),
            from_version: s.mode.info.as_ref().map(|i| i.version.clone()).unwrap_or_default(),
            to_version: fixed.version.clone(),
        });
        match entry.path() {
            Some(path) => {
                let backup = path.with_extension("luau.bak");
                if let Err(e) = std::fs::copy(&path, &backup).and_then(|_| config::write_file(&path, &script)) {
                    f.error = Some(format!("cannot save the mode: {e}"));
                    return;
                }
                log::info!("{} fixed (previous version in {})", path.display(), backup.display());
                entry.save_feedback(&history);
                f.answers.clear();
                f.ticked = Default::default();
                f.words.clear();
                f.answer.clear();
                f.error = None;
                f.note = Some(format!("✔ Applied. The previous version is kept in {}.", backup.display()));
                self.send(Command::ReloadMode);
            }
            None => {
                *f = State { game: std::mem::take(&mut f.game), short: f.short, ..State::default() };
                // The history follows the tuned copy.
                let game = s.game.as_ref().filter(|g| g.modes.contains(&entry.id)).map(|g| g.id.clone());
                if let Some(copy) = self.create_mode(&format!("{}-tuned", entry.key), &script, Some(&entry.id), game.as_deref()) {
                    copy.save_feedback(&history);
                }
                // The mode's page, within its game.
                self.page = Page::Library;
            }
        }
    }
}

/// The answers away from "fine" then the common complaints ticked, and the
/// setting linked to each problem (by index).
fn answers(f: &State, info: &ModeInfo) -> (Vec<String>, BTreeMap<usize, String>) {
    let (mut problems, mut settings) = (Vec::new(), BTreeMap::new());
    for q in &info.feedback {
        let answer = f.answers.get(&q.id).copied().unwrap_or(q.default);
        match (q.options.is_empty(), answer == q.default) {
            (_, true) => {}
            (true, false) => problems.push(q.label.clone()),
            (false, false) => {
                problems.push(format!("{}: **{}**", q.label, q.options[answer]));
                if let Some(p) = &q.param {
                    settings.insert(problems.len() - 1, p.clone());
                }
            }
        }
    }
    problems.extend(PROBLEMS.iter().zip(f.ticked).filter(|(_, ticked)| *ticked).map(|(p, _)| p.to_string()));
    (problems, settings)
}

/// One of the mode's questions; returns the quick fix value when its button is clicked.
fn question_ui(
    ui: &mut egui::Ui,
    q: &Question,
    answer: &mut usize,
    info: &ModeInfo,
    values: &BTreeMap<String, ParamValue>,
) -> Option<f64> {
    if q.options.is_empty() {
        let mut ticked = *answer == 1;
        ui.checkbox(&mut ticked, &q.label);
        *answer = ticked as usize;
        return None;
    }
    ui.add_space(2.0);
    ui.label(&q.label);
    ui.horizontal_wrapped(|ui| {
        for (i, option) in q.options.iter().enumerate() {
            if ui.selectable_label(*answer == i, option).clicked() {
                *answer = i;
            }
        }
    });
    let fix = q.param.as_ref().and_then(|name| info.params.iter().find(|p| p.name == *name)).and_then(|def| {
        let current = match values.get(&def.name).unwrap_or(&def.default) {
            ParamValue::Number(n) => *n,
            _ => return None,
        };
        Some((def, current, q.quick_fix(*answer, def, current)?))
    });
    let Some((def, current, value)) = fix else {
        if *answer != q.default {
            ui.label(RichText::new("In the request.").color(ACCENT_TEXT).size(12.5));
        }
        return None;
    };
    let mut apply = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("In the request. Or fix it now: {} {current} → {value}", def.label)).color(ACCENT_TEXT).size(12.5));
        apply = ui.small_button("Apply now").on_hover_text("Changes the setting and takes the answer out of the request").clicked();
    });
    apply.then_some(value)
}

fn step(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(n.to_string()).strong().color(ACCENT));
        ui.label(RichText::new(text).strong());
    });
}
