//! A game's captures and zones (Games › game › Signals › Captures and
//! zones), in two columns: on the left its captures (saved images of its
//! scenes, which are also the examples scenes are recognized with), filtered
//! by scene, each with what the zone selected reads on it, and how to add
//! some; on the right the zone editor — the game's zones, the places of the
//! one selected, the capture shown (zoomable) to draw them on, the zone's
//! settings (shared by its places) and the place's.
//!
//! A zone is picked first, then one of its places. Leaving a zone or a place
//! for another saves its changes; Save does it in place.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::game::{self, valid_name, Direction, Game, Zone, ZoneKind};
use crate::models::Model;
use crate::screen::{zones, Frame};

/// Images are never shown wider than this (before zooming).
const MAX_ZOOM: f32 = 8.0;
/// Width of the captures column.
const SIDE_WIDTH: f32 = 340.0;
/// The captures column's width in a narrow window.
const SIDE_MIN_WIDTH: f32 = 250.0;
/// Inner margin of the two columns.
const PANEL_MARGIN: f32 = 14.0;
/// Distance (in points) from a rectangle's edge that grabs the edge.
const HANDLE: f32 = 6.0;
/// Seconds "Saved" shows after Save.
const SAVED_SECS: f64 = 2.0;

pub struct State {
    /// The game whose captures are loaded, and each capture: its image and texture.
    loaded_game: Option<String>,
    captures: HashMap<String, (Arc<Frame>, egui::TextureHandle)>,
    /// The capture zones are drawn on.
    selected: Option<String>,
    /// The scene the captures shown belong to; None: all, "": to sort.
    filter: Option<String>,
    draft: Draft,
    /// The zoom the image was last shown with, the point of the image in the
    /// middle of the view, and where to scroll the image to next.
    last_zoom: f32,
    view_center: Pos2,
    scroll_to: Option<Vec2>,
    /// A zone was just opened: bring it into view.
    center_zone: bool,
    /// Which zones the image shows.
    show: Show,
    /// When Save was last pressed, and whether deleting the zone waits for confirmation.
    saved_at: f64,
    confirm_delete: bool,
    /// Heights taken last frame under the captures grid and under the image
    /// edited (which take what is left), and where the latter began.
    captures_below: f32,
    editor_below: f32,
    below_image_top: f32,
    pub(super) port: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            loaded_game: None,
            captures: HashMap::new(),
            selected: None,
            filter: None,
            draft: Draft::default(),
            last_zoom: 1.0,
            view_center: Pos2::new(0.5, 0.5),
            scroll_to: None,
            center_zone: false,
            show: Show::All,
            saved_at: f64::NEG_INFINITY,
            confirm_delete: false,
            captures_below: 0.0,
            editor_below: 0.0,
            below_image_top: 0.0,
            port: None,
        }
    }
}

/// Where the editor goes.
enum Target {
    NewZone,
    /// A zone, none of its places (one is then picked, or a new one drawn).
    Zone(String),
    /// A place, by index; true: shown on its capture.
    Place(usize, bool),
}

impl State {
    /// Opens a place (index `i`) on the capture shown (the screenshot tour).
    pub(super) fn edit_zone(&mut self, i: usize, game: &Game) {
        self.open_place(i, game, false);
    }

    /// Leaves what is edited, saving its changes (returned), for `to`.
    fn go(&mut self, to: Target, game: &Game) -> Option<Game> {
        let save = if self.dirty() { self.applied(game).map(|(g, _)| g) } else { None };
        let g = save.as_ref().unwrap_or(game);
        self.confirm_delete = false;
        match to {
            Target::NewZone => self.draft = Draft { zoom: self.draft.zoom, ..Draft::default() },
            Target::Zone(name) => self.select_zone(&name, g),
            Target::Place(i, on_capture) => self.open_place(i, g, on_capture),
        }
        save
    }

    /// A place, and its capture when `on_capture`: the one it was drawn on,
    /// else one it is found on (its scene's first).
    fn open_place(&mut self, i: usize, game: &Game, on_capture: bool) {
        let Some(zone) = game.zones.get(i) else { return };
        self.draft = Draft::edit(self.draft.zoom, i, zone);
        self.center_zone = true;
        if !on_capture {
            return;
        }
        if let Some(file) = zone.capture.as_ref().filter(|f| self.captures.contains_key(*f)) {
            self.selected = Some(file.clone());
            return;
        }
        let captures = &self.captures;
        let found = |file: &str| {
            captures.get(file).is_some_and(|(frame, _)| match zone.kind {
                ZoneKind::Bar => zones::fill_anywhere(&[zone], frame).is_some(),
                ZoneKind::Visible => zones::measure(zone, frame).is_some_and(|m| m >= zone.threshold),
            })
        };
        if self.selected.as_deref().is_some_and(found) {
            return;
        }
        let in_scene = |c: &&game::Capture| zone.scene.as_ref().is_none_or(|s| c.scene == *s);
        let best = game.captures.iter().filter(in_scene).find(|c| found(&c.file)).or_else(|| game.captures.iter().find(|c| found(&c.file)));
        if let Some(capture) = best {
            self.selected = Some(capture.file.clone());
        }
    }

    /// A zone, none of its places: its settings from its first place.
    fn select_zone(&mut self, name: &str, game: &Game) {
        match game.zones.iter().position(|z| z.name == name) {
            Some(i) => {
                let mut draft = Draft::edit(self.draft.zoom, i, &game.zones[i]);
                draft.editing = None;
                draft.start = None;
                draft.end = None;
                draft.mark_opened();
                self.draft = draft;
            }
            None => self.draft = Draft { zoom: self.draft.zoom, ..Draft::default() },
        }
    }

    /// The place the draft makes (edited or new), its look taken from the
    /// capture it was drawn on; None until a rectangle is drawn.
    fn draft_zone(&self, game: &Game) -> Option<Zone> {
        let d = &self.draft;
        let rect = d.rect()?;
        let first = d.zone.as_ref().and_then(|name| game.zones.iter().find(|z| z.name == *name));
        let base = d.editing.and_then(|i| game.zones.get(i)).or(first).cloned().unwrap_or_default();
        let drawn = d.drawn_on.as_ref().and_then(|f| self.captures.get(f)).map(|(frame, _)| frame);
        let scene = d.drawn_on.as_ref().and_then(|f| game.captures.iter().find(|c| c.file == *f)).map(|c| c.scene.clone());
        let mut zone = Zone {
            rect,
            threshold: d.threshold,
            scene: if drawn.is_some() { scene.filter(|s| !s.is_empty()) } else { base.scene.clone() },
            capture: if drawn.is_some() { d.drawn_on.clone() } else { base.capture.clone() },
            ..base.clone()
        };
        self.shared(&mut zone);
        match zone.kind {
            ZoneKind::Visible => {
                if let Some(frame) = drawn {
                    zone.reference = zones::reference(frame, rect);
                }
            }
            ZoneKind::Bar => {
                zone.reference.clear();
                if d.full.is_empty() {
                    zone.color = match drawn {
                        Some(frame) => zones::bar_color(frame, rect),
                        None => base.color,
                    };
                }
                if let Some(frame) = drawn {
                    zone.length = zones::bar_length(&zone, frame);
                }
            }
        }
        Some(zone)
    }

