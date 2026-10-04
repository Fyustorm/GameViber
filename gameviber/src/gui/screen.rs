//! Game page: what GameViber knows about the game being played. Its live
//! image, captures of its scenes (saved images, which are also the examples
//! scenes are recognized with), zones of its screen drawn on a capture with
//! zoom and checked on every capture, the scenes of the active mode, and the
//! values other programs send. Captures, zones and the declared inputs make
//! the game's profile (`profile.rs`).

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::{self, Color32, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind};

use super::audio::{model_card, scenes_card};
use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::mode::ZoneValue;
use crate::models::Model;
use crate::profile::{self, valid_name, Direction, InputDecl, InputKind, Profile, Zone, ZoneKind};
use crate::screen::{zones, Frame};

/// Images are never shown wider than this (before zooming).
const PREVIEW_WIDTH: f32 = 640.0;
const THUMB_WIDTH: f32 = 112.0;
const MAX_ZOOM: f32 = 8.0;

#[derive(Default)]
pub struct State {
    texture: Option<egui::TextureHandle>,
    /// Count of the frame in the texture.
    shown: Option<u32>,
    /// A new scene name typed for captures.
    new_scene: String,
    /// The game whose captures are loaded, and each capture: its image and texture.
    loaded_game: Option<String>,
    captures: HashMap<String, (Arc<Frame>, egui::TextureHandle)>,
    /// The capture zones are drawn on.
    selected: Option<String>,
    draft: Draft,
    new_input: InputDecl,
    port: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Full,
    Empty,
}

/// The zone being drawn on the selected capture.
struct Draft {
    zoom: f32,
    /// Corners of the rectangle, as fractions of the image.
    start: Option<Pos2>,
    end: Option<Pos2>,
    name: String,
    kind: ZoneKind,
    direction: Direction,
    /// The next click on the image picks this color of the bar.
    picking: Option<Pick>,
    full: Option<[u8; 3]>,
    empty: Option<[u8; 3]>,
    /// The bar moves: it is looked for in the zone.
    floating: bool,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            start: None,
            end: None,
            name: String::new(),
            kind: ZoneKind::Visible,
            direction: Direction::Right,
            picking: None,
            full: None,
            empty: None,
            floating: false,
        }
    }
}

impl Draft {
    fn rect(&self) -> Option<[f32; 4]> {
        let (a, b) = (self.start?, self.end?);
        let (x0, y0, x1, y1) = (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        (x1 - x0 > 0.002 && y1 - y0 > 0.002).then_some([x0, y0, x1 - x0, y1 - y0])
    }
}

fn color_image(frame: &Frame) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([frame.width as usize, frame.height as usize], &frame.pixels)
}

fn image_size(width: f32, frame: &Frame) -> egui::Vec2 {
    egui::vec2(width, width * frame.height as f32 / frame.width.max(1) as f32)
}

/// A zone's rectangle on an image drawn in `area`.
fn on_image(area: Rect, rect: [f32; 4]) -> Rect {
    Rect::from_min_size(
        area.min + egui::vec2(rect[0] * area.width(), rect[1] * area.height()),
        egui::vec2(rect[2] * area.width(), rect[3] * area.height()),
    )
}

fn swatch(ui: &mut egui::Ui, color: Option<[u8; 3]>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
    match color {
        Some([r, g, b]) => ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(r, g, b)),
        None => ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, MUTED), StrokeKind::Inside),
    };
}

/// What a zone reads on a capture, as text.
fn reading(zone: &Zone, measure: f32) -> String {
    match zone.kind {
        ZoneKind::Visible => format!("{:.2}{}", measure, if measure >= zone.threshold { " shown" } else { "" }),
        ZoneKind::Bar => format!("{:.0}%", measure * 100.0),
    }
}

