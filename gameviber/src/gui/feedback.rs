//! "Doesn't feel right?" dialog: the player says what feels wrong with the
//! active mode and picks a recorded session; GameViber replays the session
//! into the mode and builds a request for an AI assistant with all of it,
//! then applies the corrected mode pasted back.

use std::path::PathBuf;

use eframe::egui::{self, RichText};

use super::theme::*;
use super::{App, Page};
use crate::config::{self, ModeEntry};
use crate::engine::{Command, Shared, RECENT_SECS};
use crate::mode::{prompt, report, ModeRuntime};
use crate::session::Session;

/// Common complaints, ticked rather than typed.
const PROBLEMS: [&str; 7] = [
    "Too strong overall",
    "Too weak overall",
    "Vibrates when nothing is happening (menus, cutscenes, exploration)",
    "Misses moments that should be felt",
    "Parries, dodges or special moves are not detected",
    "Reacts too late, or lasts too long",
    "Monotonous: not enough variation",
];

#[derive(Default)]
pub struct State {
    pub open: bool,
    ticked: [bool; PROBLEMS.len()],
    words: String,
    game: String,
    /// Recording to replay into the mode (None: no session).
    session: Option<PathBuf>,
    /// The last minutes were just saved: select the new recording once listed
    /// (holds the number of recordings before).
    select_new: Option<usize>,
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
        f.open = true;
        f.note = None;
        if f.session.as_ref().is_none_or(|p| !s.recordings.iter().any(|r| r.path == *p)) {
            f.session = s.recordings.first().map(|r| r.path.clone());
        }
        if f.game.is_empty() {
            f.game = s.overlay_clients.first().map(|c| c.exe.clone()).unwrap_or_default();
        }
    }

    pub(super) fn feedback_ui(&mut self, ctx: &egui::Context, s: &Shared) {
        if !self.feedback.open {
            return;
        }
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
        let mut copy = false;
        let mut apply = false;
        let mut save_recent = false;
        let modal = egui::Modal::new(egui::Id::new("mode-feedback")).show(ctx, |ui| {
            ui.set_width(600.0);
            let f = &mut self.feedback;
            heading(ui, &format!("{} doesn't feel right?", info.name));
            ui.label(muted(
                "Tell what feels wrong. GameViber replays a session you recorded into the mode and prepares a \
                 request for an AI assistant with everything it needs to fix it; you paste the answer back.",
            ));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(ctx.content_rect().height() * 0.7).show(ui, |ui| {
                step(ui, 1, "What feels wrong?");
                for (problem, ticked) in PROBLEMS.iter().zip(f.ticked.iter_mut()) {
                    ui.checkbox(ticked, *problem);
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
                ui.add_space(10.0);

                step(ui, 2, "Show what happened");
                ui.label(muted(
                    "Pick a recorded session where it felt wrong. Just played it? Save the last minutes.",
                ));
                ui.horizontal(|ui| {
                    let label = |path: &Option<PathBuf>| match path {
                        None => "No session".to_owned(),
                        Some(p) => s.recordings.iter().find(|r| r.path == *p).map_or("?".to_owned(), |r| {
                            let what = r.header.game.as_deref().unwrap_or(&r.header.mode);
                            format!("{} · {what} · {:.0} s", r.header.started, r.header.duration)
                        }),
                    };
                    egui::ComboBox::from_id_salt("feedback-session").width(330.0).selected_text(label(&f.session)).show_ui(
                        ui,
                        |ui| {
                            ui.selectable_value(&mut f.session, None, "No session");
                            for r in &s.recordings {
                                let path = Some(r.path.clone());
                                let text = label(&path);
                                ui.selectable_value(&mut f.session, path, text);
                            }
                        },
                    );
                    let save = ui
                        .add_enabled(f.select_new.is_none(), egui::Button::new(format!("⏺ Save the last {:.0} min", RECENT_SECS / 60.0)))
                        .on_hover_text("GameViber always keeps the last minutes of play in memory");
                    if save.clicked() {
                        f.select_new = Some(s.recordings.len());
                        save_recent = true;
                    }
                });
                ui.add_space(10.0);

                step(ui, 3, "Send the request to an AI assistant");
                ui.horizontal(|ui| {
                    copy = ui.add(primary("📋 Copy the request")).clicked();
                    if f.copied {
                        ui.label(RichText::new("✔ Copied: paste it in a new conversation").color(OK));
                    }
                });
                ui.label(muted("The conversation that wrote the mode works best, if you still have it.").size(12.0));
                ui.add_space(10.0);

                step(ui, 4, "Paste the answer");
                ui.label(muted("If it only suggests new settings, set them on the Play page instead."));
                let edit = egui::TextEdit::multiline(&mut f.answer)
                    .code_editor()
                    .hint_text("mode { api = 1, ... }")
                    .desired_width(f32::INFINITY)
                    .desired_rows(6);
                if ui.add(edit).changed() {
                    f.error = None;
                    f.note = None;
                }
                if let Some(error) = &f.error {
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
                }
                if let Some(note) = &f.note {
                    ui.label(RichText::new(note).color(OK));
                }
            });
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                apply = ui.add_enabled(!f.answer.trim().is_empty(), primary("Apply the fix")).clicked();
                if ui.button("Close").clicked() {
                    f.open = false;
                }
            });
        });
        if modal.should_close() {
            self.feedback.open = false;
        }
        if save_recent {
            self.send(Command::SaveRecent);
        }
        if copy {
            match self.feel_request(s) {
                Ok(request) => {
                    ctx.copy_text(request);
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

    /// The request for the AI assistant, with the session replayed into the mode.
    fn feel_request(&self, s: &Shared) -> Result<String, String> {
        let f = &self.feedback;
        let info = s.mode.info.as_ref().ok_or("no mode loaded")?;
        let entry = ModeEntry::from_id(&s.mode.id);
        let source = entry.source().map_err(|e| format!("cannot read the mode: {e}"))?;
        let session = match &f.session {
            Some(path) => {
                let session = Session::open(path).map_err(|e| format!("cannot read the session: {e:#}"))?;
                let sim = report::simulate(&entry.chunk_name(), &source, &s.mode.values, session)?;
                Some(sim.report())
            }
            None => None,
        };
        let mut problems: Vec<String> =
            PROBLEMS.iter().zip(f.ticked).filter(|(_, ticked)| *ticked).map(|(p, _)| p.to_string()).collect();
        if !f.words.trim().is_empty() {
            problems.push(format!("In the player's words: {}", f.words.trim()));
        }
        Ok(prompt::feel_prompt(&prompt::FeelReport {
            name: &info.name,
            game: &f.game,
            problems: &problems,
            params: &info.params,
            values: &s.mode.values,
            source: &source,
            session: session.as_deref(),
        }))
    }

    /// Replaces the active mode by the corrected one: in place for a user mode
    /// (keeping a .bak copy), as a new mode for a built-in one.
    fn apply_feedback_answer(&mut self, s: &Shared) {
        let f = &mut self.feedback;
        if !prompt::has_script(&f.answer) {
            f.error = None;
            f.note = Some("No mode code in this answer: set the values it suggests on the Play page.".into());
            return;
        }
        let script = prompt::extract_script(&f.answer);
        if let Err(e) = ModeRuntime::probe("fixed", &script) {
            f.error = Some(e);
            f.fix_copied = false;
            return;
        }
        let entry = ModeEntry::from_id(&s.mode.id);
        match entry.path() {
            Some(path) => {
                let backup = path.with_extension("luau.bak");
                if let Err(e) = std::fs::copy(&path, &backup).and_then(|_| config::write_file(&path, &script)) {
                    f.error = Some(format!("cannot save the mode: {e}"));
                    return;
                }
                log::info!("{} fixed (previous version in {})", path.display(), backup.display());
                f.answer.clear();
                f.error = None;
                f.note = Some(format!("✔ Applied. The previous version is kept in {}.", backup.display()));
                self.send(Command::ReloadMode);
            }
            None => {
                *f = State { game: std::mem::take(&mut f.game), ..State::default() };
                self.create_mode(&format!("{}-tuned", entry.key), &script);
                self.page = Page::Play;
                self.play.show_user_modes();
            }
        }
    }
}

fn step(ui: &mut egui::Ui, n: usize, text: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(n.to_string()).strong().color(ACCENT));
        ui.label(RichText::new(text).strong());
    });
}
