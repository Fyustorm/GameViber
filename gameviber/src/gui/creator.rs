//! Creator page: the active mode's workspace, in tabs that can be visited in
//! any order — its phases, captures and indicators, values from other
//! programs, its script (written by an AI assistant, started from a built-in
//! mode, or by hand), sessions and a simulator to try it without the game,
//! logs — and how a mode works one click away. What the mode does while
//! playing is on the Live page.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Margin, RichText};

use super::diagram::{mode_diagram, Part};
use super::library::mode_game;
use super::luau;
use super::theme::*;
use super::{main_of, App, Page, Route};
use crate::config::{self, ModeEntry, NEW_MODE_TEMPLATE};
use crate::engine::{Command, Shared, RECENT_SECS};
use crate::gamepad::BUTTONS;
use crate::session::RecordingInfo;

const SIM_HIT: Duration = Duration::from_millis(300);
/// From this width the header's buttons sit on its first line, on the right.
const WIDE_HEADER: f32 = 860.0;

/// The Creator's tabs.
#[derive(PartialEq, Clone, Copy, Default)]
pub(super) enum Tab {
    #[default]
    Phases,
    /// Captures and indicators.
    Screen,
    /// Values from other programs.
    Programs,
    Script,
    Sessions,
    Logs,
}

impl Tab {
    /// Where the tab's part is in the diagram.
    fn part(self) -> Option<Part> {
        match self {
            Tab::Phases => Some(Part::Phases),
            Tab::Screen => Some(Part::Image),
            Tab::Programs => Some(Part::Programs),
            Tab::Script => Some(Part::Script),
            Tab::Sessions | Tab::Logs => None,
        }
    }
}

/// How the script is got.
#[derive(PartialEq, Clone, Copy)]
pub(super) enum Way {
    Ai,
    BuiltIn,
    Write,
}

#[derive(Default)]
pub(super) struct Editor {
    /// Mode id the buffer was loaded from.
    pub(super) id: String,
    pub(super) text: String,
    dirty: bool,
    pub(super) message: Option<String>,
    /// Last highlighted text and error line, with its layout.
    highlighted: Option<(String, Option<usize>, LayoutJob)>,
    /// Line to scroll to on the next frame.
    goto: Option<usize>,
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
    pub(super) tab: Tab,
    /// How a mode works, shown above the tab.
    pub(super) help: bool,
    /// How the script is got; None: as fits the script (an AI assistant for a new mode).
    pub(super) way: Option<Way>,
    pub(super) editor: Editor,
    pub sim: Simulator,
    only_mode_logs: bool,
    /// Replays drive the toys too.
    replay_to_toys: bool,
    /// Recording whose deletion is being confirmed.
    deleting: Option<PathBuf>,
}

impl State {
    pub fn show_sessions(&mut self) {
        self.tab = Tab::Sessions;
    }

    /// A mode just made: its phases first, its script asked of an AI assistant.
    pub(super) fn open_new(&mut self) {
        self.tab = Tab::Phases;
        self.help = false;
        self.way = None;
    }

    /// The tab shows the game's image (captures, phases recognized from it).
    pub(super) fn watches_screen(&self) -> bool {
        matches!(self.tab, Tab::Phases | Tab::Screen)
    }
}

