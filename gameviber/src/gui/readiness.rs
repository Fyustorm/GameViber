//! Whether GameViber gets what modes read: the status bar's chips (gamepad
//! and rumble, toys, the game's image, its sound) and, on a mode's page and in
//! the Creator, what is missing for that mode, said plainly with the way to fix it.

use eframe::egui::{self, Color32, Margin, RichText};

use super::theme::*;
use super::{capture_status, intiface_status, setup, status_chip, App, Page, RECENT_RUMBLE_SECS};
use crate::config::AudioSource;
use crate::engine::{Shared, SourceHealth};
use crate::overlay;

/// The gamepad and its rumble, in one chip: the worse of both sets its color.
pub(super) fn pad_chip(s: &Shared) -> (Color32, String, String) {
    let (rumble_color, rumble_text) = capture_status(s);
    let pad = match s.gamepads.first() {
        Some(name) if s.gamepads.len() > 1 => format!("{name} +{}", s.gamepads.len() - 1),
        Some(name) => name.clone(),
        None => "No gamepad found".to_owned(),
    };
    let (color, text) = match &s.source_health {
        SourceHealth::Off => (IDLE, "Gamepad capture off".to_owned()),
        SourceHealth::Waiting(why) => (WARN, format!("Gamepad: {why}")),
        SourceHealth::Failed(_) => (DANGER_TEXT, "Rumble not captured".to_owned()),
        SourceHealth::Working if s.gamepads.is_empty() => (WARN, pad),
        SourceHealth::Working if s.last_rumble.is_some_and(|t| s.time - t < RECENT_RUMBLE_SECS) => (OK, format!("{pad} · rumble ✔")),
        SourceHealth::Working => (OK, pad),
    };
    let hover = format!("{rumble_text}\n{}", if s.buttons_seen { "Buttons received" } else { "No button pressed yet" });
    (if color == OK { rumble_color } else { color }, text, hover)
}

/// Where the game's image stands: the overlay on Linux, the game's window on Windows.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Image {
    /// Modes are kept from seeing it (Captures & indicators).
    Off,
    /// The overlay is not installed.
    NotInstalled,
    /// Another GameViber holds the overlay's connection.
    Unavailable,
    /// No game shows the overlay (or, on Windows, is in front).
    NoGame,
    /// A game shows the overlay, its image not copied yet.
    Connected(String),
    Reading(String),
}

impl Image {
    fn of(s: &Shared, installed: bool) -> Self {
        let client = s.overlay_clients.first().map(|c| c.exe.clone());
        match client {
            _ if !s.settings.screen => Image::Off,
            _ if s.overlay_unavailable => Image::Unavailable,
            Some(exe) if s.screen.rate > 0.0 => Image::Reading(exe),
            Some(exe) => Image::Connected(exe),
            None if !installed && !overlay::WINDOW_CAPTURE => Image::NotInstalled,
            None => Image::NoGame,
        }
    }

    /// The chip's color and text. On Linux, no overlay is red: modes reading
    /// the image cannot work; on Windows, problems show when the active mode needs it.
    fn chip(&self, needed: bool) -> (Color32, String) {
        let name = if overlay::WINDOW_CAPTURE { "Image" } else { "Overlay" };
        let problem = if !overlay::WINDOW_CAPTURE { DANGER_TEXT } else if needed { WARN } else { IDLE };
        match self {
            Image::Off => (problem, "Image: off".to_owned()),
            Image::NotInstalled => (problem, "Overlay not installed".to_owned()),
            Image::Unavailable => (DANGER_TEXT, "Overlay held elsewhere".to_owned()),
            Image::NoGame => (problem, format!("{name}: no game")),
            Image::Connected(exe) | Image::Reading(exe) => (OK, format!("{name}: {}", exe.strip_suffix(".exe").unwrap_or(exe))),
        }
    }

    /// What to do to get the image, in a line; None when it comes.
    fn fix(&self, game: &str) -> Option<String> {
        match self {
            Image::Reading(_) | Image::Connected(_) => None,
            Image::Off => Some("Turn it back on in Creator › Captures & indicators.".to_owned()),
            Image::NotInstalled => Some("Install the in-game overlay: on Linux, it is required.".to_owned()),
            Image::Unavailable => Some("Close the other GameViber using the overlay.".to_owned()),
            Image::NoGame if overlay::WINDOW_CAPTURE => Some(format!("Bring {game} to the front, fullscreen or borderless.")),
            Image::NoGame => Some(format!("Launch {game} with GAMEVIBER_OVERLAY=1 %command% in its launch options.")),
        }
    }
}

