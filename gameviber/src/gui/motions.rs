//! The Creator's Funscripts tab: motions for strokers in the active mode's
//! package, which its script plays on an event (`funscript("name")`, docs/spec-modes.md
//! §8.5): adding them, seeing them, trying them on the toys, renaming and removing
//! them, and a simple editor to write one or make one of a part of another. Each
//! segment is colored for how fast it moves, against what the strokers connected
//! can do.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use eframe::egui::{self, Color32, Margin, RichText, Sense, Stroke, Vec2};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::funscript::{self, Track};
use crate::game::Game;
use crate::stroke::{MotionLimits, StrokeSettings};

/// Points drawn of a funscript's curve, at most.
const CURVE_POINTS: usize = 400;
/// Points shown as dots in the editor while fewer are in view.
const DOTS: usize = 300;
/// Distance to a dot it can be picked from, in pixels.
const PICK: f32 = 8.0;
/// Undo steps kept.
const UNDO: usize = 50;
/// A point this close to the cursor (s) is at it.
const CURSOR_TOLERANCE: f64 = 0.0005;
const RENAME_NOTE: &str = "Scripts calling it by its old name no longer load until they use the new one.";

/// The files picked, read in a thread: those to add, and what to say (None: none picked).
type Import = Option<(Vec<PathBuf>, String)>;

#[derive(Default)]
pub struct State {
    import: Option<mpsc::Receiver<Import>>,
    message: Option<String>,
    /// A funscript being renamed in the list, and the name being typed.
    renaming: Option<(String, String)>,
    editor: Option<Editor>,
}

/// Points being dragged: as they were, and where the pointer took them from (time, position).
type Drag = (Vec<(f64, f64)>, f64, f64);

/// A funscript being written: its points, what is shown and selected.
struct Editor {
    /// The mode whose funscript it is: another one active closes it.
    mode: String,
    /// The funscript it was opened from (None: a new one).
    original: Option<String>,
    name: String,
    /// (time in s, position 0..1), sorted.
    points: Vec<(f64, f64)>,
    undo: Vec<Vec<(f64, f64)>>,
    dirty: bool,
    /// The seconds shown: from, how many.
    view: (f64, f64),
    cursor: f64,
    /// The part of the timeline selected: from, to (s).
    selection: Option<(f64, f64)>,
    /// The points selected, by index.
    picked: BTreeSet<usize>,
    drag: Option<Drag>,
    /// A box being drawn to select points: where it started, where the pointer is.
    boxing: Option<(egui::Pos2, egui::Pos2)>,
    /// Where a part of the timeline being dragged started.
    selecting: Option<f64>,
    /// A click on the curve adds a point there (else it only moves the cursor).
    click_adds: bool,
    /// Strokes to insert: how many, a half-stroke's seconds, low and high positions (%).
    strokes: (usize, f64, f64, f64),
    /// How far ← and → move the cursor, in milliseconds (×10 with Shift), without the grid.
    step: f64,
    /// Times snap to the grid: the cursor, points added or moved, the selection;
    /// ← and → then move a grid step.
    snap: bool,
    /// The grid's step, in milliseconds.
    grid: f64,
    /// Name of the funscript to make of the selection.
    extract: String,
    /// Where a try started in the funscript, to show where it is.
    tried_from: f64,
    /// Closing with changes not saved was asked.
    closing: bool,
}

impl Editor {
    fn new(mode: &str, original: Option<String>, name: String, points: Vec<(f64, f64)>) -> Self {
        let length = points.last().map_or(10.0, |p| p.0.max(1.0) * 1.05);
        Self {
            mode: mode.to_owned(),
            original,
            name,
            points,
            undo: Vec::new(),
            dirty: false,
            view: (0.0, length),
            cursor: 0.0,
            selection: None,
            picked: BTreeSet::new(),
            drag: None,
            boxing: None,
            selecting: None,
            click_adds: false,
            strokes: (4, 0.4, 10.0, 90.0),
            step: 100.0,
            snap: false,
            grid: 100.0,
            extract: String::new(),
            tried_from: 0.0,
            closing: false,
        }
    }

    /// Keeps the points as they are for undo, before changing them.
    fn change(&mut self) {
        self.undo.push(self.points.clone());
        if self.undo.len() > UNDO {
            self.undo.remove(0);
        }
        self.dirty = true;
    }

    /// Replaces the points (the indices of those selected no longer hold).
    fn set(&mut self, points: Vec<(f64, f64)>) {
        self.change();
        self.points = points;
        self.picked.clear();
    }

    fn fit(&mut self) {
        self.view = (0.0, self.points.last().map_or(10.0, |p| p.0.max(1.0) * 1.05));
    }

    fn track(&self) -> Result<Track, String> {
        Track::new(self.points.clone())
    }

