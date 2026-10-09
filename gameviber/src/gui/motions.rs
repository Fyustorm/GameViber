//! The Creator's Funscripts tab: motions for strokers in the active mode's
//! package, which its script plays on an event (`funscript("name")`, docs/spec-modes.md
//! §8.5): adding them, seeing them, trying them on the toys, renaming and removing
//! them, and a simple editor to write one or make one of a part of another.

use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use eframe::egui::{self, Margin, RichText, Sense, Stroke, Vec2};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::funscript::{self, Track};
use crate::game::Game;

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
    selection: Option<(f64, f64)>,
    dragging: Option<usize>,
    /// Where a selection being dragged started.
    selecting: Option<f64>,
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
            dragging: None,
            selecting: None,
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

    fn set(&mut self, points: Vec<(f64, f64)>) {
        self.change();
        self.points = points;
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
        if pressed(Key::Delete, Modifiers::NONE) || pressed(Key::Backspace, Modifiers::NONE) {
            match (self.selection, self.at_cursor()) {
                (Some((from, to)), _) => {
                    let points = funscript::remove(&self.points, from, to, false);
                    self.set(points);
                }
                (None, Some(i)) => {
                    self.change();
                    self.points.remove(i);
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
        for (name, track) in &s.funscripts {
            let trying = s.trying.as_ref().filter(|(n, _)| n == name).map(|(_, at)| *at);
            card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.motions.renaming.as_ref().is_some_and(|(n, _)| n == name) {
                    self.rename_row(ui, s, name);
                } else {
                    self.funscript_row(ui, &s.mode.id, name, track, trying);
                }
                curve(ui, track, trying);
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
        canvas(ui, &mut e, trying);
        ui.label(
            muted("Click: add a point · drag a point: move it · right-click a point: remove it · drag on the timeline: select · Ctrl+wheel: zoom · Shift+wheel: scroll")
                .size(11.5),
        );
        ui.label(
            muted("0 to 9: a point at the cursor, 0% to 100% · ← →: move the cursor a step (Shift: 10) · ↑ ↓: next, previous point · Delete: the point at the cursor, or the selection's · Space: try, stop · Ctrl+Z: undo")
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
    }
}

/// The curve to edit, over the timeline to select on.
fn canvas(ui: &mut egui::Ui, e: &mut Editor, trying: Option<f64>) {
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

    // Points: add, move, remove.
    if let Some(pos) = response.interact_pointer_pos() {
        if response.drag_started() {
            if let Some(i) = near(pos, &e.points) {
                e.change();
                e.dragging = Some(i);
            }
        }
        if let Some(i) = e.dragging {
            let low = if i == 0 { 0.0 } else { e.points[i - 1].0 + 0.001 };
            let high = e.points.get(i + 1).map_or(f64::INFINITY, |p| p.0 - 0.001);
            e.points[i] = (t_at(pos.x).clamp(low, high.max(low)), p_at(pos.y));
        }
        if response.clicked() {
            if near(pos, &e.points).is_none() {
                let t = t_at(pos.x);
                e.change();
                match e.points.iter().position(|p| p.0 == t) {
                    Some(i) => e.points[i].1 = p_at(pos.y),
                    None => {
                        let i = e.points.partition_point(|p| p.0 < t);
                        e.points.insert(i, (t, p_at(pos.y)));
                    }
                }
            }
            e.cursor = t_at(pos.x);
        }
        if response.secondary_clicked() {
            if let Some(i) = near(pos, &e.points) {
                e.change();
                e.points.remove(i);
            }
        }
    }
    if response.drag_stopped() {
        e.dragging = None;
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
    let shown: Vec<egui::Pos2> = e.points[first.saturating_sub(1)..(last + 1).min(e.points.len())]
        .iter()
        .map(|&(t, p)| egui::pos2(x(t), y(p)))
        .collect();
    let clip = painter.with_clip_rect(rect);
    clip.add(egui::Shape::line(shown, Stroke::new(1.5, ACCENT)));
    if dots {
        let hovered = response.hover_pos().and_then(|pos| near(pos, &e.points));
        for i in first..last {
            let color = if Some(i) == hovered || Some(i) == e.dragging { TEXT } else { ACCENT };
            clip.circle_filled(egui::pos2(x(e.points[i].0), y(e.points[i].1)), 3.5, color);
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

/// Its positions over time (bottom: 0), where the try is at.
fn curve(ui: &mut egui::Ui, track: &Track, at: Option<f64>) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, RAISED);
    let duration = track.duration().max(1e-3);
    let x = |t: f64| rect.left() + rect.width() * (t / duration) as f32;
    let y = |p: f64| rect.bottom() - 4.0 - (rect.height() - 8.0) * p as f32;
    let points = track.points();
    let step = points.len().div_ceil(CURVE_POINTS).max(1);
    let line: Vec<egui::Pos2> = points.iter().step_by(step).chain(points.last()).map(|&(t, p)| egui::pos2(x(t), y(p))).collect();
    painter.add(egui::Shape::line(line, Stroke::new(1.5, ACCENT)));
    if let Some(at) = at {
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
}