    /// The draft's settings shared by every place of its zone.
    fn shared(&self, zone: &mut Zone) {
        let d = &self.draft;
        zone.name = d.name.clone();
        zone.kind = d.kind;
        zone.direction = d.direction;
        zone.tolerance = d.tolerance;
        if d.kind == ZoneKind::Bar {
            if let Some(color) = d.full.first() {
                zone.color = *color;
            }
            zone.more_colors = d.full.iter().skip(1).copied().collect();
            zone.empty_color = d.empty.first().copied();
            zone.more_empty = d.empty.iter().skip(1).copied().collect();
        }
    }

    /// Why the draft cannot be saved.
    fn problem(&self, game: &Game) -> Option<&'static str> {
        let d = &self.draft;
        let placing = d.zone.is_none() || d.editing.is_some() || d.rect().is_some();
        if d.zone.is_none() && !d.kind_chosen {
            Some("Choose what the zone reads.")
        } else if placing && d.rect().is_none() {
            Some("Draw it on the image.")
        } else if d.kind == ZoneKind::Visible && d.editing.is_none() && d.rect().is_some() && d.drawn_on.is_none() {
            Some("Draw the rectangle on a capture that shows the element.")
        } else if !valid_name(&d.name) {
            Some("Name: letters, digits and _, starting with a letter.")
        } else if game.zones.iter().any(|z| z.name == d.name && Some(&z.name) != d.zone.as_ref()) {
            Some("Another zone has this name.")
        } else {
            None
        }
    }

    /// The game with the draft applied — the zone's settings on all its places
    /// (renamed with the scene it is a sign of), the place edited or added —
    /// and the place now edited; None when it cannot be saved.
    fn applied(&self, game: &Game) -> Option<(Game, Option<usize>)> {
        if self.problem(game).is_some() {
            return None;
        }
        let d = &self.draft;
        let mut g = game.clone();
        if let Some(old) = &d.zone {
            for zone in g.zones.iter_mut().filter(|z| z.name == *old) {
                self.shared(zone);
            }
            if *old != d.name {
                g.scenes.iter_mut().filter(|sc| sc.zone.as_ref() == Some(old)).for_each(|sc| sc.zone = Some(d.name.clone()));
            }
        }
        let place = match (d.editing, self.draft_zone(game)) {
            (Some(i), Some(zone)) if i < g.zones.len() => {
                g.zones[i] = zone;
                Some(i)
            }
            (None, Some(zone)) => {
                g.zones.push(zone);
                Some(g.zones.len() - 1)
            }
            _ => None,
        };
        Some((g, place))
    }

    /// The draft changed since it was opened (whether it can be saved or not).
    fn dirty(&self) -> bool {
        self.draft.changed()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Show {
    All,
    Zone,
    Nothing,
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Full,
    Empty,
}

/// What dragging on the image does.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    /// Draws a new rectangle.
    New,
    /// Moves the rectangle, from where the drag started and the rectangle then.
    Move { from: Pos2, rect: [f32; 4] },
    /// Moves its left, right, top and bottom edges.
    Resize([bool; 4]),
}

impl Grab {
    /// What dragging from `p` does to the rectangle drawn at `r` (both on screen).
    fn at(r: Rect, p: Pos2) -> Self {
        // Edges are grabbed a little outside, and inside only as far as leaves room to move thin bars.
        let (inner_x, inner_y) = (HANDLE.min(r.width() / 4.0), HANDLE.min(r.height() / 4.0));
        let within_x = p.x >= r.left() - HANDLE && p.x <= r.right() + HANDLE;
        let within_y = p.y >= r.top() - HANDLE && p.y <= r.bottom() + HANDLE;
        let mut left = within_y && p.x >= r.left() - HANDLE && p.x <= r.left() + inner_x;
        let mut right = within_y && p.x <= r.right() + HANDLE && p.x >= r.right() - inner_x;
        let mut top = within_x && p.y >= r.top() - HANDLE && p.y <= r.top() + inner_y;
        let mut bottom = within_x && p.y <= r.bottom() + HANDLE && p.y >= r.bottom() - inner_y;
        if left && right {
            let closer_left = (p.x - r.left()).abs() < (p.x - r.right()).abs();
            (left, right) = (closer_left, !closer_left);
        }
        if top && bottom {
            let closer_top = (p.y - r.top()).abs() < (p.y - r.bottom()).abs();
            (top, bottom) = (closer_top, !closer_top);
        }
        if left || right || top || bottom {
            Grab::Resize([left, right, top, bottom])
        } else if r.contains(p) {
            Grab::Move { from: Pos2::ZERO, rect: [0.0; 4] }
        } else {
            Grab::New
        }
    }

    fn cursor(self) -> egui::CursorIcon {
        match self {
            Grab::New => egui::CursorIcon::Crosshair,
            Grab::Move { .. } => egui::CursorIcon::Move,
            Grab::Resize([l, r, t, b]) => match (l || r, t || b) {
                (true, false) => egui::CursorIcon::ResizeHorizontal,
                (false, _) => egui::CursorIcon::ResizeVertical,
                _ if (l && t) || (r && b) => egui::CursorIcon::ResizeNwSe,
                _ => egui::CursorIcon::ResizeNeSw,
            },
        }
    }
}


/// The zone and place being edited: the zone's settings (shared by its
/// places) and the place's rectangle and threshold.
#[derive(Clone)]
struct Draft {
    zoom: f32,
    /// The zone selected, by its saved name; None for a new zone.
    zone: Option<String>,
    /// A new zone: what it reads was chosen.
    kind_chosen: bool,
    /// Index of the place edited; None for a new place (or none yet).
    editing: Option<usize>,
    /// The capture the rectangle was drawn on: the place's look (and a bar's
    /// length) is taken from it. None while editing without redrawing.
    drawn_on: Option<String>,
    threshold: f32,
    tolerance: f32,
    /// Corners of the rectangle, as fractions of the image.
    start: Option<Pos2>,
    end: Option<Pos2>,
    /// The drag going on.
    grab: Option<Grab>,
    name: String,
    kind: ZoneKind,
    direction: Direction,
    /// The next click on the image picks this color of the bar.
    picking: Option<Pick>,
    /// The bar's full and empty colors (several shades when it blinks); no
    /// full color: taken from the capture.
    full: Vec<[u8; 3]>,
    empty: Vec<[u8; 3]>,
    /// The draft as it was opened, to tell whether it changed.
    opened: Option<Box<Draft>>,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            zone: None,
            kind_chosen: false,
            editing: None,
            drawn_on: None,
            threshold: Zone::default().threshold,
            tolerance: Zone::default().tolerance,
            start: None,
            end: None,
            grab: None,
            name: String::new(),
            kind: ZoneKind::Visible,
            direction: Direction::Right,
            picking: None,
            full: Vec::new(),
            empty: Vec::new(),
            opened: None,
        }
    }
}

impl Draft {
    /// What it is now is what it was opened as.
    fn mark_opened(&mut self) {
        self.opened = None;
        self.opened = Some(Box::new(self.clone()));
    }

