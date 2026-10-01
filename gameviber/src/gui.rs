//! egui front end: status bar with panic stop, mode list and parameters,
//! and tabs for live graphs, the mode editor, routing, the simulator and logs.

use std::collections::{BTreeMap, BTreeSet};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{self, ModeEntry, SourceChoice, NEW_MODE_TEMPLATE};
use crate::engine::{Command, ModeView, SharedHandle, HISTORY_SECS};
use crate::gamepad::BUTTONS;
use crate::logging::LogBuffer;
use crate::mode::{ModeInfo, ParamDef, ParamKind, ParamValue};

const REPAINT: Duration = Duration::from_millis(33);
const SIM_HIT: Duration = Duration::from_millis(300);
const DANGER: Color32 = Color32::from_rgb(200, 40, 40);
const OK: Color32 = Color32::from_rgb(60, 170, 90);

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Monitor,
    Editor,
    Routing,
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

pub struct App {
    shared: SharedHandle,
    logs: LogBuffer,
    commands: UnboundedSender<Command>,
    engine: Option<JoinHandle<()>>,
    tab: Tab,
    editor: Editor,
    new_mode_name: String,
    new_preset_name: String,
    /// Preset whose deletion waits for confirmation.
    confirm_delete: Option<String>,
    sim_strong: f64,
    sim_weak: f64,
    sim_hit_until: Option<Instant>,
    sim_sent: (f64, f64),
    sim_held: BTreeSet<&'static str>,
    only_mode_logs: bool,
}

impl App {
    pub fn new(shared: SharedHandle, logs: LogBuffer, commands: UnboundedSender<Command>, engine: JoinHandle<()>) -> Self {
        Self {
            shared,
            logs,
            commands,
            engine: Some(engine),
            tab: Tab::Monitor,
            editor: Editor::default(),
            new_mode_name: String::new(),
            new_preset_name: String::new(),
            confirm_delete: None,
            sim_strong: 0.0,
            sim_weak: 0.0,
            sim_hit_until: None,
            sim_sent: (0.0, 0.0),
            sim_held: BTreeSet::new(),
            only_mode_logs: false,
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Creates a user mode file and activates it.
    fn create_mode(&mut self, stem: &str, source: &str) {
        let stem: String = stem
            .trim()
            .chars()
            .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let stem = if stem.is_empty() { "my-mode".to_owned() } else { stem };
        let path = config::unused_mode_path(&stem);
        match config::write_file(&path, source) {
            Ok(()) => {
                log::info!("created {}", path.display());
                self.send(Command::RefreshModes);
                self.send(Command::SelectMode(path.to_string_lossy().into_owned()));
                self.tab = Tab::Editor;
            }
            Err(e) => log::error!("cannot create {}: {e}", path.display()),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(REPAINT);
        if self.shared.lock().unwrap().stopped {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.status_bar(ui);
        self.side_panel(ui);
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                for (tab, label) in [
                    (Tab::Monitor, "Monitor"),
                    (Tab::Editor, "Editor"),
                    (Tab::Routing, "Routing"),
                    (Tab::Simulator, "Simulator"),
                    (Tab::Log, "Log"),
                ] {
                    ui.selectable_value(&mut self.tab, tab, label);
                }
            });
            ui.separator();
            match self.tab {
                Tab::Monitor => self.monitor(ui),
                Tab::Editor => self.editor(ui),
                Tab::Routing => self.routing(ui),
                Tab::Simulator => self.simulator(ui),
                Tab::Log => self.log(ui),
            }
        });
        self.update_simulated_rumble();
    }

    fn on_exit(&mut self) {
        self.send(Command::Shutdown);
        if let Some(engine) = self.engine.take() {
            let _ = engine.join();
        }
    }
}

