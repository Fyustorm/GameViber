//! Games page: the library of games, then each game by breadcrumb — its
//! modes (a compact list, each mode with a page of its own), its inputs
//! (`inputs.rs`, with captures and indicators in `screen.rs`) and its sessions.
//! Built-in modes can also be played without a game.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::{App, GameView, Route};
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::mode::prompt::Depth;

const CARD_MIN_WIDTH: f32 = 300.0;
const CARD_HEIGHT: f32 = 132.0;

#[derive(Default)]
pub struct State {
    /// The "Add a game" dialog: the name typed, and whether to link the running executable.
    adding: Option<(String, bool)>,
    /// A game just asked for, to open once the engine has it: its name and the view.
    pending: Option<(String, GameView)>,
    /// The game's name being edited.
    renaming: Option<String>,
    confirm_delete: bool,
    /// Built-in mode picked to add to a game.
    builtin: String,
}

impl State {
    pub(super) fn open_dialog(&mut self) {
        self.adding.get_or_insert_with(|| (String::new(), false));
    }

    pub(super) fn close_dialog(&mut self) {
        self.adding = None;
    }

    /// Opens the game named `name` once the engine lists it.
    pub(super) fn open_when_listed(&mut self, name: &str, view: GameView) {
        self.pending = Some((name.to_owned(), view));
    }
}

