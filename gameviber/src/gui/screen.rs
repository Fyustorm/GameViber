//! Game page: what GameViber knows about the game being played. Its image as
//! the in-game overlay copies it, the zones of its screen modes can read
//! (drawn on a frozen image), example images of its scenes, the scenes of the
//! active mode, and the values other programs send. Zones, examples and the
//! declared inputs make the game's profile (`profile.rs`).

use std::sync::Arc;

use eframe::egui::{self, Color32, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind};

use super::audio::{model_card, scenes_card};
use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::models::Model;
use crate::profile::{valid_name, Direction, InputDecl, InputKind, Profile, Zone, ZoneKind};
use crate::screen::{zones, Frame};

/// Images are never shown wider than this.
const PREVIEW_WIDTH: f32 = 640.0;

#[derive(Default)]
pub struct State {
    texture: Option<egui::TextureHandle>,
    /// Count of the frame in the texture.
    shown: Option<u32>,
    draft: Option<Draft>,
    /// Name typed for a new scene of examples.
    new_scene: String,
    new_input: InputDecl,
    port: Option<String>,
}

/// A zone being drawn on a frozen image.
struct Draft {
    frame: Arc<Frame>,
    texture: egui::TextureHandle,
    /// Corners of the rectangle, as fractions of the image.
    start: Option<Pos2>,
    end: Option<Pos2>,
    name: String,
    kind: ZoneKind,
    direction: Direction,
}

impl Draft {
    fn rect(&self) -> Option<[f32; 4]> {
        let (a, b) = (self.start?, self.end?);
        let (x0, y0, x1, y1) = (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        (x1 - x0 > 0.005 && y1 - y0 > 0.005).then_some([x0, y0, x1 - x0, y1 - y0])
    }
}

fn texture(ctx: &egui::Context, name: &str, frame: &Frame) -> egui::TextureHandle {
    let image = egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.pixels);
    ctx.load_texture(name, image, egui::TextureOptions::LINEAR)
}

fn image_size(ui: &egui::Ui, frame: &Frame) -> egui::Vec2 {
    let width = ui.available_width().min(PREVIEW_WIDTH);
    egui::vec2(width, width * frame.height as f32 / frame.width.max(1) as f32)
}

/// A zone's rectangle on an image drawn in `area`.
fn on_image(area: Rect, rect: [f32; 4]) -> Rect {
    Rect::from_min_size(
        area.min + egui::vec2(rect[0] * area.width(), rect[1] * area.height()),
        egui::vec2(rect[2] * area.width(), rect[3] * area.height()),
    )
}

