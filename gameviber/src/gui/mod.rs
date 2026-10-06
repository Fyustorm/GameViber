//! egui front end. A first-launch setup guides players through Intiface
//! Central, their toys, the gamepad and a first mode; afterwards a status bar
//! (the game being played, the panic stop) sits above the pages: Games (the
//! library, then each game by breadcrumb: its modes, each with a page of its
//! own, its signals — scenes, captures, zones, sound — and its sessions),
//! Toys (with Intiface Central), Setup (gamepad, combos, overlay, default
//! sound, other programs: what does not depend on the game) and Creator (mode
//! editor, graphs, simulator, sessions, logs). Dialogs help players get a
//! mode made for their game by an AI assistant, and get one fixed when it
//! does not feel right; modes are shared with their game as files.

mod audio;
mod creator;
mod feedback;
mod gamepad;
mod games;
mod generator;
mod keybindings;
mod live;
mod luau;
mod onboarding;
mod overlay;
mod settings;
mod setup;
mod sharing;
mod play;
mod screen;
mod signals;
mod theme;
mod toys;
mod tour;
mod updates;

use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui::{self, Margin, RichText, Vec2};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{self, ModeEntry};
use crate::engine::{Command, Shared, SharedHandle, SourceHealth, HISTORY_SECS};
use crate::logging::LogBuffer;
use theme::*;

const REPAINT: Duration = Duration::from_millis(33);
/// The first-launch setup guide is off while the app changes too much for it
/// to stay up to date; it comes back once the app is stabilized.
const ONBOARDING: bool = false;
/// Rumble newer than this counts as "the game is vibrating right now".
const RECENT_RUMBLE_SECS: f64 = 5.0;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Games,
    Live,
    Toys,
    Setup,
    Creator,
    Settings,
}

/// Where the Games page is.
#[derive(PartialEq, Clone, Debug)]
enum Route {
    Library,
    /// The built-in modes, to play without a game.
    BuiltIn,
    /// The active mode's page, without a game.
    FreeMode,
    Game { id: String, view: GameView },
}

#[derive(PartialEq, Clone, Copy, Debug)]
enum GameView {
    Modes,
    /// The active mode's page.
    Mode,
    Signals,
    /// Signals › captures and zones.
    Screen,
    Sessions,
}

pub struct App {
    shared: SharedHandle,
    logs: LogBuffer,
    commands: UnboundedSender<Command>,
    engine: Option<JoinHandle<()>>,
    page: Page,
    route: Route,
    setup_tab: setup::Tab,
    /// Current step of the first-launch setup, when it is shown.
    onboarding: Option<usize>,
    /// The setup was considered for this session (shown at most once automatically).
    onboarding_checked: bool,
    play: play::State,
    toys: toys::State,
    overlay: overlay::State,
    settings: settings::State,
    creator: creator::State,
    generator: generator::State,
    feedback: feedback::State,
    screen: screen::State,
    games: games::State,
    signals: signals::State,
    sharing: sharing::State,
    /// The engine was told the Screen page is open.
    watching_screen: bool,
    /// Development: screenshots of every page (`GAMEVIBER_SCREENSHOTS`).
    tour: Option<tour::Tour>,
    /// New versions: started once the settings are known.
    updater: Option<crate::update::Updater>,
    /// The banner announcing a new version was closed ("Later") for this session.
    update_banner_closed: bool,
}

