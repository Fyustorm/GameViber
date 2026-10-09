//! The Creator's Funscripts tab: motions for strokers in the active mode's
//! package, which its script plays on an event (`funscript("name")`, docs/spec-modes.md
//! §8.5): adding them, seeing them, trying them on the toys, removing them.

use std::path::PathBuf;
use std::sync::mpsc;

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::funscript::{self, Track};
use crate::game::Game;

/// Points drawn of a funscript's curve, at most.
const CURVE_POINTS: usize = 400;

/// The files picked, read in a thread: those to add, and what to say (None: none picked).
type Import = Option<(Vec<PathBuf>, String)>;

#[derive(Default)]
pub struct State {
    import: Option<mpsc::Receiver<Import>>,
    message: Option<String>,
}

impl App {
    pub(super) fn motions_tab(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        if self.inputs_of(ui, s, game).is_none() {
            return;
        }
        self.poll_funscripts();
        ui.label(muted(
            "Motions for strokers, played by the script on an event: play(funscript(\"name\")). Strokers follow \
             them as fast as they can go; other toys feel how fast they move.",
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let full = s.funscripts.len() >= funscript::MAX_FUNSCRIPTS;
            let busy = self.motions.import.is_some();
            if ui.add_enabled(!full && !busy, primary("+ Add funscripts")).on_hover_text(".funscript files, from any funscript editor or site").clicked() {
                self.import_funscripts(ui.ctx());
            }
            if busy {
                ui.spinner();
            }
            if let Some(message) = &self.motions.message {
                ui.label(muted(message));
            }
        });
        ui.add_space(8.0);
        if s.funscripts.is_empty() {
            ui.label(muted("None yet."));
            return;
        }
        if !s.intiface.toys.iter().any(|t| t.stroker) {
            ui.label(muted("No stroker connected: trying one plays it on your toys as the speed it moves at.").size(12.5));
        }
        for (name, track) in &s.funscripts {
            let trying = s.trying.as_ref().filter(|(n, _)| n == name).map(|(_, at)| *at);
            card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("funscript(\"{name}\")")).monospace().strong());
                    ui.label(muted(format!("{:.1} s · {} points", track.duration(), track.points().len())));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remove").on_hover_text("A script still playing it no longer loads").clicked() {
                            self.send(Command::RemoveFunscript(name.clone()));
                        }
                        if trying.is_some() {
                            if ui.button("■ Stop").clicked() {
                                self.send(Command::TryFunscript(None));
                            }
                        } else if ui.button("▶ Try").on_hover_text("Plays it once on every toy").clicked() {
                            self.send(Command::TryFunscript(Some(name.clone())));
                        }
                    });
                });
                curve(ui, track, trying);
            });
            ui.add_space(6.0);
        }
    }

    /// Asks for funscript files, keeps those GameViber reads.
    fn import_funscripts(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        self.motions.import = Some(rx);
        self.motions.message = None;
        std::thread::spawn(move || {
            let outcome = match crate::platform::open_files("Add funscripts", "Funscripts", &[funscript::EXTENSION]) {
                Ok(paths) if paths.is_empty() => None,
                Ok(paths) => {
                    let (mut good, mut failed) = (Vec::new(), Vec::new());
                    for path in paths {
                        match funscript::read(&path) {
                            Ok(_) => good.push(path),
                            Err(e) => {
                                log::warn!("cannot read the funscript {}: {e}", path.display());
                                failed.push(path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()));
                            }
                        }
                    }
                    let added = good.len();
                    let message = match failed.is_empty() {
                        true => format!("{added} added."),
                        false => format!("{added} added; cannot read {}.", failed.join(", ")),
                    };
                    Some((good, message))
                }
                Err(e) => Some((Vec::new(), format!("Cannot open files: {e:#}"))),
            };
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
    }

    /// The import's outcome, once its dialog closed.
    fn poll_funscripts(&mut self) {
        let Some(rx) = &self.motions.import else { return };
        match rx.try_recv() {
            Ok(outcome) => {
                self.motions.import = None;
                if let Some((paths, message)) = outcome {
                    if !paths.is_empty() {
                        self.send(Command::AddFunscripts(paths));
                    }
                    self.motions.message = Some(message);
                }
            }
            Err(mpsc::TryRecvError::Disconnected) => self.motions.import = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
}

/// Its positions over time (bottom: 0), where the try is at.
fn curve(ui: &mut egui::Ui, track: &Track, at: Option<f64>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, RAISED);
    let duration = track.duration().max(1e-3);
    let x = |t: f64| rect.left() + rect.width() * (t / duration) as f32;
    let y = |p: f64| rect.bottom() - 4.0 - (rect.height() - 8.0) * p as f32;
    let points = track.points();
    let step = points.len().div_ceil(CURVE_POINTS).max(1);
    let line: Vec<egui::Pos2> = points.iter().step_by(step).chain(points.last()).map(|&(t, p)| egui::pos2(x(t), y(p))).collect();
    painter.add(egui::Shape::line(line, egui::Stroke::new(1.5, ACCENT)));
    if let Some(at) = at {
        painter.line_segment([egui::pos2(x(at), rect.top()), egui::pos2(x(at), rect.bottom())], egui::Stroke::new(1.5, TEXT));
    }
}