    /// What a try plays: the selection, else from the cursor to the end; with where it starts.
    fn part(&self) -> Option<(f64, Track)> {
        let (from, to) = self.selection.unwrap_or((self.cursor, self.points.last().map_or(0.0, |p| p.0)));
        let part = Track::new(funscript::slice(&self.points, from, to)).ok();
        part.filter(|_| to > from).map(|part| (from, part))
    }

    /// The grid's step in seconds, while times snap to it.
    fn grid(&self) -> Option<f64> {
        self.snap.then_some(self.grid / 1000.0)
    }

    /// The point at the cursor, if any.
    fn at_cursor(&self) -> Option<usize> {
        self.points.iter().position(|p| (p.0 - self.cursor).abs() < CURSOR_TOLERANCE)
    }

    /// Keeps the cursor in view.
    fn follow(&mut self) {
        let (from, length) = self.view;
        if self.cursor < from || self.cursor > from + length {
            self.view.0 = (self.cursor - length * 0.2).max(0.0);
        }
    }

    /// Editing with the keyboard, as in usual funscript editors (unless a text field
    /// has it); says whether Space asked to try or stop.
    fn keys(&mut self, ui: &egui::Ui) -> bool {
        use egui::{Key, Modifiers};
        if ui.memory(|m| m.focused().is_some()) {
            return false;
        }
        const DIGITS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
        let pressed = |key, modifiers| ui.ctx().input_mut(|i| i.consume_key(modifiers, key));
        for (digit, key) in DIGITS.into_iter().enumerate() {
            if pressed(key, Modifiers::NONE) {
                let position = digit as f64 / 9.0;
                self.cursor = snapped(self.cursor, self.grid());
                self.change();
                match self.at_cursor() {
                    Some(i) => self.points[i].1 = position,
                    None => {
                        let i = self.points.partition_point(|p| p.0 < self.cursor);
                        self.points.insert(i, (self.cursor, position));
                        self.picked.clear();
                    }
                }
            }
        }
        let step = self.grid().unwrap_or(self.step / 1000.0);
        // Shift first: without it, a key matches with Shift held too.
        for (key, by) in [(Key::ArrowLeft, -step), (Key::ArrowRight, step)] {
            if pressed(key, Modifiers::SHIFT) {
                self.cursor = snapped((self.cursor + 10.0 * by).max(0.0), self.grid());
            }
            if pressed(key, Modifiers::NONE) {
                self.cursor = snapped((self.cursor + by).max(0.0), self.grid());
            }
        }
        if pressed(Key::ArrowUp, Modifiers::NONE) {
            if let Some(p) = self.points.iter().find(|p| p.0 > self.cursor + CURSOR_TOLERANCE) {
                self.cursor = p.0;
            }
        }
        if pressed(Key::ArrowDown, Modifiers::NONE) {
            if let Some(p) = self.points.iter().rev().find(|p| p.0 < self.cursor - CURSOR_TOLERANCE) {
                self.cursor = p.0;
            }
        }
        if pressed(Key::Escape, Modifiers::NONE) {
            self.picked.clear();
            self.selection = None;
        }
        if pressed(Key::A, Modifiers::COMMAND) {
            self.picked = (0..self.points.len()).collect();
        }
        if pressed(Key::Delete, Modifiers::NONE) || pressed(Key::Backspace, Modifiers::NONE) {
            match (self.selection, self.at_cursor()) {
                _ if !self.picked.is_empty() => {
                    let points = self.points.iter().enumerate().filter(|(i, _)| !self.picked.contains(i)).map(|(_, p)| *p).collect();
                    self.set(points);
                }
                (Some((from, to)), _) => {
                    let points = funscript::remove(&self.points, from, to, false);
                    self.set(points);
                }
                (None, Some(i)) => {
                    self.change();
                    self.points.remove(i);
                    self.picked.clear();
                }
                (None, None) => {}
            }
        }
        if pressed(Key::Z, Modifiers::COMMAND) {
            undo(self);
        }
        self.follow();
        pressed(Key::Space, Modifiers::NONE)
    }
}

/// What a name typed for a funscript is worth: None if it can be used.
fn name_problem(name: &str, s: &Shared, own: Option<&str>) -> Option<&'static str> {
    if !funscript::valid_name(name) {
        Some("1 to 48 of a-z, 0-9, _ and -")
    } else if s.funscripts.contains_key(name) && own != Some(name) {
        Some("another funscript has this name")
    } else {
        None
    }
}

