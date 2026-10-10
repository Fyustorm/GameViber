//! Library page: the player's modes, by game (the built-in ones under "Any
//! game"); creating a mode, which starts with its game and how its script is
//! got (what an AI assistant is asked for it, ticked there) before opening the
//! Creator; and the active mode's page: its
//! settings, what it reads, sharing it, and its game (how GameViber
//! recognizes it, which sound it listens to).

use eframe::egui::{self, Margin, RichText};

use super::creator::Way;
use super::diagram::mode_diagram;
use super::generator::wishes_ui;
use super::theme::*;
use super::{main_of, mode_icon, App, Page, Route};
use crate::config::{AudioSource, ModeEntry, NEW_MODE_TEMPLATE};
use crate::engine::{Command, Shared};
use crate::game::{slug, Game};
use crate::mode::prompt;
use crate::package::Inputs;

const TILE_MIN_WIDTH: f32 = 260.0;
const TILE_HEIGHT: f32 = 92.0;

/// The modes shown.
#[derive(Default, PartialEq, Clone)]
enum Filter {
    #[default]
    All,
    /// One game's, by id.
    Game(String),
    /// The built-in modes, for any game.
    AnyGame,
}

#[derive(Default)]
pub struct State {
    filter: Filter,
    /// The game typed on the Create page; None until the page fills it in.
    create: Option<String>,
    /// A game asked for to make a mode for, by name: the mode is made once the engine lists it.
    pending: Option<String>,
    /// A game just imported, by name: shown alone once the engine lists it.
    imported: Option<String>,
    /// The game's name being edited.
    renaming: Option<String>,
    confirm_delete_game: bool,
    /// How the Create page's mode gets its script (None: an AI assistant writes it).
    start: Option<Way>,
    /// How a mode works, shown on the Create page.
    diagram: bool,
}

impl State {
    /// Shows the game named `name` alone once the engine lists it.
    pub(super) fn show_when_listed(&mut self, name: &str) {
        self.imported = Some(name.to_owned());
    }
}