impl App {
    pub fn new(
        ctx: &egui::Context,
        shared: SharedHandle,
        logs: LogBuffer,
        commands: UnboundedSender<Command>,
        engine: JoinHandle<()>,
    ) -> Self {
        apply(ctx);
        Self {
            shared,
            logs,
            commands,
            engine: Some(engine),
            page: Page::Games,
            route: Route::Library,
            setup_tab: setup::Tab::default(),
            onboarding: None,
            onboarding_checked: false,
            play: play::State::default(),
            toys: toys::State::default(),
            overlay: overlay::State::default(),
            settings: settings::State::default(),
            creator: creator::State::default(),
            generator: generator::State::default(),
            feedback: feedback::State::default(),
            screen: screen::State::default(),
            games: games::State::default(),
            signals: signals::State::default(),
            sharing: sharing::State::default(),
            watching_screen: false,
            tour: tour::Tour::from_env(),
            updater: None,
            update_banner_closed: false,
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Creates a user mode, activates it and opens it in the Creator. It starts
    /// with the inputs of the mode `copy_of`, or else, made from a game's page,
    /// with those of the active mode when it is one of that game's.
    fn create_mode(&mut self, stem: &str, source: &str, copy_of: Option<&str>) -> Option<ModeEntry> {
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
                let id = path.to_string_lossy().into_owned();
                let from = copy_of.map(str::to_owned).or_else(|| {
                    let Route::Game { id: game, .. } = &self.route else { return None };
                    let s = self.shared.lock().unwrap();
                    s.games.iter().find(|g| g.id == *game).filter(|g| g.modes.contains(&s.mode.id)).map(|_| s.mode.id.clone())
                });
                let inputs = from.map(|from| crate::package::Inputs::of(&ModeEntry::from_id(&from))).filter(|i| !i.is_empty());
                if let (Some(inputs), Some(dir)) = (inputs, ModeEntry::from_id(&id).dir()) {
                    inputs.copy_to(&dir).save();
                }
                self.send(Command::RefreshModes);
                self.send(Command::SelectMode(id.clone()));
                // A mode made from a game's page belongs to that game.
                if let Route::Game { id: game, .. } = &self.route {
                    self.send(Command::AddGameMode { game: game.clone(), mode: id.clone() });
                }
                self.page = Page::Creator;
                Some(ModeEntry::from_id(&id))
            }
            Err(e) => {
                log::error!("cannot create {}: {e}", path.display());
                None
            }
        }
    }