impl App {
    pub(super) fn screen_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Game");
                ui.label(muted(
                    "What GameViber knows about the game you play, for every mode: its image, the zones of its \
                     screen, examples of its scenes, and values other programs send. Images are analysed on your \
                     computer and never saved.",
                ));
                ui.add_space(8.0);
                self.screen_preview(ui, s);
                ui.add_space(8.0);
                self.zones_card(ui, s);
                ui.add_space(8.0);
                self.examples_card(ui, s);
                ui.add_space(8.0);
                self.inputs_card(ui, s);
            });
        });
    }

    fn screen_preview(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let view = &s.screen;
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Seen right now");
            let mut on = s.settings.screen;
            if ui
                .checkbox(&mut on, "Modes see the game's image")
                .on_hover_text("Scenes from the image, zones, flashes and motion. Needs the in-game overlay.")
                .changed()
            {
                self.send(Command::SetScreen(on));
            }
            let Some(frame) = &view.frame else {
                self.screen.texture = None;
                self.screen.shown = None;
                let hint = if s.overlay_unavailable {
                    "Another GameViber holds the in-game overlay."
                } else if s.overlay_clients.is_empty() {
                    "Nothing: start a game with the in-game overlay (see the Overlay page)."
                } else {
                    "Waiting for the game's image..."
                };
                ui.horizontal(|ui| {
                    dot(ui, WARN);
                    ui.label(hint);
                });
                return;
            };
            ui.horizontal(|ui| {
                dot(ui, OK);
                ui.label(format!(
                    "{}: {}x{} copies of a {}x{} image, {:.0} per second",
                    view.game.as_deref().unwrap_or("game"),
                    frame.width,
                    frame.height,
                    frame.source_width,
                    frame.source_height,
                    view.rate
                ));
            });
            if self.screen.shown != Some(frame.count) {
                let image = egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.pixels);
                match &mut self.screen.texture {
                    Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                    None => self.screen.texture = Some(texture(ui.ctx(), "game-image", frame)),
                }
                self.screen.shown = Some(frame.count);
            }
            if let Some(texture) = &self.screen.texture {
                let response = ui.add(egui::Image::new(texture).fit_to_exact_size(image_size(ui, frame)).corner_radius(6.0));
                // The profile's zones over the image, lit while shown.
                for zone in s.profile.iter().flat_map(|p| &p.zones) {
                    let lit = view.zones.iter().any(|(n, _, v)| *n == zone.name && *v == Some(crate::mode::ZoneValue::Visible(true)));
                    let color = if lit || zone.kind == ZoneKind::Bar { ACCENT } else { MUTED };
                    let r = on_image(response.rect, zone.rect);
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, color), StrokeKind::Outside);
                    ui.painter().text(r.left_top() - egui::vec2(0.0, 2.0), egui::Align2::LEFT_BOTTOM, &zone.name, egui::FontId::proportional(11.0), color);
                }
            }
            if let Some(levels) = view.levels {
                ui.add_space(6.0);
                egui::Grid::new("screen-levels").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (label, value, hint) in [
                        ("Brightness", levels.brightness, "input.screen.brightness"),
                        ("Motion", levels.motion, "input.screen.motion: change between two copies"),
                        ("Action", levels.action, "input.screen.action: motion over the last seconds"),
                    ] {
                        ui.label(muted(label)).on_hover_text(hint);
                        meter(ui, 240.0, value as f64, GAME);
                        ui.end_row();
                    }
                });
            }
        });
    }

    fn zones_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Zones of the screen");
            let Some(profile) = &s.profile else {
                ui.label(muted("Zones belong to a game: start one with the in-game overlay."));
                return;
            };
            ui.label(muted(
                "Draw a zone around something the game shows only at certain times (the battle interface, a \
                 warning) or around a bar (health). Modes read it as input.zones.<name>.",
            ));
            let mut changed: Option<Profile> = None;
            let mut remove = None;
            if !profile.zones.is_empty() {
                egui::Grid::new("zones").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                    for (i, zone) in profile.zones.iter().enumerate() {
                        let reading = s.screen.zones.iter().find(|(n, _, _)| *n == zone.name);
                        ui.label(RichText::new(&zone.name).strong()).on_hover_text(zone.kind.label());
                        match (zone.kind, reading) {
                            (ZoneKind::Visible, Some((_, similarity, value))) => {
                                let shown = *value == Some(crate::mode::ZoneValue::Visible(true));
                                meter(ui, 140.0, similarity.max(0.0) as f64, if shown { ACCENT } else { GAME });
                                ui.label(muted(if shown { "shown" } else { "not shown" }));
                            }
                            (ZoneKind::Bar, Some((_, fill, _))) => {
                                meter(ui, 140.0, *fill as f64, GAME);
                                ui.label(muted(format!("{:.0}%", fill * 100.0)));
                            }
                            _ => {
                                ui.label(muted("no image"));
                                ui.label("");
                            }
                        }
                        ui.horizontal(|ui| {
                            let mut edited = zone.clone();
                            let tuned = match zone.kind {
                                ZoneKind::Visible => ui
                                    .add(egui::Slider::new(&mut edited.threshold, 0.1..=0.95).text("threshold"))
                                    .on_hover_text("Similarity with its look when drawn above which it counts as shown"),
                                ZoneKind::Bar => ui
                                    .add(egui::Slider::new(&mut edited.tolerance, 10.0..=150.0).text("color tolerance"))
                                    .on_hover_text("How far from the bar's color a pixel may be"),
                            };
                            if tuned.drag_stopped() || (tuned.changed() && !tuned.dragged()) {
                                let mut p = profile.clone();
                                p.zones[i] = edited;
                                changed = Some(p);
                            }
                            if ui.small_button("Delete").clicked() {
                                remove = Some(i);
                            }
                        });
                        ui.end_row();
                    }
                });
            }
            if let Some(i) = remove {
                let mut p = profile.clone();
                p.zones.remove(i);
                changed = Some(p);
            }
            ui.add_space(6.0);
            if let Some(p) = self.zone_draft(ui, profile) {
                changed = Some(p);
            } else if self.screen.draft.is_none() {
                let frame = s.screen.frame.clone();
                if ui.add_enabled(frame.is_some(), primary("Draw a zone")).on_disabled_hover_text("Needs the game's image").clicked() {
                    if let Some(frame) = frame {
                        let texture = texture(ui.ctx(), "zone-draft", &frame);
                        self.screen.draft = Some(Draft {
                            frame,
                            texture,
                            start: None,
                            end: None,
                            name: String::new(),
                            kind: ZoneKind::Visible,
                            direction: Direction::Right,
                        });
                    }
                }
            }
            if let Some(p) = changed {
                self.send(Command::SaveProfile(p));
            }
        });
    }

    /// The zone being drawn: the frozen image, a name, a kind. Returns the profile with the new zone once saved.
    fn zone_draft(&mut self, ui: &mut egui::Ui, profile: &Profile) -> Option<Profile> {
        let draft = self.screen.draft.as_mut()?;
        ui.label("Drag a rectangle on the image (frozen when you clicked), then name the zone:");
        let size = image_size(ui, &draft.frame);
        let (area, response) = ui.allocate_exact_size(size, Sense::drag());
        ui.painter().image(draft.texture.id(), area, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        let to_fraction = |p: Pos2| Pos2::new(((p.x - area.left()) / area.width()).clamp(0.0, 1.0), ((p.y - area.top()) / area.height()).clamp(0.0, 1.0));
        if response.drag_started() {
            draft.start = response.interact_pointer_pos().map(to_fraction);
            draft.end = draft.start;
        }
        if response.dragged() {
            draft.end = response.interact_pointer_pos().map(to_fraction);
        }
        if let Some(rect) = draft.rect() {
            ui.painter().rect_stroke(on_image(area, rect), 2.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
        }
        let mut save = None;
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut draft.name).hint_text("battle_hud").desired_width(140.0));
            egui::ComboBox::from_id_salt("zone-kind").selected_text(draft.kind.label()).show_ui(ui, |ui| {
                for kind in [ZoneKind::Visible, ZoneKind::Bar] {
                    ui.selectable_value(&mut draft.kind, kind, kind.label());
                }
            });
            if draft.kind == ZoneKind::Bar {
                egui::ComboBox::from_id_salt("zone-direction").selected_text(draft.direction.label()).show_ui(ui, |ui| {
                    for d in Direction::ALL {
                        ui.selectable_value(&mut draft.direction, d, d.label());
                    }
                });
            }
        });
        let taken = profile.zones.iter().any(|z| z.name == draft.name);
        let problem = if draft.rect().is_none() {
            Some("Drag a rectangle on the image.")
        } else if !valid_name(&draft.name) {
            Some("Name: letters, digits and _, starting with a letter.")
        } else if taken {
            Some("A zone has this name already.")
        } else {
            None
        };
        if draft.kind == ZoneKind::Bar {
            ui.label(muted("Draw it while the bar is full, from its empty end to its full end."));
        } else {
            ui.label(muted("Draw it while the element is shown."));
        }
        let mut cancel = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(problem.is_none(), primary("Save the zone")).clicked() {
                if let Some(rect) = draft.rect() {
                    let mut zone = Zone { name: draft.name.clone(), kind: draft.kind, rect, direction: draft.direction, ..Zone::default() };
                    match zone.kind {
                        ZoneKind::Visible => zone.reference = zones::reference(&draft.frame, rect),
                        ZoneKind::Bar => zone.color = zones::bar_color(&draft.frame, rect),
                    }
                    let mut p = profile.clone();
                    p.zones.push(zone);
                    save = Some(p);
                }
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
            if let Some(problem) = problem {
                ui.label(muted(problem));
            }
        });
        if save.is_some() || cancel {
            self.screen.draft = None;
        }
        save
    }

    fn examples_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        if let Some(command) = model_card(ui, Model::Image, &s.screen.model) {
            self.send(command);
        }
        ui.add_space(8.0);
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Examples of scenes");
            let Some(profile) = &s.profile else {
                ui.label(muted("Examples belong to a game: start one with the in-game overlay."));
                return;
            };
            ui.label(muted(
                "While the game shows a scene, add its image as an example: a few of each scene (in different \
                 places) make the recognition much more reliable than descriptions alone. The images are kept \
                 as numbers, not pictures.",
            ));
            // The active mode's scenes first, then the profile's others.
            let mut names: Vec<String> = s.mode.info.iter().flat_map(|i| i.scenes.iter().map(|sc| sc.name.clone())).collect();
            for name in profile.examples.keys() {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            let can_tag = s.screen.can_tag;
            egui::Grid::new("examples").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                for name in &names {
                    let count = profile.examples.get(name).map_or(0, Vec::len);
                    ui.label(RichText::new(name).strong());
                    ui.label(muted(format!("{count} examples")));
                    ui.horizontal(|ui| {
                        let add = ui.add_enabled(can_tag, egui::Button::new(format!("This is {name} now")));
                        if add.on_disabled_hover_text("Needs the game's image and the image model").clicked() {
                            self.send(Command::AddExample(name.clone()));
                        }
                        if count > 0 && ui.small_button("Forget").clicked() {
                            self.send(Command::ClearExamples(name.clone()));
                        }
                    });
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.screen.new_scene).hint_text("another scene").desired_width(140.0));
                let name = self.screen.new_scene.trim().to_owned();
                if ui.add_enabled(can_tag && valid_name(&name), egui::Button::new("Add an example")).clicked() {
                    self.send(Command::AddExample(name));
                    self.screen.new_scene.clear();
                }
            });
        });
        ui.add_space(8.0);
        scenes_card(ui, &s.scenes, s.mode.info.as_ref().map(|i| i.name.as_str()));
    }

    fn inputs_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let view = &s.inputs;
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Values from other programs");
            ui.label(muted(
                "A game's mod, or a script reading a game's API, can send values and events to modes \
                 (input.custom, on_event) as JSON: {\"set\": {\"hp\": 0.4}} or {\"event\": \"kill\"}.",
            ));
            ui.horizontal(|ui| {
                match (&view.address, &view.error) {
                    (Some(address), _) => {
                        dot(ui, OK);
                        ui.label(format!("WebSocket: {address}"));
                        if view.clients > 0 {
                            pill(ui, &format!("{} connected", view.clients), ON_ACCENT, ACCENT);
                        }
                    }
                    (None, Some(error)) => {
                        dot(ui, DANGER);
                        ui.label(error);
                    }
                    (None, None) => {
                        dot(ui, IDLE);
                        ui.label("WebSocket off");
                    }
                }
            });
            if let Some(pipe) = &view.pipe {
                ui.label(muted(format!("Or one message per line to the pipe {}", pipe.display())));
            }
            let mut apply = None;
            let port = self.screen.port.get_or_insert_with(|| s.settings.inputs_port.to_string());
            ui.horizontal(|ui| {
                ui.label("Port");
                ui.add(egui::TextEdit::singleline(port).desired_width(60.0));
                let parsed = port.trim().parse::<u16>().ok();
                if ui.add_enabled(parsed.is_some_and(|p| p != s.settings.inputs_port), egui::Button::new("Apply")).clicked() {
                    apply = parsed;
                }
                ui.label(muted("0 turns the WebSocket off"));
            });
            if let Some(port) = apply {
                self.send(Command::SetInputsPort(port));
            }
            if let Some(rejected) = &view.rejected {
                ui.label(RichText::new(format!("Last message refused: {rejected}")).color(DANGER_TEXT).size(12.0));
            }
            if !view.values.is_empty() || !view.events.is_empty() {
                ui.add_space(4.0);
                egui::Grid::new("custom-values").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                    for (name, value) in &view.values {
                        ui.label(RichText::new(format!("input.custom.{name}")).monospace());
                        ui.label(value);
                        ui.end_row();
                    }
                    for (t, name) in view.events.iter().rev() {
                        ui.label(RichText::new(format!("event {name}")).monospace());
                        ui.label(muted(format!("{:.0} s ago", s.time - t)));
                        ui.end_row();
                    }
                });
            }
            let Some(profile) = &s.profile else { return };
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Declared for {}", profile.game)).strong());
            ui.label(muted("Say what the program sends, so that an AI assistant writing a mode for this game can use it."));
            let mut changed = None;
            for (i, input) in profile.inputs.iter().enumerate() {
                ui.horizontal(|ui| {
                    let kind = match input.kind {
                        InputKind::Value => format!("input.custom.{}", input.name),
                        InputKind::Event => format!("event {}", input.name),
                    };
                    ui.label(RichText::new(kind).monospace());
                    ui.label(&input.description);
                    if ui.small_button("Delete").clicked() {
                        let mut p = profile.clone();
                        p.inputs.remove(i);
                        changed = Some(p);
                    }
                });
            }
            let new = &mut self.screen.new_input;
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("input-kind")
                    .selected_text(if new.kind == InputKind::Value { "Value" } else { "Event" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut new.kind, InputKind::Value, "Value");
                        ui.selectable_value(&mut new.kind, InputKind::Event, "Event");
                    });
                ui.add(egui::TextEdit::singleline(&mut new.name).hint_text("hp").desired_width(100.0));
                ui.add(egui::TextEdit::singleline(&mut new.description).hint_text("health, 0 to 100").desired_width(220.0));
                let ok = valid_name(&new.name) && !new.description.trim().is_empty() && !profile.inputs.iter().any(|i| i.name == new.name);
                if ui.add_enabled(ok, egui::Button::new("Declare")).clicked() {
                    let mut p = profile.clone();
                    p.inputs.push(std::mem::take(new));
                    changed = Some(p);
                }
            });
            if let Some(p) = changed {
                self.send(Command::SaveProfile(p));
            }
        });
    }
}