/// The sound listened to: the game's own choice, or else the default.
fn sound_source(s: &Shared) -> AudioSource {
    s.game.as_ref().and_then(|g| g.audio.clone()).unwrap_or_else(|| s.settings.audio.clone())
}

fn sound_chip(s: &Shared, needed: bool) -> (Color32, String) {
    match (&s.audio.status.target, &s.audio.status.error) {
        _ if sound_source(s) == AudioSource::Off => (if needed { WARN } else { IDLE }, "Sound: off".to_owned()),
        (_, Some(_)) => (DANGER_TEXT, "Sound: error".to_owned()),
        (Some(target), None) => (OK, format!("Sound: {target}")),
        (None, None) => (IDLE, "Sound: waiting".to_owned()),
    }
}

/// What the active mode reads that GameViber may not get.
struct Needs {
    /// The indicators it reads from the game's image.
    indicators: Vec<String>,
    /// It recognizes phases from the image.
    image_phases: bool,
    /// It recognizes phases from the sound.
    sound: bool,
}

impl Needs {
    fn image(&self) -> bool {
        !self.indicators.is_empty() || self.image_phases
    }

    /// What stops working without the image: "6 indicators and the image phases".
    fn without_image(&self) -> String {
        let indicators = match self.indicators.as_slice() {
            [] => None,
            [one] => Some(format!("the indicator {one}")),
            many => Some(format!("{} indicators", many.len())),
        };
        match (indicators, self.image_phases) {
            (Some(i), true) => format!("{i} and the image phases"),
            (Some(i), false) => i,
            (None, _) => "the image phases".to_owned(),
        }
    }
}

fn needs(s: &Shared) -> Needs {
    let mut indicators: Vec<String> = s.mode_inputs.iter().flat_map(|i| i.zones.iter().map(|z| z.indicator.clone())).collect();
    indicators.sort();
    indicators.dedup();
    Needs { indicators, image_phases: s.phases.screen.0, sound: s.phases.sound.0 }
}

/// A line of the card: what goes wrong, what to do, the way there.
struct Problem {
    color: Color32,
    title: String,
    fix: String,
    hover: Option<String>,
    button: &'static str,
    /// None: the Toys page.
    tab: Option<setup::Tab>,
}

/// The card stays readable on wide windows.
const CARD_WIDTH: f32 = 620.0;

impl App {
    /// The status bar's chips: gamepad and rumble, toys, the game's image, its sound.
    pub(super) fn status_chips(&mut self, ui: &mut egui::Ui, s: &Shared, width: f32) {
        let needs = needs(s);
        let (color, text, hover) = pad_chip(s);
        if status_chip(ui, color, "🎮", &text, width).on_hover_text(hover).clicked() {
            self.open_setup(setup::Tab::Gamepad);
        }
        let (color, text) = intiface_status(s);
        if status_chip(ui, color, "🔌", &text, width).clicked() {
            self.page = Page::Toys;
        }
        let image = Image::of(s, self.overlay_installed(s));
        let (color, text) = image.chip(needs.image());
        let hover = match image.fix("the game") {
            Some(fix) => format!("No game image: indicators and image phases stay empty.\n{fix}"),
            None => "Modes see the game's image: phases, indicators, flashes".to_owned(),
        };
        if status_chip(ui, color, "🖼", &text, width).on_hover_text(hover).clicked() {
            self.open_setup(setup::Tab::Overlay);
        }
        let (color, text) = sound_chip(s, needs.sound);
        let hover = s.audio.status.error.clone().unwrap_or_else(|| "The game's sound: hits, levels, phases".to_owned());
        if status_chip(ui, color, "🔊", &text, width).on_hover_text(hover).clicked() {
            self.open_setup(setup::Tab::Sound);
        }
    }

    fn open_setup(&mut self, tab: setup::Tab) {
        if self.page != Page::Setup {
            self.overlay.forget_install_state();
        }
        self.page = Page::Setup;
        self.setup_tab = tab;
    }