    fn duplicate_mode(&mut self, id: &str) {
        let entry = ModeEntry::from_id(id);
        match entry.source() {
            Ok(source) => {
                self.create_mode(&format!("{}-copy", entry.key), &source, Some(id));
            }
            Err(e) => log::error!("cannot read {}: {e}", entry.id),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(REPAINT);
        crate::platform::window_focused(ui.ctx().input(|i| i.focused));
        let s = self.shared.lock().unwrap().clone();
        if s.stopped {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // The settings are only known after the engine's first tick.
        if !self.onboarding_checked && s.time > 0.0 {
            self.onboarding_checked = true;
            if ONBOARDING && !s.settings.onboarded {
                self.onboarding = Some(0);
            }
        }
        self.start_updater(&s);
        self.tour(ui.ctx(), &s);
        if let Some(step) = self.onboarding.filter(|_| self.tour.is_none()) {
            self.onboarding_ui(ui, &s, step);
            self.generator_ui(ui.ctx(), &s);
            return;
        }

        self.status_bar(ui, &s);
        self.update_banner(ui);
        self.rail(ui);
        let mode_page = self.page == Page::Games && matches!(self.route, Route::FreeMode | Route::Game { view: GameView::Mode, .. });
        // The fix page needs the room.
        let fixing = mode_page && self.feedback.open;
        if (mode_page || self.page == Page::Toys) && !fixing {
            live_strip(ui, &s);
        }
        if mode_page && !fixing {
            gamepad_strip(ui, &s);
        }
        match self.page {
            Page::Games => self.games_ui(ui, &s),
            Page::Live => self.live_ui(ui, &s),
            Page::Toys => self.toys_ui(ui, &s),
            Page::Setup => self.setup_ui(ui, &s),
            Page::Creator => self.creator_ui(ui, &s),
            Page::Settings => self.settings_ui(ui, &s),
        }
        self.generator_ui(ui.ctx(), &s);
        self.update_simulated_rumble();
        // The overlay copies the game's image while it is looked at, even when modes do not see it.
        let watching = self.page == Page::Games
            && matches!(self.route, Route::Game { view: GameView::Screen | GameView::Signals, .. });
        if watching != self.watching_screen {
            self.watching_screen = watching;
            self.send(Command::WatchScreen(watching));
        }
    }

    fn on_exit(&mut self) {
        crate::platform::window_closing();
        self.send(Command::Shutdown);
        if let Some(engine) = self.engine.take() {
            let _ = engine.join();
        }
    }
}

impl App {
    fn status_bar(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(PANEL).inner_margin(Margin::symmetric(16, 10));
        egui::Panel::top("status").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("〰 GameViber").size(16.0).strong().color(ACCENT));
                ui.add_space(8.0);
                self.game_picker(ui, s);
                let (pad_color, pad_text) = gamepad_status(s);
                if status_chip(ui, pad_color, "🎮", &pad_text).clicked() {
                    self.page = Page::Setup;
                    self.setup_tab = setup::Tab::Gamepad;
                }
                let (rumble_color, rumble_text) = capture_status(s);
                if status_chip(ui, rumble_color, "📳", &rumble_text).clicked() {
                    self.page = Page::Setup;
                    self.setup_tab = setup::Tab::Gamepad;
                }
                let (toy_color, toy_text) = intiface_status(s);
                if status_chip(ui, toy_color, "🔌", &toy_text).clicked() {
                    self.page = Page::Toys;
                }
                let session = match (s.recording, &s.replay) {
                    (Some(secs), _) => Some((DANGER_TEXT, "⏺", format!("Recording {:.0} s", secs))),
                    (None, Some(_)) => Some((WARN, "▶", "Replaying a session".to_owned())),
                    (None, None) => None,
                };
                if let Some((color, icon, text)) = session {
                    if status_chip(ui, color, icon, &text).on_hover_text("Creator › Sessions").clicked() {
                        self.page = Page::Creator;
                        self.creator.show_sessions();
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if s.panic {
                        if ui.add(primary("Re-arm")).clicked() {
                            self.send(Command::Rearm);
                        }
                        ui.label(RichText::new("⛔ PANIC STOP").strong().color(DANGER_TEXT));
                    } else {
                        let stop = egui::Button::new(RichText::new("STOP ALL").strong().color(egui::Color32::WHITE))
                            .fill(DANGER)
                            .min_size(Vec2::new(0.0, 34.0));
                        let hint = format!(
                            "Stops every toy. Also: hold {} on the gamepad",
                            crate::gamepad::combo_text(&s.settings.panic_combo)
                        );
                        if ui.add(stop).on_hover_text(hint).clicked() {
                            self.send(Command::Panic);
                        }
                    }
                    ui.add_space(8.0);
                    let mut cap = s.settings.global_cap * 100.0;
                    ui.spacing_mut().slider_width = 120.0;
                    let slider = egui::Slider::new(&mut cap, 0.0..=100.0).suffix("%").integer();
                    if ui.add(slider).on_hover_text("No toy ever goes above this intensity").changed() {
                        self.send(Command::SetCap(cap / 100.0));
                    }
                    ui.label(muted("Max"));
                });
            });
        });
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(8, 14));
        egui::Panel::left("rail").frame(frame).exact_size(84.0).resizable(false).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                for (page, icon, label) in [
                    (Page::Games, "🎮", "Games"),
                    (Page::Live, "📺", "Live"),
                    (Page::Toys, "📳", "Toys"),
                    (Page::Setup, "🛠", "Setup"),
                    (Page::Creator, "🔧", "Creator"),
                ] {
                    if nav_item(ui, icon, label, self.page == page).clicked() {
                        if page == Page::Setup && self.page != page {
                            // Check the installed overlay files again.
                            self.overlay.forget_install_state();
                        }
                        // The Games button always leads back to the library.
                        if page == Page::Games && self.page == Page::Games {
                            self.route = Route::Library;
                        }
                        self.page = page;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                    if nav_item(ui, "⚙", "Settings", self.page == Page::Settings).clicked() {
                        self.page = Page::Settings;
                    }
                });
            });
        });
    }

    /// The game being played, to pick another.
    fn game_picker(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let current = s.game.as_ref().map_or("No game".to_owned(), |g| g.name.clone());
        egui::ComboBox::from_id_salt("game-picker").selected_text(RichText::new(format!("🎮 {current}")).strong()).width(200.0).show_ui(ui, |ui| {
            for game in &s.games {
                if ui.selectable_label(s.game.as_ref().is_some_and(|g| g.id == game.id), &game.name).clicked() {
                    self.send(Command::SelectGame(Some(game.id.clone())));
                }
            }
            if ui.selectable_label(s.game.is_none(), "No game").clicked() {
                self.send(Command::SelectGame(None));
            }
        });
        ui.add_space(4.0);
    }

    /// Sends the simulated rumble whenever it changes (sliders or an expiring hit).
    fn update_simulated_rumble(&mut self) {
        if let Some(command) = self.creator.sim.update() {
            self.send(command);
        }
    }
}