impl App {
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let (source, intiface, enabled, panic, mut cap, choice, hide) = {
            let s = self.shared.lock().unwrap();
            let settings = &s.settings;
            (s.source.clone(), s.intiface.clone(), s.intiface_enabled, s.panic, settings.global_cap, settings.source, settings.hide)
        };
        egui::Panel::top("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                if panic {
                    ui.label(RichText::new("⛔ PANIC STOP").strong().color(DANGER));
                    if ui.button("Re-arm").clicked() {
                        self.send(Command::Rearm);
                    }
                } else {
                    let stop = egui::Button::new(RichText::new("STOP ALL").strong().color(Color32::WHITE)).fill(DANGER);
                    if ui.add(stop).on_hover_text("Also: hold BACK + START on the gamepad").clicked() {
                        self.send(Command::Panic);
                    }
                }
                ui.separator();
                if ui.add(egui::Slider::new(&mut cap, 0.0..=1.0).text("Max intensity")).changed() {
                    self.send(Command::SetCap(cap));
                }
                ui.separator();
                if !enabled {
                    ui.label(RichText::new("Intiface: disabled").weak());
                } else if intiface.connected {
                    let text = format!("● Intiface: {} ({} toy(s))", intiface.server, intiface.toys.len());
                    ui.label(RichText::new(text).color(OK));
                } else {
                    let error = intiface.error.unwrap_or_else(|| "connecting...".into());
                    ui.label(RichText::new(format!("○ Intiface: {error}")).color(DANGER));
                }
            });
            ui.horizontal(|ui| {
                let (mut new_choice, mut new_hide) = (choice, hide);
                egui::ComboBox::from_id_salt("source")
                    .selected_text(source_label(choice))
                    .show_ui(ui, |ui| {
                        for option in [SourceChoice::Proxy, SourceChoice::Ebpf, SourceChoice::None] {
                            ui.selectable_value(&mut new_choice, option, source_label(option))
                                .on_hover_text(source_help(option));
                        }
                    })
                    .response
                    .on_hover_text("How the game's rumble is intercepted");
                if choice == SourceChoice::Proxy {
                    ui.checkbox(&mut new_hide, "Hide real gamepad")
                        .on_hover_text("Games only see the virtual copy (asks for your password)");
                }
                if (new_choice, new_hide) != (choice, hide) {
                    self.send(Command::SetSource { source: new_choice, hide: new_hide });
                }
                ui.label(RichText::new(source).small());
            });
        });
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let (modes, mode) = {
            let s = self.shared.lock().unwrap();
            (s.modes.clone(), s.mode.clone())
        };
        egui::Panel::left("modes").resizable(true).default_size(300.0).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("Modes");
                for entry in &modes {
                    let label = if entry.builtin { format!("{} (built-in)", entry.key) } else { entry.key.clone() };
                    if ui.selectable_label(entry.id == mode.id, label).clicked() && entry.id != mode.id {
                        self.send(Command::SelectMode(entry.id.clone()));
                    }
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_mode_name).hint_text("new mode name").desired_width(140.0));
                    if ui.button("New").clicked() {
                        let name = std::mem::take(&mut self.new_mode_name);
                        let display = if name.trim().is_empty() { "My mode" } else { name.trim() };
                        self.create_mode(&name, &NEW_MODE_TEMPLATE.replace("NAME", display));
                    }
                });
                if ui.button("Duplicate active mode").clicked() {
                    let entry = ModeEntry::from_id(&mode.id);
                    match entry.source() {
                        Ok(source) => self.create_mode(&format!("{}-copy", entry.key), &source),
                        Err(e) => log::error!("cannot read {}: {e}", entry.id),
                    }
                }
                ui.separator();

                if let Some(info) = &mode.info {
                    ui.heading(&info.name);
                    if !info.description.is_empty() {
                        ui.label(&info.description);
                    }
                    if !info.author.is_empty() || !info.version.is_empty() {
                        ui.label(RichText::new(format!("{} {}", info.author, info.version)).small().weak());
                    }
                }
                if let Some(error) = &mode.error {
                    ui.label(RichText::new(error).color(DANGER).monospace());
                }
                if mode.suspended && ui.button("Resume").clicked() {
                    self.send(Command::Resume);
                }
                if let Some(info) = &mode.info {
                    ui.separator();
                    self.presets(ui, info, &mode);
                    ui.separator();
                    for def in &info.params {
                        if let Some(value) = mode.values.get(&def.name) {
                            if let Some(new) = param_widget(ui, def, value) {
                                self.send(Command::SetParam(def.name.clone(), new));
                            }
                        }
                    }
                }
            });
        });
    }

    fn presets(&mut self, ui: &mut egui::Ui, info: &ModeInfo, mode: &ModeView) {
        let presets = &mode.presets.presets;
        let active = mode.presets.active.as_deref().filter(|name| presets.contains_key(*name));
        let modified = active.is_some_and(|name| !matches_values(info, &presets[name], &mode.values));
        let current = match active {
            Some(name) if modified => format!("{name} (modified)"),
            Some(name) => name.to_owned(),
            None => "(none)".to_owned(),
        };
        ui.horizontal(|ui| {
            ui.label("Preset");
            egui::ComboBox::from_id_salt("preset").selected_text(current).show_ui(ui, |ui| {
                if presets.is_empty() {
                    ui.label(RichText::new("no preset saved").weak());
                }
                for name in presets.keys() {
                    // Selecting the active preset again reverts its unsaved changes.
                    if ui.selectable_label(Some(name.as_str()) == active, name).clicked() {
                        self.send(Command::LoadPreset(name.clone()));
                    }
                }
            });
        });
        ui.horizontal(|ui| {
            if let Some(name) = active {
                if ui.add_enabled(modified, egui::Button::new("Save")).on_hover_text("Overwrite this preset").clicked() {
                    self.send(Command::SavePreset(name.to_owned()));
                }
                if self.confirm_delete.as_deref() == Some(name) {
                    if ui.button(RichText::new("Really delete?").color(DANGER)).clicked() {
                        self.send(Command::DeletePreset(name.to_owned()));
                        self.confirm_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.confirm_delete = None;
                    }
                } else if ui.button("Delete").clicked() {
                    self.confirm_delete = Some(name.to_owned());
                }
            }
            if ui.button("Defaults").on_hover_text("Put every parameter back to its default").clicked() {
                self.send(Command::ResetParams);
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_preset_name).hint_text("new preset name").desired_width(140.0));
            let name = self.new_preset_name.trim().to_owned();
            let label = if presets.contains_key(&name) { "Replace" } else { "Save as" };
            if ui.add_enabled(!name.is_empty(), egui::Button::new(label)).clicked() {
                self.send(Command::SavePreset(name));
                self.new_preset_name.clear();
            }
        });
    }

    fn monitor(&mut self, ui: &mut egui::Ui) {
        let (history, plots, now, held, toys) = {
            let s = self.shared.lock().unwrap();
            (s.history.clone(), s.plots.clone(), s.time, s.held.clone(), s.toy_levels.clone())
        };
        let series = |f: &dyn Fn(&crate::engine::Sample) -> Option<f64>| -> PlotPoints<'static> {
            history.iter().filter_map(|s| f(s).map(|v| [s.t - now, v])).collect::<Vec<_>>().into()
        };
        let channels: BTreeSet<String> = history.iter().flat_map(|s| s.channels.keys().cloned()).collect();
        ui.label("Game rumble and mode output (last 10 s)");
        Plot::new("rumble")
            .height(220.0)
            .legend(Legend::default())
            .include_x(-HISTORY_SECS)
            .include_x(0.0)
            .include_y(0.0)
            .include_y(1.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .show(ui, |plot| {
                plot.line(Line::new("rumble strong", series(&|s| Some(s.strong))));
                plot.line(Line::new("rumble weak", series(&|s| Some(s.weak))));
                for channel in &channels {
                    plot.line(Line::new(format!("out: {channel}"), series(&|s| s.channels.get(channel).copied())).width(2.0));
                }
            });
        if !plots.is_empty() {
            ui.label("Mode plot() values");
            Plot::new("plots")
                .height(180.0)
                .legend(Legend::default())
                .include_x(-HISTORY_SECS)
                .include_x(0.0)
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .show(ui, |plot| {
                    for (name, points) in &plots {
                        let points: Vec<[f64; 2]> = points.iter().map(|p| [p[0] - now, p[1]]).collect();
                        plot.line(Line::new(name.clone(), PlotPoints::from(points)));
                    }
                });
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Held buttons:");
            ui.label(if held.is_empty() { "-".into() } else { held.join(" ") });
        });
        for (toy, level) in &toys {
            ui.horizontal(|ui| {
                ui.label(toy);
                ui.add(egui::ProgressBar::new(*level as f32).show_percentage());
            });
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui) {
        let mode = self.shared.lock().unwrap().mode.clone();
        let entry = ModeEntry::from_id(&mode.id);
        if self.editor.id != mode.id && !self.editor.dirty {
            self.editor = Editor { id: mode.id.clone(), ..Default::default() };
            match entry.source() {
                Ok(text) => self.editor.text = text,
                Err(e) => self.editor.message = Some(format!("cannot read: {e}")),
            }
        }
        let editing = ModeEntry::from_id(&self.editor.id);
        let save = ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S));
        ui.horizontal(|ui| {
            ui.label(RichText::new(&editing.id).monospace());
            if editing.builtin {
                ui.label(RichText::new("built-in modes are read-only").weak());
                if ui.button("Duplicate to edit").clicked() {
                    let source = self.editor.text.clone();
                    self.create_mode(&format!("{}-copy", editing.key), &source);
                }
            } else {
                let label = if self.editor.dirty { "Save (Ctrl+S) *" } else { "Save (Ctrl+S)" };
                if ui.add_enabled(self.editor.dirty, egui::Button::new(label)).clicked() || (save && self.editor.dirty) {
                    if let Some(path) = editing.path() {
                        match config::write_file(&path, &self.editor.text) {
                            Ok(()) => {
                                self.editor.dirty = false;
                                self.editor.message = None;
                                if editing.id == mode.id {
                                    self.send(Command::ReloadMode);
                                }
                            }
                            Err(e) => self.editor.message = Some(format!("cannot save: {e}")),
                        }
                    }
                }
                if ui.add_enabled(self.editor.dirty, egui::Button::new("Revert")).clicked() {
                    self.editor = Editor::default();
                }
            }
        });
        if self.editor.dirty && editing.id != mode.id {
            ui.label(RichText::new("Unsaved changes to another mode: save or revert them.").color(DANGER));
        }
        if let Some(message) = &self.editor.message {
            ui.label(RichText::new(message).color(DANGER));
        }
        if let Some(error) = &mode.error {
            ui.label(RichText::new(error).color(DANGER).monospace());
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            let edit = egui::TextEdit::multiline(&mut self.editor.text)
                .code_editor()
                .interactive(!editing.builtin)
                .desired_width(f32::INFINITY)
                .desired_rows(30);
            if ui.add(edit).changed() {
                self.editor.dirty = true;
            }
        });
    }

    fn routing(&mut self, ui: &mut egui::Ui) {
        let (mode, toys, routing) = {
            let s = self.shared.lock().unwrap();
            (s.mode.clone(), s.intiface.toys.clone(), s.settings.routing.clone())
        };
        let Some(info) = mode.info else {
            ui.label("No mode loaded.");
            return;
        };
        if toys.is_empty() {
            ui.label("No toy connected in Intiface.");
            return;
        }
        ui.label("Which toys each channel of the mode drives (all their actuators).");
        egui::Grid::new("routing").striped(true).show(ui, |ui| {
            ui.label("");
            for toy in &toys {
                ui.label(RichText::new(&toy.name).strong());
            }
            ui.end_row();
            for channel in &info.channels {
                ui.label(channel);
                let current: Vec<String> = match routing.get(channel) {
                    Some(list) => list.clone(),
                    None if channel == "main" => toys.iter().map(|t| t.name.clone()).collect(),
                    None => Vec::new(),
                };
                for toy in &toys {
                    let mut on = current.contains(&toy.name);
                    if ui.checkbox(&mut on, "").changed() {
                        let mut list: Vec<String> = current.iter().filter(|t| **t != toy.name).cloned().collect();
                        if on {
                            list.push(toy.name.clone());
                        }
                        self.send(Command::SetRouting { channel: channel.clone(), toys: list });
                    }
                }
                ui.end_row();
            }
        });
    }

    fn simulator(&mut self, ui: &mut egui::Ui) {
        ui.label("Fake game rumble and button presses, to test modes without a game.");
        ui.add(egui::Slider::new(&mut self.sim_strong, 0.0..=1.0).text("strong motor"));
        ui.add(egui::Slider::new(&mut self.sim_weak, 0.0..=1.0).text("weak motor"));
        ui.horizontal(|ui| {
            if ui.button("Hit (0.3 s at full strength)").clicked() {
                self.sim_hit_until = Some(Instant::now() + SIM_HIT);
            }
            if ui.button("Reset").clicked() {
                self.sim_strong = 0.0;
                self.sim_weak = 0.0;
            }
        });
        ui.separator();
        ui.label("Buttons (hold with the mouse):");
        ui.horizontal_wrapped(|ui| {
            for name in BUTTONS {
                let down = ui.button(name).is_pointer_button_down_on();
                let was_down = self.sim_held.contains(name);
                if down != was_down {
                    if down {
                        self.sim_held.insert(name);
                    } else {
                        self.sim_held.remove(name);
                    }
                    self.send(Command::SimButton { name: name.to_owned(), pressed: down });
                }
            }
        });
    }

    /// Sends the simulated rumble whenever it changes (sliders or an expiring hit).
    fn update_simulated_rumble(&mut self) {
        let hit = self.sim_hit_until.is_some_and(|t| Instant::now() < t);
        if !hit {
            self.sim_hit_until = None;
        }
        let wanted = if hit { (1.0, 1.0) } else { (self.sim_strong, self.sim_weak) };
        if wanted != self.sim_sent {
            self.sim_sent = wanted;
            self.send(Command::SimRumble { strong: wanted.0, weak: wanted.1 });
        }
    }

    fn log(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.only_mode_logs, "Only mode logs (log / print)");
        let lines: Vec<_> = self.logs.lock().unwrap().iter().cloned().collect();
        egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            for line in lines.iter().filter(|l| !self.only_mode_logs || l.target == "mode") {
                let color = match line.level {
                    log::Level::Error => DANGER,
                    log::Level::Warn => Color32::from_rgb(210, 150, 30),
                    _ => ui.visuals().text_color(),
                };
                ui.label(RichText::new(format!("{:8.2}  {}", line.seconds, line.message)).monospace().color(color));
            }
        });
    }
}