    /// Something saved would change since it was opened (a new zone or place: once drawn).
    fn changed(&self) -> bool {
        let Some(o) = &self.opened else { return self.rect().is_some() };
        self.name != o.name
            || self.direction != o.direction
            || self.tolerance != o.tolerance
            || self.full != o.full
            || self.empty != o.empty
            || self.threshold != o.threshold
            || self.start != o.start
            || self.end != o.end
            || self.drawn_on.is_some()
    }

    /// A draft to edit the place `zone` (index `i`), keeping the zoom.
    fn edit(zoom: f32, i: usize, zone: &Zone) -> Self {
        let mut draft = Self { zoom, zone: Some(zone.name.clone()), kind_chosen: true, ..Self::default() };
        draft.take_zone(zone);
        draft.take_place(i, zone);
        draft.mark_opened();
        draft
    }

    /// The zone's settings, from one of its places.
    fn take_zone(&mut self, zone: &Zone) {
        self.name = zone.name.clone();
        self.kind = zone.kind;
        self.direction = zone.direction;
        self.tolerance = zone.tolerance;
        self.full = if zone.kind == ZoneKind::Bar { std::iter::once(zone.color).chain(zone.more_colors.iter().copied()).collect() } else { Vec::new() };
        self.empty = zone.empty_color.into_iter().chain(zone.more_empty.iter().copied()).collect();
    }

    /// The place's own settings.
    fn take_place(&mut self, i: usize, zone: &Zone) {
        let [x, y, w, h] = zone.rect;
        self.editing = Some(i);
        self.drawn_on = None;
        self.threshold = zone.threshold;
        self.start = Some(Pos2::new(x, y));
        self.end = Some(Pos2::new(x + w, y + h));
        self.grab = None;
        self.picking = None;
        // The place as it is saved is where its changes count from.
        if let Some(o) = &mut self.opened {
            (o.editing, o.start, o.end, o.threshold, o.drawn_on) = (self.editing, self.start, self.end, self.threshold, None);
        }
    }

    /// Moves the rectangle by `delta` (fractions of the image), inside it.
    fn nudge(&mut self, delta: Vec2) {
        let Some([x, y, w, h]) = self.rect() else { return };
        let dx = delta.x.clamp(-x, 1.0 - x - w);
        let dy = delta.y.clamp(-y, 1.0 - y - h);
        self.start = Some(Pos2::new(x + dx, y + dy));
        self.end = Some(Pos2::new(x + dx + w, y + dy + h));
    }

    fn rect(&self) -> Option<[f32; 4]> {
        let (a, b) = (self.start?, self.end?);
        let (x0, y0, x1, y1) = (a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        (x1 - x0 > 0.002 && y1 - y0 > 0.002).then_some([x0, y0, x1 - x0, y1 - y0])
    }

    /// Follows a drag to `p` (a fraction of the image).
    fn drag_to(&mut self, p: Pos2) {
        match self.grab {
            Some(Grab::New) => self.end = Some(p),
            Some(Grab::Move { from, rect: [x, y, w, h] }) => {
                let dx = (p.x - from.x).clamp(-x, 1.0 - x - w);
                let dy = (p.y - from.y).clamp(-y, 1.0 - y - h);
                self.start = Some(Pos2::new(x + dx, y + dy));
                self.end = Some(Pos2::new(x + dx + w, y + dy + h));
            }
            Some(Grab::Resize([left, right, top, bottom])) => {
                let (Some(start), Some(end)) = (&mut self.start, &mut self.end) else { return };
                if left {
                    start.x = p.x;
                }
                if right {
                    end.x = p.x;
                }
                if top {
                    start.y = p.y;
                }
                if bottom {
                    end.y = p.y;
                }
            }
            None => {}
        }
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

/// Keeps the height laid out since `top` for the next frame, repainting when it changed.
fn remember_height(ui: &egui::Ui, height: &mut f32, top: f32) {
    let used = ui.cursor().top() - top;
    if (used - *height).abs() > 0.5 {
        *height = used;
        ui.ctx().request_repaint();
    }
}

/// A color swatch to click.
fn color_button(ui: &mut egui::Ui, [r, g, b]: [u8; 3]) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
    ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(r, g, b));
    if response.hovered() {
        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.5, TEXT), StrokeKind::Outside);
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A label on a dark background, drawn over the image.
fn tag(ui: &egui::Ui, at: Pos2, align: egui::Align2, text: String, color: Color32) {
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), color);
    let rect = align.anchor_size(at, galley.size() + Vec2::new(10.0, 4.0));
    painter.rect_filled(rect, 4.0, BG.gamma_multiply(0.9));
    painter.galley(rect.min + Vec2::new(5.0, 2.0), galley, color);
}

/// What a zone drawn in `places` reads on an image, as text and color.
fn reading_places(places: &[&Zone], frame: &Frame) -> (String, Color32) {
    let Some(first) = places.first() else { return (String::new(), MUTED) };
    match first.kind {
        ZoneKind::Visible => {
            let (shown, best) = zones::shown_anywhere(places, frame, false);
            if shown { (format!("shown {best:.2}"), ACCENT_TEXT) } else { (format!("hidden {best:.2}"), MUTED) }
        }
        ZoneKind::Bar => reading(first, zones::fill_anywhere(places, frame)),
    }
}

/// What a zone reads on an image, as text and color.
fn reading(zone: &Zone, measure: Option<f32>) -> (String, Color32) {
    match (zone.kind, measure) {
        (ZoneKind::Visible, Some(m)) if m >= zone.threshold => (format!("shown {m:.2}"), ACCENT_TEXT),
        (ZoneKind::Visible, m) => (format!("hidden {:.2}", m.unwrap_or(0.0)), MUTED),
        (ZoneKind::Bar, Some(m)) => (format!("{:.0}%", m * 100.0), ACCENT_TEXT),
        (ZoneKind::Bar, None) => ("unknown".to_owned(), WARN),
    }
}