impl App {
    pub(super) fn motions_tab(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        if self.inputs_of(ui, s, game).is_none() {
            return;
        }
        self.poll_funscripts();
        if self.motions.editor.is_some() {
            self.funscript_editor(ui, s);
            return;
        }
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
            if ui.add_enabled(!full, egui::Button::new("✏ New")).on_hover_text("Write one: points, strokes").clicked() {
                let name = (1..).map(|n| format!("motion_{n}")).find(|n| !s.funscripts.contains_key(n)).unwrap_or_default();
                self.motions.editor = Some(Editor::new(&s.mode.id, None, name, Vec::new()));
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
        let (limits, _) = limits(s);
        for (name, track) in &s.funscripts {
            let trying = s.trying.as_ref().filter(|(n, _)| n == name).map(|(_, at)| *at);
            card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.motions.renaming.as_ref().is_some_and(|(n, _)| n == name) {
                    self.rename_row(ui, s, name);
                } else {
                    self.funscript_row(ui, &s.mode.id, name, track, trying);
                }
                curve(ui, track, trying, &limits);
            });
            ui.add_space(6.0);
        }
    }

    fn funscript_row(&mut self, ui: &mut egui::Ui, mode: &str, name: &str, track: &Arc<Track>, trying: Option<f64>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("funscript(\"{name}\")")).monospace().strong());
            ui.label(muted(format!("{:.1} s · {} points", track.duration(), track.points().len())));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Remove").on_hover_text("A script still playing it no longer loads").clicked() {
                    self.send(Command::RemoveFunscript(name.to_owned()));
                }
                if ui.button("Rename").clicked() {
                    self.motions.renaming = Some((name.to_owned(), name.to_owned()));
                }
                if ui.button("✏ Edit").on_hover_text("Change it, or make a funscript of a part of it").clicked() {
                    self.motions.editor = Some(Editor::new(mode, Some(name.to_owned()), name.to_owned(), track.points().to_vec()));
                }
                if trying.is_some() {
                    if ui.button("■ Stop").clicked() {
                        self.send(Command::TryFunscript(None));
                    }
                } else if ui.button("▶ Try").on_hover_text("Plays it once on every toy").clicked() {
                    self.send(Command::TryFunscript(Some((name.to_owned(), track.clone()))));
                }
            });
        });
    }

    fn rename_row(&mut self, ui: &mut egui::Ui, s: &Shared, name: &str) {
        let Some((_, typed)) = self.motions.renaming.as_mut() else { return };
        let mut done = None;
        ui.horizontal(|ui| {
            ui.label(muted("Rename"));
            let field = ui.add(egui::TextEdit::singleline(typed).desired_width(220.0).font(egui::TextStyle::Monospace));
            let problem = name_problem(typed, s, Some(name));
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.add_enabled(problem.is_none(), primary("OK")).clicked() || (enter && problem.is_none()) {
                done = Some(Some(typed.clone()));
            }
            if ui.button("Cancel").clicked() {
                done = Some(None);
            }
            if let Some(problem) = problem {
                ui.label(RichText::new(problem).size(12.0).color(WARN));
            }
        });
        ui.label(muted(RENAME_NOTE).size(12.0));
        if let Some(to) = done {
            self.motions.renaming = None;
            if let Some(to) = to.filter(|to| to != name) {
                self.send(Command::RenameFunscript { from: name.to_owned(), to });
            }
        }
    }

    /// The editor: its name and saving, its tools, the curve, the selection.
    fn funscript_editor(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let Some(mut e) = self.motions.editor.take().filter(|e| e.mode == s.mode.id) else { return };
        let mut close = false;
        let mut commands = Vec::new();

        // Name and saving.
        ui.horizontal(|ui| {
            if ui.button("‹ Funscripts").clicked() {
                if e.dirty {
                    e.closing = true;
                } else {
                    close = true;
                }
            }
            ui.label(muted("Name"));
            ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(200.0).font(egui::TextStyle::Monospace));
            let problem = name_problem(&e.name, s, e.original.as_deref());
            let track = e.track();
            let renamed = e.original.as_deref() != Some(e.name.as_str());
            let can_save = problem.is_none() && track.is_ok() && (e.dirty || renamed);
            if ui.add_enabled(can_save, primary("Save")).on_hover_text("The mode reloads with it").clicked() {
                if let Ok(track) = &track {
                    commands.push(Command::SaveFunscript { name: e.name.clone(), track: Arc::new(track.clone()), replaces: e.original.clone() });
                    e.original = Some(e.name.clone());
                    e.dirty = false;
                }
            }
            match (problem, &track) {
                (Some(problem), _) => {
                    ui.label(RichText::new(problem).size(12.0).color(WARN));
                }
                (None, Err(_)) => {
                    ui.label(RichText::new("at least two points at different times").size(12.0).color(WARN));
                }
                (None, Ok(_)) if renamed && e.original.is_some() => {
                    ui.label(muted(RENAME_NOTE).size(12.0));
                }
                _ => {}
            }
        });
        if e.closing {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Changes not saved.").color(WARN));
                if ui.button("Discard them").clicked() {
                    close = true;
                }
                if ui.button("Keep editing").clicked() {
                    e.closing = false;
                }
            });
        }
        ui.add_space(6.0);

        // Tools.
        let trying = s.trying.as_ref().filter(|(n, _)| *n == e.name).map(|(_, at)| e.tried_from + at);
        let mut try_now = e.keys(ui);
        ui.horizontal(|ui| {
            if ui.add_enabled(!e.undo.is_empty(), egui::Button::new("↶ Undo")).on_hover_text("Ctrl+Z").clicked() {
                undo(&mut e);
            }
            if ui.button("Fit").on_hover_text("Shows it whole").clicked() {
                e.fit();
            }
            ui.separator();
            if trying.is_some() {
                try_now |= ui.button("■ Stop").on_hover_text("Space").clicked();
            } else {
                let label = if e.selection.is_some() { "▶ Try the selection" } else { "▶ Try from the cursor" };
                try_now |= ui.add_enabled(e.part().is_some(), egui::Button::new(label)).on_hover_text("Plays it once on every toy (Space)").clicked();
            }
            ui.separator();
            ui.checkbox(&mut e.click_adds, "Click adds points").on_hover_text("Else a click only moves the cursor: 0 to 9 add points there");
            ui.separator();
            ui.checkbox(&mut e.snap, "Grid").on_hover_text("Times snap to it: the cursor, points added or moved, the selection");
            if e.snap {
                ui.add(egui::DragValue::new(&mut e.grid).range(5.0..=10000.0).speed(1.0).suffix(" ms"))
                    .on_hover_text(format!("{:.0} steps a minute; ← and → move one step (×10 with Shift)", 60000.0 / e.grid));
            } else {
                ui.label(muted("Step"));
                ui.add(egui::DragValue::new(&mut e.step).range(1.0..=5000.0).speed(1.0).suffix(" ms")).on_hover_text("How far ← and → move the cursor (×10 with Shift)");
            }
            ui.separator();
            ui.label(muted(format!("cursor {:.3} s · {} points · {:.2} s", e.cursor, e.points.len(), e.points.last().map_or(0.0, |p| p.0))));
            if !e.picked.is_empty() {
                ui.separator();
                ui.label(RichText::new(format!("{} picked", e.picked.len())).strong());
                if ui.small_button("Delete").on_hover_text("Delete").clicked() {
                    let points = e.points.iter().enumerate().filter(|(i, _)| !e.picked.contains(i)).map(|(_, p)| *p).collect();
                    e.set(points);
                }
            }
        });
        if try_now {
            match (trying, e.part()) {
                (Some(_), _) => commands.push(Command::TryFunscript(None)),
                (None, Some((from, part))) => {
                    e.tried_from = from;
                    commands.push(Command::TryFunscript(Some((e.name.clone(), Arc::new(part)))));
                }
                (None, None) => {}
            }
        }
        ui.add_space(4.0);
        let (limits, toys) = limits(s);
        canvas(ui, &mut e, trying, &limits);
        speed_legend(ui, &limits, &toys);
        ui.label(
            muted("Click: the cursor, or pick a point · drag: pick the points in a box · drag picked points: move them · Shift: add to the points picked · right-click a point: remove it · drag on the timeline: select a part · Ctrl+wheel: zoom · Shift+wheel: scroll")
                .size(11.5),
        );
        ui.label(
            muted("0 to 9: a point at the cursor, 0% to 100% · ← →: move the cursor a step (Shift: 10) · ↑ ↓: next, previous point · Delete: the points picked, else the part's, else the one at the cursor · Ctrl+A: pick all · Esc: pick none · Space: try, stop · Ctrl+Z: undo")
                .size(11.5),
        );
        ui.add_space(8.0);

        // The selection.
        if let Some((from, to)) = e.selection {
            card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Selection {from:.2} s – {to:.2} s ({:.2} s)", to - from)).strong());
                    if ui.button("Delete its points").clicked() {
                        let points = funscript::remove(&e.points, from, to, false);
                        e.set(points);
                    }
                    if ui.button("Cut it out").on_hover_text("Removes it, what follows comes earlier").clicked() {
                        let points = funscript::remove(&e.points, from, to, true);
                        e.set(points);
                        e.selection = None;
                    }
                    if ui.button("Clear").clicked() {
                        e.selection = None;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Make a funscript of it:");
                    ui.add(egui::TextEdit::singleline(&mut e.extract).desired_width(200.0).hint_text("name").font(egui::TextStyle::Monospace));
                    let problem = name_problem(&e.extract, s, None).filter(|_| !e.extract.is_empty());
                    let part = Track::new(funscript::slice(&e.points, from, to));
                    let full = s.funscripts.len() >= funscript::MAX_FUNSCRIPTS;
                    let ok = problem.is_none() && !e.extract.is_empty() && part.is_ok() && !full;
                    if ui.add_enabled(ok, primary("Extract")).on_hover_text("A new funscript starting at 0; this one is not changed").clicked() {
                        if let Ok(part) = part {
                            commands.push(Command::SaveFunscript { name: e.extract.clone(), track: Arc::new(part), replaces: None });
                            e.extract.clear();
                        }
                    }
                    if let Some(problem) = problem {
                        ui.label(RichText::new(problem).size(12.0).color(WARN));
                    }
                });
            });
            ui.add_space(6.0);
        }

        // Strokes at the cursor.
        card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let (count, half, low, high) = &mut e.strokes;
                ui.label(format!("At the cursor ({:.2} s):", e.cursor));
                ui.add(egui::DragValue::new(count).range(1..=200).suffix(" strokes"));
                ui.label("of");
                ui.add(egui::DragValue::new(half).range(0.05..=10.0).speed(0.01).fixed_decimals(2).suffix(" s"))
                    .on_hover_text("Each way: a whole stroke takes twice this");
                ui.label("each way, between");
                ui.add(egui::DragValue::new(low).range(0.0..=100.0).suffix("%"));
                ui.label("and");
                ui.add(egui::DragValue::new(high).range(0.0..=100.0).suffix("%"));
                if ui.add(primary("Insert")).on_hover_text("What follows the cursor comes later").clicked() {
                    let (count, half, low, high) = e.strokes;
                    let points = funscript::insert_strokes(&e.points, e.cursor, count, half, low / 100.0, high / 100.0);
                    e.set(points);
                    let end = e.points.last().map_or(0.0, |p| p.0);
                    if end > e.view.0 + e.view.1 {
                        e.fit();
                    }
                }
            });
        });

        for command in commands {
            self.send(command);
        }
        if !close {
            self.motions.editor = Some(e);
        } else if trying.is_some() {
            self.send(Command::TryFunscript(None));
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

/// `t` on the grid's nearest line, if there is a grid.
fn snapped(t: f64, grid: Option<f64>) -> f64 {
    match grid {
        Some(g) if g > 0.0 => ((t / g).round() * g * 1e6).round() / 1e6,
        _ => t,
    }
}

fn undo(e: &mut Editor) {
    if let Some(points) = e.undo.pop() {
        e.points = points;
        e.dirty = true;
        e.picked.clear();
    }
}

/// The points `picked` (indices) moved from where they were in `before` by `dt`
/// and `dp`: positions within 0..1, times at 0 or later and in the same order
/// with the points around them.
fn move_points(before: &[(f64, f64)], picked: &BTreeSet<usize>, dt: f64, dp: f64) -> Vec<(f64, f64)> {
    let (mut low, mut high) = (f64::NEG_INFINITY, f64::INFINITY);
    for &i in picked {
        let t = before[i].0;
        low = low.max(-t);
        if i > 0 && !picked.contains(&(i - 1)) {
            low = low.max(before[i - 1].0 + 0.001 - t);
        }
        if let Some(next) = before.get(i + 1).filter(|_| !picked.contains(&(i + 1))) {
            high = high.min(next.0 - 0.001 - t);
        }
    }
    let dt = if low <= high { dt.clamp(low, high) } else { 0.0 };
    before.iter().enumerate().map(|(i, &(t, p))| if picked.contains(&i) { (t + dt, (p + dp).clamp(0.0, 1.0)) } else { (t, p) }).collect()
}

/// The curve to edit, over the timeline to select on.
fn canvas(ui: &mut egui::Ui, e: &mut Editor, trying: Option<f64>, limits: &MotionLimits) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 220.0), Sense::click_and_drag());
    let (strip, strip_response) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::click_and_drag());
    let (from, length) = e.view;
    let t_at = |px: f32| from + ((px - rect.left()) / rect.width()) as f64 * length;
    let y = |p: f64| rect.bottom() - 6.0 - (rect.height() - 12.0) * p as f32;
    let p_at = |py: f32| ((rect.bottom() - 6.0 - py) / (rect.height() - 12.0)).clamp(0.0, 1.0) as f64;

    // Zoom around the pointer (Ctrl+wheel, pinch), scroll sideways (Shift+wheel);
    // the wheel alone scrolls the page.
    if response.hovered() || strip_response.hovered() {
        let (zoom, scroll, pointer) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
        if zoom != 1.0 {
            let at = pointer.map_or(from + length / 2.0, |p| t_at(p.x));
            let new = (length / zoom as f64).clamp(0.05, 3600.0);
            e.view = ((at - (at - from) * new / length).max(0.0), new);
        } else if scroll.x != 0.0 {
            e.view.0 = (e.view.0 - scroll.x as f64 / rect.width() as f64 * length).max(0.0);
        }
    }
    let (from, length) = e.view;
    let grid = e.grid();
    let x = |t: f64| rect.left() + rect.width() * ((t - from) / length) as f32;
    let t_at = |px: f32| snapped((from + ((px - rect.left()) / rect.width()) as f64 * length).max(0.0), grid);

    let first = e.points.partition_point(|p| p.0 < from);
    let last = e.points.partition_point(|p| p.0 <= from + length);
    let dots = last - first <= DOTS;
    let near = |pos: egui::Pos2, points: &[(f64, f64)]| {
        (first..last).filter(|_| dots).map(|i| (i, egui::pos2(x(points[i].0), y(points[i].1)).distance(pos))).filter(|(_, d)| *d <= PICK).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    };

    // Points: pick, move, remove (and add, if a click does); Shift adds to the points picked.
    let raw_t = |px: f32| (from + ((px - rect.left()) / rect.width()) as f64 * length).max(0.0);
    let shift = ui.input(|i| i.modifiers.shift);
    if let Some(pos) = response.interact_pointer_pos() {
        if response.drag_started() {
            match near(pos, &e.points) {
                Some(i) => {
                    if !e.picked.contains(&i) {
                        if !shift {
                            e.picked.clear();
                        }
                        e.picked.insert(i);
                    }
                    e.change();
                    e.drag = Some((e.points.clone(), raw_t(pos.x), p_at(pos.y)));
                }
                None => {
                    if !shift {
                        e.picked.clear();
                    }
                    e.boxing = Some((pos, pos));
                }
            }
        }
        if let Some((before, t0, p0)) = &e.drag {
            // The point grabbed first snaps to the grid; the others follow it.
            let grabbed = e.picked.iter().map(|&i| before[i].0).fold(f64::INFINITY, f64::min);
            let dt = snapped(grabbed + raw_t(pos.x) - t0, grid) - grabbed;
            e.points = move_points(before, &e.picked, dt, p_at(pos.y) - p0);
        }
        if let Some((_, now)) = &mut e.boxing {
            *now = pos;
        }
        if response.clicked() {
            match near(pos, &e.points) {
                Some(i) => {
                    if !shift {
                        e.picked.clear();
                    }
                    if !e.picked.remove(&i) {
                        e.picked.insert(i);
                    }
                    e.cursor = e.points[i].0;
                }
                None => {
                    let t = t_at(pos.x);
                    if e.click_adds {
                        e.change();
                        match e.points.iter().position(|p| p.0 == t) {
                            Some(i) => e.points[i].1 = p_at(pos.y),
                            None => {
                                let i = e.points.partition_point(|p| p.0 < t);
                                e.points.insert(i, (t, p_at(pos.y)));
                                e.picked.clear();
                            }
                        }
                    } else if !shift {
                        e.picked.clear();
                    }
                    e.cursor = t;
                }
            }
        }
        if response.secondary_clicked() {
            if let Some(i) = near(pos, &e.points) {
                e.change();
                e.points.remove(i);
                e.picked.clear();
            }
        }
    }
    if response.drag_stopped() {
        e.drag = None;
        if let Some((a, b)) = e.boxing.take() {
            let (t0, t1) = (raw_t(a.x.min(b.x)), raw_t(a.x.max(b.x)));
            let (p0, p1) = (p_at(a.y.max(b.y)), p_at(a.y.min(b.y)));
            let inside = e.points.iter().enumerate().filter(|(_, p)| (t0..=t1).contains(&p.0) && (p0..=p1).contains(&p.1)).map(|(i, _)| i);
            e.picked.extend(inside);
        }
    }

    // The timeline: the cursor, the selection.
    if let Some(pos) = strip_response.interact_pointer_pos() {
        let t = t_at(pos.x);
        if strip_response.drag_started() {
            e.selecting = Some(t);
        }
        if let Some(start) = e.selecting {
            e.selection = (t - start).abs().gt(&0.01).then_some((start.min(t), start.max(t)));
        }
        if strip_response.clicked() {
            e.cursor = t;
            e.selection = None;
        }
    }
    if strip_response.drag_stopped() {
        e.selecting = None;
        if let Some((a, _)) = e.selection {
            e.cursor = a;
        }
    }

    let painter = ui.painter();
    painter.rect_filled(rect, 6, RAISED);
    painter.rect_filled(strip, 4, PANEL);
    // The grid, while its lines are apart enough to see.
    if let Some(g) = grid.filter(|g| rect.width() as f64 * g / length >= 6.0) {
        let mut t = (from / g).ceil() * g;
        while t <= from + length {
            painter.line_segment([egui::pos2(x(t), rect.top()), egui::pos2(x(t), rect.bottom())], Stroke::new(1.0, LINE.gamma_multiply(0.45)));
            t += g;
        }
    }
    // A line a second (or every 10, 60 s when zoomed out), labeled on the timeline.
    let step = [0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 300.0].into_iter().find(|s| length / s <= 20.0).unwrap_or(600.0);
    let mut t = (from / step).ceil() * step;
    while t <= from + length {
        painter.line_segment([egui::pos2(x(t), rect.top()), egui::pos2(x(t), rect.bottom())], Stroke::new(1.0, LINE));
        let label = if step < 1.0 { format!("{t:.1}") } else { format!("{t:.0}") };
        painter.text(egui::pos2(x(t) + 2.0, strip.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(10.5), MUTED);
        t += step;
    }
    if let Some((a, b)) = e.selection {
        let area = egui::Rect::from_x_y_ranges(x(a).max(rect.left())..=x(b).min(rect.right()), rect.top()..=strip.bottom());
        painter.rect_filled(area, 0, ACCENT.gamma_multiply(0.15));
    }
    let clip = painter.with_clip_rect(rect);
    for pair in e.points[first.saturating_sub(1)..(last + 1).min(e.points.len())].windows(2) {
        let ((t0, p0), (t1, p1)) = (pair[0], pair[1]);
        let color = speed_color(speed(pair[0], pair[1]), limits);
        clip.line_segment([egui::pos2(x(t0), y(p0)), egui::pos2(x(t1), y(p1))], Stroke::new(2.0, color));
    }
    if dots {
        let hovered = response.hover_pos().and_then(|pos| near(pos, &e.points));
        let early = early_turns(&e.points, limits.min_turn);
        for i in first..last {
            let at = egui::pos2(x(e.points[i].0), y(e.points[i].1));
            if e.picked.contains(&i) {
                clip.circle_filled(at, 5.0, TEXT);
                clip.circle_stroke(at, 5.0, Stroke::new(1.5, ACCENT));
            } else {
                clip.circle_filled(at, 3.5, if Some(i) == hovered { TEXT } else { ACCENT });
            }
            if early.contains(&i) {
                clip.circle_stroke(at, 7.0, Stroke::new(1.5, WARN));
            }
        }
    }
    if let Some((a, b)) = e.boxing {
        let area = egui::Rect::from_two_pos(a, b);
        clip.rect_filled(area, 0, ACCENT.gamma_multiply(0.1));
        clip.rect_stroke(area, 0, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
    }
    // The segment under the pointer: how fast it moves.
    if let Some(pos) = response.hover_pos() {
        let t = from + ((pos.x - rect.left()) / rect.width()) as f64 * length;
        let i = e.points.partition_point(|p| p.0 <= t);
        if i > 0 && i < e.points.len() {
            let (a, b) = (e.points[i - 1], e.points[i]);
            let v = speed(a, b);
            let verdict = match () {
                _ if v < 1e-6 => "still",
                _ if v < limits.slowest => "too slow: the toy may jerk",
                _ if v > limits.fastest => "too fast: shortened to what the toy can do",
                _ => "the toy can follow",
            };
            let text = format!("{:.2} → {:.2} s · {:.0} units/s · {verdict}", a.0, b.0, v * 100.0);
            let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.5), speed_color(v, limits));
            let at = rect.left_top() + Vec2::new(8.0, 6.0);
            painter.rect_filled(egui::Rect::from_min_size(at - Vec2::splat(3.0), galley.size() + Vec2::splat(6.0)), 4, PANEL);
            painter.galley(at, galley, TEXT);
        }
    }
    let line = |t: f64, color| clip.line_segment([egui::pos2(x(t), rect.top()), egui::pos2(x(t), rect.bottom())], Stroke::new(1.5, color));
    line(e.cursor, WARN);
    if let Some(at) = trying {
        line(at, TEXT);
    }
    if e.points.is_empty() {
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Click to add points, or insert strokes below", egui::FontId::proportional(13.0), MUTED);
    }
}

