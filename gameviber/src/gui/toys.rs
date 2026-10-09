//! Toys page: the connection to Intiface Central, what it found, a test buzz,
//! which output channel of the active mode each toy plays, and per-toy
//! intensity settings (and what a stroker can do, found step by step with the
//! player: its calibration).

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{intiface_status, App};
use crate::config::ToySettings;
use crate::engine::{Command, Shared, TEST_LEVEL};
use crate::intiface::{Control, Toy};
use crate::stroke::{Calibration, Ramp, StrokeSettings, StrokeStyle};

pub const INTIFACE_DOWNLOAD: &str = "https://intiface.com/central/";

#[derive(Default)]
pub struct State {
    /// Intiface address being edited (None: show the saved one).
    url: Option<String>,
    /// Stroker being calibrated: the page shows its steps instead of the toys.
    calibration: Option<Wizard>,
}

struct Wizard {
    toy: String,
    step: Step,
    /// What the step's ramp found, once the player answered or it ended.
    found: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Range,
    Ramp(Ramp),
    Done,
}

impl Step {
    const ALL: [Step; 5] = [Step::Range, Step::Ramp(Ramp::Fastest), Step::Ramp(Ramp::Turns), Step::Ramp(Ramp::Slowest), Step::Done];

    fn label(self) -> &'static str {
        match self {
            Step::Range => "Range",
            Step::Ramp(Ramp::Fastest) => "Fastest",
            Step::Ramp(Ramp::Turns) => "Turns",
            Step::Ramp(Ramp::Slowest) => "Slowest",
            Step::Done => "Done",
        }
    }

    fn next(self) -> Step {
        let i = Step::ALL.iter().position(|s| *s == self).unwrap_or(0);
        Step::ALL[(i + 1).min(Step::ALL.len() - 1)]
    }
}