impl App {
    pub(super) fn library_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.create_pending_mode(s);
        self.filter_imported(s);
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.sharing_ui(ui);
            if self.route == Route::Mode && self.feedback.open {
                self.feedback_page(ui, s);
                return;
            }
            egui::ScrollArea::vertical().show(ui, |ui| match self.route {
                Route::Library => self.library(ui, s),
                Route::Create => self.create_page(ui, s),
                Route::Mode => self.mode_route(ui, s),
            });
        });
    }

    /// Opens the Create page, its game filled in with `game` (else the game being played).
    pub(super) fn open_create(&mut self, game: Option<&str>) {
        self.library.create = game.map(str::to_owned);
        self.page = Page::Library;
        self.route = Route::Create;
    }

    /// Plays the mode `id` of `game` (None: a built-in mode for any game) and opens its page.
    fn open_mode(&mut self, s: &Shared, game: Option<&Game>, id: &str) {
        if let Some(game) = game.filter(|g| s.game.as_ref().is_none_or(|p| p.id != g.id)) {
            self.send(Command::SelectGame(Some(game.id.clone())));
        }
        if main_of(&s.mode.id) != id {
            self.send(Command::SelectMode(id.to_owned()));
        }
        self.feedback.open = false;
        self.page = Page::Library;
        self.route = Route::Mode;
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
                            self.route = *route;
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

    // --- the library

    fn library(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.community_banner(ui, s);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                heading(ui, "Library");
                ui.label(muted("Your modes, by game. The one you open is the one played."));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(primary("+ Create a mode")).clicked() {
                    self.open_create(None);
                }
                let import = ui
                    .add_enabled(!self.sharing.busy(), egui::Button::new("Import a file"))
                    .on_hover_text("A .gameviber file someone shared: a mode with its inputs and its game");
                if import.clicked() {
                    self.import_mode();
                }
            });
        });
        ui.add_space(10.0);
        self.unlinked_executable(ui, s);

        let groups = groups(s);
        ui.horizontal_wrapped(|ui| {
            let mut chip = |ui: &mut egui::Ui, filter: Filter, label: String| {
                if ui.selectable_label(self.library.filter == filter, label).clicked() {
                    self.library.filter = filter;
                }
            };
            chip(ui, Filter::All, "All".to_owned());
            for (game, modes) in &groups {
                if let Some(game) = game {
                    chip(ui, Filter::Game(game.id.clone()), format!("{} · {}", game.name, modes.len()));
                }
            }
            chip(ui, Filter::AnyGame, "Any game".to_owned());
        });
        ui.add_space(10.0);
        if groups.is_empty() {
            card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No mode of your own yet").strong().size(16.0));
                ui.label(muted(
                    "Create one for the game you play: an AI assistant writes it for that game. Or find one other players \
                     made in Community, or play a built-in mode below.",
                ));
            });
            ui.add_space(12.0);
        }
        let mut open: Option<(Option<Game>, String)> = None;
        for (game, modes) in &groups {
            let shown = match &self.library.filter {
                Filter::All => true,
                Filter::Game(id) => game.as_ref().is_some_and(|g| g.id == *id),
                Filter::AnyGame => false,
            };
            if !shown {
                continue;
            }
            ui.horizontal(|ui| {
                let name = game.as_ref().map_or("In no game", |g| g.name.as_str());
                ui.label(RichText::new(name).strong().size(16.0));
                if let Some(game) = game {
                    let running = s.running_executable.as_deref().is_some_and(|exe| game.is_running(exe, s.running_app));
                    if running {
                        ui.label(RichText::new("● Running").color(OK).size(12.5));
                    }
                    if ui.small_button("+ Mode").on_hover_text(format!("Create another mode for {}", game.name)).clicked() {
                        self.open_create(Some(&game.name));
                    }
                }
            });
            ui.add_space(4.0);
            if let Some(id) = self.mode_grid(ui, s, game.as_ref(), modes) {
                open = Some((game.clone(), id));
            }
            ui.add_space(8.0);
        }
        if matches!(self.library.filter, Filter::All | Filter::AnyGame) {
            ui.label(RichText::new("Any game · built-in").strong().size(16.0).color(MUTED));
            ui.label(muted("Made for a genre, they know nothing of your game: good to try GameViber right away."));
            ui.add_space(4.0);
            let builtins: Vec<String> = s.modes.iter().filter(|e| e.builtin).map(|e| e.id.clone()).collect();
            if let Some(id) = self.mode_grid(ui, s, None, &builtins) {
                open = Some((None, id));
            }
        }
        if let Some((game, id)) = open {
            self.open_mode(s, game.as_ref(), &id);
        }
    }

    /// Tiles of the modes `ids`; returns the one clicked.
    fn mode_grid(&mut self, ui: &mut egui::Ui, s: &Shared, game: Option<&Game>, ids: &[String]) -> Option<String> {
        let mut clicked = None;
        let playing_game = game.is_some_and(|g| s.game.as_ref().is_some_and(|p| p.id == g.id)) || game.is_none();
        // As many columns as fit, even for one mode: a tile keeps its width.
        let columns = ((ui.available_width() + 12.0) / (TILE_MIN_WIDTH + 12.0)).floor().max(1.0) as usize;
        tile_grid(ui, ids.len().max(columns), TILE_MIN_WIDTH, TILE_HEIGHT, |ui, i, size| {
            let Some(id) = ids.get(i) else { return };
            let entry = ModeEntry::from_id(id);
            let info = s.catalog.get(id).and_then(|r| r.as_ref().ok());
            let name = info.map_or(entry.key.clone(), |i| i.name.clone());
            let kind = match s.catalog.get(id) {
                None if !s.modes.iter().any(|e| e.id == *id) => "missing file".to_owned(),
                Some(Err(_)) => "does not load".to_owned(),
                _ if entry.builtin => info.map_or(String::new(), |i| i.category.clone()),
                _ if self.origin_of(id).is_some_and(|o| !o.own) => "Community".to_owned(),
                Some(Ok(i)) if !i.author.is_empty() => format!("by {}", i.author),
                _ => "Yours".to_owned(),
            };
            let active = playing_game && main_of(&s.mode.id) == *id;
            let body = info.map_or(String::new(), |i| i.description.clone());
            let tile = Tile { icon: mode_icon(&entry), title: &name, subtitle: &kind, body: &body, selected: active, badge: active.then_some("Playing") };
            if tile.show(ui, size).clicked() {
                clicked = Some(id.clone());
            }
        });
        clicked
    }

    /// A game running with the overlay belongs to no game: making a mode for it is one click away.
    fn unlinked_executable(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let Some(exe) = &s.unlinked_executable else { return };
        let mut create = false;
        card(PANEL).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                dot(ui, WARN);
                ui.label(RichText::new(exe).strong());
                ui.label("is running and has no mode.");
                create = ui.button("Create a mode for it").clicked();
                if !s.games.is_empty() {
                    egui::ComboBox::from_id_salt("link-exe").selected_text("It is...").show_ui(ui, |ui| {
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
        if create {
            self.open_create(Some(exe_name(exe)));
        }
    }

    /// A game just imported is shown alone once listed.
    fn filter_imported(&mut self, s: &Shared) {
        let Some(name) = &self.library.imported else { return };
        if let Some(game) = s.games.iter().find(|g| g.name == *name) {
            self.library.filter = Filter::Game(game.id.clone());
            self.library.imported = None;
        }
    }

    // --- creating a mode

    fn create_page(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.breadcrumb(ui, &[("Library", Some(Route::Library)), ("Create a mode", None)]);
        let typed = self.library.create.get_or_insert_with(|| default_game(s)).clone();
        let mut name = typed.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("A mode for").size(20.0).strong());
            let edit = egui::TextEdit::singleline(&mut name).hint_text("the game's name, e.g. Hades II").font(egui::TextStyle::Heading).desired_width(360.0);
            ui.add(edit);
            if s.unlinked_executable.as_deref().is_some_and(|exe| slug(exe_name(exe)) == slug(&name)) || running_game(s).is_some_and(|g| slug(&g.name) == slug(&name)) {
                ui.label(RichText::new("● Running now").color(OK));
            }
        });
        let others: Vec<&Game> = s.games.iter().filter(|g| slug(&g.name) != slug(&name)).collect();
        if !others.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(muted("Or:"));
                for game in others {
                    if ui.small_button(&game.name).clicked() {
                        name = game.name.clone();
                    }
                }
            });
        }
        ui.label(muted("AI assistants look the game up by this name: write it as the game calls itself.").size(12.5));
        self.library.create = Some(name.clone());
        // What other players made for it, when the library knows the game.
        if let Some(game) = s.games.iter().find(|g| slug(&g.name) == slug(&name)).cloned() {
            ui.add_space(6.0);
            self.community_card(ui, s, &game);
        }
        ui.add_space(12.0);

        let start = self.library.start.unwrap_or(Way::Ai);
        card(PANEL).inner_margin(Margin::symmetric(20, 16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("How do you want to start?").strong().size(17.0));
            ui.add_space(4.0);
            for (way, label, note) in [
                (Way::Ai, "✨ An AI assistant writes it for this game", "Recommended"),
                (Way::BuiltIn, "Start from a built-in mode", "made for a genre"),
                (Way::Write, "Write it yourself", "in Luau"),
            ] {
                ui.horizontal(|ui| {
                    if ui.radio(start == way, label).clicked() {
                        self.library.start = Some(way);
                    }
                    ui.label(muted(note).size(12.5));
                });
            }
        });
        if start == Way::Ai {
            ui.add_space(10.0);
            card(PANEL).inner_margin(Margin::symmetric(20, 16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                wishes_ui(ui, &mut self.generator.new_wishes);
                ui.add_space(6.0);
                ui.label(RichText::new("Anything else?").strong());
                ui.add(
                    egui::TextEdit::multiline(&mut self.generator.new_instructions)
                        .hint_text("e.g. parries should feel like a release, not a hit")
                        .desired_width(f32::INFINITY)
                        .desired_rows(2),
                );
                ui.label(muted("Kept with the mode: the Creator's AI assistant tab builds the requests from all this.").size(12.0));
            });
        }
        if self.library.diagram {
            ui.add_space(10.0);
            card(SIDEBAR).inner_margin(Margin::symmetric(20, 16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("How a mode works").strong().size(17.0));
                ui.label(muted(
                    "All of it is set in the Creator. Only the script is needed; the rest makes the mode feel like the game.",
                ));
                ui.add_space(8.0);
                mode_diagram(ui, None);
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("⟲ No fixed order.").strong().color(GAME));
                    ui.label(muted("The mode is live while you make it: play, feel, change the phases or the script, play again."));
                });
            });
        }
        ui.add_space(12.0);
        let name = name.trim().to_owned();
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                self.library.create = None;
                self.route = Route::Library;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!name.is_empty(), primary("Open the Creator ›")).clicked() {
                    self.start_mode_for(s, &name);
                }
                let diagram = self.library.diagram;
                if ui.selectable_label(diagram, "How a mode works ?").on_hover_text("Also one click away in the Creator: ?").clicked() {
                    self.library.diagram = !diagram;
                }
            });
        });
    }

    /// Makes a mode for the game named `name`, adding the game first when the library has none by that name.
    fn start_mode_for(&mut self, s: &Shared, name: &str) {
        self.library.create = None;
        match s.games.iter().find(|g| slug(&g.name) == slug(name)) {
            Some(game) => self.create_draft(game),
            None => {
                let executable = s.unlinked_executable.clone().filter(|exe| slug(exe_name(exe)) == slug(name));
                self.send(Command::CreateGame { name: name.to_owned(), executable });
                self.library.pending = Some(name.to_owned());
            }
        }
    }

    /// The game asked for is in the library: its mode is made.
    fn create_pending_mode(&mut self, s: &Shared) {
        let Some(name) = &self.library.pending else { return };
        if let Some(game) = s.games.iter().find(|g| slug(&g.name) == slug(name)) {
            self.library.pending = None;
            self.create_draft(game);
        }
    }

    /// A new mode for `game`, still to be written, opened in the Creator.
    fn create_draft(&mut self, game: &Game) {
        let source = NEW_MODE_TEMPLATE.replace("NAME", &format!("My {} mode", game.name));
        let stem = match prompt::file_stem(&game.name) {
            stem if stem.is_empty() => "my-mode".to_owned(),
            stem => stem,
        };
        let way = self.library.start.unwrap_or(Way::Ai);
        if let Some(entry) = self.create_mode(&stem, &source, None, Some(&game.id)) {
            // What the assistant is asked for, kept with the mode (after its selection, so the engine has it).
            if way == Way::Ai {
                let mut inputs = Inputs::of(&entry);
                inputs.wishes = Some(self.generator.new_wishes.clone());
                inputs.instructions = self.generator.new_instructions.trim().to_owned();
                self.generator.new_instructions.clear();
                if inputs.has_package() {
                    self.send(Command::SaveInputs(inputs));
                }
            }
            self.creator.open_new(way);
        }
        self.route = Route::Mode;
    }

    // --- the mode's page

    fn mode_route(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let game = mode_game(s).cloned();
        let name = s.mode.info.as_ref().map_or("Mode".to_owned(), |i| i.name.clone());
        match &game {
            Some(g) => self.breadcrumb(ui, &[("Library", Some(Route::Library)), (&g.name, Some(Route::Library)), (&name, None)]),
            None => self.breadcrumb(ui, &[("Library", Some(Route::Library)), (&name, None)]),
        }
        self.mode_page(ui, s);
        if s.mode.id.is_empty() {
            return;
        }
        ui.add_space(14.0);
        self.reads_card(ui, s);
        ui.add_space(14.0);
        let entry = ModeEntry::from_id(&s.mode.id);
        if !entry.builtin {
            ui.label(RichText::new("Share").strong().size(17.0));
            ui.add_space(4.0);
            match &game {
                Some(game) => {
                    let main = main_of(&s.mode.id);
                    self.publish_suggestion(ui, s, game, &main);
                    ui.horizontal(|ui| self.community_update_button(ui, s, &main));
                    self.mode_sharing(ui, s, game);
                }
                None => {
                    ui.label(muted("A mode is shared with its game: give it one below."));
                }
            }
            ui.add_space(14.0);
        }
        self.game_card(ui, s, game.as_ref());
    }

    /// What the mode reads, in a few lines, and the way to the Creator.
    fn reads_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mut open = false;
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("What it reads").strong().size(17.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| open = ui.button("Open in Creator ›").clicked());
            });
            let Some(inputs) = &s.mode_inputs else {
                ui.label(muted("The rumble, your buttons and the game's sound: built-in modes read nothing set up for a game."));
                return;
            };
            let mut indicators: Vec<&str> = inputs.zones.iter().map(|z| z.indicator.as_str()).collect();
            indicators.sort();
            indicators.dedup();
            let rows = [
                ("Phases", inputs.phases.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ")),
                ("Captures", if inputs.captures.is_empty() { String::new() } else { inputs.captures.len().to_string() }),
                ("Indicators", indicators.join(", ")),
                ("Other programs", inputs.external.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")),
            ];
            egui::Grid::new("mode-reads").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                for (what, value) in rows {
                    ui.label(muted(what));
                    if value.is_empty() {
                        ui.label(muted("none"));
                    } else {
                        ui.label(value);
                    }
                    ui.end_row();
                }
            });
        });
        if open {
            self.page = Page::Creator;
        }
    }

    /// The mode's game: its name, how GameViber recognizes it, the sound it listens to.
    fn game_card(&mut self, ui: &mut egui::Ui, s: &Shared, game: Option<&Game>) {
        card(SIDEBAR).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(game) = game else {
                ui.label(RichText::new("Game").strong().size(17.0));
                let entry = ModeEntry::from_id(&s.mode.id);
                if entry.builtin {
                    ui.label(muted("Built-in modes go with any game."));
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label("This mode belongs to no game.");
                    egui::ComboBox::from_id_salt("mode-game").selected_text("Give it one...").show_ui(ui, |ui| {
                        for game in &s.games {
                            if ui.selectable_label(false, &game.name).clicked() {
                                self.send(Command::AddGameMode { game: game.id.clone(), mode: main_of(&s.mode.id) });
                                self.send(Command::SelectGame(Some(game.id.clone())));
                            }
                        }
                    });
                });
                return;
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new("Game").strong().size(17.0));
                let others = game.modes.len().saturating_sub(1);
                if others > 0 {
                    ui.label(muted(format!("shared by its {} modes", game.modes.len())));
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("mode-game").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label(muted("For"));
                ui.horizontal(|ui| match &mut self.library.renaming {
                    Some(name) => {
                        ui.add(egui::TextEdit::singleline(name).desired_width(260.0));
                        let name = name.trim().to_owned();
                        if ui.add_enabled(!name.is_empty(), egui::Button::new("Save")).clicked() {
                            self.send(Command::SaveGame(Game { name, ..game.clone() }));
                            self.library.renaming = None;
                        }
                        if ui.button("Cancel").clicked() {
                            self.library.renaming = None;
                        }
                    }
                    None => {
                        ui.label(RichText::new(&game.name).strong());
                        if ui.small_button("Rename").clicked() {
                            self.library.renaming = Some(game.name.clone());
                        }
                    }
                });
                ui.end_row();

                ui.label(muted("Recognized by"));
                ui.horizontal_wrapped(|ui| {
                    if game.is_running(s.running_executable.as_deref().unwrap_or(""), s.running_app) {
                        ui.label(RichText::new("● Running").color(OK));
                    }
                    let mut ways = game.executables.clone();
                    ways.extend(game.steam_app_id.map(|app| format!("Steam app {app}")));
                    if ways.is_empty() {
                        ui.label(muted("nothing: pick it in the top bar when you play it"));
                    } else {
                        ui.label(ways.join(" · "));
                    }
                    if !game.executables.is_empty() && ui.small_button("Unlink").on_hover_text("Pick the game by hand instead").clicked() {
                        self.send(Command::SaveGame(Game { executables: Vec::new(), ..game.clone() }));
                    }
                    if let Some(exe) = &s.unlinked_executable {
                        if ui.small_button(format!("Link {exe}")).on_hover_text("The game becomes the active one whenever it runs").clicked() {
                            self.send(Command::LinkExecutable { game: game.id.clone(), executable: exe.clone() });
                        }
                    }
                });
                ui.end_row();

                ui.label(muted("Sound"));
                if let Some(audio) = game_sound(ui, s, game) {
                    self.send(Command::SaveGame(Game { audio, ..game.clone() }));
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let main = main_of(&s.mode.id);
                if ui.small_button(format!("Take this mode out of {}", game.name)).on_hover_text("Its file stays").clicked() {
                    self.send(Command::RemoveGameMode { game: game.id.clone(), mode: main });
                }
                if self.library.confirm_delete_game {
                    ui.label(RichText::new(format!("Delete {}? Its modes stay.", game.name)).color(DANGER_TEXT));
                    let delete = egui::Button::new(RichText::new("Delete for good").color(egui::Color32::WHITE)).fill(DANGER);
                    if ui.add(delete).clicked() {
                        self.send(Command::DeleteGame(game.id.clone()));
                        self.library.confirm_delete_game = false;
                        self.route = Route::Library;
                    }
                    if ui.button("Cancel").clicked() {
                        self.library.confirm_delete_game = false;
                    }
                } else if ui.small_button(format!("🗑 Delete {}", game.name)).clicked() {
                    self.library.confirm_delete_game = true;
                }
            });
        });
    }
}