    /// What keeps the active mode from working as it should, each with the way
    /// to fix it; nothing when all is well. In the Creator (`creating`), the
    /// image is needed anyway: captures and sessions' images come from it;
    /// there, not everything is wanted while making a mode: the card folds.
    pub(super) fn readiness_card(&mut self, ui: &mut egui::Ui, s: &Shared, creating: bool) {
        let needs = needs(s);
        let game = s.game.as_ref().map_or("the game", |g| g.name.as_str());
        let mut problems = Vec::new();
        let mut problem = |color, title: &str, fix: String, button, tab| {
            problems.push(Problem { color, title: title.to_owned(), fix, hover: None, button, tab });
        };
        if !s.intiface_enabled || !s.intiface.connected {
            problem(WARN, "No toy will move", "Start Intiface Central, then connect to it.".to_owned(), "Toys ›", None);
        } else if s.intiface.toys.is_empty() {
            problem(WARN, "No toy connected", "Turn a toy on, then Scan.".to_owned(), "Toys ›", None);
        }
        let gamepad = Some(setup::Tab::Gamepad);
        match &s.source_health {
            SourceHealth::Off => problem(WARN, "Rumble not captured", "Choose how to capture it.".to_owned(), "Gamepad ›", gamepad),
            SourceHealth::Failed(e) => problem(DANGER_TEXT, "Rumble not captured", e.clone(), "Gamepad ›", gamepad),
            SourceHealth::Waiting(why) => problem(WARN, "Rumble not captured yet", why.clone(), "Gamepad ›", gamepad),
            SourceHealth::Working => {}
        }
        if needs.sound && sound_source(s) == AudioSource::Off {
            problem(WARN, "Sound turned off", "This mode's phases listen to it.".to_owned(), "Sound ›", Some(setup::Tab::Sound));
        }
        let image = Image::of(s, self.overlay_installed(s));
        let image_color = if overlay::WINDOW_CAPTURE { WARN } else { DANGER_TEXT };
        if let Some(fix) = image.fix(game) {
            let title = if needs.image() {
                Some((image_color, format!("No game image: {} won't work", needs.without_image())))
            } else if creating {
                Some((IDLE, "No game image: no captures, no images in sessions".to_owned()))
            } else {
                None
            };
            if let Some((color, title)) = title {
                let hover = (needs.indicators.len() > 1).then(|| format!("Indicators: {}", needs.indicators.join(", ")));
                problems.insert(0, Problem { color, title, fix, hover, button: "Overlay ›", tab: Some(setup::Tab::Overlay) });
            }
        }
        if problems.is_empty() {
            return;
        }
        let border = if problems.iter().any(|p| p.color == DANGER_TEXT) {
            DANGER
        } else if problems.iter().any(|p| p.color != IDLE) {
            WARN
        } else {
            LINE
        };
        card(PANEL).stroke(egui::Stroke::new(1.0, border)).inner_margin(Margin::symmetric(16, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width().min(CARD_WIDTH));
            let foldable = self.page == Page::Creator;
            let folded = foldable && self.creator.readiness_folded;
            ui.horizontal(|ui| {
                eyebrow(ui, if creating { "To get everything" } else { "Before you play" });
                if foldable {
                    if folded {
                        let worst = problems.iter().find(|p| p.color != IDLE).map_or(IDLE, |p| p.color);
                        dot(ui, worst);
                        ui.label(muted(format!("{} to set up", problems.len())).size(12.5)).on_hover_text(
                            problems.iter().map(|p| p.title.as_str()).collect::<Vec<_>>().join("\n"),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (text, hover) = if folded { ("Show", "Show what is missing") } else { ("Hide", "Not everything is needed while making a mode") };
                        if ui.small_button(text).on_hover_text(hover).clicked() {
                            self.creator.readiness_folded = !folded;
                        }
                    });
                }
            });
            if folded {
                return;
            }
            for p in problems {
                ui.horizontal_top(|ui| {
                    ui.add_space(2.0);
                    dot(ui, p.color);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let title = ui.label(RichText::new(&p.title).strong().size(13.5));
                        if let Some(hover) = &p.hover {
                            title.on_hover_text(hover);
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(muted(&p.fix).size(12.5));
                            if ui.small_button(p.button).clicked() {
                                match p.tab {
                                    Some(tab) => self.open_setup(tab),
                                    None => self.page = Page::Toys,
                                }
                            }
                        });
                    });
                });
            }
        });
        ui.add_space(8.0);
    }
}