impl App {
    pub(super) fn toys_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.toys.calibration.is_some() {
                    self.calibration_ui(ui, s);
                    return;
                }
                heading(ui, "Your toys");
                ui.label(muted(
                    "Found by Intiface Central. Turn a toy on while it scans to add it.",
                ));
                ui.add_space(8.0);
                self.intiface_card(ui, s);
                ui.add_space(8.0);
                if !s.intiface.connected {
                    return;
                }
                if s.intiface.toys.is_empty() {
                    waiting_for_toys(ui);
                    return;
                }
                let channels = s.mode.info.as_ref().map(|i| i.channels.clone()).unwrap_or_else(|| vec!["main".into()]);
                let height = if s.intiface.toys.iter().any(|t| t.stroker) { 470.0 } else { 270.0 };
                tile_grid(ui, s.intiface.toys.len(), 360.0, height, |ui, i, size| {
                    let toy = &s.intiface.toys[i];
                    card(PANEL).show(ui, |ui| {
                        ui.set_width(size.x - 32.0);
                        ui.set_min_height(size.y - 32.0);
                        self.toy_card(ui, s, toy, &channels);
                    });
                });
                ui.add_space(8.0);
                if s.intiface.toys.iter().any(|t| t.numbered) {
                    card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            "ℹ Several toys have the same name, so GameViber numbers them in the order they \
                             connect. Give each one its own name in Intiface Central to keep their settings \
                             attached to the right toy.",
                        );
                    });
                    ui.add_space(8.0);
                }
                if channels.len() > 1 {
                    card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(format!(
                            "ℹ This mode sends {} different feelings ({}). Pick which ones each toy plays.",
                            channels.len(),
                            channels.join(", ")
                        ));
                    });
                }
            });
        });
    }

    /// Connection to Intiface Central: its state, how to get it running, its
    /// address, and the buttons to disconnect, reconnect and scan for toys.
    fn intiface_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let status = &s.intiface;
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (color, text) = intiface_status(s);
            ui.horizontal(|ui| {
                dot(ui, color);
                ui.label(RichText::new("🔌 Intiface Central").strong());
                ui.label(text);
                if status.connected && !status.server.is_empty() {
                    ui.label(muted(&status.server).size(12.0));
                }
                if !s.intiface_enabled {
                    return;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if status.paused {
                        if ui.add(primary("Connect")).clicked() {
                            self.send(Command::Intiface(Control::Connect));
                        }
                    } else if status.connected {
                        if ui.button("Disconnect").on_hover_text("Stops your toys and leaves Intiface Central").clicked() {
                            self.send(Command::Intiface(Control::Disconnect));
                        }
                        if ui.button("⟳ Reconnect").on_hover_text("Connects again, for a toy or a server that misbehaves").clicked() {
                            self.send(Command::Intiface(Control::Connect));
                        }
                    } else {
                        if ui.button("Stop trying").on_hover_text("Stays disconnected until you press Connect").clicked() {
                            self.send(Command::Intiface(Control::Disconnect));
                        }
                        if ui.button("⟳ Retry now").clicked() {
                            self.send(Command::Intiface(Control::Connect));
                        }
                    }
                });
            });
            if status.paused {
                ui.label(muted("Disconnected: your toys get nothing from GameViber until you connect again."));
            } else if s.intiface_enabled && !status.connected {
                ui.label(muted(
                    "GameViber reaches your toys through Intiface Central, a free app: open it and press its \
                     Start button. GameViber connects on its own.",
                ));
                ui.hyperlink_to("Get Intiface Central ↗", INTIFACE_DOWNLOAD);
            }
            if status.connected {
                ui.horizontal(|ui| {
                    if status.scanning {
                        ui.spinner();
                        ui.label("Looking for new toys");
                        if ui.button("Stop scanning").on_hover_text("Toys already found stay connected").clicked() {
                            self.send(Command::Intiface(Control::StopScanning));
                        }
                    } else {
                        dot(ui, IDLE);
                        ui.label("Not looking for new toys");
                        if ui.button("🔍 Start scanning").on_hover_text("Turn the toy on first").clicked() {
                            self.send(Command::Intiface(Control::StartScanning));
                        }
                    }
                });
                if let Some(e) = &status.error {
                    ui.label(RichText::new(e).monospace().size(12.0).color(MUTED));
                }
            }
            egui::CollapsingHeader::new("Server address").id_salt("intiface-address").show(ui, |ui| {
                self.intiface_address(ui, s);
                if let (false, Some(e)) = (status.connected, &status.error) {
                    ui.label(RichText::new(e).monospace().size(12.0).color(MUTED));
                }
            });
        });
    }

    /// Server address editor (Intiface on another machine or port).
    pub(super) fn intiface_address(&mut self, ui: &mut egui::Ui, s: &Shared) {
        ui.horizontal(|ui| {
            ui.label("Server address");
            let url = self.toys.url.get_or_insert_with(|| s.settings.url.clone());
            ui.add(egui::TextEdit::singleline(url).desired_width(260.0).font(egui::TextStyle::Monospace));
            let changed = *url != s.settings.url;
            if ui.add_enabled(changed, egui::Button::new("Apply")).clicked() {
                let url = url.clone();
                self.send(Command::SetUrl(url));
            }
            if !changed {
                // Follow the saved value until the user edits it.
                self.toys.url = None;
            }
        });
        ui.label(muted("Intiface Central shows it on its main screen. The default is ws://127.0.0.1:12345.").size(12.0));
    }

    fn toy_card(&mut self, ui: &mut egui::Ui, s: &Shared, toy: &Toy, channels: &[String]) {
        let name = toy.name.as_str();
        ui.horizontal(|ui| {
            ui.label(RichText::new(if toy.stroker { "↕" } else { "📳" }).size(20.0).color(ACCENT));
            ui.label(RichText::new(name).size(15.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, hint) = if toy.stroker {
                    ("↕ Stroke", "A few strokes to find which toy this is (it first gets in place slowly)")
                } else {
                    ("⚡ Buzz", "Short vibration to find which toy this is")
                };
                if ui.button(label).on_hover_text(hint).clicked() {
                    self.send(Command::TestToy(name.to_owned(), TEST_LEVEL));
                }
            });
        });
        let level = s.toy_levels.get(name).copied().unwrap_or(0.0);
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Now")));
            meter(ui, (ui.available_width() - 50.0).max(40.0), level, ACCENT);
            ui.label(RichText::new(format!("{:.0}%", level * 100.0)).monospace().size(12.0));
        });
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Plays")));
            for channel in channels {
                let mut toys: Vec<String> = match s.settings.routing.get(channel) {
                    Some(list) => list.clone(),
                    None if channel == "main" => s.intiface.toys.iter().map(|t| t.name.clone()).collect(),
                    None => Vec::new(),
                };
                let on = toys.iter().any(|t| t == name);
                let label = if channels.len() == 1 { if on { "On" } else { "Off" }.to_owned() } else { channel.clone() };
                if ui.selectable_label(on, label).clicked() {
                    toys.retain(|t| t != name);
                    if !on {
                        toys.push(name.to_owned());
                    }
                    self.send(Command::SetRouting { channel: channel.clone(), toys });
                }
            }
        });
        ui.separator();
        let settings = s.settings.toys.get(name).copied().unwrap_or_default();
        if let Some(settings) = toy_settings(ui, name, settings) {
            self.send(Command::SetToySettings { toy: name.to_owned(), settings });
        }
        if toy.stroker {
            ui.separator();
            if let Some(stroke) = stroke_settings(ui, settings.stroke) {
                self.send(Command::SetToySettings { toy: name.to_owned(), settings: ToySettings { stroke, ..settings } });
            }
            if ui.button("🎯 Calibrate").on_hover_text("Find what this toy can do, step by step, by watching it").clicked() {
                self.send(Command::Calibrate(Some((name.to_owned(), Calibration::Pause))));
                self.toys.calibration = Some(Wizard { toy: name.to_owned(), step: Step::Range, found: None });
            }
        }
        let hints = if toy.stroker {
            ["The slowest, shortest strokes a mode can ask for", "Strokes at half strength", "The fastest, longest strokes a mode can ask for (within Strongest)"]
        } else {
            ["The gentlest vibration a mode can ask for", "A vibration at half strength", "The strongest vibration a mode can ask for (within Strongest)"]
        };
        ui.horizontal(|ui| {
            ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Feel")));
            for ((label, level), hint) in [("Weakest", ToySettings::SILENT), ("Medium", TEST_LEVEL), ("Strongest", 1.0)].into_iter().zip(hints) {
                if ui.button(label).on_hover_text(hint).clicked() {
                    self.send(Command::TestToy(name.to_owned(), level));
                }
            }
            if settings != ToySettings::default() && ui.button("Defaults").clicked() {
                self.send(Command::SetToySettings { toy: name.to_owned(), settings: ToySettings::default() });
            }
        });
    }
}

