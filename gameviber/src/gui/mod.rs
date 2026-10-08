//! egui front end. A first-launch setup guides players through Intiface
//! Central, their toys, the gamepad and a first mode; afterwards a status bar
//! (the game being played, the panic stop) sits above the pages: Library (the
//! player's modes by game, each with a page of its own, and creating a mode
//! for a game), Toys (with Intiface Central), Setup (gamepad, combos, overlay,
//! default sound, other programs: what does not depend on the game) and
//! Creator (the active mode's workspace: its phases, captures, indicators,
//! values from other programs and script, sessions and a simulator).
//! Players get a mode written by an AI assistant from the Creator, and get
//! one fixed when it does not feel right; modes are shared with their game
//! as files.

mod audio;
mod creator;
mod diagram;
mod feedback;
mod gamepad;
mod community;
mod generator;
mod keybindings;
mod library;
mod live;
mod luau;
mod onboarding;
mod pad_setup;
mod overlay;
mod settings;
mod setup;
mod sharing;
mod play;
mod screen;
mod sessions;
mod inputs;
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
    Library,
    Community,
    Live,
    Toys,
    Setup,
    Creator,
    Settings,
}

/// Where the Library page is.
#[derive(PartialEq, Clone, Copy, Debug)]
enum Route {
    /// The player's modes, by game.
    Library,
    /// Creating a mode: for which game, and how a mode works.
    Create,
    /// The active mode's page: its settings, what it reads, sharing it, its game.
    Mode,
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
    community: community::State,
    toys: toys::State,
    overlay: overlay::State,
    settings: settings::State,
    /// Setting up the gamepad's buttons (Gamepad page).
    pad_setup: Option<pad_setup::PadSetup>,
    creator: creator::State,
    generator: generator::State,
    feedback: feedback::State,
    screen: screen::State,
    library: library::State,
    inputs: inputs::State,
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
            // The usual way in: finding a mode made for one's game.
            page: Page::Community,
            route: Route::Library,
            setup_tab: setup::Tab::default(),
            onboarding: None,
            onboarding_checked: false,
            play: play::State::default(),
            community: community::State::default(),
            toys: toys::State::default(),
            overlay: overlay::State::default(),
            settings: settings::State::default(),
            pad_setup: None,
            creator: creator::State::default(),
            generator: generator::State::default(),
            feedback: feedback::State::default(),
            screen: screen::State::default(),
            library: library::State::default(),
            inputs: inputs::State::default(),
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