/// What motions are checked against: the strictest of the strokers connected (their
/// settings on the Toys page), else the default settings; and whose they are.
fn limits(s: &Shared) -> (MotionLimits, String) {
    let strokers: Vec<&str> = s.intiface.toys.iter().filter(|t| t.stroker).map(|t| t.name.as_str()).collect();
    let limits = strokers
        .iter()
        .map(|name| s.settings.toys.get(*name).copied().unwrap_or_default().stroke.motion_limits())
        .reduce(MotionLimits::strictest);
    match limits {
        Some(limits) => (limits, strokers.join(", ")),
        None => (StrokeSettings::default().motion_limits(), "a stroker with the default settings (none connected)".into()),
    }
}

/// How fast a segment moves: positions (0..1) per second.
fn speed((t0, p0): (f64, f64), (t1, p1): (f64, f64)) -> f64 {
    (p1 - p0).abs() / (t1 - t0).max(1e-6)
}

/// A segment's color for how fast it moves: still: gray, too slow: blue, then green
/// to orange as it nears the toy's fastest, red beyond.
fn speed_color(speed: f64, limits: &MotionLimits) -> Color32 {
    if speed < 1e-6 {
        IDLE
    } else if speed < limits.slowest {
        GAME
    } else if speed > limits.fastest {
        DANGER
    } else {
        let t = ((speed - limits.slowest) / (limits.fastest - limits.slowest).max(1e-9)) as f32;
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color32::from_rgb(mix(OK.r(), WARN.r()), mix(OK.g(), WARN.g()), mix(OK.b(), WARN.b()))
    }
}