/// Sliders for a toy's weakest and strongest vibration and its curve, next to a
/// preview of the curve. Returns the new settings when one changed.
fn toy_settings(ui: &mut egui::Ui, name: &str, settings: ToySettings) -> Option<ToySettings> {
    let mut new = settings;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 190.0).max(80.0);
            let mut min = new.min * 100.0;
            let mut max = new.max * 100.0;
            labeled(ui, "Weakest", |ui| ui.add(egui::Slider::new(&mut min, 0.0..=100.0).suffix("%").integer()))
                .on_hover_text("Raise it until the gentlest vibrations are felt: many toys do nothing below 10-20%");
            labeled(ui, "Strongest", |ui| ui.add(egui::Slider::new(&mut max, 0.0..=100.0).suffix("%").integer()))
                .on_hover_text("Lower it if this toy is too strong compared to the others");
            labeled(ui, "Curve", |ui| {
                ui.add(egui::Slider::new(&mut new.curve, ToySettings::CURVE_RANGE).step_by(0.05).fixed_decimals(2))
            })
            .on_hover_text("Below 1: gentle vibrations feel stronger. Above 1: they feel softer and peaks stand out");
            // Weakest and strongest push each other rather than crossing.
            if min != new.min * 100.0 {
                new.min = min / 100.0;
                new.max = new.max.max(new.min);
            } else if max != new.max * 100.0 {
                new.max = max / 100.0;
                new.min = new.min.min(new.max);
            }
        });
        curve_preview(ui, name, &new);
    });
    (new != settings).then_some(new)
}

fn labeled(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> egui::Response) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted(label)));
        add(ui)
    })
    .inner
}