/// The library's groups: each game with modes, the game being played first,
/// then the player's modes in no game.
fn groups(s: &Shared) -> Vec<(Option<Game>, Vec<String>)> {
    let mut games: Vec<&Game> = s.games.iter().filter(|g| !g.modes.is_empty()).collect();
    games.sort_by_key(|g| (s.game.as_ref().is_none_or(|p| p.id != g.id), g.name.to_lowercase()));
    let mut groups: Vec<(Option<Game>, Vec<String>)> = games.into_iter().map(|g| (Some(g.clone()), g.modes.clone())).collect();
    let loose: Vec<String> = s
        .modes
        .iter()
        .filter(|e| !e.builtin && e.variant.is_none() && !s.games.iter().any(|g| g.modes.contains(&e.id)))
        .map(|e| e.id.clone())
        .collect();
    if !loose.is_empty() {
        groups.push((None, loose));
    }
    groups
}

/// The game the active mode belongs to: the one being played when it is one of its.
pub(super) fn mode_game(s: &Shared) -> Option<&Game> {
    let main = main_of(&s.mode.id);
    s.game.as_ref().filter(|g| g.modes.contains(&main)).or_else(|| s.games.iter().find(|g| g.modes.contains(&main)))
}

/// The game being played, when it runs.
fn running_game(s: &Shared) -> Option<&Game> {
    s.game.as_ref().filter(|g| s.running_executable.as_deref().is_some_and(|exe| g.is_running(exe, s.running_app)))
}