    /// Creates a user mode for `game`, activates it and opens it in the Creator.
    /// It starts with the inputs of the mode `copy_of`, or else with those of
    /// the active mode when it is one of that game's.
    fn create_mode(&mut self, stem: &str, source: &str, copy_of: Option<&str>, game: Option<&str>) -> Option<ModeEntry> {
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
                    let s = self.shared.lock().unwrap();
                    s.games.iter().find(|g| Some(g.id.as_str()) == game).filter(|g| g.modes.contains(&main_of(&s.mode.id))).map(|_| s.mode.id.clone())
                });
                let inputs = from.map(|from| crate::package::Inputs::of(&ModeEntry::from_id(&from))).filter(|i| !i.is_empty());
                if let (Some(inputs), Some(dir)) = (inputs, ModeEntry::from_id(&id).dir()) {
                    inputs.copy_to(&dir).save();
                }
                self.send(Command::RefreshModes);
                if let Some(game) = game {
                    self.send(Command::AddGameMode { game: game.to_owned(), mode: id.clone() });
                    self.send(Command::SelectGame(Some(game.to_owned())));
                }
                self.send(Command::SelectMode(id.clone()));
                self.page = Page::Creator;
                Some(ModeEntry::from_id(&id))
            }
            Err(e) => {
                log::error!("cannot create {}: {e}", path.display());
                None
            }
        }
    }

    /// Makes a variant of the user mode `id` named after `name`, starting from
    /// its script, and plays it.
    fn create_variant(&mut self, id: &str, name: &str) {
        let entry = ModeEntry::from_id(id);
        let (Some(dir), Ok(source)) = (entry.dir(), entry.source()) else {
            return log::error!("cannot make a variant of {id}");
        };
        let stem: String = name.trim().chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        let path = config::unused_variant_path(&dir, if stem.is_empty() { "variant" } else { &stem });
        match config::write_file(&path, &source) {
            Ok(()) => {
                log::info!("created the variant {}", path.display());
                self.send(Command::RefreshModes);
                self.send(Command::SelectMode(path.to_string_lossy().into_owned()));
            }
            Err(e) => log::error!("cannot create {}: {e}", path.display()),
        }
    }

    fn duplicate_mode(&mut self, id: &str) {
        let entry = ModeEntry::from_id(id);
        // The copy stays in the game being played, when the mode is one of its.
        let game = {
            let s = self.shared.lock().unwrap();
            s.game.as_ref().filter(|g| g.modes.contains(&main_of(id))).map(|g| g.id.clone())
        };
        match entry.source() {
            Ok(source) => {
                self.create_mode(&format!("{}-copy", entry.key), &source, Some(id), game.as_deref());
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
            return;
        }

        self.status_bar(ui, &s);
        self.update_banner(ui);
        self.rail(ui);
        let mode_page = self.page == Page::Library && self.route == Route::Mode;
        // The fix page needs the room.
        let fixing = mode_page && self.feedback.open;
        if (mode_page || self.page == Page::Toys) && !fixing {
            live_strip(ui, &s);
        }
        if mode_page && !fixing {
            gamepad_strip(ui, &s);
        }
        match self.page {
            Page::Library => self.library_ui(ui, &s),
            Page::Community => self.community_ui(ui, &s),
            Page::Live => self.live_ui(ui, &s),
            Page::Toys => self.toys_ui(ui, &s),
            Page::Setup => self.setup_ui(ui, &s),
            Page::Creator => self.creator_ui(ui, &s),
            Page::Settings => self.settings_ui(ui, &s),
        }
        if self.tour.is_none() || self.community.consent_preview {
            self.stats_consent(ui.ctx(), &s);
        }
        self.update_simulated_rumble();
        // The overlay copies the game's image while it is looked at, even when modes do not see it.
        let watching = self.page == Page::Creator && self.creator.watches_screen();
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
                    (Page::Community, "🌐", "Community"),
                    (Page::Library, "📚", "Library"),
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
                        // The Library button always leads back to the list of modes.
                        if page == Page::Library && self.page == Page::Library {
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
    pad_view(ui, |name| s.held.contains(&name), |name| s.axes.get(name).copied().unwrap_or(0.0));
    ui.add_space(8.0);
    if !s.buttons_seen {
        ui.label(muted("Press a button: it lights up when GameViber receives it.").size(11.5));
    }
}

/// The gamepad's buttons (`held`) and sticks and triggers (`axis`).
fn pad_view(ui: &mut egui::Ui, held: impl Fn(&str) -> bool, axis: impl Fn(&str) -> f64) {
    const LABELS: [(&str, &str); 20] = [
        ("A", "A"), ("B", "B"), ("X", "X"), ("Y", "Y"), ("LB", "LB"), ("RB", "RB"), ("LS", "LS"), ("RS", "RS"),
        ("BACK", "Back"), ("START", "Start"), ("GUIDE", "Guide"),
        ("DPAD_UP", "⏶"), ("DPAD_DOWN", "⏷"), ("DPAD_LEFT", "⏴"), ("DPAD_RIGHT", "⏵"),
        ("P1", "P1"), ("P2", "P2"), ("P3", "P3"), ("P4", "P4"), ("SHARE", "Share"),
    ];
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
    } else if s.intiface.paused {
        (IDLE, "Intiface disconnected".into())
    } else if !s.intiface.connected {
        (WARN, "Intiface not running".into())
    } else {
        let n = s.intiface.toys.len();
        (if n > 0 { OK } else { WARN }, format!("{n} toy{} via Intiface", if n == 1 { "" } else { "s" }))
    }
}

/// Icon of a mode tile.
/// The mode `id` is a variant of (`id` for a mode).
pub(super) fn main_of(id: &str) -> String {
    ModeEntry::from_id(id).main_id()
}

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
