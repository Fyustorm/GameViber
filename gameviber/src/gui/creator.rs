//! Creator page, for mode authors: mode files, the Luau editor with hot
//! reload, and tools to watch and drive the mode (graphs, simulator, logs).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints};

use super::theme::*;
use super::App;
use crate::config::{self, ModeEntry, NEW_MODE_TEMPLATE};
use crate::engine::{Command, Sample, Shared, HISTORY_SECS};
use crate::gamepad::BUTTONS;

const SIM_HIT: Duration = Duration::from_millis(300);

#[derive(PartialEq, Clone, Copy, Default)]
enum Tool {
    #[default]
    Graphs,
    Simulator,
    Log,
}

#[derive(Default)]
struct Editor {
    /// Mode id the buffer was loaded from.
    id: String,
    text: String,
    dirty: bool,
    message: Option<String>,
}

/// Fake game rumble and button presses, to test modes without a game.
#[derive(Default)]
pub struct Simulator {
    strong: f64,
    weak: f64,
    hit_until: Option<Instant>,
    sent: (f64, f64),
    held: BTreeSet<&'static str>,
}

impl Simulator {
    /// The rumble command to send when the simulated levels changed (sliders or an expiring hit).
    pub fn update(&mut self) -> Option<Command> {
        let hit = self.hit_until.is_some_and(|t| Instant::now() < t);
        if !hit {
            self.hit_until = None;
        }
        let wanted = if hit { (1.0, 1.0) } else { (self.strong, self.weak) };
        (wanted != self.sent).then(|| {
            self.sent = wanted;
            Command::SimRumble { strong: wanted.0, weak: wanted.1 }
        })
    }
}

#[derive(Default)]
pub struct State {
    tool: Tool,
    editor: Editor,
    new_mode_name: String,
    pub sim: Simulator,
    only_mode_logs: bool,
}

