//! egui front end. A first-launch setup guides players through Intiface
//! Central, their toys, the gamepad and a first mode; afterwards a status bar
//! (with the panic stop) sits above four pages: Play (mode tiles and their
//! settings), Toys, Connection (troubleshooting) and Creator (mode editor,
//! graphs, simulator, logs). A dialog helps players get a mode made for
//! their game by an AI assistant.

mod connection;
mod creator;
mod generator;
mod onboarding;
mod play;
mod theme;
mod toys;

use std::thread::JoinHandle;
use std::time::Duration;

use eframe::egui::{self, Margin, RichText, Vec2};
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{self, ModeEntry};
use crate::engine::{Command, Shared, SharedHandle, SourceHealth, HISTORY_SECS};
use crate::logging::LogBuffer;
use theme::*;

const REPAINT: Duration = Duration::from_millis(33);
/// Rumble newer than this counts as "the game is vibrating right now".
const RECENT_RUMBLE_SECS: f64 = 5.0;

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Play,
    Toys,
    Connection,
    Creator,
}

pub struct App {
    shared: SharedHandle,
    logs: LogBuffer,
    commands: UnboundedSender<Command>,
    engine: Option<JoinHandle<()>>,
    page: Page,
    /// Current step of the first-launch setup, when it is shown.
    onboarding: Option<usize>,
    /// The setup was considered for this session (shown at most once automatically).
    onboarding_checked: bool,
    play: play::State,
    connection: connection::State,
    creator: creator::State,
    generator: generator::State,
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
            page: Page::Play,
            onboarding: None,
            onboarding_checked: false,
            play: play::State::default(),
            connection: connection::State::default(),
            creator: creator::State::default(),
            generator: generator::State::default(),
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Creates a user mode file, activates it and opens it in the Creator.
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
                self.page = Page::Creator;
            }
            Err(e) => log::error!("cannot create {}: {e}", path.display()),
        }
    }

    fn duplicate_mode(&mut self, id: &str) {
        let entry = ModeEntry::from_id(id);
        match entry.source() {
            Ok(source) => self.create_mode(&format!("{}-copy", entry.key), &source),
            Err(e) => log::error!("cannot read {}: {e}", entry.id),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().request_repaint_after(REPAINT);
        crate::helper::dialog::set_focused(ui.ctx().input(|i| i.focused));
        let s = self.shared.lock().unwrap().clone();
        if s.stopped {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // The settings are only known after the engine's first tick.
        if !self.onboarding_checked && s.time > 0.0 {
            self.onboarding_checked = true;
            if !s.settings.onboarded {
                self.onboarding = Some(0);
            }
        }
        if let Some(step) = self.onboarding {
            self.onboarding_ui(ui, &s, step);
            return;
        }

        self.status_bar(ui, &s);
        self.rail(ui);
        if matches!(self.page, Page::Play | Page::Toys) {
            live_strip(ui, &s);
        }
        match self.page {
            Page::Play => self.play_ui(ui, &s),
            Page::Toys => self.toys_ui(ui, &s),
            Page::Connection => self.connection_ui(ui, &s),
            Page::Creator => self.creator_ui(ui, &s),
        }
        self.generator_ui(ui.ctx());
        self.update_simulated_rumble();
    }

    fn on_exit(&mut self) {
        crate::helper::dialog::forget_window();
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
                let (pad_color, pad_text) = gamepad_status(s);
                if status_chip(ui, pad_color, "🎮", &pad_text).clicked() {
                    self.page = Page::Connection;
                }
                let (rumble_color, rumble_text) = capture_status(s);
                if status_chip(ui, rumble_color, "📳", &rumble_text).clicked() {
                    self.page = Page::Connection;
                }
                let (toy_color, toy_text) = intiface_status(s);
                if status_chip(ui, toy_color, "🔌", &toy_text).clicked() {
                    self.page = Page::Toys;
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
                        if ui.add(stop).on_hover_text("Stops every toy. Also: hold BACK + START on the gamepad").clicked() {
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
                    (Page::Play, "▶", "Play"),
                    (Page::Toys, "📳", "Toys"),
                    (Page::Connection, "🔌", "Connection"),
                    (Page::Creator, "🔧", "Creator"),
                ] {
                    if nav_item(ui, icon, label, self.page == page).clicked() {
                        self.page = page;
                    }
                }
            });
        });
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