fn nav_item(ui: &mut egui::Ui, icon: &str, label: &str, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(68.0, 54.0), egui::Sense::click());
    let painter = ui.painter();
    if active || response.hovered() {
        painter.rect_filled(rect, 10, if active { RAISED } else { PANEL });
    }
    let color = if active { egui::Color32::WHITE } else { MUTED };
    painter.text(
        rect.center_top() + Vec2::new(0.0, 18.0),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        if active { ACCENT } else { MUTED },
    );
    painter.text(rect.center_bottom() - Vec2::new(0.0, 12.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.5), color);
    ui.add_space(4.0);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn status_chip(ui: &mut egui::Ui, color: egui::Color32, icon: &str, text: &str) -> egui::Response {
    let response = egui::Frame::new()
        .fill(RAISED)
        .stroke(egui::Stroke::new(1.0, LINE))
        .corner_radius(15)
        .inner_margin(Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            dot(ui, color);
            ui.label(RichText::new(format!("{icon} {text}")).size(12.5));
        })
        .response;
    response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Bottom band: game rumble and mode output over the last seconds, toy levels.
fn live_strip(ui: &mut egui::Ui, s: &Shared) {
    let frame = egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(20, 12));
    egui::Panel::bottom("live").frame(frame).exact_size(72.0).resizable(false).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.vertical(|ui| {
                eyebrow(ui, "Live");
                ui.label(muted(format!("last {HISTORY_SECS:.0} s")).size(11.5));
            });
            ui.add_space(12.0);
            let game: Vec<(f64, f64)> = s.history.iter().map(|x| (x.t - s.time, x.strong.max(x.weak))).collect();
            let output: Vec<(f64, f64)> =
                s.history.iter().map(|x| (x.t - s.time, x.channels.values().copied().fold(0.0, f64::max))).collect();
            ui.label(RichText::new("Game").size(12.0).color(GAME));
            sparkline(ui, Vec2::new(200.0, 40.0), HISTORY_SECS, &game, GAME);
            ui.label(muted("›"));
            ui.label(RichText::new("Output").size(12.0).color(ACCENT_TEXT));
            sparkline(ui, Vec2::new(200.0, 40.0), HISTORY_SECS, &output, ACCENT);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (toy, level) in s.toy_levels.iter().take(3) {
                    ui.horizontal(|ui| {
                        ui.add_sized(Vec2::new(120.0, 14.0), egui::Label::new(RichText::new(toy).size(11.5)).truncate());
                        meter(ui, 100.0, *level, ACCENT);
                    });
                }
            });
        });
    });
}

/// Band showing the buttons, triggers and sticks the gamepad sends right now.
fn gamepad_strip(ui: &mut egui::Ui, s: &Shared) {
    let frame = egui::Frame::new().fill(SIDEBAR).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(20, 8));
    egui::Panel::bottom("gamepad").frame(frame).exact_size(42.0).resizable(false).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            eyebrow(ui, "Gamepad");
            ui.add_space(8.0);
            gamepad_inputs(ui, s);
        });
    });
}