impl App {
    pub(super) fn screen_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.load_captures(ui.ctx(), s.profile.as_ref());
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Game");
                ui.label(muted(
                    "What GameViber knows about the game you play, for every mode. Capture a few images of each \
                     scene (a battle, exploring, a dialogue): they teach GameViber to recognize the scenes, and \
                     you draw the zones modes read on them. Captures are saved with the game's profile; nothing \
                     else of the image is kept, and nothing leaves your computer.",
                ));
                ui.add_space(8.0);
                self.screen_preview(ui, s);
                ui.add_space(8.0);
                self.captures_card(ui, s);
                ui.add_space(8.0);
                self.zones_card(ui, s);
                ui.add_space(8.0);
                if let Some(command) = model_card(ui, Model::Image, &s.screen.model) {
                    self.send(command);
                }
                ui.add_space(8.0);
                scenes_card(ui, &s.scenes, s.mode.info.as_ref().map(|i| i.name.as_str()));
                ui.add_space(8.0);
                self.inputs_card(ui, s);
            });
        });
    }

    /// Keeps the profile's captures in memory, with their textures.
    fn load_captures(&mut self, ctx: &egui::Context, profile: Option<&Profile>) {
        let st = &mut self.screen;
        let game = profile.map(|p| p.game.clone());
        if st.loaded_game != game {
            st.loaded_game = game;
            st.captures.clear();
            st.selected = None;
            st.draft = Draft::default();
        }
        let Some(profile) = profile else { return };
        st.captures.retain(|file, _| profile.captures.iter().any(|c| c.file == *file));
        for capture in &profile.captures {
            if st.captures.contains_key(&capture.file) {
                continue;
            }
            if let Some(frame) = profile::load_capture(&profile.game, &capture.file) {
                // Sharp pixels when zoomed in.
                let texture = ctx.load_texture(&capture.file, color_image(&frame), egui::TextureOptions::NEAREST);
                st.captures.insert(capture.file.clone(), (Arc::new(frame), texture));
            }
        }
        if st.selected.as_ref().is_some_and(|f| !st.captures.contains_key(f)) {
            st.selected = None;
        }
        if st.selected.is_none() {
            st.selected = profile.captures.iter().find(|c| st.captures.contains_key(&c.file)).map(|c| c.file.clone());
        }
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
                match &mut self.screen.texture {
                    Some(texture) => texture.set(color_image(frame), egui::TextureOptions::LINEAR),
                    None => {
                        self.screen.texture = Some(ui.ctx().load_texture("game-image", color_image(frame), egui::TextureOptions::LINEAR))
                    }
                }
                self.screen.shown = Some(frame.count);
            }
            if let Some(texture) = &self.screen.texture {
                let size = image_size(ui.available_width().min(PREVIEW_WIDTH), frame);
                let response = ui.add(egui::Image::new(texture).fit_to_exact_size(size).corner_radius(6.0));
                // The profile's zones over the image, lit while shown.
                for zone in s.profile.iter().flat_map(|p| &p.zones) {
                    let value = view.zones.iter().find(|(n, _, _)| *n == zone.name).and_then(|(_, _, v)| *v);
                    let (lit, label) = match value {
                        Some(ZoneValue::Visible(shown)) => (shown, zone.name.clone()),
                        Some(ZoneValue::Bar(fill)) => (true, format!("{} {:.0}%", zone.name, fill * 100.0)),
                        None => (false, zone.name.clone()),
                    };
                    let color = if lit { ACCENT } else { MUTED };
                    let r = on_image(response.rect, zone.rect);
                    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, color), StrokeKind::Outside);
                    ui.painter().text(r.left_top() - egui::vec2(0.0, 2.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(11.0), color);
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
            let Some(profile) = &s.profile else { return };
            ui.add_space(6.0);
            // Scenes to capture: the active mode's, then the profile's.
            let mut scenes: Vec<String> = s.mode.info.iter().flat_map(|i| i.scenes.iter().map(|sc| sc.name.clone())).collect();
            for scene in profile.scenes() {
                if !scenes.contains(&scene) {
                    scenes.push(scene);
                }
            }
            let current = s.capture_scene.clone();
            let label = |scene: &str| if scene.is_empty() { "To sort later".to_owned() } else { scene.to_owned() };
            let mut target = None;
            let mut capture = false;
            ui.horizontal(|ui| {
                ui.label("Captures go to");
                egui::ComboBox::from_id_salt("capture-scene").selected_text(label(&current)).show_ui(ui, |ui| {
                    for scene in std::iter::once(String::new()).chain(scenes.iter().cloned()) {
                        if ui.selectable_label(scene == current, label(&scene)).clicked() {
                            target = Some(scene);
                        }
                    }
                });
                let new = &mut self.screen.new_scene;
                ui.add(egui::TextEdit::singleline(new).desired_width(110.0).hint_text("a new scene"));
                if ui.add_enabled(valid_name(new.trim()), egui::Button::new("Use")).clicked() {
                    target = Some(new.trim().to_owned());
                    new.clear();
                }
                capture = ui.button("📸 Capture now").clicked();
            });
            ui.label(
                muted(format!(
                    "In game, hold {} on the gamepad to capture without leaving it (Keybindings page): the image \
                     keeps the game's gamepad prompts.",
                    crate::gamepad::combo_text(&s.settings.capture_combo)
                ))
                .size(12.0),
            );
            if let Some(scene) = target {
                self.send(Command::SetCaptureScene(scene));
            }
            if capture {
                self.send(Command::CaptureScene(current));
            }
        });
    }

    fn captures_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            eyebrow(ui, "Captures");
            let Some(profile) = &s.profile else {
                ui.label(muted("Captures belong to a game: start one with the in-game overlay."));
                return;
            };
            if profile.captures.is_empty() {
                ui.label(muted("None yet: capture a few images of each scene above."));
                return;
            }
            let analysed = profile.captures.iter().filter(|c| !c.embedding.is_empty()).count();
            if analysed < profile.captures.len() {
                let why = if Model::Image.ready() { "analysing..." } else { "download the image scene model below to use them for scenes" };
                ui.label(muted(format!("{analysed} of {} captures analysed: {why}", profile.captures.len())));
            }
            let mut delete = None;
            let mut moved = None;
            let mut scenes = profile.scenes();
            // Where captures can be filed: the active mode's scenes too.
            let mut targets: Vec<String> = s.mode.info.iter().flat_map(|i| i.scenes.iter().map(|sc| sc.name.clone())).collect();
            for scene in &scenes {
                if !targets.contains(scene) {
                    targets.push(scene.clone());
                }
            }
            if profile.captures.iter().any(|c| c.scene.is_empty()) {
                scenes.insert(0, String::new());
            }
            for scene in &scenes {
                let count = profile.captures.iter().filter(|c| c.scene == *scene).count();
                let title = if scene.is_empty() { format!("To sort ({count}): right-click to file them") } else { format!("{scene} ({count})") };
                ui.label(RichText::new(title).strong());
                ui.horizontal_wrapped(|ui| {
                    for capture in profile.captures.iter().filter(|c| c.scene == *scene) {
                        let Some((frame, texture)) = self.screen.captures.get(&capture.file) else { continue };
                        let selected = self.screen.selected.as_deref() == Some(capture.file.as_str());
                        let image = egui::Image::new(texture).fit_to_exact_size(image_size(THUMB_WIDTH, frame)).corner_radius(4.0);
                        let response = ui.add(egui::Button::image(image).selected(selected)).on_hover_text("Draw zones on this capture");
                        if response.clicked() {
                            self.screen.selected = Some(capture.file.clone());
                        }
                        response.context_menu(|ui| {
                            for other in targets.iter().filter(|o| *o != scene) {
                                if ui.button(format!("Move to {other}")).clicked() {
                                    moved = Some((capture.file.clone(), other.clone()));
                                }
                            }
                            if ui.button("Delete").clicked() {
                                delete = Some(capture.file.clone());
                            }
                        });
                    }
                });
            }
            ui.label(muted("Right-click a capture to delete it or move it to another scene.").size(12.0));
            if let Some(file) = delete {
                self.send(Command::DeleteCapture(file));
            }
            if let Some((file, scene)) = moved {
                self.send(Command::MoveCapture { file, scene });
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
                "Draw a zone around something the game shows only in some scenes (the battle interface) or \
                 around a bar (health). Modes read it as input.zones.<name>.",
            ));
            let mut changed: Option<Profile> = None;
            if !profile.zones.is_empty() {
                ui.add_space(4.0);
                for (i, zone) in profile.zones.iter().enumerate() {
                    if let Some(p) = self.zone_row(ui, s, profile, i, zone) {
                        changed = Some(p);
                    }
                    ui.separator();
                }
            }
            match self.screen.selected.clone().and_then(|f| self.screen.captures.get(&f).cloned().map(|c| (f, c))) {
                Some((file, (frame, texture))) => {
                    let scene = profile.captures.iter().find(|c| c.file == file).map(|c| c.scene.clone()).filter(|s| !s.is_empty());
                    if let Some(p) = self.zone_editor(ui, profile, &frame, &texture, scene) {
                        changed = Some(p);
                    }
                }
                None => {
                    ui.label(muted("Capture an image of the game first: zones are drawn on captures."));
                }
            }
            if let Some(p) = changed {
                self.send(Command::SaveProfile(p));
            }
        });
    }

    /// One zone: its live reading, what it reads on every capture, its settings.
    fn zone_row(&mut self, ui: &mut egui::Ui, s: &Shared, profile: &Profile, i: usize, zone: &Zone) -> Option<Profile> {
        let mut changed = None;
        // What the zone reads on each capture, by scene.
        let mut by_scene: Vec<(String, Vec<f32>)> = Vec::new();
        for capture in &profile.captures {
            let Some((frame, _)) = self.screen.captures.get(&capture.file) else { continue };
            let measure = zones::measure(zone, frame);
            match by_scene.iter_mut().find(|(sc, _)| *sc == capture.scene) {
                Some((_, values)) => values.push(measure),
                None => by_scene.push((capture.scene.clone(), vec![measure])),
            }
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(&zone.name).strong()).on_hover_text(zone.kind.label());
            ui.label(muted(if zone.floating { "Bar that moves" } else { zone.kind.label() }));
            if let Some((_, measure, _)) = s.screen.zones.iter().find(|(n, _, _)| *n == zone.name) {
                ui.label(format!("now: {}", reading(zone, *measure)));
            }
            if ui.small_button("Delete").clicked() {
                let mut p = profile.clone();
                p.zones.remove(i);
                changed = Some(p);
            }
        });
        for (scene, values) in &by_scene {
            let text = match zone.kind {
                ZoneKind::Visible => {
                    let shown = values.iter().filter(|v| **v >= zone.threshold).count();
                    format!("{scene}: shown on {shown} of {}", values.len())
                }
                ZoneKind::Bar => {
                    let (min, max) = values.iter().fold((1f32, 0f32), |(a, b), v| (a.min(*v), b.max(*v)));
                    format!("{scene}: {:.0}% to {:.0}%", min * 100.0, max * 100.0)
                }
            };
            let detail = values.iter().map(|v| reading(zone, *v)).collect::<Vec<_>>().join(", ");
            ui.label(muted(text)).on_hover_text(detail);
        }
        let mut edited = zone.clone();
        ui.horizontal(|ui| {
            let tuned = match zone.kind {
                ZoneKind::Visible => ui
                    .add(egui::Slider::new(&mut edited.threshold, 0.1..=0.95).text("threshold"))
                    .on_hover_text("Similarity with its look when drawn above which it counts as shown"),
                ZoneKind::Bar => ui
                    .add(egui::Slider::new(&mut edited.tolerance, 10.0..=150.0).text("color tolerance"))
                    .on_hover_text("How far from the full color a pixel may be"),
            };
            if tuned.drag_stopped() || (tuned.changed() && !tuned.dragged()) {
                changed = Some(Profile { zones: replaced(&profile.zones, i, edited.clone()), ..profile.clone() });
            }
            if let (ZoneKind::Visible, Some(scene)) = (zone.kind, &zone.scene) {
                let shown: Vec<f32> = by_scene.iter().filter(|(sc, _)| sc == scene).flat_map(|(_, v)| v.clone()).collect();
                let hidden: Vec<f32> = by_scene.iter().filter(|(sc, _)| sc != scene).flat_map(|(_, v)| v.clone()).collect();
                match zones::suggest_threshold(&shown, &hidden) {
                    Some(t) if (t - zone.threshold).abs() > 0.01 => {
                        if ui.button(format!("Use {t:.2}")).on_hover_text(format!("Tells {scene} captures from the others")).clicked() {
                            let zone = Zone { threshold: t, ..zone.clone() };
                            changed = Some(Profile { zones: replaced(&profile.zones, i, zone), ..profile.clone() });
                        }
                    }
                    Some(_) => {
                        ui.label(RichText::new(format!("✔ tells {scene} from the rest")).color(OK).size(12.0));
                    }
                    None if !hidden.is_empty() && !shown.is_empty() => {
                        ui.label(RichText::new("no threshold tells the scenes apart: draw it tighter, on fixed parts").color(WARN).size(12.0));
                    }
                    None => {}
                }
            }
        });
        changed
    }

    /// The selected capture, zoomable, to draw a new zone on. Returns the profile with the zone once saved.
    fn zone_editor(&mut self, ui: &mut egui::Ui, profile: &Profile, frame: &Frame, texture: &egui::TextureHandle, scene: Option<String>) -> Option<Profile> {
        let draft = &mut self.screen.draft;
        ui.add_space(4.0);
        ui.label(RichText::new(format!("New zone, on a {} capture", scene.as_deref().unwrap_or("?"))).strong());
        ui.horizontal(|ui| {
            ui.label("Zoom");
            ui.add(egui::Slider::new(&mut draft.zoom, 1.0..=MAX_ZOOM).logarithmic(true).suffix("x"));
            ui.label(muted("or Ctrl + wheel over the image").size(12.0));
        });
        // As large as the window allows.
        let max_height = ui.ctx().content_rect().height() * 0.75;
        let fit = image_size(ui.available_width(), frame);
        let base = if fit.y > max_height { fit.x * max_height / fit.y } else { fit.x };
        let size = image_size(base * draft.zoom, frame);
        let mut save = None;
        egui::ScrollArea::both().id_salt("zone-editor").max_height(max_height + 16.0).show(ui, |ui| {
            let (area, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            ui.painter().image(texture.id(), area, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            if response.hovered() {
                // egui turns Ctrl + wheel (and pinches) into a zoom factor.
                let zoom = ui.input(|i| i.zoom_delta());
                if zoom != 1.0 {
                    draft.zoom = (draft.zoom * zoom).clamp(1.0, MAX_ZOOM);
                }
            }
            let to_fraction = |p: Pos2| Pos2::new(((p.x - area.left()) / area.width()).clamp(0.0, 1.0), ((p.y - area.top()) / area.height()).clamp(0.0, 1.0));
            if let Some(pick) = draft.picking {
                if response.clicked() {
                    if let Some(p) = response.interact_pointer_pos().map(to_fraction) {
                        let color = zones::pick_color(frame, p.x, p.y);
                        match pick {
                            Pick::Full => draft.full = Some(color),
                            Pick::Empty => draft.empty = Some(color),
                        }
                    }
                    draft.picking = None;
                }
            } else {
                if response.drag_started() {
                    draft.start = response.interact_pointer_pos().map(to_fraction);
                    draft.end = draft.start;
                }
                if response.dragged() {
                    draft.end = response.interact_pointer_pos().map(to_fraction);
                }
            }
            // The existing zones, with what they read on this capture.
            for zone in &profile.zones {
                let r = on_image(area, zone.rect);
                ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, MUTED), StrokeKind::Outside);
                let label = format!("{} {}", zone.name, reading(zone, zones::measure(zone, frame)));
                ui.painter().text(r.left_top() - egui::vec2(0.0, 2.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(11.0), MUTED);
            }
            if let Some(rect) = draft.rect() {
                ui.painter().rect_stroke(on_image(area, rect), 1.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut draft.name).hint_text("battle_hud").desired_width(130.0));
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
        if draft.kind == ZoneKind::Bar {
            ui.horizontal(|ui| {
                for (pick, label, color) in [(Pick::Full, "Full color", draft.full), (Pick::Empty, "Empty color", draft.empty)] {
                    swatch(ui, color);
                    let picking = draft.picking == Some(pick);
                    if ui.selectable_label(picking, format!("🖊 {label}")).on_hover_text("Then click on the image").clicked() {
                        draft.picking = if picking { None } else { Some(pick) };
                    }
                }
                ui.label(muted("Pick them on a part of the bar that is full, and one that is empty.").size(12.0));
            });
            ui.checkbox(&mut draft.floating, "The bar moves").on_hover_text(
                "Draw the zone over every place the bar can be: the bar is then found in it as the longest run of \
                 its two colors (needs the empty color)",
            );
        }
        let problem = if draft.kind == ZoneKind::Bar && draft.floating && draft.empty.is_none() {
            Some("A bar that moves needs its empty color.")
        } else if draft.rect().is_none() {
            Some(if draft.kind == ZoneKind::Bar { "Drag a rectangle along the bar, from its empty end to its full end." } else { "Drag a rectangle around fixed parts of the element (not text that changes)." })
        } else if !valid_name(&draft.name) {
            Some("Name: letters, digits and _, starting with a letter.")
        } else if profile.zones.iter().any(|z| z.name == draft.name) {
            Some("A zone has this name already.")
        } else {
            None
        };
        ui.horizontal(|ui| {
            if ui.add_enabled(problem.is_none(), primary("Save the zone")).clicked() {
                if let Some(rect) = draft.rect() {
                    let mut zone = Zone { name: draft.name.clone(), kind: draft.kind, rect, direction: draft.direction, scene: scene.clone(), ..Zone::default() };
                    match zone.kind {
                        ZoneKind::Visible => zone.reference = zones::reference(frame, rect),
                        ZoneKind::Bar => {
                            zone.color = draft.full.unwrap_or_else(|| zones::bar_color(frame, rect));
                            zone.empty_color = draft.empty;
                            zone.floating = draft.floating;
                        }
                    }
                    let mut p = profile.clone();
                    p.zones.push(zone);
                    save = Some(p);
                }
            }
            if let Some(problem) = problem {
                ui.label(muted(problem));
            }
        });
        if save.is_some() {
            let zoom = draft.zoom;
            *draft = Draft { zoom, ..Draft::default() };
        }
        save
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

fn replaced(zones: &[Zone], i: usize, zone: Zone) -> Vec<Zone> {
    let mut zones = zones.to_vec();
    zones[i] = zone;
    zones
}