fn source_label(source: SourceChoice) -> &'static str {
    match source {
        SourceChoice::Proxy => "Source: proxy",
        SourceChoice::Ebpf => "Source: eBPF",
        SourceChoice::None => "Source: none",
    }
}

fn source_help(source: SourceChoice) -> &'static str {
    match source {
        SourceChoice::Proxy => "Virtual copy of the gamepad; works without privileges",
        SourceChoice::Ebpf => "Passive kernel probe; games keep the real gamepad (asks for your password)",
        SourceChoice::None => "No interception: simulator only",
    }
}

/// True when applying `preset` would leave `values` unchanged.
fn matches_values(info: &ModeInfo, preset: &BTreeMap<String, ParamValue>, values: &BTreeMap<String, ParamValue>) -> bool {
    info.params.iter().all(|def| {
        let wanted = preset.get(&def.name).and_then(|v| def.accept(v)).unwrap_or_else(|| def.default.clone());
        values.get(&def.name) == Some(&wanted)
    })
}

/// Draws a parameter control; returns the new value when the user changed it.
fn param_widget(ui: &mut egui::Ui, def: &ParamDef, value: &ParamValue) -> Option<ParamValue> {
    match (&def.kind, value) {
        (ParamKind::Number { min, max, step }, ParamValue::Number(v)) => {
            let mut v = *v;
            let mut slider = egui::Slider::new(&mut v, *min..=*max).text(&def.label);
            if let Some(step) = step {
                slider = slider.step_by(*step);
            }
            ui.add(slider).changed().then_some(ParamValue::Number(v))
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => {
            let mut b = *b;
            ui.checkbox(&mut b, &def.label).changed().then_some(ParamValue::Bool(b))
        }
        (ParamKind::Choice(options), ParamValue::Text(current)) => {
            combo(ui, def, current, options.iter().map(String::as_str))
        }
        (ParamKind::Button, ParamValue::Text(current)) => combo(ui, def, current, BUTTONS.iter().copied()),
        _ => None,
    }
}

fn combo<'a>(
    ui: &mut egui::Ui,
    def: &ParamDef,
    current: &str,
    options: impl Iterator<Item = &'a str>,
) -> Option<ParamValue> {
    let mut selected = current.to_owned();
    egui::ComboBox::from_label(&def.label).selected_text(&selected).show_ui(ui, |ui| {
        for option in options {
            ui.selectable_value(&mut selected, option.to_owned(), option);
        }
    });
    (selected != current).then_some(ParamValue::Text(selected))
}