/// The buttons, triggers and sticks the gamepad sends right now, so players
/// can check their gamepad reaches GameViber.
fn gamepad_inputs(ui: &mut egui::Ui, s: &Shared) {
    const LABELS: [(&str, &str); 20] = [
        ("A", "A"), ("B", "B"), ("X", "X"), ("Y", "Y"), ("LB", "LB"), ("RB", "RB"), ("LS", "LS"), ("RS", "RS"),
        ("BACK", "Back"), ("START", "Start"), ("GUIDE", "Guide"),
        ("DPAD_UP", "⏶"), ("DPAD_DOWN", "⏷"), ("DPAD_LEFT", "⏴"), ("DPAD_RIGHT", "⏵"),
        ("P1", "P1"), ("P2", "P2"), ("P3", "P3"), ("P4", "P4"), ("SHARE", "Share"),
    ];
    let axis = |name: &str| s.axes.get(name).copied().unwrap_or(0.0);
    let held = |name: &str| s.held.contains(&name);
    ui.spacing_mut().item_spacing.x = 4.0;
    // Triggers show their travel; the half-way button press lights them fully.
    for name in ["LT", "RT"] {
        pad_chip(ui, name, if held(name) { 1.0 } else { axis(name) });
    }
    ui.add_space(4.0);
    for (name, label) in LABELS {
        pad_chip(ui, label, if held(name) { 1.0 } else { 0.0 });
    }
    ui.add_space(4.0);
    stick(ui, "Left stick", axis("LX"), axis("LY"));
    stick(ui, "Right stick", axis("RX"), axis("RY"));
    ui.add_space(8.0);
    if !s.buttons_seen {
        ui.label(muted("Press a button: it lights up when GameViber receives it.").size(11.5));
    }
}

fn gamepad_status(s: &Shared) -> (egui::Color32, String) {
    match (s.gamepads.first(), &s.source_health) {
        (_, SourceHealth::Off) => (IDLE, "No gamepad capture".into()),
        (Some(name), _) if s.gamepads.len() > 1 => (OK, format!("{name} +{}", s.gamepads.len() - 1)),
        (Some(name), _) => (OK, name.clone()),
        (None, _) => (WARN, "No gamepad found".into()),
    }
}

fn capture_status(s: &Shared) -> (egui::Color32, String) {
    match &s.source_health {
        SourceHealth::Off => (IDLE, "Rumble: off".into()),
        SourceHealth::Waiting(why) => (WARN, format!("Rumble: {why}")),
        SourceHealth::Failed(_) => (DANGER_TEXT, "Rumble: not captured".into()),
        SourceHealth::Working if s.last_rumble.is_some_and(|t| s.time - t < RECENT_RUMBLE_SECS) => {
            (OK, "Rumble: receiving".into())
        }
        SourceHealth::Working => (OK, "Rumble: listening".into()),
    }
}

fn intiface_status(s: &Shared) -> (egui::Color32, String) {
    if !s.intiface_enabled {
        (IDLE, "Intiface disabled".into())
    } else if !s.intiface.connected {
        (WARN, "Intiface not running".into())
    } else {
        let n = s.intiface.toys.len();
        (if n > 0 { OK } else { WARN }, format!("{n} toy{} via Intiface", if n == 1 { "" } else { "s" }))
    }
}

/// Icon of a mode tile.
fn mode_icon(entry: &ModeEntry) -> &'static str {
    if !entry.builtin {
        return "📝";
    }
    match entry.key.as_str() {
        "simple" => "〰",
        "accumulation" => "📈",
        "combo" => "👊",
        "overheat" => "🔥",
        "tension" => "⚔",
        "engine" => "🚗",
        "heartbeat" => "❤",
        "all_or_nothing" => "🛡",
        "ambient" => "🍃",
        _ => "🎮",
    }
}