impl App {
    pub(super) fn creator_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(12, 16));
        egui::Panel::left("mode-files").frame(frame).default_size(210.0).resizable(true).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.mode_files(ui, s));
        });
        let frame = egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(14, 12));
        egui::Panel::right("mode-tools").frame(frame).default_size(380.0).resizable(true).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (tool, label) in [(Tool::Graphs, "Graphs"), (Tool::Simulator, "Simulator"), (Tool::Log, "Log")] {
                    ui.selectable_value(&mut self.creator.tool, tool, label);
                }
            });
            ui.separator();
            match self.creator.tool {
                Tool::Graphs => graphs(ui, s),
                Tool::Simulator => self.simulator(ui),
                Tool::Log => self.log(ui),
            }
        });
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(14, 12));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| self.editor(ui, s));
    }

    fn mode_files(&mut self, ui: &mut egui::Ui, s: &Shared) {
        eyebrow(ui, "My modes");
        for entry in s.modes.iter().filter(|e| !e.builtin) {
            if ui.selectable_label(entry.id == s.mode.id, &entry.key).clicked() && entry.id != s.mode.id {
                self.send(Command::SelectMode(entry.id.clone()));
            }
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.creator.new_mode_name).hint_text("new mode").desired_width(110.0));
            if ui.button("➕ New").clicked() {
                let name = std::mem::take(&mut self.creator.new_mode_name);
                let display = if name.trim().is_empty() { "My mode" } else { name.trim() };
                self.create_mode(&name, &NEW_MODE_TEMPLATE.replace("NAME", display));
            }
        });
        if ui.button("✨ Generate with an AI").clicked() {
            self.open_generator();
        }
        if ui.button("Duplicate active mode").clicked() {
            self.duplicate_mode(&s.mode.id);
        }
        ui.add_space(12.0);
        eyebrow(ui, "Built-in (read-only)");
        for entry in s.modes.iter().filter(|e| e.builtin) {
            if ui.selectable_label(entry.id == s.mode.id, &entry.key).clicked() && entry.id != s.mode.id {
                self.send(Command::SelectMode(entry.id.clone()));
            }
        }
        ui.add_space(12.0);
        ui.label(muted("Mode API: docs/spec-modes.md").size(12.0));
    }

    fn editor(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mode = &s.mode;
        let editor = &mut self.creator.editor;
        if editor.id != mode.id && !editor.dirty {
            *editor = Editor { id: mode.id.clone(), ..Default::default() };
            match ModeEntry::from_id(&mode.id).source() {
                Ok(text) => editor.text = text,
                Err(e) => editor.message = Some(format!("cannot read: {e}")),
            }
        }
        let editing = ModeEntry::from_id(&editor.id);
        let save = ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S));
        let mut duplicate = false;
        let mut reload = false;
        let mut refresh = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new(&editing.id).monospace());
            if editing.builtin {
                ui.label(muted("built-in modes are read-only"));
                duplicate = ui.button("Duplicate to edit").clicked();
            } else {
                let label = if editor.dirty { "Save and reload (Ctrl+S) *" } else { "Save and reload (Ctrl+S)" };
                if ui.add_enabled(editor.dirty, primary(label)).clicked() || (save && editor.dirty) {
                    if let Some(path) = editing.path() {
                        match config::write_file(&path, &editor.text) {
                            Ok(()) => {
                                editor.dirty = false;
                                editor.message = None;
                                reload = editing.id == mode.id;
                                // Updates the catalog, which holds the load error of other modes.
                                refresh = !reload;
                            }
                            Err(e) => editor.message = Some(format!("cannot save: {e}")),
                        }
                    }
                }
                if ui.add_enabled(editor.dirty, egui::Button::new("Revert")).clicked() {
                    *editor = Editor::default();
                }
            }
        });
        if editor.dirty && editing.id != mode.id {
            ui.label(RichText::new("Unsaved changes to another mode: save or revert them.").color(DANGER_TEXT));
        }
        if let Some(message) = &editor.message {
            ui.label(RichText::new(message).color(DANGER_TEXT));
        }
        // The error of the file being edited: the active mode's runtime error, or
        // why another mode does not load.
        let error = if editing.id == mode.id {
            mode.error.as_deref()
        } else {
            s.catalog.get(&editing.id).and_then(|info| info.as_ref().err()).map(String::as_str)
        };
        if let Some(error) = error {
            ui.label(RichText::new(error).color(DANGER_TEXT).monospace());
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let edit = egui::TextEdit::multiline(&mut editor.text)
                .code_editor()
                .interactive(!editing.builtin)
                .desired_width(f32::INFINITY)
                .desired_rows(30);
            if ui.add(edit).changed() {
                editor.dirty = true;
            }
        });
        if reload {
            self.send(Command::ReloadMode);
        }
        if refresh {
            self.send(Command::RefreshModes);
        }
        if duplicate {
            let source = self.creator.editor.text.clone();
            self.create_mode(&format!("{}-copy", editing.key), &source);
        }
    }

    fn simulator(&mut self, ui: &mut egui::Ui) {
        let sim = &mut self.creator.sim;
        ui.label(muted("Fake game rumble and button presses, to test modes without a game."));
        ui.add(egui::Slider::new(&mut sim.strong, 0.0..=1.0).text("strong motor"));
        ui.add(egui::Slider::new(&mut sim.weak, 0.0..=1.0).text("weak motor"));
        ui.horizontal(|ui| {
            if ui.button("Hit (0.3 s at full strength)").clicked() {
                sim.hit_until = Some(Instant::now() + SIM_HIT);
            }
            if ui.button("Reset").clicked() {
                sim.strong = 0.0;
                sim.weak = 0.0;
            }
        });
        ui.separator();
        ui.label("Buttons (hold with the mouse):");
        let mut changes = Vec::new();
        ui.horizontal_wrapped(|ui| {
            for name in BUTTONS {
                let down = ui.button(name).is_pointer_button_down_on();
                if down != sim.held.contains(name) {
                    if down {
                        sim.held.insert(name);
                    } else {
                        sim.held.remove(name);
                    }
                    changes.push(Command::SimButton { name: name.to_owned(), pressed: down });
                }
            }
        });
        for command in changes {
            self.send(command);
        }
    }

    fn log(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.creator.only_mode_logs, "Only mode logs (log / print)");
        let lines: Vec<_> = self.logs.lock().unwrap().iter().cloned().collect();
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for line in lines.iter().filter(|l| !self.creator.only_mode_logs || l.target == "mode") {
                let color = match line.level {
                    log::Level::Error => DANGER_TEXT,
                    log::Level::Warn => WARN,
                    _ => TEXT,
                };
                ui.label(RichText::new(format!("{:8.2}  {}", line.seconds, line.message)).monospace().size(12.0).color(color));
            }
        });
    }
}

fn graphs(ui: &mut egui::Ui, s: &Shared) {
    let series = |f: &dyn Fn(&Sample) -> Option<f64>| -> PlotPoints<'static> {
        s.history.iter().filter_map(|x| f(x).map(|v| [x.t - s.time, v])).collect::<Vec<_>>().into()
    };
    let channels: BTreeSet<String> = s.history.iter().flat_map(|x| x.channels.keys().cloned()).collect();
    ui.label(muted(format!("Game rumble and mode output (last {HISTORY_SECS:.0} s)")));
    Plot::new("rumble")
        .height(200.0)
        .legend(Legend::default())
        .include_x(-HISTORY_SECS)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(1.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .show(ui, |plot| {
            plot.line(Line::new("rumble strong", series(&|x| Some(x.strong))).color(GAME));
            plot.line(Line::new("rumble weak", series(&|x| Some(x.weak))).color(GAME.gamma_multiply(0.5)));
            for channel in &channels {
                plot.line(Line::new(format!("out: {channel}"), series(&|x| x.channels.get(channel).copied())).width(2.0));
            }
        });
    if !s.plots.is_empty() {
        ui.label(muted("Mode plot() values"));
        Plot::new("plots")
            .height(180.0)
            .legend(Legend::default())
            .include_x(-HISTORY_SECS)
            .include_x(0.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show(ui, |plot| {
                for (name, points) in &s.plots {
                    let points: Vec<[f64; 2]> = points.iter().map(|p| [p[0] - s.time, p[1]]).collect();
                    plot.line(Line::new(name.clone(), PlotPoints::from(points)));
                }
            });
    }
    ui.horizontal(|ui| {
        ui.label(muted("Held buttons:"));
        ui.label(if s.held.is_empty() { "-".into() } else { s.held.join(" ") });
    });
}