impl App {
    /// The page, fitting the window (the Games page does not scroll it): the
    /// image edited takes what the rest leaves.
    pub(super) fn screen_page(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        self.load_captures(ui.ctx(), Some(game));
        heading(ui, "Captures and zones");
        ui.label(muted("Zones are drawn on captures and checked on all of them. Captures stay on your computer."));
        ui.add_space(4.0);
        // What the zone selected reads on each capture: its places, the one edited as drawn now.
        let st = &self.screen;
        let d = &st.draft;
        let tested = st.draft_zone(game);
        let mut places: Vec<&Zone> =
            game.zones.iter().enumerate().filter(|(i, z)| d.zone.as_ref() == Some(&z.name) && Some(*i) != d.editing).map(|(_, z)| z).collect();
        places.extend(tested.as_ref());
        let readings: HashMap<String, (String, Color32)> = if places.is_empty() {
            HashMap::new()
        } else {
            st.captures.iter().map(|(file, (frame, _))| (file.clone(), reading_places(&places, frame))).collect()
        };
        // The captures its places were drawn on.
        let marked: HashSet<String> = game.zones.iter().filter(|z| d.zone.as_ref() == Some(&z.name)).filter_map(|z| z.capture.clone()).collect();
        let mut changed = None;
        let gap = 16.0;
        let side = (ui.available_width() * 0.3).clamp(SIDE_MIN_WIDTH, SIDE_WIDTH);
        let main = ui.available_width() - side - gap;
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            let layout = egui::Layout::top_down(egui::Align::Min);
            ui.allocate_ui_with_layout(Vec2::new(side, height), layout, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                self.captures_panel(ui, s, game, &readings, &marked);
            });
            ui.allocate_ui_with_layout(Vec2::new(main, height), layout, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                changed = self.zone_panel(ui, game);
            });
        });
        if let Some(p) = changed {
            self.send(Command::SaveGame(p));
        }
    }

    /// Keeps the game's captures in memory, with their textures.
    pub(super) fn load_captures(&mut self, ctx: &egui::Context, profile: Option<&Game>) {
        let st = &mut self.screen;
        let game = profile.map(|p| p.id.clone());
        if st.loaded_game != game {
            st.loaded_game = game;
            st.captures.clear();
            st.selected = None;
            st.filter = None;
            st.draft = Draft::default();
        }
        let Some(profile) = profile else { return };
        st.captures.retain(|file, _| profile.captures.iter().any(|c| c.file == *file));
        for capture in &profile.captures {
            if st.captures.contains_key(&capture.file) {
                continue;
            }
            if let Some(frame) = game::load_capture(&profile.id, &capture.file) {
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

    /// The left column: the captures by scene (scrolling if they must), then how to add some.
    fn captures_panel(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, readings: &HashMap<String, (String, Color32)>, marked: &HashSet<String>) {
        let inner = ui.available_size() - Vec2::splat(2.0 * PANEL_MARGIN + 2.0);
        card(PANEL).inner_margin(Margin::same(PANEL_MARGIN as i8)).show(ui, |ui| {
            ui.set_width(inner.x);
            ui.set_height(inner.y);
            ui.label(RichText::new("Captures").strong().size(16.0));
            if !game.captures.is_empty() {
                self.capture_filters(ui, game);
            }
            // The grid takes what the part below it took on the last frame leaves.
            let height = (ui.available_height() - self.screen.captures_below).max(60.0);
            if game.captures.is_empty() {
                ui.allocate_ui(Vec2::new(ui.available_width(), height), |ui| {
                    ui.label(muted("No capture yet: capture each scene a few times, in different places."));
                });
            } else {
                egui::ScrollArea::vertical().id_salt("captures").auto_shrink([false, false]).max_height(height).min_scrolled_height(height).show(ui, |ui| {
                    self.capture_grid(ui, game, readings, marked);
                });
            }
            let top = ui.cursor().top();
            let analysed = game.captures.iter().filter(|c| !c.embedding.is_empty()).count();
            if analysed < game.captures.len() {
                let why = if Model::Image.ready() { "analysing..." } else { "download the image model in Signals, step 2, to use them for scenes" };
                ui.label(muted(format!("{analysed} of {} analysed: {why}", game.captures.len())).size(11.5));
            }
            self.add_captures(ui, s, game);
            remember_height(ui, &mut self.screen.captures_below, top);
        });
    }

    /// Filters: all, each scene, to sort.
    fn capture_filters(&mut self, ui: &mut egui::Ui, game: &Game) {
        let mut filters: Vec<(Option<String>, String)> = vec![(None, format!("All · {}", game.captures.len()))];
        for scene in game.scenes() {
            let n = game.captures.iter().filter(|c| c.scene == scene).count();
            filters.push((Some(scene.clone()), format!("{scene} · {n}")));
        }
        let to_sort = game.captures.iter().filter(|c| c.scene.is_empty()).count();
        if to_sort > 0 {
            filters.push((Some(String::new()), format!("to sort · {to_sort}")));
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (filter, label) in filters {
                let on = self.screen.filter == filter;
                let text = if filter.as_deref() == Some("") { RichText::new(label).color(WARN) } else { RichText::new(label) };
                if ui.selectable_label(on, text).clicked() {
                    self.screen.filter = filter;
                }
            }
        });
    }

    /// The captures shown by the scene filter, three per row, each with what
    /// the zone selected reads on it (📍: a place of it was drawn there); a
    /// click opens one in the editor.
    fn capture_grid(&mut self, ui: &mut egui::Ui, game: &Game, readings: &HashMap<String, (String, Color32)>, marked: &HashSet<String>) {
        let shown: Vec<&game::Capture> =
            game.captures.iter().filter(|c| self.screen.filter.as_ref().is_none_or(|f| c.scene == *f)).collect();
        let targets = game.scenes();
        let mut delete = None;
        let mut moved = None;
        let gap = 6.0;
        let columns = 3;
        let width = ((ui.available_width() - gap * (columns as f32 - 1.0)) / columns as f32).floor();
        for row in shown.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for capture in row {
                    let Some((frame, texture)) = self.screen.captures.get(&capture.file) else { continue };
                    let selected = self.screen.selected.as_deref() == Some(capture.file.as_str());
                    ui.allocate_ui_with_layout(Vec2::new(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let response = thumbnail(ui, texture, image_size(width, frame), selected)
                            .on_hover_text("Open it in the editor; right-click to file it under another scene or delete it");
                        if marked.contains(&capture.file) {
                            tag(ui, response.rect.right_top() + Vec2::new(-3.0, 3.0), egui::Align2::RIGHT_TOP, "📍".to_owned(), ACCENT_TEXT);
                        }
                        if response.clicked() {
                            self.screen.selected = Some(capture.file.clone());
                        }
                        response.context_menu(|ui| {
                            for other in targets.iter().filter(|o| **o != capture.scene) {
                                if ui.button(format!("Move to {other}")).clicked() {
                                    moved = Some((capture.file.clone(), other.clone()));
                                }
                            }
                            if ui.button("Delete").clicked() {
                                delete = Some(capture.file.clone());
                            }
                        });
                        let (scene, color) = if capture.scene.is_empty() { ("to sort", WARN) } else { (capture.scene.as_str(), MUTED) };
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
                            ui.label(RichText::new(scene).color(color).size(11.5));
                            if let Some((text, color)) = readings.get(&capture.file) {
                                ui.label(RichText::new(text).color(*color).size(11.5).strong());
                            }
                        });
                    });
                }
            });
            ui.add_space(4.0);
        }
        if let Some(file) = delete {
            self.send(Command::DeleteCapture { game: game.id.clone(), file });
        }
        if let Some((file, scene)) = moved {
            self.send(Command::MoveCapture { game: game.id.clone(), file, scene });
        }
    }

    /// How captures are taken: in game with the combo, or from here, into a scene.
    fn add_captures(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let view = &s.screen;
        let playing = s.game.as_ref().is_some_and(|g| g.id == game.id);
        egui::Frame::new().fill(BG).corner_radius(CornerRadius::same(10)).inner_margin(Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Add captures").strong());
                if playing && view.frame.is_some() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(muted(format!("live · {:.0} images/s", view.rate)).size(11.5));
                        dot(ui, OK);
                    });
                }
            });
            if !playing {
                ui.label(muted(format!("Captures go to the game being played: play {} to capture it.", game.name)).size(12.5));
                if ui.add(primary("Play this game")).clicked() {
                    self.send(Command::SelectGame(Some(game.id.clone())));
                }
                return;
            }
            let scenes = game.scenes();
            let current = s.capture_scene.clone();
            let label = |scene: &str| if scene.is_empty() { "to sort later".to_owned() } else { scene.to_owned() };
            ui.label(muted(format!("Hold {} in game, or from here:", crate::gamepad::combo_text(&s.settings.capture_combo))).size(12.5))
                .on_hover_text("Captured in game, the image keeps the game's gamepad prompts");
            if view.frame.is_none() {
                let hint = if s.overlay_unavailable {
                    "Another GameViber holds the in-game overlay."
                } else if s.overlay_clients.is_empty() {
                    "Start the game with the in-game overlay (Setup › In-game overlay)."
                } else {
                    "Waiting for the game's image..."
                };
                ui.label(RichText::new(hint).color(WARN).size(12.0));
            }
            let mut target = None;
            let mut capture = false;
            ui.horizontal(|ui| {
                ui.label("Into");
                egui::ComboBox::from_id_salt("capture-scene").selected_text(label(&current)).width(110.0).show_ui(ui, |ui| {
                    for scene in std::iter::once(String::new()).chain(scenes.iter().cloned()) {
                        if ui.selectable_label(scene == current, label(&scene)).clicked() {
                            target = Some(scene);
                        }
                    }
                });
                capture = ui.add_enabled(view.frame.is_some(), egui::Button::new("📸 Capture")).clicked();
            });
            let mut on = s.settings.screen;
            if ui.checkbox(&mut on, "Modes see the game's image").on_hover_text("Scenes from the image, zones, flashes and motion").changed() {
                self.send(Command::SetScreen(on));
            }
            if let Some(scene) = target {
                self.send(Command::SetCaptureScene(scene));
            }
            if capture {
                self.send(Command::CaptureScene(current));
            }
        });
    }

    /// The right column: the zone editor, once there is a capture to draw on.
    fn zone_panel(&mut self, ui: &mut egui::Ui, game: &Game) -> Option<Game> {
        let inner = ui.available_size() - Vec2::splat(2.0 * PANEL_MARGIN + 2.0);
        let mut changed = None;
        card(PANEL).inner_margin(Margin::same(PANEL_MARGIN as i8)).show(ui, |ui| {
            ui.set_width(inner.x);
            ui.set_height(inner.y);
            match self.screen.selected.clone().and_then(|f| self.screen.captures.get(&f).cloned().map(|c| (f, c))) {
                Some((file, (frame, texture))) => {
                    changed = self.zone_editor(ui, game, &file, &frame, &texture);
                }
                None => {
                    ui.label(RichText::new("Zones").strong().size(16.0));
                    ui.label(muted(
                        "A zone is a part of the screen modes read: whether something is shown (the battle interface), \
                         or how full a bar is (health). They are drawn on captures: capture the game first (left).",
                    ));
                }
            }
            remember_height(ui, &mut self.screen.editor_below, self.screen.below_image_top);
        });
        changed
    }

    /// The zones, the places of the one selected, the capture shown to draw
    /// them on, the zone's and the place's settings. Returns the game to save.
    fn zone_editor(&mut self, ui: &mut egui::Ui, game: &Game, file: &str, frame: &Frame, texture: &egui::TextureHandle) -> Option<Game> {
        let now = ui.input(|i| i.time);
        let tested = self.screen.draft_zone(game);
        let problem = self.screen.problem(game);
        let dirty = self.screen.dirty();
        let mut target: Option<Target> = None;
        let mut save_now = false;
        let mut cancel = false;
        let mut delete_place = None;
        let mut delete_zone = false;
        let mut linked = None;
        let mut show_capture = None;
        let st = &mut self.screen;
        let draft = &mut st.draft;
        let zone_name = draft.zone.clone();
        let places: Vec<usize> = match &zone_name {
            Some(name) => game.zones.iter().enumerate().filter(|(_, z)| z.name == *name).map(|(i, _)| i).collect(),
            None => Vec::new(),
        };

        // The zones.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            ui.label(RichText::new("Zones").strong());
            let mut seen: Vec<&str> = Vec::new();
            for zone in &game.zones {
                if seen.contains(&zone.name.as_str()) {
                    continue;
                }
                seen.push(&zone.name);
                let n = game.zones.iter().filter(|z| z.name == zone.name).count();
                let kind = if zone.kind == ZoneKind::Bar { "bar" } else { "shown or not" };
                let on = zone_name.as_deref() == Some(zone.name.as_str());
                let text = if n > 1 { format!("{} · {kind} · {n}", zone.name) } else { format!("{} · {kind}", zone.name) };
                if ui.selectable_label(on, text).on_hover_text(format!("{n} place(s)")).clicked() && !on {
                    target = Some(Target::Zone(zone.name.clone()));
                }
            }
            if ui.selectable_label(zone_name.is_none(), RichText::new("+ New zone").color(ACCENT_TEXT)).clicked() && zone_name.is_some() {
                target = Some(Target::NewZone);
            }
        });
        // Its places, or what a new zone reads.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            match &zone_name {
                Some(name) => {
                    ui.label(RichText::new("Places").strong());
                    for (n, &i) in places.iter().enumerate() {
                        let on = draft.editing == Some(i);
                        let scene = game.zones[i].scene.as_deref().unwrap_or("any scene");
                        let hover = if on { "Leave this place" } else { "Edit this place, on its capture" };
                        if ui.selectable_label(on, format!("{} · {scene}", n + 1)).on_hover_text(hover).clicked() {
                            target = Some(if on { Target::Zone(name.clone()) } else { Target::Place(i, true) });
                        }
                    }
                    let hover = "Draw it again where, or as, the game also shows it: the health bar out of battles, \
                                 another menu. Its value comes from the place where it is found; shown when any place is.";
                    if ui.selectable_label(draft.editing.is_none(), "+ Place").on_hover_text(hover).clicked() && draft.editing.is_some() {
                        target = Some(Target::Zone(name.clone()));
                    }
                }
                None => {
                    ui.label(RichText::new("New zone, reads").strong());
                    for (kind, help) in [
                        (ZoneKind::Visible, "Whether an element is on screen: a battle menu, a dialogue box"),
                        (ZoneKind::Bar, "How full a bar is: health, mana, a timer"),
                    ] {
                        if ui.selectable_label(draft.kind_chosen && draft.kind == kind, kind.label()).on_hover_text(help).clicked() {
                            draft.kind = kind;
                            draft.kind_chosen = true;
                        }
                    }
                }
            }
        });
        // Which zones the image shows, and its zoom.
        ui.horizontal(|ui| {
            ui.label(muted("Show"));
            for (show, label, help) in [
                (Show::All, "All zones", "Every zone"),
                (Show::Zone, "This zone", "Only the places of the zone selected"),
                (Show::Nothing, "None", "Hide every zone, to see the image"),
            ] {
                ui.selectable_value(&mut st.show, show, label).on_hover_text(help);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add(egui::Slider::new(&mut draft.zoom, 1.0..=MAX_ZOOM).logarithmic(true).suffix("x").show_value(true))
                    .on_hover_text("Also Ctrl + wheel over the image");
                ui.label(muted("Zoom"));
            });
        });

        // The image, taking the height the settings below it (last frame) leave.
        let max_height = (ui.available_height() - st.editor_below - 12.0).max(120.0);
        let fit = image_size(ui.available_width(), frame);
        let base = if fit.y > max_height { fit.x * max_height / fit.y } else { fit.x };
        let size = image_size(base * draft.zoom, frame);
        let visible = Vec2::new(ui.available_width(), size.y.min(max_height));
        // Zoomed with the slider, or a zone opened: the zone (else what was in the middle) stays in the middle.
        if st.scroll_to.is_none() && ((draft.zoom - st.last_zoom).abs() > 1e-4 || st.center_zone) {
            let focus = draft.rect().map(|[x, y, w, h]| Pos2::new(x + w / 2.0, y + h / 2.0)).unwrap_or(st.view_center);
            st.scroll_to = Some(Vec2::new(focus.x * size.x, focus.y * size.y) - visible / 2.0);
        }
        st.center_zone = false;
        st.last_zoom = draft.zoom;
        let mut scroll = egui::ScrollArea::both()
            .id_salt("zone-editor")
            .min_scrolled_height(visible.y + 12.0)
            .max_height(visible.y + 12.0)
            .auto_shrink([false, true]);
        if let Some(offset) = st.scroll_to.take() {
            scroll = scroll.scroll_offset(offset.max(Vec2::ZERO));
        }
        let can_draw = draft.zone.is_some() || draft.kind_chosen;
        let output = scroll.show(ui, |ui| {
            // Centered when narrower than the column.
            let (outer, response) = ui.allocate_exact_size(Vec2::new(size.x.max(ui.available_width()), size.y), Sense::click_and_drag());
            let area = Rect::from_min_size(outer.min + Vec2::new((outer.width() - size.x) / 2.0, 0.0), size);
            ui.painter().image(texture.id(), area, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            let to_fraction = |p: Pos2| Pos2::new(((p.x - area.left()) / area.width()).clamp(0.0, 1.0), ((p.y - area.top()) / area.height()).clamp(0.0, 1.0));
            let hover = response.hover_pos();
            if hover.is_some() {
                // egui turns Ctrl + wheel (and pinches) into a zoom factor; the point under the pointer stays there.
                let zoom = ui.input(|i| i.zoom_delta());
                if zoom != 1.0 {
                    let new = (draft.zoom * zoom).clamp(1.0, MAX_ZOOM);
                    if let Some(p) = hover {
                        let f = to_fraction(p);
                        let new_size = image_size(base * new, frame);
                        st.scroll_to = Some(Vec2::new(f.x * new_size.x, f.y * new_size.y) - (p - ui.clip_rect().min));
                    }
                    draft.zoom = new;
                    st.last_zoom = new;
                }
            }
            // The place of the zone selected under a point, when none is being edited.
            let place_at = |p: Pos2| places.iter().copied().find(|&i| on_image(area, game.zones[i].rect).expand(3.0).contains(p));
            if let Some(pick) = draft.picking {
                if hover.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                if response.clicked() {
                    if let Some(p) = response.interact_pointer_pos().map(to_fraction) {
                        let color = zones::pick_color(frame, p.x, p.y);
                        match pick {
                            Pick::Full if !draft.full.contains(&color) => draft.full.push(color),
                            Pick::Empty if !draft.empty.contains(&color) => draft.empty.push(color),
                            _ => {}
                        }
                    }
                    draft.picking = None;
                }
            } else if can_draw {
                let grab_at = |draft: &Draft, p: Pos2| match draft.rect() {
                    Some(rect) => Grab::at(on_image(area, rect), p),
                    // On one of the zone's places: picks it and moves it.
                    None if draft.editing.is_none() && place_at(p).is_some() => Grab::Move { from: Pos2::ZERO, rect: [0.0; 4] },
                    None => Grab::New,
                };
                if let Some(p) = hover.filter(|_| draft.grab.is_none()) {
                    ui.ctx().set_cursor_icon(grab_at(draft, p).cursor());
                }
                if response.drag_started() {
                    // Where the button went down: egui only calls it a drag once the pointer moved a little.
                    if let Some(origin) = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos()) {
                        if draft.rect().is_none() && draft.editing.is_none() {
                            if let Some(i) = place_at(origin) {
                                draft.take_place(i, &game.zones[i]);
                            }
                        }
                        let mut grab = grab_at(draft, origin);
                        if let Some([x, y, w, h]) = draft.rect() {
                            draft.start = Some(Pos2::new(x, y));
                            draft.end = Some(Pos2::new(x + w, y + h));
                        }
                        match &mut grab {
                            Grab::New => {
                                draft.start = Some(to_fraction(origin));
                                draft.end = draft.start;
                            }
                            Grab::Move { from, rect } => {
                                *from = to_fraction(origin);
                                *rect = draft.rect().unwrap_or_default();
                            }
                            Grab::Resize(_) => {}
                        }
                        draft.grab = Some(grab);
                        draft.drawn_on = Some(file.to_owned());
                    }
                }
                if let Some(grab) = draft.grab {
                    ui.ctx().set_cursor_icon(grab.cursor());
                    if let Some(p) = response.interact_pointer_pos().filter(|_| response.dragged()) {
                        draft.drag_to(to_fraction(p));
                    }
                }
                if response.drag_stopped() {
                    draft.grab = None;
                }
            }
            // A click on another zone's place, or one of this zone, opens it.
            if response.clicked() && draft.picking.is_none() {
                if let Some(p) = response.interact_pointer_pos() {
                    let hit = game
                        .zones
                        .iter()
                        .enumerate()
                        .filter(|(i, z)| draft.editing != Some(*i) && on_image(area, z.rect).expand(3.0).contains(p))
                        .min_by(|(_, a), (_, b)| (a.rect[2] * a.rect[3]).total_cmp(&(b.rect[2] * b.rect[3])))
                        .map(|(i, _)| i);
                    if let Some(i) = hit {
                        target = Some(Target::Place(i, false));
                    }
                }
            }
            // The other places; their name and reading when hovered.
            for (i, zone) in game.zones.iter().enumerate() {
                let same = zone_name.as_deref() == Some(zone.name.as_str());
                let shown = match st.show {
                    Show::All => true,
                    Show::Zone => same,
                    Show::Nothing => false,
                };
                if draft.editing == Some(i) || !shown {
                    continue;
                }
                let r = on_image(area, zone.rect);
                let hovered = hover.is_some_and(|p| r.expand(3.0).contains(p)) && draft.grab.is_none();
                // The zone's places stand out.
                let stroke = match (hovered, same) {
                    (true, _) => Stroke::new(1.5, TEXT),
                    (false, true) => Stroke::new(1.5, ACCENT.gamma_multiply(0.8)),
                    (false, false) => Stroke::new(1.0, MUTED.gamma_multiply(0.7)),
                };
                ui.painter().rect_stroke(r, 2.0, stroke, StrokeKind::Outside);
                if hovered {
                    let (text, color) = reading(zone, zones::measure(zone, frame));
                    tag(ui, r.left_top() - Vec2::new(0.0, 4.0), egui::Align2::LEFT_BOTTOM, format!("{} · {text} · click to edit", zone.name), color);
                }
            }
            if let Some(rect) = draft.rect().filter(|_| st.show != Show::Nothing) {
                let r = on_image(area, rect);
                ui.painter().rect_stroke(r, 1.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
                // Handles at the corners.
                for corner in [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()] {
                    ui.painter().rect_filled(Rect::from_center_size(corner, Vec2::splat(5.0)), 1.0, ACCENT);
                }
                if let Some(zone) = &tested {
                    let (text, color) = reading(zone, zones::measure(zone, frame));
                    let label = if draft.name.is_empty() { text } else { format!("{} · {text}", draft.name) };
                    tag(ui, r.left_top() - Vec2::new(0.0, 6.0), egui::Align2::LEFT_BOTTOM, label, color);
                    // The bar as found: its full part green, its empty part red, along the zone.
                    if zone.kind == ZoneKind::Bar && zone.empty_color.is_some() {
                        let ((a, b), (c, d)) = zones::bar_extent(zone, frame);
                        let horizontal = matches!(zone.direction, Direction::Right | Direction::Left);
                        let segment = |from: f32, to: f32| {
                            if horizontal {
                                Rect::from_min_max(Pos2::new(r.left() + from * r.width(), r.bottom() + 3.0), Pos2::new(r.left() + to * r.width(), r.bottom() + 7.0))
                            } else {
                                Rect::from_min_max(Pos2::new(r.right() + 3.0, r.top() + from * r.height()), Pos2::new(r.right() + 7.0, r.top() + to * r.height()))
                            }
                        };
                        ui.painter().rect_filled(segment(a, b), 0.0, OK);
                        ui.painter().rect_filled(segment(c, d), 0.0, DANGER);
                    }
                }
            }
        });
        st.below_image_top = ui.cursor().top();
        let content = output.content_size;
        if content.x > 0.0 && content.y > 0.0 {
            let center = output.state.offset + output.inner_rect.size() / 2.0;
            st.view_center = Pos2::new((center.x / content.x).clamp(0.0, 1.0), (center.y / content.y).clamp(0.0, 1.0));
        }

        // Keys, while nothing is typed: Esc leaves (the color picking, the
        // place, the zone), Delete deletes the place, arrows nudge it.
        if ui.ctx().memory(|m| m.focused().is_none()) {
            let (escape, delete, step) = ui.input(|i| {
                let px = if i.modifiers.shift { 10.0 } else { 1.0 };
                let key = |k: egui::Key| if i.key_pressed(k) { px } else { 0.0 };
                let step = Vec2::new(key(egui::Key::ArrowRight) - key(egui::Key::ArrowLeft), key(egui::Key::ArrowDown) - key(egui::Key::ArrowUp));
                (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Delete), step)
            });
            if escape {
                if draft.picking.is_some() {
                    draft.picking = None;
                } else if let (Some(_), Some(name)) = (draft.editing, &zone_name) {
                    target = Some(Target::Zone(name.clone()));
                } else if zone_name.is_some() {
                    target = Some(Target::NewZone);
                }
            }
            if delete && places.len() > 1 {
                delete_place = draft.editing;
            }
            if step != Vec2::ZERO && draft.rect().is_some() {
                draft.nudge(Vec2::new(step.x / frame.width.max(1) as f32, step.y / frame.height.max(1) as f32));
                draft.drawn_on = Some(file.to_owned());
            }
        }

        let hint = match (draft.picking, draft.rect()) {
            (Some(Pick::Full), _) => "Click the bar's full part on the image (Esc: stop).",
            (Some(Pick::Empty), _) => "Click the bar's empty part on the image (Esc: stop).",
            (None, None) if !can_draw => "Choose what the new zone reads, above, then draw it on the image.",
            (None, None) if zone_name.is_some() => "Pick a place above, or click or drag it on the image; or draw a new place.",
            (None, None) if draft.kind == ZoneKind::Bar => "Drag a rectangle along the bar, from its empty end to its full end.",
            (None, None) => "Drag a rectangle around fixed parts of the element (not text that changes).",
            (None, Some(_)) => "Drag its edges to resize it, its inside to move it; arrow keys nudge it (Shift: 10 px). Esc leaves it.",
        };
        ui.label(muted(hint).size(12.0));

        // The zone's settings, shared by its places.
        if can_draw {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Zone").strong());
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut draft.name).hint_text("battle_menu").desired_width(130.0).font(egui::TextStyle::Monospace))
                    .on_hover_text("Renaming renames all its places, and keeps the scene it is a sign of");
                if draft.zone.is_some() {
                    ui.label(muted(draft.kind.label())).on_hover_text("Set when the zone was made: for another kind, make a new zone");
                }
                if draft.kind == ZoneKind::Bar {
                    egui::ComboBox::from_id_salt("zone-direction").selected_text(draft.direction.label()).show_ui(ui, |ui| {
                        for d in Direction::ALL {
                            ui.selectable_value(&mut draft.direction, d, d.label());
                        }
                    });
                }
                // The scene this zone is a sure sign of (saved right away).
                if let Some(name) = zone_name.as_ref().filter(|_| !game.scenes.is_empty()) {
                    let current = game.scenes.iter().find(|sc| sc.zone.as_ref() == Some(name)).map(|sc| sc.name.clone());
                    let mut chosen = current.clone();
                    ui.label("Sure sign of").on_hover_text(
                        "While this zone is shown (any of its places), GameViber is sure of the scene, right away (a \
                         battle menu: battle). How long the scene is kept once it hides is set with the scene, in Signals.",
                    );
                    egui::ComboBox::from_id_salt("zone-scene").selected_text(chosen.clone().unwrap_or_else(|| "no scene".to_owned())).show_ui(ui, |ui| {
                        ui.selectable_value(&mut chosen, None, "no scene");
                        for scene in &game.scenes {
                            ui.selectable_value(&mut chosen, Some(scene.name.clone()), &scene.name);
                        }
                    });
                    if chosen != current {
                        let mut g = game.clone();
                        for scene in &mut g.scenes {
                            if Some(&scene.name) == chosen.as_ref() {
                                scene.zone = Some(name.clone());
                            } else if scene.zone.as_ref() == Some(name) {
                                scene.zone = None;
                            }
                        }
                        linked = Some(g);
                    }
                }
            });
            if draft.kind == ZoneKind::Bar {
                ui.horizontal_wrapped(|ui| {
                    let mut remove = None;
                    for (pick, label) in [(Pick::Full, "Full"), (Pick::Empty, "Empty")] {
                        let colors = if pick == Pick::Full { &draft.full } else { &draft.empty };
                        ui.label(label);
                        if colors.is_empty() {
                            swatch(ui, None);
                        }
                        for (k, color) in colors.iter().enumerate() {
                            if color_button(ui, *color).on_hover_text("Click to remove this color").clicked() {
                                remove = Some((pick, k));
                            }
                        }
                        let picking = draft.picking == Some(pick);
                        let help = "Then click on the image. Pick several shades when the bar blinks or changes color. With both \
                                    colors the bar is found inside the zone (green and red under it): the zone may be larger than \
                                    the bar, and a bar gone from the screen reads unknown (nil).";
                        let text = if colors.is_empty() { "🖊 Pick" } else { "🖊 +" };
                        if ui.selectable_label(picking, text).on_hover_text(help).clicked() {
                            draft.picking = if picking { None } else { Some(pick) };
                        }
                        ui.add_space(8.0);
                    }
                    match remove {
                        Some((Pick::Full, k)) => {
                            draft.full.remove(k);
                        }
                        Some((Pick::Empty, k)) => {
                            draft.empty.remove(k);
                        }
                        None => {}
                    }
                    ui.add(egui::Slider::new(&mut draft.tolerance, 10.0..=150.0).text("tolerance")).on_hover_text("How far from the colors a pixel may be");
                });
                if draft.empty.is_empty() {
                    ui.label(RichText::new("Pick the empty color too: the bar is then found inside the zone, and reads unknown (nil) when not on screen.").color(WARN).size(12.0));
                }
            }
        }

        // The place's own settings.
        if draft.rect().is_some() {
            let saved = draft.editing.and_then(|i| game.zones.get(i));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Place").strong());
                if draft.kind == ZoneKind::Visible {
                    ui.add(egui::Slider::new(&mut draft.threshold, 0.1..=0.95).text("threshold"))
                        .on_hover_text("Similarity with its look (where it was drawn) above which it counts as shown");
                }
                match saved.and_then(|z| z.capture.as_ref()) {
                    Some(capture) if capture == file => {
                        ui.label(RichText::new("📍 drawn on this capture").color(OK).size(12.0));
                    }
                    Some(capture) if st.captures.contains_key(capture) && ui.small_button("Show the capture it was drawn on").clicked() => {
                        show_capture = Some(capture.clone());
                    }
                    _ => {}
                }
            });
            if draft.kind == ZoneKind::Visible {
                if draft.editing.is_some() && draft.drawn_on.is_some() {
                    ui.label(muted("Its look is taken again from this capture.").size(12.0));
                } else if saved.is_some_and(|z| zones::measure(z, frame).is_none_or(|m| m < z.threshold)) {
                    ui.label(
                        RichText::new("Hidden on this capture: moving it here would take its look from an image without it.").color(WARN).size(12.0),
                    );
                }
            }
            // A threshold telling the place's scene from the others, from all the captures.
            if let Some(zone) = tested.as_ref().filter(|z| z.kind == ZoneKind::Visible) {
                if let Some(scene) = &zone.scene {
                    let (mut shown_in, mut others) = (Vec::new(), Vec::new());
                    for capture in game.captures.iter().filter(|c| !c.scene.is_empty()) {
                        let Some((capture_frame, _)) = st.captures.get(&capture.file) else { continue };
                        let m = zones::measure(zone, capture_frame).unwrap_or(-1.0);
                        if capture.scene == *scene { shown_in.push(m) } else { others.push(m) }
                    }
                    ui.horizontal(|ui| match zones::suggest_threshold(&shown_in, &others) {
                        Some(t) if (t - draft.threshold).abs() > 0.01 => {
                            if ui.button(format!("Use threshold {t:.2}")).on_hover_text(format!("Tells the {scene} captures from the others")).clicked() {
                                draft.threshold = t;
                            }
                        }
                        Some(_) => {
                            ui.label(RichText::new(format!("✔ tells {scene} from the other scenes")).color(OK).size(12.0));
                        }
                        None if !shown_in.is_empty() && !others.is_empty() => {
                            ui.label(RichText::new("No threshold tells the scenes apart: draw it tighter, on fixed parts").color(WARN).size(12.0));
                        }
                        None => {}
                    });
                }
            }
        }

        // Where things stand, and the actions.
        ui.add_space(2.0);
        ui.horizontal_wrapped(|ui| {
            match problem {
                Some(problem) if dirty || draft.rect().is_some() => {
                    ui.label(RichText::new(problem).color(WARN).size(12.5));
                }
                _ if dirty => {
                    ui.label(RichText::new("Unsaved changes: saved when you leave it").color(WARN).size(12.5));
                }
                _ if now - st.saved_at < SAVED_SECS => {
                    ui.label(RichText::new("✔ Saved").color(OK).size(12.5));
                }
                _ => {}
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = match (&zone_name, draft.editing, draft.rect()) {
                    (None, ..) => "Save the zone",
                    (Some(_), None, Some(_)) => "Save the place",
                    _ => "Save",
                };
                if ui.add_enabled(dirty && problem.is_none(), primary(label)).clicked() {
                    save_now = true;
                }
                if dirty && ui.button("Cancel").on_hover_text("Forget the changes").clicked() {
                    cancel = true;
                }
                if let Some(name) = &zone_name {
                    if st.confirm_delete {
                        let really = egui::Button::new(RichText::new(format!("Delete {name} and its {} place(s)", places.len())).color(Color32::WHITE)).fill(DANGER);
                        if ui.add(really).clicked() {
                            delete_zone = true;
                        }
                        if ui.button("Keep it").clicked() {
                            st.confirm_delete = false;
                        }
                    } else if ui.button(RichText::new("Delete the zone").color(DANGER_TEXT)).clicked() {
                        st.confirm_delete = true;
                    }
                    if let Some(i) = draft.editing.filter(|_| places.len() > 1) {
                        if ui.button(RichText::new("Delete this place").color(DANGER_TEXT)).on_hover_text("Also the Delete key").clicked() {
                            delete_place = Some(i);
                        }
                    }
                }
            });
        });

        if let Some(capture) = show_capture {
            st.selected = Some(capture);
        }
        if let Some(i) = delete_place {
            let mut g = game.clone();
            g.zones.remove(i);
            if let Some(name) = &zone_name {
                st.select_zone(name, &g);
            }
            return Some(g);
        }
        if delete_zone {
            if let Some(name) = &zone_name {
                let mut g = game.clone();
                g.zones.retain(|z| z.name != *name);
                g.scenes.iter_mut().filter(|sc| sc.zone.as_ref() == Some(name)).for_each(|sc| sc.zone = None);
                st.draft = Draft { zoom: st.draft.zoom, ..Draft::default() };
                st.confirm_delete = false;
                return Some(g);
            }
        }
        if save_now {
            if let Some((g, place)) = st.applied(game) {
                let name = st.draft.name.clone();
                match place {
                    Some(i) => st.open_place(i, &g, false),
                    None => st.select_zone(&name, &g),
                }
                st.saved_at = now;
                return Some(g);
            }
        }
        if cancel {
            match (zone_name, st.draft.editing) {
                (Some(_), Some(i)) => st.open_place(i, game, false),
                (Some(name), None) => st.select_zone(&name, game),
                (None, _) => st.draft = Draft { zoom: st.draft.zoom, ..Draft::default() },
            }
            return None;
        }
        if let Some(target) = target {
            return st.go(target, game).or(linked);
        }
        linked
    }
}

/// A capture's thumbnail, outlined when selected.
fn thumbnail(ui: &mut egui::Ui, texture: &egui::TextureHandle, size: Vec2, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    egui::Image::new(texture).corner_radius(4.0).paint_at(ui, rect);
    let painter = ui.painter();
    if selected {
        painter.rect_stroke(rect, 4.0, Stroke::new(2.0, ACCENT), StrokeKind::Outside);
    } else if response.hovered() {
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, TEXT), StrokeKind::Outside);
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}