/// Points where the motion turns sooner after its last turn than the toy can: the
/// planner skips them.
fn early_turns(points: &[(f64, f64)], min_turn: f64) -> std::collections::HashSet<usize> {
    let mut early = std::collections::HashSet::new();
    let mut last_turn = f64::NEG_INFINITY;
    // Holds keep the direction they come after.
    let mut direction = 0.0;
    for i in 1..points.len().saturating_sub(1) {
        let before = points[i].1 - points[i - 1].1;
        if before.abs() > 1e-9 {
            direction = before.signum();
        }
        let after = points[i + 1].1 - points[i].1;
        if direction != 0.0 && after.abs() > 1e-9 && after.signum() != direction {
            if points[i].0 - last_turn < min_turn {
                early.insert(i);
            } else {
                last_turn = points[i].0;
            }
        }
    }
    early
}

/// What the colors say, and what they are checked against.
fn speed_legend(ui: &mut egui::Ui, limits: &MotionLimits, toys: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for (color, label) in [(IDLE, "still"), (GAME, "too slow"), (OK, "fine"), (WARN, "near its fastest"), (DANGER, "too fast")] {
            ui.label(RichText::new(format!("━ {label}")).size(11.5).color(color));
        }
        ui.label(RichText::new("◯ turns too soon (skipped)").size(11.5).color(WARN));
        ui.label(
            muted(format!(
                "for {toys}: {:.0} to {:.0} units/s, turns {:.2} s apart",
                limits.slowest * 100.0,
                limits.fastest * 100.0,
                limits.min_turn
            ))
            .size(11.5),
        )
        .on_hover_text("Set or calibrated on the Toys page; a motion played with less depth moves slower");
    });
}