impl App {
    /// Stops the calibration, if any: the toy plays the mode again.
    pub(super) fn close_calibration(&mut self) {
        if self.toys.calibration.take().is_some() {
            self.send(Command::Calibrate(None));
        }
    }

    /// The calibration's steps: the range, set with the toy going where the
    /// sliders are, then ramps the player stops when the toy stops following.
    fn calibration_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let Some(wizard) = self.toys.calibration.as_ref() else { return };
        let (name, step) = (wizard.toy.clone(), wizard.step);
        ui.horizontal(|ui| {
            if ui.button("‹ Toys").clicked() {
                self.close_calibration();
            }
            heading(ui, &format!("Calibrate {name}"));
        });
        ui.label(muted(
            "Watch the toy, without using it: each step finds what it can do. GameViber then keeps a margin \
             and never asks for more.",
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            for (i, s) in Step::ALL.into_iter().enumerate() {
                let text = format!("{}. {}", i + 1, s.label());
                ui.label(if s == step { RichText::new(text).strong().color(ACCENT) } else { muted(&text) });
                if s != Step::Done {
                    ui.label(muted("›"));
                }
            }
        });
        ui.add_space(8.0);
        if !s.intiface.toys.iter().any(|t| t.name == name && t.stroker) {
            ui.label(format!("{name} is not connected anymore."));
            if ui.button("Close").clicked() {
                self.close_calibration();
            }
            return;
        }
        let settings = s.settings.toys.get(&name).copied().unwrap_or_default();
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match step {
                Step::Range => self.range_step(ui, &name, settings),
                Step::Ramp(ramp) => self.ramp_step(ui, s, &name, ramp, settings),
                Step::Done => self.calibration_done(ui, settings),
            }
        });
    }

    fn range_step(&mut self, ui: &mut egui::Ui, name: &str, settings: ToySettings) {
        ui.label(RichText::new("How far it goes").strong());
        ui.label(
            "Move a slider: the toy slowly goes there. Set the lowest and the highest positions it should reach.",
        );
        ui.add_space(4.0);
        let stroke = settings.stroke;
        let (mut bottom, mut top) = (stroke.bottom * 100.0, stroke.top * 100.0);
        ui.spacing_mut().slider_width = (ui.available_width() - 260.0).max(120.0);
        let mut hold = None;
        ui.horizontal(|ui| {
            labeled(ui, "Lowest", |ui| ui.add(egui::Slider::new(&mut bottom, 0.0..=100.0).suffix("%").integer()));
            if ui.button("Go there").clicked() {
                hold = Some(stroke.bottom);
            }
        });
        ui.horizontal(|ui| {
            labeled(ui, "Highest", |ui| ui.add(egui::Slider::new(&mut top, 0.0..=100.0).suffix("%").integer()));
            if ui.button("Go there").clicked() {
                hold = Some(stroke.top);
            }
        });
        let mut new = stroke;
        if bottom != stroke.bottom * 100.0 {
            new.bottom = bottom / 100.0;
            new.top = new.top.max(new.bottom);
            hold = Some(new.bottom);
        } else if top != stroke.top * 100.0 {
            new.top = top / 100.0;
            new.bottom = new.bottom.min(new.top);
            hold = Some(new.top);
        }
        if new != stroke {
            self.send(Command::SetToySettings { toy: name.to_owned(), settings: ToySettings { stroke: new, ..settings } });
        }
        if let Some(position) = hold {
            self.send(Command::Calibrate(Some((name.to_owned(), Calibration::Hold(position)))));
        }
        ui.add_space(8.0);
        if ui.add(primary("Next ›")).clicked() {
            self.next_step(name);
        }
    }

    fn ramp_step(&mut self, ui: &mut egui::Ui, s: &Shared, name: &str, ramp: Ramp, settings: ToySettings) {
        let (title, explanation, button) = match ramp {
            Ramp::Fastest => (
                "Its fastest strokes",
                "The toy strokes faster and faster. Press the button as soon as it stops keeping up: \
                 strokes getting shorter, stutters, a strained sound.",
                "✋ It stopped keeping up",
            ),
            Ramp::Turns => (
                "Its quickest turns",
                "Short strokes, turning more and more often. Press the button as soon as turns get lost \
                 or the toy shakes in place.",
                "✋ Turns get lost",
            ),
            Ramp::Slowest => (
                "Its slowest strokes",
                "Strokes slower and slower. Press the button as soon as they stop being smooth: jerks, pauses.",
                "✋ It jerks",
            ),
        };
        ui.label(RichText::new(title).strong());
        ui.label(explanation);
        ui.add_space(8.0);
        let running = s
            .calibration
            .as_ref()
            .filter(|(toy, test, _)| toy == name && *test == Calibration::Ramp(ramp))
            .map(|(.., elapsed)| *elapsed);
        let found = self.toys.calibration.as_ref().and_then(|w| w.found);
        match running {
            Some(elapsed) => match ramp.value(elapsed) {
                Some(value) => {
                    ui.label(RichText::new(ramp_value(ramp, value)).size(18.0).strong());
                    meter(ui, ui.available_width().min(400.0), elapsed / ramp.length(), ACCENT);
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(primary(button).min_size(Vec2::new(220.0, 36.0))).clicked() {
                            self.ramp_found(name, ramp, ramp.result(elapsed), settings);
                        }
                        if ui.button("Stop").clicked() {
                            self.send(Command::Calibrate(Some((name.to_owned(), Calibration::Pause))));
                        }
                    });
                }
                // It followed every step.
                None => self.ramp_found(name, ramp, ramp.result(f64::INFINITY), settings),
            },
            None => {
                if let Some(value) = found {
                    ui.label(format!("Kept: {}, with a margin.", ramp_value(ramp, value)));
                }
                ui.horizontal(|ui| {
                    let start = if found.is_some() { "▶ Again" } else { "▶ Start" };
                    if ui.button(start).clicked() {
                        self.send(Command::Calibrate(Some((name.to_owned(), Calibration::Ramp(ramp)))));
                    }
                    let next = if found.is_some() { "Next ›" } else { "Skip ›" };
                    if ui.add(primary(next)).clicked() {
                        self.next_step(name);
                    }
                });
                ui.label(muted(format!("About {:.0} s at most.", ramp.length())).size(12.0));
            }
        }
    }

    /// Keeps what a ramp found and holds the toy still.
    fn ramp_found(&mut self, name: &str, ramp: Ramp, value: f64, settings: ToySettings) {
        let mut stroke = settings.stroke;
        match ramp {
            Ramp::Fastest => stroke.fastest = value,
            Ramp::Turns => stroke.min_turn = value,
            Ramp::Slowest => stroke.slowest = value.max(stroke.fastest),
        }
        self.send(Command::SetToySettings { toy: name.to_owned(), settings: ToySettings { stroke, ..settings } });
        self.send(Command::Calibrate(Some((name.to_owned(), Calibration::Pause))));
        if let Some(wizard) = self.toys.calibration.as_mut() {
            wizard.found = Some(value);
        }
    }

    fn next_step(&mut self, name: &str) {
        self.send(Command::Calibrate(Some((name.to_owned(), Calibration::Pause))));
        if let Some(wizard) = self.toys.calibration.as_mut() {
            wizard.step = wizard.step.next();
            wizard.found = None;
        }
    }

    fn calibration_done(&mut self, ui: &mut egui::Ui, settings: ToySettings) {
        let stroke = settings.stroke;
        ui.label(RichText::new("Calibrated").strong());
        ui.label(format!("Range: {:.0}% to {:.0}%", stroke.bottom * 100.0, stroke.top * 100.0));
        ui.label(format!("Fastest: {}", ramp_value(Ramp::Fastest, stroke.fastest)));
        ui.label(format!("Turns: {}", ramp_value(Ramp::Turns, stroke.min_turn)));
        ui.label(format!("Slowest: {}", ramp_value(Ramp::Slowest, stroke.slowest)));
        ui.label(muted("Change them on the toy's card at any time, or calibrate again."));
        ui.add_space(8.0);
        if ui.add(primary("Done")).clicked() {
            self.close_calibration();
        }
    }
}