impl App {
    pub(super) fn games_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.open_pending_game(s);
        let mode_page = matches!(self.route, Route::FreeMode | Route::Game { view: GameView::Mode, .. });
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.sharing_ui(ui);
            if mode_page && self.feedback.open {
                self.feedback_page(ui, s);
                return;
            }
            // The captures and indicators fit the window: only the image edited takes what is left.
            if matches!(self.route, Route::Game { view: GameView::Screen, .. }) {
                self.route_page(ui, s);
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| self.route_page(ui, s));
            }
        });
        self.add_game_dialog(ui.ctx(), s);
    }

    fn route_page(&mut self, ui: &mut egui::Ui, s: &Shared) {
        match self.route.clone() {
            Route::Library => self.library(ui, s),
            Route::BuiltIn => self.builtin_modes(ui, s),
            Route::FreeMode => {
                let name = s.mode.info.as_ref().map_or("Mode".to_owned(), |i| i.name.clone());
                self.breadcrumb(ui, &[("Games", Some(Route::Library)), ("Built-in modes", Some(Route::BuiltIn)), (&name, None)]);
                self.mode_page(ui, s);
            }
            Route::Game { id, view } => match s.games.iter().find(|g| g.id == id) {
                Some(game) => self.game_page(ui, s, game, view),
                None => {
                    self.route = Route::Library;
                }
            },
        }
    }

    /// Opens a game created a moment ago, once the engine lists it.
    fn open_pending_game(&mut self, s: &Shared) {
        let Some((name, view)) = &self.games.pending else { return };
        if let Some(game) = s.games.iter().filter(|g| g.name == *name).max_by_key(|g| g.id.len()) {
            self.route = Route::Game { id: game.id.clone(), view: *view };
            self.games.pending = None;
        }
    }

    /// A path of links; the last item is the current page.
    pub(super) fn breadcrumb(&mut self, ui: &mut egui::Ui, items: &[(&str, Option<Route>)]) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (i, (label, route)) in items.iter().enumerate() {
                if i > 0 {
                    ui.label(muted("›"));
                }
                match route {
                    Some(route) => {
                        let link = ui.add(egui::Label::new(muted(*label)).sense(egui::Sense::click()));
                        if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.route = route.clone();
                            self.feedback.open = false;
                        }
                    }
                    None => {
                        ui.label(RichText::new(*label).color(TEXT));
                    }
                }
            }
        });
        ui.add_space(6.0);
    }

    fn library(&mut self, ui: &mut egui::Ui, s: &Shared) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                heading(ui, "Games");
                ui.label(muted(
                    "Each game has its modes, and each mode the inputs GameViber reads from the game for it: its phases, \
                     its screen, values from other programs. Set them up before playing or while playing.",
                ));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary("+ Add a game")).clicked() {
                    self.games.adding = Some((String::new(), s.unlinked_executable.is_some()));
                }
                let import = ui
                    .add_enabled(!self.sharing.busy(), egui::Button::new("Import a mode"))
                    .on_hover_text("A .gameviber file someone shared: a mode with its inputs and its game");
                if import.clicked() {
                    self.import_mode();
                }
            });
        });
        ui.add_space(10.0);
        if let Some(exe) = &s.unlinked_executable {
            card(PANEL).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    dot(ui, WARN);
                    ui.label(RichText::new(exe).strong());
                    ui.label("is running with the overlay and belongs to no game.");
                    if ui.button("Add it as a game").clicked() {
                        let name = exe.strip_suffix(".exe").unwrap_or(exe).to_owned();
                        self.games.adding = Some((name, true));
                    }
                    if !s.games.is_empty() {
                        egui::ComboBox::from_id_salt("link-exe").selected_text("Link it to...").show_ui(ui, |ui| {
                            for game in &s.games {
                                if ui.selectable_label(false, &game.name).clicked() {
                                    self.send(Command::LinkExecutable { game: game.id.clone(), executable: exe.clone() });
                                }
                            }
                        });
                    }
                });
            });
            ui.add_space(10.0);
        }
        let count = s.games.len() + 1;
        let mut open = None;
        tile_grid(ui, count, CARD_MIN_WIDTH, CARD_HEIGHT, |ui, i, size| {
            if let Some(game) = s.games.get(i) {
                if game_card(ui, s, game, size) {
                    open = Some(Route::Game { id: game.id.clone(), view: GameView::Modes });
                }
            } else {
                let response = card(BG).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::same(14)).show(ui, |ui| {
                    ui.set_min_size(size - egui::vec2(28.0, 28.0));
                    ui.label(RichText::new("Any other game").strong().size(15.0));
                    ui.label(muted("The built-in modes work with every game: pick one to play right away."));
                });
                if response.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open = Some(Route::BuiltIn);
                }
            }
        });
        if let Some(route) = open {
            self.route = route;
        }
    }

    fn builtin_modes(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.breadcrumb(ui, &[("Games", Some(Route::Library)), ("Built-in modes", None)]);
        heading(ui, "Built-in modes");
        ui.label(muted("They suit a whole genre but know nothing about your game. Pick one to play now."));
        ui.add_space(8.0);
        if let Some(id) = super::play::mode_tiles(ui, s, false) {
            self.send(Command::SelectMode(id));
            self.route = Route::FreeMode;
        }
        if s.mode.info.is_some() && s.game.is_none() && ui.link("Back to the mode being played").clicked() {
            self.route = Route::FreeMode;
        }
    }

    fn game_page(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, view: GameView) {
        let at = |view| Some(Route::Game { id: game.id.clone(), view });
        match view {
            GameView::Mode => {
                let name = s.mode.info.as_ref().map_or("Mode".to_owned(), |i| i.name.clone());
                self.breadcrumb(ui, &[("Games", Some(Route::Library)), (&game.name, at(GameView::Modes)), ("Modes", at(GameView::Modes)), (&name, None)]);
                self.mode_page(ui, s);
                return;
            }
            GameView::Screen => {
                self.breadcrumb(
                    ui,
                    &[("Games", Some(Route::Library)), (&game.name, at(GameView::Modes)), ("Inputs", at(GameView::Inputs)), ("Captures and indicators", None)],
                );
                self.screen_page(ui, s, game);
                return;
            }
            _ => {}
        }
        let tab = match view {
            GameView::Inputs => "Inputs",
            GameView::Sessions => "Sessions",
            _ => "Modes",
        };
        self.breadcrumb(ui, &[("Games", Some(Route::Library)), (&game.name, at(GameView::Modes)), (tab, None)]);
        self.game_header(ui, s, game);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (v, label) in [(GameView::Modes, format!("Modes · {}", game.modes.len())), (GameView::Inputs, "Inputs".to_owned()), (GameView::Sessions, "Sessions".to_owned())] {
                if ui.selectable_label(view == v, RichText::new(label).size(15.0)).clicked() {
                    self.route = Route::Game { id: game.id.clone(), view: v };
                }
            }
        });
        ui.separator();
        ui.add_space(6.0);
        match view {
            GameView::Inputs => self.inputs_page(ui, s, game),
            GameView::Sessions => self.game_sessions(ui, s, game),
            _ => self.game_modes(ui, s, game),
        }
    }

    /// The game's name, whether it is being played, the executables it runs as.
    fn game_header(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let playing = s.game.as_ref().is_some_and(|g| g.id == game.id);
        ui.horizontal(|ui| {
            match &mut self.games.renaming {
                Some(name) => {
                    ui.add(egui::TextEdit::singleline(name).font(egui::TextStyle::Heading).desired_width(320.0));
                    let name = name.trim().to_owned();
                    if ui.add_enabled(!name.is_empty(), egui::Button::new("Save")).clicked() {
                        self.send(Command::SaveGame(Game { name, ..game.clone() }));
                        self.games.renaming = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.games.renaming = None;
                    }
                }
                None => {
                    heading(ui, &game.name);
                    if ui.small_button("Rename").clicked() {
                        self.games.renaming = Some(game.name.clone());
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.games.confirm_delete {
                    let delete = egui::Button::new(RichText::new("Delete for good").color(egui::Color32::WHITE)).fill(DANGER);
                    if ui.add(delete).clicked() {
                        self.send(Command::DeleteGame(game.id.clone()));
                        self.games.confirm_delete = false;
                        self.route = Route::Library;
                    }
                    if ui.button("Cancel").clicked() {
                        self.games.confirm_delete = false;
                    }
                    ui.label(RichText::new("Delete this game? Its modes stay.").color(DANGER_TEXT));
                } else {
                    if ui.small_button("🗑 Delete").clicked() {
                        self.games.confirm_delete = true;
                    }
                    if playing {
                        pill(ui, "Playing", ON_ACCENT, ACCENT);
                    } else if ui.add(primary("Play this game")).on_hover_text("Its modes become the ones to play").clicked() {
                        self.send(Command::SelectGame(Some(game.id.clone())));
                    }
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let running = s.running_executable.as_deref().is_some_and(|exe| game.runs_as(exe));
            if running {
                dot(ui, OK);
                ui.label(RichText::new("Running").color(OK));
            }
            if game.executables.is_empty() {
                ui.label(muted("Linked to no executable: pick it in the top bar when you play it."));
            } else {
                ui.label(muted(format!("Runs as {}", game.executables.join(", "))));
                if ui.small_button("Unlink").on_hover_text("Pick the game by hand instead").clicked() {
                    self.send(Command::SaveGame(Game { executables: Vec::new(), ..game.clone() }));
                }
            }
            if let Some(exe) = &s.unlinked_executable {
                if ui.small_button(format!("Link {exe}")).on_hover_text("The game becomes the active one whenever it runs").clicked() {
                    self.send(Command::LinkExecutable { game: game.id.clone(), executable: exe.clone() });
                }
            }
        });
    }

    fn game_modes(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let playing_game = s.game.as_ref().is_some_and(|g| g.id == game.id);
        if game.modes.is_empty() {
            ui.label(muted("No mode yet: get one made by an AI assistant below, or add a built-in one."));
        }
        let mut open = None;
        let mut remove = None;
        let mut export = None;
        card(PANEL).inner_margin(Margin::same(0)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for id in &game.modes {
                let entry = s.modes.iter().find(|e| e.id == *id);
                let info = s.catalog.get(id).and_then(|r| r.as_ref().ok());
                let name = info.map(|i| i.name.clone()).or_else(|| entry.map(|e| e.key.clone())).unwrap_or_else(|| id.clone());
                let kind = match (entry, s.catalog.get(id)) {
                    (None, _) => "missing file".to_owned(),
                    (_, Some(Err(_))) => "does not load".to_owned(),
                    (Some(e), _) if e.builtin => "Built-in".to_owned(),
                    (Some(_), Some(Ok(i))) if !i.author.is_empty() => format!("by {}", i.author),
                    _ => "Custom".to_owned(),
                };
                let active = playing_game && s.mode.id == *id;
                egui::Frame::new().fill(if active { SELECTED_BG } else { PANEL }).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&name).strong().size(15.0));
                            let description = info.map(|i| i.description.as_str()).unwrap_or("");
                            ui.label(muted(if description.is_empty() { kind.clone() } else { format!("{kind} · {description}") }).size(12.0));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Remove").on_hover_text("Take it out of this game (its file stays)").clicked() {
                                remove = Some(id.clone());
                            }
                            if entry.is_some() && ui.button("Settings ›").clicked() {
                                open = Some(id.clone());
                            }
                            if entry.is_some_and(|e| !e.builtin) {
                                let button = ui
                                    .add_enabled(!self.sharing.busy(), egui::Button::new("Export"))
                                    .on_hover_text("Save it in a file to share, with its inputs");
                                if button.clicked() {
                                    export = Some(id.clone());
                                }
                            }
                            if active {
                                pill(ui, "Playing", ON_ACCENT, ACCENT);
                            } else if entry.is_some() && ui.button("Play").clicked() {
                                if !playing_game {
                                    self.send(Command::SelectGame(Some(game.id.clone())));
                                }
                                self.send(Command::SelectMode(id.clone()));
                            }
                        });
                    });
                });
                ui.add(egui::Separator::default().spacing(0.0));
            }
        });
        if let Some(id) = open {
            if !playing_game {
                self.send(Command::SelectGame(Some(game.id.clone())));
            }
            if s.mode.id != id {
                self.send(Command::SelectMode(id));
            }
            self.route = Route::Game { id: game.id.clone(), view: GameView::Mode };
        }
        if let Some(mode) = export {
            self.export_mode(game, &mode);
        }
        if let Some(mode) = remove {
            self.send(Command::RemoveGameMode { game: game.id.clone(), mode });
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("New mode").strong());
            if ui.button("✨ Quick · 2 minutes").on_hover_text("An AI assistant writes it from the rumble, the buttons, phases and impacts").clicked() {
                self.open_generator_for(game, Depth::Quick);
            }
            let advanced = egui::Button::new("✨ Advanced · with the active mode's inputs").stroke(egui::Stroke::new(1.0, ACCENT));
            if ui.add(advanced).on_hover_text("Also its indicators, captures and values from other programs").clicked() {
                self.open_generator_for(game, Depth::Advanced);
            }
            let builtins: Vec<_> = s.modes.iter().filter(|e| e.builtin && !game.modes.contains(&e.id)).collect();
            egui::ComboBox::from_id_salt("add-builtin").selected_text("Add a built-in mode").show_ui(ui, |ui| {
                for entry in builtins {
                    let name = s.catalog.get(&entry.id).and_then(|r| r.as_ref().ok()).map_or(entry.key.clone(), |i| i.name.clone());
                    if ui.selectable_label(false, name).clicked() {
                        self.games.builtin = entry.id.clone();
                    }
                }
            });
            if ui.button("Empty mode").on_hover_text("Write it yourself in Creator").clicked() {
                self.create_mode("my-mode", &crate::config::NEW_MODE_TEMPLATE.replace("NAME", "My mode"), None);
            }
        });
        if !self.games.builtin.is_empty() {
            let mode = std::mem::take(&mut self.games.builtin);
            self.send(Command::AddGameMode { game: game.id.clone(), mode });
        }
    }

    fn game_sessions(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        ui.label(muted(format!(
            "Sessions recorded while playing it. Hold {} on the gamepad when something feels wrong: the last \
             minutes are saved, to fix the mode from its page (Doesn't feel right?).",
            crate::gamepad::combo_text(&s.settings.mark_combo)
        )));
        ui.add_space(8.0);
        let sessions: Vec<_> =
            s.recordings.iter().filter(|r| r.header.game.as_deref().is_some_and(|g| g == game.name || game.runs_as(g))).collect();
        if sessions.is_empty() {
            ui.label(muted("None yet."));
            return;
        }
        egui::Grid::new("game-sessions").num_columns(4).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
            for info in sessions {
                let h = &info.header;
                ui.label(&h.started);
                ui.label(muted(format!("{} · {:.0} s{}", h.mode, h.duration, if h.marks > 0 { format!(" · {} marked", h.marks) } else { String::new() })));
                if ui.button("Replay").on_hover_text("Feeds it to the mode being played (graphs only)").clicked() {
                    self.send(Command::Replay { path: info.path.clone(), to_toys: false });
                }
                if ui.small_button("Delete").clicked() {
                    self.send(Command::DeleteRecording(info.path.clone()));
                }
                ui.end_row();
            }
        });
    }

    fn add_game_dialog(&mut self, ctx: &egui::Context, s: &Shared) {
        let Some((name, link)) = &mut self.games.adding else { return };
        let mut done: Option<GameView> = None;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("add-game")).show(ctx, |ui| {
            ui.set_width(520.0);
            heading(ui, "Add a game");
            ui.label(muted("No need to launch it: give it a name, then set up its inputs and ask for modes whenever you like."));
            ui.add_space(10.0);
            ui.label(RichText::new("Name").strong());
            ui.add(egui::TextEdit::singleline(name).hint_text("e.g. Metaphor: ReFantazio").desired_width(f32::INFINITY));
            ui.label(muted("AI assistants research the game by this name: write it as the game calls itself.").size(12.0));
            ui.add_space(10.0);
            ui.label(RichText::new("How GameViber recognizes it when it runs").strong());
            if let Some(exe) = &s.unlinked_executable {
                ui.radio_value(link, true, format!("{exe}, running now: it becomes the active game whenever it runs"));
            }
            ui.radio_value(link, false, "Not linked: I pick it in the top bar when I play it");
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ok = !name.trim().is_empty();
                if ui.add_enabled(ok, primary("Add, then get a mode")).clicked() {
                    done = Some(GameView::Modes);
                }
                if ui.add_enabled(ok, egui::Button::new("Add, then set up its inputs")).clicked() {
                    done = Some(GameView::Inputs);
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if let Some(view) = done {
            let (name, link) = self.games.adding.take().unwrap_or_default();
            let executable = if link { s.unlinked_executable.clone() } else { None };
            self.send(Command::CreateGame { name: name.trim().to_owned(), executable });
            self.games.pending = Some((name.trim().to_owned(), view));
            self.page = super::Page::Games;
        } else if close || modal.should_close() {
            self.games.adding = None;
        }
    }
}

/// A game in the library: its name, whether it runs, its modes and inputs. Returns true when clicked.
fn game_card(ui: &mut egui::Ui, s: &Shared, game: &Game, size: egui::Vec2) -> bool {
    let playing = s.game.as_ref().is_some_and(|g| g.id == game.id);
    let response = card(PANEL)
        .stroke(egui::Stroke::new(1.0, if playing { ACCENT } else { LINE }))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_min_size(size - egui::vec2(28.0, 28.0));
            ui.horizontal(|ui| {
                ui.label(RichText::new(&game.name).strong().size(15.0));
                if playing {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| pill(ui, "Playing", ON_ACCENT, ACCENT));
                }
            });
            let running = s.running_executable.as_deref().is_some_and(|exe| game.runs_as(exe));
            let status = match (running, game.executables.first()) {
                (true, Some(exe)) => RichText::new(format!("● Running · {exe}")).color(OK),
                (false, Some(exe)) => muted(format!("Runs as {exe}")),
                (_, None) => muted("Picked by hand"),
            };
            ui.label(status.size(12.0));
            let playing_mode = if playing { s.mode.info.as_ref().map(|i| format!(" · playing {}", i.name)) } else { None };
            ui.label(muted(format!("{} modes{}", game.modes.len(), playing_mode.unwrap_or_default())));
        });
    response.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}