/// Its positions over time (bottom: 0), colored for how fast they move, where the try is at.
fn curve(ui: &mut egui::Ui, track: &Track, at: Option<f64>, limits: &MotionLimits) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, RAISED);
    let duration = track.duration().max(1e-3);
    let x = |t: f64| rect.left() + rect.width() * (t / duration) as f32;
    let y = |p: f64| rect.bottom() - 4.0 - (rect.height() - 8.0) * p as f32;
    let points = track.points();
    let step = points.len().div_ceil(CURVE_POINTS).max(1);
    let shown: Vec<(f64, f64)> = points.iter().step_by(step).chain(points.last()).copied().collect();
    for pair in shown.windows(2) {
        let color = speed_color(speed(pair[0], pair[1]), limits);
        painter.line_segment([egui::pos2(x(pair[0].0), y(pair[0].1)), egui::pos2(x(pair[1].0), y(pair[1].1))], Stroke::new(1.5, color));
    }
    // Not during the try's lead-in.
    if let Some(at) = at.filter(|at| *at >= 0.0) {
        painter.line_segment([egui::pos2(x(at), rect.top()), egui::pos2(x(at), rect.bottom())], Stroke::new(1.5, TEXT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_snap_to_the_grid() {
        assert_eq!(snapped(1.234, None), 1.234);
        assert_eq!(snapped(1.234, Some(0.1)), 1.2);
        assert_eq!(snapped(1.26, Some(0.1)), 1.3);
        // 120 steps a minute: lines at 0.5 s.
        assert_eq!(snapped(0.74, Some(0.5)), 0.5);
        assert_eq!(snapped(0.3 + 0.1 + 0.2, Some(0.1)), 0.6, "no floating point noise");
    }

    #[test]
    fn points_picked_move_together_in_order() {
        let points = [(0.0, 0.0), (1.0, 0.5), (2.0, 0.6), (3.0, 1.0)];
        let picked: BTreeSet<usize> = [1, 2].into();
        assert_eq!(move_points(&points, &picked, 0.5, 0.2), vec![(0.0, 0.0), (1.5, 0.7), (2.5, 0.8), (3.0, 1.0)]);
        // Not past the points around them, positions within 0..1.
        let moved = move_points(&points, &picked, 5.0, 0.6);
        assert!((moved[2].0 - 2.999).abs() < 1e-9 && moved[1].1 == 1.0 && moved[2].1 == 1.0, "{moved:?}");
        let moved = move_points(&points, &[0].into(), -1.0, 0.0);
        assert_eq!(moved[0], (0.0, 0.0));
    }

    #[test]
    fn segments_are_judged_against_the_toy() {
        let limits = MotionLimits { slowest: 0.25, fastest: 2.0, min_turn: 0.25 };
        assert_eq!(speed_color(speed((0.0, 0.5), (1.0, 0.5)), &limits), IDLE);
        assert_eq!(speed_color(speed((0.0, 0.0), (1.0, 0.1)), &limits), GAME);
        assert_eq!(speed_color(speed((0.0, 0.0), (0.25, 1.0)), &limits), DANGER);
        assert_eq!(speed_color(0.25, &limits), OK);
        assert_eq!(speed_color(2.0, &limits), WARN);
        // Turns at 1, 1.1 (too soon), and 1.5 after a hold, which keeps the direction it comes after.
        let points = [(0.0, 0.0), (1.0, 1.0), (1.1, 0.5), (1.3, 0.9), (1.5, 0.9), (2.0, 0.0)];
        assert_eq!(early_turns(&points, 0.25), [2].into());
    }
}