/// A ramp's value in words.
fn ramp_value(ramp: Ramp, value: f64) -> String {
    match ramp {
        Ramp::Fastest => format!("whole length in {value:.2} s"),
        Ramp::Turns => format!("a turn every {value:.2} s"),
        Ramp::Slowest => format!("whole length in {value:.1} s"),
    }
}

/// What a stroker can do and how far it goes: its range, its fastest and slowest
/// moves, how often it may turn, and what the intensity changes. Returns the new
/// settings when one changed.
fn stroke_settings(ui: &mut egui::Ui, settings: StrokeSettings) -> Option<StrokeSettings> {
    let mut new = settings;
    ui.spacing_mut().slider_width = (ui.available_width() - 150.0).max(80.0);
    let mut bottom = new.bottom * 100.0;
    let mut top = new.top * 100.0;
    labeled(ui, "Lowest", |ui| ui.add(egui::Slider::new(&mut bottom, 0.0..=100.0).suffix("%").integer()))
        .on_hover_text("The lowest position the toy goes to, from its whole length");
    labeled(ui, "Highest", |ui| ui.add(egui::Slider::new(&mut top, 0.0..=100.0).suffix("%").integer()))
        .on_hover_text("The highest position the toy goes to, from its whole length");
    // Lowest and highest push each other rather than crossing.
    if bottom != new.bottom * 100.0 {
        new.bottom = bottom / 100.0;
        new.top = new.top.max(new.bottom);
    } else if top != new.top * 100.0 {
        new.top = top / 100.0;
        new.bottom = new.bottom.min(new.top);
    }
    labeled(ui, "Fastest", |ui| {
        ui.add(egui::Slider::new(&mut new.fastest, StrokeSettings::FASTEST_RANGE).suffix(" s").step_by(0.05).fixed_decimals(2))
    })
    .on_hover_text(
        "Time of the fastest move over the toy's whole length. GameViber never asks for faster: \
         raise it if the toy stops following (shorter strokes, stutters)",
    );
    labeled(ui, "Slowest", |ui| {
        ui.add(egui::Slider::new(&mut new.slowest, StrokeSettings::SLOWEST_RANGE).suffix(" s").step_by(0.5).fixed_decimals(1))
    })
    .on_hover_text("Time of the slowest move over the toy's whole length: lower it if slow strokes jerk");
    labeled(ui, "Turns", |ui| {
        ui.add(egui::Slider::new(&mut new.min_turn, StrokeSettings::TURN_RANGE).suffix(" s").step_by(0.05).fixed_decimals(2))
    })
    .on_hover_text("Shortest time between two changes of direction: raise it if quick turns get lost");
    new.slowest = new.slowest.max(new.fastest);
    ui.horizontal(|ui| {
        ui.add_sized(Vec2::new(60.0, 18.0), egui::Label::new(muted("Stronger")));
        for style in StrokeStyle::ALL {
            let hint = match style {
                StrokeStyle::Both => "Faster and longer strokes",
                StrokeStyle::Speed => "Faster strokes, always over the whole range",
                StrokeStyle::Depth => "Longer strokes, at a medium speed",
            };
            if ui.selectable_label(new.style == style, style.label()).on_hover_text(hint).clicked() {
                new.style = style;
            }
        }
    })
    .response
    .on_hover_text("What a stronger feeling changes in the strokes");
    (new != settings).then_some(new)
}

/// Requested intensity (x) -> toy intensity (y).
fn curve_preview(ui: &mut egui::Ui, name: &str, settings: &ToySettings) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(72.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 6, RAISED);
    painter.line_segment([rect.left_bottom(), rect.right_top()], egui::Stroke::new(1.0, LINE));
    let points: Vec<egui::Pos2> = (0..=40)
        .map(|i| {
            let x = i as f64 / 40.0;
            let y = settings.shape(x);
            egui::pos2(rect.left() + rect.width() * x as f32, rect.bottom() - rect.height() * y as f32)
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, ACCENT)));
    response.on_hover_text(format!("How {name} answers: what the mode asks for (left to right) and what the toy plays (up)"));
}

pub(super) fn waiting_for_toys(ui: &mut egui::Ui) {
    card(BG).stroke(egui::Stroke::new(1.5, LINE)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Waiting for toys...");
        });
        ui.label(muted("Not showing up? Check it is charged, switched on and not connected to another app."));
    });
}