/// The game to make a mode for, guessed: the one running.
fn default_game(s: &Shared) -> String {
    match (&s.unlinked_executable, running_game(s)) {
        (Some(exe), _) => exe_name(exe).to_owned(),
        (None, Some(game)) => game.name.clone(),
        (None, None) => String::new(),
    }
}

/// `METAPHOR.exe` → `METAPHOR`.
fn exe_name(exe: &str) -> &str {
    exe.strip_suffix(".exe").unwrap_or(exe)
}

/// Which sound `game` listens to; returns the one picked.
pub(super) fn game_sound(ui: &mut egui::Ui, s: &Shared, game: &Game) -> Option<Option<AudioSource>> {
    let mut chosen = None;
    let label = |source: &Option<AudioSource>| match source {
        None => "Default (Setup › Sound)".to_owned(),
        Some(AudioSource::Auto) => "The game showing the overlay".to_owned(),
        Some(AudioSource::Everything) => "Everything the computer plays".to_owned(),
        Some(AudioSource::App(app)) => app.clone(),
        Some(AudioSource::Off) => "Off".to_owned(),
    };
    ui.horizontal_wrapped(|ui| {
        egui::ComboBox::from_id_salt(("game-audio", &game.id)).selected_text(label(&game.audio)).width(240.0).show_ui(ui, |ui| {
            let mut options = vec![None, Some(AudioSource::Auto), Some(AudioSource::Everything), Some(AudioSource::Off)];
            options.extend(s.audio.status.streams.iter().map(|app| Some(AudioSource::App(app.clone()))));
            for option in options {
                if ui.selectable_label(option == game.audio, label(&option)).clicked() {
                    chosen = Some(option);
                }
            }
        });
        if s.game.as_ref().is_some_and(|g| g.id == game.id) {
            match (&s.audio.status.target, &s.audio.status.error) {
                (_, Some(error)) => {
                    dot(ui, DANGER);
                    ui.label(error);
                }
                (Some(target), None) => {
                    dot(ui, OK);
                    ui.label(muted(format!("listening to {target}")));
                }
                (None, None) => {
                    dot(ui, WARN);
                    ui.label(muted("waiting for sound"));
                }
            }
        }
    });
    chosen
}