impl App {
    pub(super) fn creator_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.load_editor(s);
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(22, 14));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.creator_header(ui, s);
            self.creator_tabs(ui, s);
            if self.creator.help {
                ui.add_space(8.0);
                self.help_panel(ui);
            }
            ui.add_space(10.0);
            let game = mode_game(s).cloned();
            let tab = self.creator.tab;
            match (tab, &game) {
                (Tab::Sessions, _) => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.sessions(ui, s));
                }
                (Tab::Logs, _) => self.log(ui),
                (Tab::Script, _) => self.script_tab(ui, s, game.as_ref()),
                (_, None) => self.no_game(ui, s),
                (Tab::Screen, Some(game)) => self.screen_page(ui, s, game),
                (Tab::Phases, Some(game)) => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.phases_tab(ui, s, game));
                }
                (Tab::Programs, Some(game)) => {
                    egui::ScrollArea::vertical().show(ui, |ui| self.programs_tab(ui, s, game));
                }
            }
        });
    }

    /// The editor holds the active mode's script (unless changes to another wait).
    fn load_editor(&mut self, s: &Shared) {
        let editor = &mut self.creator.editor;
        if editor.id != s.mode.id && !editor.dirty {
            *editor = Editor { id: s.mode.id.clone(), ..Default::default() };
            match ModeEntry::from_id(&s.mode.id).source() {
                Ok(text) => editor.text = text,
                Err(e) => editor.message = Some(format!("cannot read: {e}")),
            }
        }
    }

    /// The mode's name, its game, the way to its page, other modes, the help.
    fn creator_header(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let entry = ModeEntry::from_id(&s.mode.id);
        let name = s.mode.info.as_ref().map_or_else(|| entry.key.clone(), |i| i.name.clone());
        let wide = ui.available_width() >= WIDE_HEADER;
        let draft = is_draft(&self.creator.editor.text);
        let title = |ui: &mut egui::Ui| {
            ui.label(RichText::new(&name).size(20.0).strong());
            let game = mode_game(s).map_or_else(|| if entry.builtin { "any game".to_owned() } else { "no game".to_owned() }, |g| g.name.clone());
            ui.label(muted(format!("for {game}")).size(14.0));
            if entry.builtin {
                pill(ui, "Built-in", TEXT, RAISED);
            } else if draft {
                pill(ui, "To write", ON_ACCENT, WARN);
            }
        };
        let mut buttons = |ui: &mut egui::Ui| {
                if ui.button("Live ›").on_hover_text("What the mode does while you play, its main settings").clicked() {
                    self.page = Page::Live;
                }
                let help = egui::Button::new(RichText::new("?  How a mode works").color(if self.creator.help { ON_ACCENT } else { TEXT }))
                    .fill(if self.creator.help { GAME } else { RAISED })
                    .corner_radius(14);
                if ui.add(help).clicked() {
                    self.creator.help = !self.creator.help;
                }
                if ui.button("Its page ›").on_hover_text("Its settings, sharing it, its game").clicked() {
                    self.page = Page::Library;
                    self.route = Route::Mode;
                }
                egui::ComboBox::from_id_salt("creator-mode").selected_text("Another mode").width(150.0).show_ui(ui, |ui| {
                    for entry in s.modes.iter().filter(|e| !e.builtin) {
                        let name = s.catalog.get(&entry.id).and_then(|r| r.as_ref().ok()).map_or(entry.key.clone(), |i| i.name.clone());
                        if ui.selectable_label(main_of(&s.mode.id) == entry.id, name).clicked() {
                            if let Some(game) = s.games.iter().find(|g| g.modes.contains(&entry.id)) {
                                if s.game.as_ref().is_none_or(|p| p.id != game.id) {
                                    self.send(Command::SelectGame(Some(game.id.clone())));
                                }
                            }
                            self.send(Command::SelectMode(entry.id.clone()));
                        }
                    }
                    ui.separator();
                    if ui.selectable_label(false, "+ Create a mode").clicked() {
                        self.open_create(None);
                    }
                });
        };
        if wide {
            ui.horizontal(|ui| {
                title(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui));
            });
        } else {
            ui.horizontal_wrapped(title);
            ui.horizontal_wrapped(|ui| ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| buttons(ui)));
        }
        if entry.builtin {
            let mut duplicate = false;
            card(PANEL).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label("Built-in modes read nothing set up for a game and cannot be changed.");
                    duplicate = ui.button("Duplicate it to make your own").clicked();
                    if ui.add(primary("+ Create a mode")).clicked() {
                        self.open_create(None);
                    }
                });
            });
            if duplicate {
                self.duplicate_mode(&s.mode.id);
            }
        }
    }

    fn creator_tabs(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let inputs = s.mode_inputs.as_ref();
        let count = |n: usize, none: &str| if n == 0 { none.to_owned() } else { n.to_string() };
        let phases = inputs.map_or(0, |i| i.phases.len());
        let screen = inputs.map_or(0, |i| i.captures.len() + i.zones.len());
        let programs = inputs.map_or(0, |i| i.external.len());
        let script = if s.mode.error.is_some() || s.mode.info.is_none() {
            ("does not load", DANGER_TEXT)
        } else if is_draft(&self.creator.editor.text) {
            ("to write", WARN)
        } else {
            ("loads", OK)
        };
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            let mut tab = |ui: &mut egui::Ui, tab: Tab, label: &str, state: String, color: egui::Color32| {
                let selected = self.creator.tab == tab;
                let mut job = LayoutJob::default();
                let strong = egui::TextFormat { font_id: egui::FontId::proportional(15.0), color: if selected { TEXT } else { MUTED }, ..Default::default() };
                job.append(label, 0.0, strong);
                if !state.is_empty() {
                    let color = if selected { TEXT } else { color };
                    job.append(&format!(" · {state}"), 0.0, egui::TextFormat { font_id: egui::FontId::proportional(12.5), color, ..Default::default() });
                }
                if ui.selectable_label(selected, job).clicked() {
                    self.creator.tab = tab;
                }
            };
            tab(ui, Tab::Phases, "Phases", count(phases, "recommended"), if phases > 0 { OK } else { GAME });
            tab(ui, Tab::Screen, "Captures & indicators", count(screen, "optional"), MUTED);
            tab(ui, Tab::Programs, "Other programs", count(programs, "optional"), MUTED);
            tab(ui, Tab::Script, "Script", script.0.to_owned(), script.1);
            ui.label(muted("|"));
            tab(ui, Tab::Sessions, "Sessions", String::new(), MUTED);
            tab(ui, Tab::Logs, "Logs", String::new(), MUTED);
        });
        ui.separator();
    }

    /// How a mode works, the tab open outlined.
    fn help_panel(&mut self, ui: &mut egui::Ui) {
        card(SIDEBAR).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("How a mode works").strong().size(15.0));
                ui.label(muted("No fixed order: play, feel, change anything, play again."));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Hide").clicked() {
                        self.creator.help = false;
                    }
                });
            });
            ui.add_space(4.0);
            mode_diagram(ui, self.creator.tab.part());
        });
    }

    /// The mode's inputs need its game.
    fn no_game(&mut self, ui: &mut egui::Ui, s: &Shared) {
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if ModeEntry::from_id(&s.mode.id).builtin {
                ui.label("Built-in modes read no inputs set up for a game: duplicate it, or create a mode for your game.");
                return;
            }
            ui.label("What a mode reads belongs to a game: give this mode one on its page.");
            if ui.button("Its page ›").clicked() {
                self.page = Page::Library;
                self.route = Route::Mode;
            }
        });
    }

    /// The script, to read or write by hand.
    pub(super) fn script_editor(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mode = &s.mode;
        let editor = &mut self.creator.editor;
        let editing = ModeEntry::from_id(&editor.id);
        let save = ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S));
        let mut duplicate = false;
        let mut reload = false;
        let mut refresh = false;
        ui.horizontal(|ui| {
            // A full path would widen the page past the tools panel.
            let name = if editing.builtin { editing.id.clone() } else { editing.chunk_name() };
            ui.add(egui::Label::new(RichText::new(name).monospace()).truncate()).on_hover_text(&editing.id);
            let spec = format!("https://github.com/{}/blob/main/docs/spec-modes.md", crate::update::REPOSITORY);
            ui.hyperlink_to(muted("Mode API ↗").size(12.0), spec);
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
        let error_line = error.and_then(|e| luau::error_line(e, &editing.chunk_name()));
        if let Some(error) = error {
            ui.label(RichText::new(error).color(DANGER_TEXT).monospace());
            if let Some(line) = error_line {
                if ui.small_button(format!("Go to line {line}")).clicked() {
                    editor.goto = Some(line);
                }
            }
        }
        code_view(ui, editor, !editing.builtin, error_line);
        if reload {
            self.send(Command::ReloadMode);
        }
        if refresh {
            self.send(Command::RefreshModes);
        }
        if duplicate {
            self.duplicate_mode(&editing.id);
        }
    }

    fn simulator(&mut self, ui: &mut egui::Ui) {
        let sim = &mut self.creator.sim;
        ui.label(muted("Fake game rumble and button presses.").size(12.5));
        ui.spacing_mut().slider_width = 260.0;
        ui.add(egui::Slider::new(&mut sim.strong, 0.0..=1.0).text("strong motor"));
        ui.add(egui::Slider::new(&mut sim.weak, 0.0..=1.0).text("weak motor"));
        ui.horizontal(|ui| {
            if ui.button("Hit").on_hover_text("0.3 s at full strength").clicked() {
                sim.hit_until = Some(Instant::now() + SIM_HIT);
            }
            if ui.button("Reset").clicked() {
                sim.strong = 0.0;
                sim.weak = 0.0;
            }
        });
        ui.label(muted("Buttons (hold with the mouse):").size(12.5));
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

    /// Recording real play sessions and replaying them into the mode.
    fn sessions(&mut self, ui: &mut egui::Ui, s: &Shared) {
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Without the game").strong().size(15.0));
            self.simulator(ui);
        });
        ui.add_space(10.0);
        ui.label(muted(
            "Record the game's rumble and your buttons while you play, then replay them here to tune the mode \
             on a real fight without playing it again.",
        ));
        ui.add_space(4.0);
        match (s.recording, &s.replay) {
            (Some(secs), _) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("⏺ Recording {}", clock(secs))).strong().color(DANGER_TEXT));
                    if ui.add(primary("⏹ Stop and save")).clicked() {
                        self.send(Command::StopRecording);
                    }
                });
            }
            (None, Some(replay)) => {
                let header = &replay.info.header;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("▶ Replaying {}", title(&replay.info))).strong());
                    ui.label(muted(format!("{} / {}", clock(replay.position), clock(header.duration))));
                    if ui.button("⏹ Stop").clicked() {
                        self.send(Command::StopReplay);
                    }
                });
                let progress = if header.duration > 0.0 { replay.position / header.duration } else { 1.0 };
                // Sized to the panel, which would otherwise grow with it.
                meter(ui, ui.available_width(), progress, ACCENT);
                if !replay.to_toys {
                    ui.label(muted("Toys stay still: watch the Graphs tab.").size(12.0));
                }
            }
            (None, None) => {
                ui.horizontal(|ui| {
                    if ui.add(primary("⏺ Record a session")).on_hover_text("Start it, then play the game").clicked() {
                        self.send(Command::StartRecording);
                    }
                    let save = egui::Button::new(format!("Save the last {:.0} min", RECENT_SECS / 60.0));
                    if ui.add(save).on_hover_text("GameViber always keeps the last minutes of play in memory").clicked() {
                        self.send(Command::SaveRecent);
                    }
                });
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            eyebrow(ui, "Recordings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.creator.replay_to_toys, "Toys play the replay");
            });
        });
        if s.recordings.is_empty() {
            ui.label(muted("No recording yet."));
        }
        let busy = s.recording.is_some();
        let mut command = None;
        egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            for info in &s.recordings {
                card(PANEL).inner_margin(Margin::same(8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(title(info)).strong());
                    ui.label(
                        muted(format!("{} · {} · mode {}", info.header.started, clock(info.header.duration), info.header.mode))
                            .size(12.0),
                    );
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!busy, egui::Button::new("▶ Replay")).on_hover_text("Restarts the active mode").clicked() {
                            command = Some(Command::Replay { path: info.path.clone(), to_toys: self.creator.replay_to_toys });
                        }
                        if self.creator.deleting.as_ref() == Some(&info.path) {
                            if ui.button("Cancel").clicked() {
                                self.creator.deleting = None;
                            }
                            if ui.button(RichText::new("Delete").color(DANGER_TEXT)).clicked() {
                                command = Some(Command::DeleteRecording(info.path.clone()));
                                self.creator.deleting = None;
                            }
                        } else if ui.button("🗑").on_hover_text("Delete this recording").clicked() {
                            self.creator.deleting = Some(info.path.clone());
                        }
                    });
                });
            }
        });
        if let Some(command) = command {
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

/// The editor: line numbers (the error line in red) next to the highlighted source.
fn code_view(ui: &mut egui::Ui, editor: &mut Editor, editable: bool, error_line: Option<usize>) {
    const MARGIN: egui::Margin = Margin { left: 6, right: 6, top: 4, bottom: 4 };
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let Editor { text, dirty, highlighted, goto, .. } = editor;
    // Lines do not wrap: long ones scroll sideways.
    let width = ui.available_width();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Line numbers, laid out with the same font as the code so rows match.
            let lines = text.split('\n').count();
            let mut numbers = LayoutJob::default();
            for line in 1..=lines {
                let color = if Some(line) == error_line { DANGER_TEXT } else { IDLE };
                let format = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
                numbers.append(&format!("{line:>4} {}", if line < lines { "\n" } else { "" }), 0.0, format);
            }
            let galley = ui.fonts_mut(|f| f.layout_job(numbers));
            let gutter_width = galley.size().x;
            let gutter = ui
                .vertical(|ui| {
                    ui.add_space(MARGIN.top as f32);
                    ui.label(galley.clone())
                })
                .inner;
            if let Some(line) = goto.take() {
                if let Some(row) = galley.rows.get(line - 1) {
                    let rect = row.rect().translate(gutter.rect.min.to_vec2());
                    ui.scroll_to_rect(rect, Some(egui::Align::Center));
                }
            }
            let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
                let source = buffer.as_str();
                let stale = highlighted.as_ref().is_none_or(|(t, e, _)| t != source || *e != error_line);
                if stale {
                    *highlighted = Some((source.to_owned(), error_line, luau::highlight(source, font.clone(), error_line)));
                }
                let job = highlighted.as_ref().map(|(_, _, job)| job.clone()).unwrap_or_default();
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let edit = egui::TextEdit::multiline(text)
                .code_editor()
                .interactive(editable)
                .margin(MARGIN)
                .desired_width(width - gutter_width - 2.0 * MARGIN.left as f32 - 12.0)
                .desired_rows(30)
                .layouter(&mut layouter);
            if ui.add(edit).changed() {
                *dirty = true;
            }
        });
    });
}

/// The game a recording comes from, else its mode.
fn title(info: &RecordingInfo) -> &str {
    info.header.game.as_deref().unwrap_or(&info.header.mode)
}

/// "1:05"
fn clock(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// `source` with the mode named `name`.
pub(super) fn with_name(source: &str, name: &str) -> String {
    const KEY: &str = "name = \"";
    let Some(start) = source.find(KEY).map(|i| i + KEY.len()) else { return source.to_owned() };
    let Some(len) = source[start..].find('"') else { return source.to_owned() };
    format!("{}{}{}", &source[..start], name.replace('"', "'"), &source[start + len..])
}

/// The script is still the one a new mode starts with.
pub(super) fn is_draft(source: &str) -> bool {
    with_name(source, "NAME").trim() == NEW_MODE_TEMPLATE.trim()
}
