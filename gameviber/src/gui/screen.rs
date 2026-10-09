//! The active mode's captures and indicators (the Creator's Captures &
//! indicators tab), in two columns: on the left its captures (saved images of its
//! phases, which are also the examples phases are recognized with), filtered
//! by phase, each with what the indicator selected reads on it, and how to add
//! some; on the right the indicator editor — the mode's indicators, the zones of the
//! one selected, the capture shown (zoomable) to draw them on, the indicator's
//! settings (shared by its zones) and the zone's.
//!
//! An indicator is picked first, then one of its zones. Leaving an indicator or a zone
//! for another saves its changes; Save does it in place.

use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc};

use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::package::{self, valid_name, Condition, Direction, IndicatorKind, Inputs, PhaseDef, Zone};
use crate::models::Model;
use crate::screen::{indicators, Frame};

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
    /// The mode whose captures are loaded (its package), and each capture: its image and texture.
    loaded_game: Option<std::path::PathBuf>,
    captures: HashMap<String, (Arc<Frame>, egui::TextureHandle)>,
    /// The capture indicators are drawn on.
    selected: Option<String>,
    /// The phase the captures shown belong to; None: all, "": to sort.
    filter: Option<String>,
    draft: Draft,
    /// The zoom the image was last shown with, the point of the image in the
    /// middle of the view, and where to scroll the image to next.
    last_zoom: f32,
    view_center: Pos2,
    scroll_to: Option<Vec2>,
    /// An indicator was just opened: bring it into view.
    center_zone: bool,
    /// Which indicators the image shows.
    show: Show,
    /// When Save was last pressed, and whether deleting the indicator waits for confirmation.
    saved_at: f64,
    confirm_delete: bool,
    /// Heights taken last frame under the captures grid and under the image
    /// edited (which take what is left), and where the latter began.
    captures_below: f32,
    editor_below: f32,
    below_image_top: f32,
    pub(super) port: Option<String>,
    /// Images being imported from files (their dialog open, or being read),
    /// and what the last import did (true: it went well); None: cancelled.
    import: Option<mpsc::Receiver<Option<(bool, String)>>>,
    import_message: Option<(bool, String)>,
    readings: Readings,
}

/// What the indicator selected reads on each capture, by capture file.
type ReadingMap = HashMap<String, (String, Color32)>;

/// What the indicator selected reads on each capture, worked out on another
/// thread when its zones or the captures change: reading a gauge on dozens
/// of captures takes too long to be done on every frame.
#[derive(Default)]
struct Readings {
    /// The zones, the mode's zones (for a gauge's conditions) and the
    /// captures `values` was read with...
    zones: Vec<Zone>,
    all: Vec<Zone>,
    files: Vec<String>,
    values: ReadingMap,
    /// ...and the reading under way, with its own.
    job: Option<(Vec<Zone>, Vec<Zone>, Vec<String>, mpsc::Receiver<ReadingMap>)>,
}

impl Readings {
    /// The readings of `zones` on `captures`, as last read: those read for
    /// zones or captures since changed stay until they are read again.
    fn get(&mut self, ctx: &egui::Context, zones: &[&Zone], all: &[Zone], captures: &HashMap<String, (Arc<Frame>, egui::TextureHandle)>) -> &ReadingMap {
        if let Some((_, _, _, rx)) = &self.job {
            match rx.try_recv() {
                Ok(values) => {
                    let (zones, all, files, _) = self.job.take().expect("a job");
                    (self.zones, self.all, self.files, self.values) = (zones, all, files, values);
                }
                Err(mpsc::TryRecvError::Disconnected) => self.job = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if zones.is_empty() {
            *self = Self::default();
            return &self.values;
        }
        let mut files: Vec<String> = captures.keys().cloned().collect();
        files.sort();
        let read = |z: &[Zone], f: &[String]| z.len() == zones.len() && z.iter().zip(zones).all(|(a, b)| a == *b) && f == files.as_slice();
        // One reading at a time: a change made while one is under way is read once it ends.
        if (read(&self.zones, &self.files) && self.all == all) || self.job.is_some() {
            return &self.values;
        }
        let owned: Vec<Zone> = zones.iter().map(|z| (*z).clone()).collect();
        let frames: Vec<(String, Arc<Frame>)> = captures.iter().map(|(file, (frame, _))| (file.clone(), frame.clone())).collect();
        let (tx, rx) = mpsc::channel();
        let (job_zones, job_all, ctx) = (owned.clone(), all.to_vec(), ctx.clone());
        std::thread::spawn(move || {
            let zones: Vec<&Zone> = job_zones.iter().collect();
            let values = frames.iter().map(|(file, frame)| (file.clone(), reading_zones(&zones, &job_all, frame))).collect();
            let _ = tx.send(values);
            ctx.request_repaint();
        });
        self.job = Some((owned, all.to_vec(), files, rx));
        &self.values
    }
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
            import: None,
            import_message: None,
            readings: Readings::default(),
        }
    }
}

/// Where the editor goes.
enum Target {
    NewIndicator,
    /// A new indicator an assistant proposed: its name and what it reads.
    Planned(String, IndicatorKind),
    /// An indicator, none of its zones (one is then picked, or a new one drawn).
    Indicator(String),
    /// A zone, by index; true: shown on its capture.
    Zone(usize, bool),
}

impl State {
    /// The texture of a capture loaded (`App::load_captures`).
    pub(super) fn capture_texture(&self, file: &str) -> Option<egui::TextureHandle> {
        self.captures.get(file).map(|(_, texture)| texture.clone())
    }
}

impl State {
    /// Opens a zone (index `i`) on the capture shown (the screenshot tour).
    pub(super) fn edit_zone(&mut self, i: usize, inputs: &Inputs) {
        self.open_zone(i, inputs, false);
    }

    /// Leaves what is edited, saving its changes (returned), for `to`.
    fn go(&mut self, to: Target, inputs: &Inputs) -> Option<Inputs> {
        let save = if self.dirty() { self.applied(inputs).map(|(g, _)| g) } else { None };
        let g = save.as_ref().unwrap_or(inputs);
        self.confirm_delete = false;
        match to {
            Target::NewIndicator => self.draft = Draft { zoom: self.draft.zoom, ..Draft::default() },
            Target::Planned(name, kind) => self.draft = Draft { zoom: self.draft.zoom, name, kind, kind_chosen: true, ..Draft::default() },
            Target::Indicator(name) => self.select_indicator(&name, g),
            Target::Zone(i, on_capture) => self.open_zone(i, g, on_capture),
        }
        save
    }

    /// A zone, and its capture when `on_capture`: the one it was drawn on,
    /// else one it is found on (its phase's first).
    fn open_zone(&mut self, i: usize, inputs: &Inputs, on_capture: bool) {
        let Some(zone) = inputs.zones.get(i) else { return };
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
                IndicatorKind::Gauge => indicators::fill_anywhere(&[zone], frame).is_some(),
                IndicatorKind::Visibility => indicators::measure(zone, frame).is_some_and(|m| m >= zone.threshold),
            })
        };
        if self.selected.as_deref().is_some_and(found) {
            return;
        }
        let in_phase = |c: &&package::Capture| zone.phase.as_ref().is_none_or(|s| c.phase == *s);
        let best = inputs.captures.iter().filter(in_phase).find(|c| found(&c.file)).or_else(|| inputs.captures.iter().find(|c| found(&c.file)));
        if let Some(capture) = best {
            self.selected = Some(capture.file.clone());
        }
    }

    /// An indicator, none of its zones: its settings from its first zone.
    fn select_indicator(&mut self, name: &str, inputs: &Inputs) {
        match inputs.zones.iter().position(|z| z.indicator == name) {
            Some(i) => {
                let mut draft = Draft::edit(self.draft.zoom, i, &inputs.zones[i]);
                draft.editing = None;
                draft.start = None;
                draft.end = None;
                draft.mark_opened();
                self.draft = draft;
            }
            None => self.draft = Draft { zoom: self.draft.zoom, ..Draft::default() },
        }
    }

    /// The zone the draft makes (edited or new), its look taken from the
    /// capture it was drawn on; None until a rectangle is drawn.
    fn draft_zone(&self, inputs: &Inputs) -> Option<Zone> {
        let d = &self.draft;
        let rect = d.rect()?;
        let first = d.indicator.as_ref().and_then(|name| inputs.zones.iter().find(|z| z.indicator == *name));
        let base = d.editing.and_then(|i| inputs.zones.get(i)).or(first).cloned().unwrap_or_default();
        let drawn = d.drawn_on.as_ref().and_then(|f| self.captures.get(f)).map(|(frame, _)| frame);
        let phase = d.drawn_on.as_ref().and_then(|f| inputs.captures.iter().find(|c| c.file == *f)).map(|c| c.phase.clone());
        let mut zone = Zone {
            rect,
            threshold: d.threshold,
            phase: if drawn.is_some() { phase.filter(|s| !s.is_empty()) } else { base.phase.clone() },
            capture: if drawn.is_some() { d.drawn_on.clone() } else { base.capture.clone() },
            ..base.clone()
        };
        self.shared(&mut zone);
        match zone.kind {
            IndicatorKind::Visibility => {
                if let Some(frame) = drawn {
                    zone.reference = indicators::reference(frame, rect);
                }
                zone.weights = match &d.weights {
                    Some((at, weights)) if at.iter().zip(rect).all(|(a, b)| (a - b).abs() < 1e-5) => weights.clone(),
                    _ => Vec::new(),
                };
                (zone.shown_on, zone.hidden_on) = (d.shown_on.clone(), d.hidden_on.clone());
            }
            IndicatorKind::Gauge => {
                zone.reference.clear();
                zone.weights.clear();
                (zone.shown_on, zone.hidden_on) = (Vec::new(), Vec::new());
                if d.full.is_empty() {
                    zone.color = match drawn {
                        Some(frame) => indicators::bar_color(frame, rect),
                        None => base.color,
                    };
                }
                match d.look.as_ref().filter(|l| d.by_look && l.fits(rect, zone.direction)) {
                    Some(look) => (zone.full_look, zone.empty_look) = (look.full.clone(), look.empty.clone()),
                    None => (zone.full_look, zone.empty_look) = (Vec::new(), Vec::new()),
                }
                if let Some(frame) = drawn {
                    zone.length = indicators::bar_length(&zone, frame);
                }
            }
        }
        Some(zone)
    }

    /// The draft's settings shared by every zone of its indicator.
    fn shared(&self, zone: &mut Zone) {
        let d = &self.draft;
        zone.indicator = d.name.clone();
        zone.kind = d.kind;
        zone.direction = d.direction;
        zone.tolerance = d.tolerance;
        if d.kind == IndicatorKind::Gauge {
            if let Some(color) = d.full.first() {
                zone.color = *color;
            }
            zone.more_colors = d.full.iter().skip(1).copied().collect();
            zone.empty_color = d.empty.first().copied();
            zone.more_empty = d.empty.iter().skip(1).copied().collect();
            zone.tiers = d.tiers.iter().filter(|t| !t.is_empty()).cloned().collect();
            zone.read_when = d.read_when.clone();
        }
    }

    /// Why the draft cannot be saved.
    fn problem(&self, inputs: &Inputs) -> Option<&'static str> {
        let d = &self.draft;
        let placing = d.indicator.is_none() || d.editing.is_some() || d.rect().is_some();
        if d.indicator.is_none() && !d.kind_chosen {
            Some("Choose what the indicator reads.")
        } else if placing && d.rect().is_none() {
            Some("Draw it on the image.")
        } else if d.kind == IndicatorKind::Visibility && d.editing.is_none() && d.rect().is_some() && d.drawn_on.is_none() {
            Some("Draw the rectangle on a capture that shows the element.")
        } else if d.kind == IndicatorKind::Gauge && d.by_look && placing && !d.rect().is_some_and(|r| d.look.as_ref().is_some_and(|l| l.fits(r, d.direction))) {
            Some("Take the bar's look full, on a capture where it is full.")
        } else if !valid_name(&d.name) {
            Some("Name: letters, digits and _, starting with a letter.")
        } else if inputs.zones.iter().any(|z| z.indicator == d.name && Some(&z.indicator) != d.indicator.as_ref()) {
            Some("Another indicator has this name.")
        } else {
            None
        }
    }

    /// The inputs with the draft applied — the indicator's settings on all its zones
    /// (renamed with the phase it is a sign of), the zone edited or added —
    /// and the zone now edited; None when it cannot be saved.
    fn applied(&self, inputs: &Inputs) -> Option<(Inputs, Option<usize>)> {
        if self.problem(inputs).is_some() {
            return None;
        }
        let d = &self.draft;
        let mut g = inputs.clone();
        if let Some(old) = &d.indicator {
            for zone in g.zones.iter_mut().filter(|z| z.indicator == *old) {
                self.shared(zone);
            }
            if *old != d.name {
                g.zones.iter_mut().flat_map(|z| z.read_when.iter_mut()).filter(|c| c.indicator == *old).for_each(|c| c.indicator = d.name.clone());
                g.phases.iter_mut().flat_map(|sc| sc.indicators.iter_mut()).filter(|z| *z == old).for_each(|z| *z = d.name.clone());
            }
        }
        let place = match (d.editing, self.draft_zone(inputs)) {
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
    Indicator,
    Nothing,
}

#[derive(Clone, Copy, PartialEq)]
enum Pick {
    Full,
    Empty,
    /// A color of the bar's tier after the first (`Draft::tiers` index).
    Tier(usize),
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


/// The indicator and zone being edited: the indicator's settings (shared by its
/// zones) and the zone's rectangle and threshold.
#[derive(Clone)]
struct Draft {
    zoom: f32,
    /// The indicator selected, by its saved name; None for a new indicator.
    indicator: Option<String>,
    /// A new indicator: what it reads was chosen.
    kind_chosen: bool,
    /// Index of the zone edited; None for a new zone (or none yet).
    editing: Option<usize>,
    /// The capture the rectangle was drawn on: the zone's look (and a bar's
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
    kind: IndicatorKind,
    direction: Direction,
    /// The next click on the image picks this color of the bar.
    picking: Option<Pick>,
    /// The bar's full and empty colors (several shades when it blinks); no
    /// full color: taken from the capture.
    full: Vec<[u8; 3]>,
    empty: Vec<[u8; 3]>,
    /// The colors of its next tiers (`Zone::tiers`; one being picked may have none yet).
    tiers: Vec<Vec<[u8; 3]>>,
    /// The visibility indicators it is read under (`Zone::read_when`).
    read_when: Vec<Condition>,
    /// The bar read by its look rather than its colors...
    by_look: bool,
    /// ...taken on captures.
    look: Option<Look>,
    /// A visibility zone's cells that tell whether it is shown (`Zone::weights`),
    /// learned from captures for this rectangle: once it changes, they must be learned again.
    weights: Option<([f32; 4], Vec<u8>)>,
    /// The captures marked as showing it, and as not (`Zone::shown_on`, `hidden_on`).
    shown_on: Vec<String>,
    hidden_on: Vec<String>,
    /// The draft as it was opened, to tell whether it changed.
    opened: Option<Box<Draft>>,
}

/// A bar's look (`Zone::full_look`, `empty_look`), and the rectangle and
/// axis it was taken with: once they change, it must be taken again.
#[derive(Clone, PartialEq)]
struct Look {
    rect: [f32; 4],
    horizontal: bool,
    full: Vec<[u8; 3]>,
    empty: Vec<Option<[u8; 3]>>,
}

impl Look {
    fn fits(&self, rect: [f32; 4], direction: Direction) -> bool {
        self.horizontal == horizontal(direction) && self.rect.iter().zip(rect).all(|(a, b)| (a - b).abs() < 1e-5)
    }
}

fn horizontal(direction: Direction) -> bool {
    matches!(direction, Direction::Right | Direction::Left)
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            indicator: None,
            kind_chosen: false,
            editing: None,
            drawn_on: None,
            threshold: Zone::default().threshold,
            tolerance: Zone::default().tolerance,
            start: None,
            end: None,
            grab: None,
            name: String::new(),
            kind: IndicatorKind::Visibility,
            direction: Direction::Right,
            picking: None,
            full: Vec::new(),
            empty: Vec::new(),
            tiers: Vec::new(),
            read_when: Vec::new(),
            by_look: false,
            look: None,
            weights: None,
            shown_on: Vec::new(),
            hidden_on: Vec::new(),
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

    /// Something saved would change since it was opened (a new indicator or zone: once drawn).
    fn changed(&self) -> bool {
        let Some(o) = &self.opened else { return self.rect().is_some() };
        self.name != o.name
            || self.direction != o.direction
            || self.tolerance != o.tolerance
            || self.full != o.full
            || self.empty != o.empty
            || self.tiers != o.tiers
            || self.read_when != o.read_when
            || self.by_look != o.by_look
            || self.look != o.look
            || self.weights != o.weights
            || self.shown_on != o.shown_on
            || self.hidden_on != o.hidden_on
            || self.threshold != o.threshold
            || self.start != o.start
            || self.end != o.end
            || self.drawn_on.is_some()
    }

    /// A draft to edit the zone `zone` (index `i`), keeping the zoom.
    fn edit(zoom: f32, i: usize, zone: &Zone) -> Self {
        let mut draft = Self { zoom, indicator: Some(zone.indicator.clone()), kind_chosen: true, ..Self::default() };
        draft.take_indicator(zone);
        draft.take_zone(i, zone);
        draft.mark_opened();
        draft
    }

    /// The indicator's settings, from one of its zones.
    fn take_indicator(&mut self, zone: &Zone) {
        self.name = zone.indicator.clone();
        self.kind = zone.kind;
        self.direction = zone.direction;
        self.tolerance = zone.tolerance;
        self.full = if zone.kind == IndicatorKind::Gauge { std::iter::once(zone.color).chain(zone.more_colors.iter().copied()).collect() } else { Vec::new() };
        self.empty = zone.empty_color.into_iter().chain(zone.more_empty.iter().copied()).collect();
        self.tiers = if zone.kind == IndicatorKind::Gauge { zone.tiers.clone() } else { Vec::new() };
        self.read_when = if zone.kind == IndicatorKind::Gauge { zone.read_when.clone() } else { Vec::new() };
    }

    /// The zone's own settings.
    fn take_zone(&mut self, i: usize, zone: &Zone) {
        let [x, y, w, h] = zone.rect;
        self.editing = Some(i);
        self.drawn_on = None;
        self.threshold = zone.threshold;
        self.start = Some(Pos2::new(x, y));
        self.end = Some(Pos2::new(x + w, y + h));
        self.grab = None;
        self.picking = None;
        self.by_look = indicators::has_look(zone);
        self.look = self.by_look.then(|| Look {
            rect: zone.rect,
            horizontal: horizontal(zone.direction),
            full: zone.full_look.clone(),
            empty: zone.empty_look.clone(),
        });
        self.weights = indicators::has_weights(zone).then(|| (zone.rect, zone.weights.clone()));
        (self.shown_on, self.hidden_on) = (zone.shown_on.clone(), zone.hidden_on.clone());
        // The zone as it is saved is where its changes count from.
        if let Some(o) = &mut self.opened {
            (o.editing, o.start, o.end, o.threshold, o.drawn_on) = (self.editing, self.start, self.end, self.threshold, None);
            (o.by_look, o.look, o.weights) = (self.by_look, self.look.clone(), self.weights.clone());
            (o.shown_on, o.hidden_on) = (self.shown_on.clone(), self.hidden_on.clone());
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

/// The visibility indicators a gauge is read under: each shown or hidden.
fn read_when(ui: &mut egui::Ui, draft: &mut Draft, inputs: &Inputs) {
    let mut visibility: Vec<&str> = Vec::new();
    for zone in inputs.zones.iter().filter(|z| z.kind == IndicatorKind::Visibility) {
        if !visibility.contains(&zone.indicator.as_str()) {
            visibility.push(&zone.indicator);
        }
    }
    let help = "Outside the game's interface (a menu, a map), the screen where the bar is may look like an empty bar \
                (a dark empty color): read it only while a visibility indicator is shown (the frame or an icon next \
                to the bar), or hidden (a menu's button). Otherwise it reads unknown (nil), as when the bar is not found.";
    ui.horizontal_wrapped(|ui| {
        ui.label("Read only when").on_hover_text(help);
        if visibility.is_empty() && draft.read_when.is_empty() {
            ui.label(muted("always (draw a visibility indicator to read it only while it is shown or hidden)").size(12.0));
            return;
        }
        if draft.read_when.is_empty() {
            ui.label(muted("always").size(12.0));
        }
        let mut remove = None;
        for (k, condition) in draft.read_when.iter_mut().enumerate() {
            if k > 0 {
                ui.label("and");
            }
            egui::ComboBox::from_id_salt(("read-when", k)).selected_text(condition.indicator.as_str()).show_ui(ui, |ui| {
                for name in &visibility {
                    ui.selectable_value(&mut condition.indicator, (*name).to_owned(), *name);
                }
            });
            egui::ComboBox::from_id_salt(("read-when-shown", k)).selected_text(if condition.shown { "is shown" } else { "is hidden" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut condition.shown, true, "is shown");
                ui.selectable_value(&mut condition.shown, false, "is hidden");
            });
            if !visibility.contains(&condition.indicator.as_str()) {
                ui.label(RichText::new("no such visibility indicator: left out").color(WARN).size(12.0));
            }
            if ui.small_button("✕").on_hover_text("Remove this condition").clicked() {
                remove = Some(k);
            }
        }
        if let Some(k) = remove {
            draft.read_when.remove(k);
        }
        if let Some(first) = visibility.first() {
            if ui.small_button("+ Condition").on_hover_text(help).clicked() {
                draft.read_when.push(Condition { indicator: (*first).to_owned(), shown: true });
            }
        }
    });
}

/// A step of a guide: its number, or a tick once done.
fn step(ui: &mut egui::Ui, n: usize, done: bool, contents: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mark = if done { RichText::new("✔").color(OK) } else { RichText::new(format!("{n}.")).color(ACCENT_TEXT) };
        ui.label(mark.strong());
        contents(ui);
    });
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

/// What an indicator drawn in `zones` reads on an image, as text and color;
/// `all`: the mode's zones, for a gauge's conditions.
fn reading_zones(indicator_zones: &[&Zone], all: &[Zone], frame: &Frame) -> (String, Color32) {
    let Some(first) = indicator_zones.first() else { return (String::new(), MUTED) };
    match first.kind {
        IndicatorKind::Visibility => {
            let (shown, best) = indicators::shown_anywhere(indicator_zones, frame, false);
            if shown { (format!("shown {best:.2}"), ACCENT_TEXT) } else { (format!("hidden {best:.2}"), MUTED) }
        }
        IndicatorKind::Gauge if unmet_on(first, all, frame).is_some() => reading(first, None),
        IndicatorKind::Gauge => reading(first, indicators::fill_anywhere(indicator_zones, frame)),
    }
}

/// The first of a gauge's conditions that does not hold on an image, as text.
fn unmet_on(zone: &Zone, all: &[Zone], frame: &Frame) -> Option<String> {
    let condition = indicators::unmet(zone, |name| indicators::shown_on(name, all, frame))?;
    Some(format!("{} {}", condition.indicator, if condition.shown { "hidden" } else { "shown" }))
}

/// What a zone reads on an image as `reading` says, and that it reads unknown
/// there when one of its gauge's conditions does not hold.
fn reading_here(zone: &Zone, all: &[Zone], frame: &Frame) -> (String, Color32) {
    let (text, color) = reading(zone, indicators::measure(zone, frame));
    match unmet_on(zone, all, frame) {
        Some(why) => (format!("{text} · unknown here: {why}"), WARN),
        None => (text, color),
    }
}

/// What an indicator reads on an image, as text and color.
fn reading(zone: &Zone, measure: Option<f32>) -> (String, Color32) {
    match (zone.kind, measure) {
        (IndicatorKind::Visibility, Some(m)) if m >= zone.threshold => (format!("shown {m:.2}"), ACCENT_TEXT),
        (IndicatorKind::Visibility, m) => (format!("hidden {:.2}", m.unwrap_or(0.0)), MUTED),
        (IndicatorKind::Gauge, Some(m)) => (format!("{:.0}%", m * 100.0), ACCENT_TEXT),
        (IndicatorKind::Gauge, None) => ("unknown".to_owned(), WARN),
    }
}

impl App {
    /// The tab, fitting the window (the Creator does not scroll it): the
    /// image edited takes what the rest leaves.
    pub(super) fn screen_page(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        let Some(inputs) = self.inputs_of(ui, s, game) else { return };
        self.load_captures(ui.ctx(), Some(inputs));
        ui.label(muted(
            "Captures show GameViber what each phase looks like. Indicators (a health bar, a menu) are drawn on them \
             and checked on all of them. Captures stay on your computer.",
        ));
        ui.add_space(4.0);
        // What the indicator selected reads on each capture: its zones, the one edited as drawn now.
        let st = &mut self.screen;
        let tested = st.draft_zone(inputs);
        let d = &st.draft;
        let mut indicator_zones: Vec<&Zone> =
            inputs.zones.iter().enumerate().filter(|(i, z)| d.indicator.as_ref() == Some(&z.indicator) && Some(*i) != d.editing).map(|(_, z)| z).collect();
        indicator_zones.extend(tested.as_ref());
        let readings = st.readings.get(ui.ctx(), &indicator_zones, &inputs.zones, &st.captures).clone();
        let d = &st.draft;
        // The captures its zones were drawn on.
        let marked: HashSet<String> = inputs.zones.iter().filter(|z| d.indicator.as_ref() == Some(&z.indicator)).filter_map(|z| z.capture.clone()).collect();
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
                self.captures_panel(ui, s, game, inputs, &readings, &marked);
            });
            ui.allocate_ui_with_layout(Vec2::new(main, height), layout, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                changed = self.indicator_panel(ui, inputs);
            });
        });
        if let Some(p) = changed {
            self.send(Command::SaveInputs(p));
        }
    }

    /// Keeps the game's captures in memory, with their textures.
    pub(super) fn load_captures(&mut self, ctx: &egui::Context, loaded: Option<&Inputs>) {
        let st = &mut self.screen;
        let dir = loaded.map(|p| p.dir.clone());
        if st.loaded_game != dir {
            st.loaded_game = dir;
            st.captures.clear();
            st.selected = None;
            st.filter = None;
            st.draft = Draft::default();
        }
        let Some(loaded) = loaded else { return };
        st.captures.retain(|file, _| loaded.captures.iter().any(|c| c.file == *file));
        for capture in &loaded.captures {
            if st.captures.contains_key(&capture.file) {
                continue;
            }
            if let Some(frame) = package::load_capture(&loaded.dir, &capture.file) {
                // Sharp pixels when zoomed in.
                let texture = ctx.load_texture(&capture.file, color_image(&frame), egui::TextureOptions::NEAREST);
                st.captures.insert(capture.file.clone(), (Arc::new(frame), texture));
            }
        }
        if st.selected.as_ref().is_some_and(|f| !st.captures.contains_key(f)) {
            st.selected = None;
        }
        if st.selected.is_none() {
            st.selected = loaded.captures.iter().find(|c| st.captures.contains_key(&c.file)).map(|c| c.file.clone());
        }
    }

    /// The left column: the captures by phase (scrolling if they must), then how to add some.
    fn captures_panel(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, readings: &ReadingMap, marked: &HashSet<String>) {
        let inner = ui.available_size() - Vec2::splat(2.0 * PANEL_MARGIN + 2.0);
        card(PANEL).inner_margin(Margin::same(PANEL_MARGIN as i8)).show(ui, |ui| {
            ui.set_width(inner.x);
            ui.set_height(inner.y);
            ui.label(RichText::new("Captures").strong().size(16.0));
            if !inputs.captures.is_empty() {
                self.capture_filters(ui, inputs);
            }
            // The grid takes what the part below it took on the last frame leaves.
            let height = (ui.available_height() - self.screen.captures_below).max(60.0);
            if inputs.captures.is_empty() {
                ui.allocate_ui(Vec2::new(ui.available_width(), height), |ui| {
                    ui.label(muted("No capture yet: capture each phase a few times, in different spots."));
                });
            } else {
                egui::ScrollArea::vertical().id_salt("captures").auto_shrink([false, false]).max_height(height).min_scrolled_height(height).show(ui, |ui| {
                    self.capture_grid(ui, inputs, readings, marked);
                });
            }
            let top = ui.cursor().top();
            let analysed = inputs.captures.iter().filter(|c| !c.embedding.is_empty()).count();
            if analysed < inputs.captures.len() {
                let why = if Model::Image.ready() { "analysing..." } else { "download the image model (Phases tab) to use them for phases" };
                ui.label(muted(format!("{analysed} of {} analysed: {why}", inputs.captures.len())).size(11.5));
            }
            self.add_captures(ui, s, game, inputs);
            remember_height(ui, &mut self.screen.captures_below, top);
        });
    }

    /// Filters: all, each phase, to sort.
    fn capture_filters(&mut self, ui: &mut egui::Ui, inputs: &Inputs) {
        let mut filters: Vec<(Option<String>, String)> = vec![(None, format!("All · {}", inputs.captures.len()))];
        for phase in inputs.phase_names() {
            let n = inputs.captures.iter().filter(|c| c.phase == phase).count();
            filters.push((Some(phase.clone()), format!("{phase} · {n}")));
        }
        let to_sort = inputs.captures.iter().filter(|c| c.phase.is_empty()).count();
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

    /// The captures shown by the phase filter, three per row, each with what
    /// the indicator selected reads on it (📍: a zone of it was drawn there); a
    /// click opens one in the editor.
    fn capture_grid(&mut self, ui: &mut egui::Ui, inputs: &Inputs, readings: &ReadingMap, marked: &HashSet<String>) {
        let shown: Vec<&package::Capture> =
            inputs.captures.iter().filter(|c| self.screen.filter.as_ref().is_none_or(|f| c.phase == *f)).collect();
        let targets = inputs.phase_names();
        // A visibility zone being edited: whether it shows on each capture can be marked.
        let marking = self.screen.draft.kind == IndicatorKind::Visibility && self.screen.draft.rect().is_some();
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
                            .on_hover_text(if marking {
                                "Open it in the editor; right-click to mark whether the indicator shows on it, file it under another phase or delete it"
                            } else {
                                "Open it in the editor; right-click to file it under another phase or delete it"
                            });
                        if marked.contains(&capture.file) {
                            tag(ui, response.rect.right_top() + Vec2::new(-3.0, 3.0), egui::Align2::RIGHT_TOP, "📍".to_owned(), ACCENT_TEXT);
                        }
                        let draft = &mut self.screen.draft;
                        let (shown, hidden) = (draft.shown_on.contains(&capture.file), draft.hidden_on.contains(&capture.file));
                        if marking && (shown || hidden) {
                            let (text, color) = if shown { ("👁", OK) } else { ("⊘", DANGER) };
                            tag(ui, response.rect.left_top() + Vec2::new(3.0, 3.0), egui::Align2::LEFT_TOP, text.to_owned(), color);
                        }
                        if response.clicked() {
                            self.screen.selected = Some(capture.file.clone());
                        }
                        response.context_menu(|ui| {
                            if marking {
                                let draft = &mut self.screen.draft;
                                if ui.selectable_label(shown, "👁 Shown here").on_hover_text("The indicator edited is on this capture, in its zone").clicked() {
                                    draft.hidden_on.retain(|f| *f != capture.file);
                                    if shown { draft.shown_on.retain(|f| *f != capture.file) } else { draft.shown_on.push(capture.file.clone()) }
                                    ui.close();
                                }
                                if ui.selectable_label(hidden, "⊘ Not shown here").on_hover_text("The indicator edited is not on this capture").clicked() {
                                    draft.shown_on.retain(|f| *f != capture.file);
                                    if hidden { draft.hidden_on.retain(|f| *f != capture.file) } else { draft.hidden_on.push(capture.file.clone()) }
                                    ui.close();
                                }
                                ui.separator();
                            }
                            for other in targets.iter().filter(|o| **o != capture.phase) {
                                if ui.button(format!("Move to {other}")).clicked() {
                                    moved = Some((capture.file.clone(), other.clone()));
                                }
                            }
                            if ui.button("Delete").clicked() {
                                delete = Some(capture.file.clone());
                            }
                        });
                        let (phase, color) = if capture.phase.is_empty() { ("to sort", WARN) } else { (capture.phase.as_str(), MUTED) };
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
                            ui.label(RichText::new(phase).color(color).size(11.5));
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
            self.send(Command::DeleteCapture { dir: inputs.dir.clone(), file });
        }
        if let Some((file, phase)) = moved {
            self.send(Command::MoveCapture { dir: inputs.dir.clone(), file, phase });
        }
    }

    /// How captures are taken: in game with the combo, or from here, into a
    /// phase; or images imported from files.
    fn add_captures(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs) {
        let view = &s.screen;
        let playing = s.game.as_ref().is_some_and(|g| g.id == game.id);
        self.poll_import();
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
            if playing {
                ui.label(muted(format!("Hold {} in game, or from here:", crate::gamepad::combo_text(&s.settings.capture_combo))).size(12.5))
                    .on_hover_text("Captured in game, the image keeps the game's gamepad prompts");
                if view.frame.is_none() {
                    let hint = if s.overlay_unavailable {
                        "Another GameViber holds the in-game overlay."
                    } else if s.overlay_clients.is_empty() {
                        crate::overlay::NO_IMAGE_HINT
                    } else {
                        "Waiting for the game's image..."
                    };
                    ui.label(RichText::new(hint).color(WARN).size(12.0));
                }
            } else {
                ui.label(muted(format!("Images of the game from your computer (screenshots), or play {} to capture it.", game.name)).size(12.5));
            }
            let phases = inputs.phase_names();
            let current = s.capture_phase.clone();
            let label = |phase: &str| if phase.is_empty() { "to sort later".to_owned() } else { phase.to_owned() };
            let mut target = None;
            let mut capture = false;
            let mut import = false;
            ui.horizontal_wrapped(|ui| {
                ui.label("Into");
                egui::ComboBox::from_id_salt("capture-phase").selected_text(label(&current)).width(110.0).show_ui(ui, |ui| {
                    for phase in std::iter::once(String::new()).chain(phases.iter().cloned()) {
                        if ui.selectable_label(phase == current, label(&phase)).clicked() {
                            target = Some(phase);
                        }
                    }
                });
                if playing {
                    capture = ui.add_enabled(view.frame.is_some(), egui::Button::new("📸 Capture")).clicked();
                }
                let busy = self.screen.import.is_some();
                import = ui
                    .add_enabled(!busy, egui::Button::new(if busy { "Importing..." } else { "🖼 From files..." }))
                    .on_hover_text("Add images from your computer: screenshots of the game (PNG, JPEG, WebP, BMP)")
                    .clicked();
                if ui.button("🎞 From a session").on_hover_text("Pick images among those recorded with your play sessions").clicked() {
                    self.creator.show_sessions();
                }
            });
            if let Some((ok, message)) = &self.screen.import_message {
                ui.label(RichText::new(message).color(if *ok { OK } else { WARN }).size(12.0));
            }
            if playing {
                let mut on = s.settings.screen;
                if ui.checkbox(&mut on, "Modes see the game's image").on_hover_text("Phases from the image, indicators, flashes and motion").changed() {
                    self.send(Command::SetScreen(on));
                }
            } else if ui.button("Play this game").on_hover_text("To capture it in game").clicked() {
                self.send(Command::SelectGame(Some(game.id.clone())));
            }
            if let Some(phase) = target {
                self.send(Command::SetCapturePhase(phase));
            }
            if capture {
                self.send(Command::CapturePhase(current.clone()));
            }
            if import {
                self.import_captures(ui.ctx(), inputs, current);
            }
        });
    }

    /// Asks for image files, and adds them to the mode's captures under `phase`.
    fn import_captures(&mut self, ctx: &egui::Context, inputs: &Inputs, phase: String) {
        let (tx, rx) = mpsc::channel();
        let (commands, dir, ctx) = (self.commands.clone(), inputs.dir.clone(), ctx.clone());
        self.screen.import = Some(rx);
        self.screen.import_message = None;
        std::thread::spawn(move || {
            let outcome = match crate::platform::open_files("Add captures", "Images", &package::IMAGE_EXTENSIONS) {
                Ok(paths) if paths.is_empty() => None,
                Ok(paths) => {
                    let mut frames = Vec::new();
                    let mut failed = Vec::new();
                    for path in &paths {
                        match package::read_image(path) {
                            Ok(frame) => frames.push(frame),
                            Err(e) => {
                                log::warn!("cannot read the image {}: {e:#}", path.display());
                                failed.push(path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned()));
                            }
                        }
                    }
                    let added = frames.len();
                    if added > 0 {
                        let _ = commands.send(Command::AddCaptures { dir, phase, frames });
                    }
                    Some(match (added, failed.is_empty()) {
                        (_, true) => (true, format!("{added} image{} added.", if added == 1 { "" } else { "s" })),
                        (_, false) => (false, format!("{added} added; cannot read {}.", failed.join(", "))),
                    })
                }
                Err(e) => Some((false, format!("Cannot open images: {e:#}"))),
            };
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
    }

    /// The import's outcome, once its dialog closed.
    fn poll_import(&mut self) {
        let Some(rx) = &self.screen.import else { return };
        match rx.try_recv() {
            Ok(outcome) => {
                self.screen.import = None;
                self.screen.import_message = outcome;
            }
            Err(mpsc::TryRecvError::Disconnected) => self.screen.import = None,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    /// The right column: the indicator editor, once there is a capture to draw on.
    fn indicator_panel(&mut self, ui: &mut egui::Ui, inputs: &Inputs) -> Option<Inputs> {
        let inner = ui.available_size() - Vec2::splat(2.0 * PANEL_MARGIN + 2.0);
        let mut changed = None;
        card(PANEL).inner_margin(Margin::same(PANEL_MARGIN as i8)).show(ui, |ui| {
            ui.set_width(inner.x);
            ui.set_height(inner.y);
            match self.screen.selected.clone().and_then(|f| self.screen.captures.get(&f).cloned().map(|c| (f, c))) {
                Some((file, (frame, texture))) => {
                    changed = self.indicator_editor(ui, inputs, &file, &frame, &texture);
                }
                None => {
                    ui.label(RichText::new("Indicators").strong().size(16.0));
                    ui.label(muted(
                        "An indicator is a part of the screen modes read: whether something is shown (the battle interface), \
                         or how full a bar is (health). They are drawn on captures: capture the game first (left).",
                    ));
                }
            }
            remember_height(ui, &mut self.screen.editor_below, self.screen.below_image_top);
        });
        changed
    }

    /// The indicators, the zones of the one selected, the capture shown to draw
    /// them on, the indicator's and the zone's settings. Returns the inputs to save.
    fn indicator_editor(&mut self, ui: &mut egui::Ui, inputs: &Inputs, file: &str, frame: &Frame, texture: &egui::TextureHandle) -> Option<Inputs> {
        let now = ui.input(|i| i.time);
        let tested = self.screen.draft_zone(inputs);
        let problem = self.screen.problem(inputs);
        let dirty = self.screen.dirty();
        let mut target: Option<Target> = None;
        let mut save_now = false;
        let mut cancel = false;
        let mut delete_zone = None;
        let mut delete_indicator = false;
        let mut linked = None;
        let mut show_capture = None;
        let st = &mut self.screen;
        let draft = &mut st.draft;
        let indicator_name = draft.indicator.clone();
        let indicator_zones: Vec<usize> = match &indicator_name {
            Some(name) => inputs.zones.iter().enumerate().filter(|(_, z)| z.indicator == *name).map(|(i, _)| i).collect(),
            None => Vec::new(),
        };

        // The indicators.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            ui.label(RichText::new("Indicators").strong());
            let mut seen: Vec<&str> = Vec::new();
            for zone in &inputs.zones {
                if seen.contains(&zone.indicator.as_str()) {
                    continue;
                }
                seen.push(&zone.indicator);
                let n = inputs.zones.iter().filter(|z| z.indicator == zone.indicator).count();
                let kind = if zone.kind == IndicatorKind::Gauge { "gauge" } else { "visibility" };
                let on = indicator_name.as_deref() == Some(zone.indicator.as_str());
                let text = if n > 1 { format!("{} · {kind} · {n}", zone.indicator) } else { format!("{} · {kind}", zone.indicator) };
                if ui.selectable_label(on, text).on_hover_text(format!("{n} zone(s)")).clicked() && !on {
                    target = Some(Target::Indicator(zone.indicator.clone()));
                }
            }
            if ui.selectable_label(indicator_name.is_none(), RichText::new("+ New indicator").color(ACCENT_TEXT)).clicked() && indicator_name.is_some() {
                target = Some(Target::NewIndicator);
            }
        });
        // Those an assistant's analysis proposed, to draw.
        let to_draw: Vec<_> = inputs.to_draw().collect();
        if !to_draw.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                ui.label(muted("To draw"));
                for planned in &to_draw {
                    let on = indicator_name.is_none() && draft.name == planned.name;
                    let hover = format!("{}\n{}\n\nProposed by the assistant: click, then draw it.", planned.kind.label(), planned.place);
                    if ui.selectable_label(on, format!("✨ {}", planned.name)).on_hover_text(hover).clicked() && !on {
                        target = Some(Target::Planned(planned.name.clone(), planned.kind));
                    }
                }
                if ui.small_button("✕").on_hover_text("Forget the indicators proposed").clicked() {
                    linked = Some(Inputs { planned: Vec::new(), ..inputs.clone() });
                }
            });
            if let Some(planned) = to_draw.iter().find(|p| indicator_name.is_none() && draft.name == p.name) {
                ui.label(RichText::new(format!("Where: {}", planned.place)).color(ACCENT_TEXT).size(12.5));
            }
        }
        // Its zones, or what a new indicator reads.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            match &indicator_name {
                Some(name) => {
                    ui.label(RichText::new("Zones").strong());
                    for (n, &i) in indicator_zones.iter().enumerate() {
                        let on = draft.editing == Some(i);
                        let phase = inputs.zones[i].phase.as_deref().unwrap_or("any phase");
                        let hover = if on { "Leave this zone" } else { "Edit this zone, on its capture" };
                        if ui.selectable_label(on, format!("{} · {phase}", n + 1)).on_hover_text(hover).clicked() {
                            target = Some(if on { Target::Indicator(name.clone()) } else { Target::Zone(i, true) });
                        }
                    }
                    let hover = "Draw it again where, or as, the game also shows it: the health bar out of battles, \
                                 another menu. Its value comes from the zone where it is found; shown when any zone is.";
                    if ui.selectable_label(draft.editing.is_none(), "+ Zone").on_hover_text(hover).clicked() && draft.editing.is_some() {
                        target = Some(Target::Indicator(name.clone()));
                    }
                }
                None => {
                    ui.label(RichText::new("New indicator, reads").strong());
                    for (kind, help) in [
                        (IndicatorKind::Visibility, "Whether an element is on screen: a battle menu, a dialogue box"),
                        (IndicatorKind::Gauge, "How full a bar is: health, mana, a timer"),
                    ] {
                        if ui.selectable_label(draft.kind_chosen && draft.kind == kind, kind.label()).on_hover_text(help).clicked() {
                            draft.kind = kind;
                            draft.kind_chosen = true;
                        }
                    }
                }
            }
        });
        // Which indicators the image shows, and its zoom.
        ui.horizontal(|ui| {
            ui.label(muted("Show"));
            for (show, label, help) in [
                (Show::All, "All indicators", "Every indicator"),
                (Show::Indicator, "This indicator", "Only the zones of the indicator selected"),
                (Show::Nothing, "None", "Hide every indicator, to see the image"),
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
        // Zoomed with the slider, or an indicator opened: the indicator (else what was in the middle) stays in the middle.
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
        let can_draw = draft.indicator.is_some() || draft.kind_chosen;
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
            // The zone of the indicator selected under a point, when none is being edited.
            let zone_at = |p: Pos2| indicator_zones.iter().copied().find(|&i| on_image(area, inputs.zones[i].rect).expand(3.0).contains(p));
            if let Some(pick) = draft.picking {
                if hover.is_some() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                }
                if response.clicked() {
                    if let Some(p) = response.interact_pointer_pos().map(to_fraction) {
                        let color = indicators::pick_color(frame, p.x, p.y);
                        match pick {
                            Pick::Full if !draft.full.contains(&color) => draft.full.push(color),
                            Pick::Empty if !draft.empty.contains(&color) => draft.empty.push(color),
                            Pick::Tier(k) => {
                                if let Some(tier) = draft.tiers.get_mut(k).filter(|t| !t.contains(&color)) {
                                    tier.push(color);
                                }
                            }
                            _ => {}
                        }
                    }
                    draft.picking = None;
                }
            } else if can_draw {
                let grab_at = |draft: &Draft, p: Pos2| match draft.rect() {
                    Some(rect) => Grab::at(on_image(area, rect), p),
                    // On one of the indicator's zones: picks it and moves it.
                    None if draft.editing.is_none() && zone_at(p).is_some() => Grab::Move { from: Pos2::ZERO, rect: [0.0; 4] },
                    None => Grab::New,
                };
                if let Some(p) = hover.filter(|_| draft.grab.is_none()) {
                    ui.ctx().set_cursor_icon(grab_at(draft, p).cursor());
                }
                if response.drag_started() {
                    // Where the button went down: egui only calls it a drag once the pointer moved a little.
                    if let Some(origin) = ui.input(|i| i.pointer.press_origin()).or(response.interact_pointer_pos()) {
                        if draft.rect().is_none() && draft.editing.is_none() {
                            if let Some(i) = zone_at(origin) {
                                draft.take_zone(i, &inputs.zones[i]);
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
            // A click on another indicator's zone, or one of this indicator, opens it.
            if response.clicked() && draft.picking.is_none() {
                if let Some(p) = response.interact_pointer_pos() {
                    let hit = inputs
                        .zones
                        .iter()
                        .enumerate()
                        .filter(|(i, z)| draft.editing != Some(*i) && on_image(area, z.rect).expand(3.0).contains(p))
                        .min_by(|(_, a), (_, b)| (a.rect[2] * a.rect[3]).total_cmp(&(b.rect[2] * b.rect[3])))
                        .map(|(i, _)| i);
                    if let Some(i) = hit {
                        target = Some(Target::Zone(i, false));
                    }
                }
            }
            // The other zones; their name and reading when hovered.
            for (i, zone) in inputs.zones.iter().enumerate() {
                let same = indicator_name.as_deref() == Some(zone.indicator.as_str());
                let shown = match st.show {
                    Show::All => true,
                    Show::Indicator => same,
                    Show::Nothing => false,
                };
                if draft.editing == Some(i) || !shown {
                    continue;
                }
                let r = on_image(area, zone.rect);
                let hovered = hover.is_some_and(|p| r.expand(3.0).contains(p)) && draft.grab.is_none();
                // The indicator's zones stand out.
                let stroke = match (hovered, same) {
                    (true, _) => Stroke::new(1.5, TEXT),
                    (false, true) => Stroke::new(1.5, ACCENT.gamma_multiply(0.8)),
                    (false, false) => Stroke::new(1.0, MUTED.gamma_multiply(0.7)),
                };
                ui.painter().rect_stroke(r, 2.0, stroke, StrokeKind::Outside);
                if hovered {
                    let (text, color) = reading_here(zone, &inputs.zones, frame);
                    tag(ui, r.left_top() - Vec2::new(0.0, 4.0), egui::Align2::LEFT_BOTTOM, format!("{} · {text} · click to edit", zone.indicator), color);
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
                    let (text, color) = reading_here(zone, &inputs.zones, frame);
                    let label = if draft.name.is_empty() { text } else { format!("{} · {text}", draft.name) };
                    tag(ui, r.left_top() - Vec2::new(0.0, 6.0), egui::Align2::LEFT_BOTTOM, label, color);
                    // The cells it is compared on, when it compares only what stays.
                    if indicators::has_weights(zone) {
                        let cell = Vec2::new(r.width() / indicators::REF_WIDTH as f32, r.height() / indicators::REF_HEIGHT as f32);
                        for (k, &w) in zone.weights.iter().enumerate().filter(|(_, w)| **w > 0) {
                            let at = r.left_top() + Vec2::new((k % indicators::REF_WIDTH) as f32 * cell.x, (k / indicators::REF_WIDTH) as f32 * cell.y);
                            ui.painter().rect_filled(Rect::from_min_size(at, cell), 0.0, ACCENT.gamma_multiply(0.15 + 0.35 * w as f32 / 255.0));
                        }
                    }
                    // The bar as found: its full part green, its empty part red, along the indicator.
                    if zone.kind == IndicatorKind::Gauge && (zone.empty_color.is_some() || indicators::has_look(zone)) {
                        let ((a, b), (c, d)) = indicators::bar_extent(zone, frame);
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
        // zone, the indicator), Delete deletes the zone, arrows nudge it.
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
                } else if let (Some(_), Some(name)) = (draft.editing, &indicator_name) {
                    target = Some(Target::Indicator(name.clone()));
                } else if indicator_name.is_some() {
                    target = Some(Target::NewIndicator);
                }
            }
            if delete && indicator_zones.len() > 1 {
                delete_zone = draft.editing;
            }
            if step != Vec2::ZERO && draft.rect().is_some() {
                draft.nudge(Vec2::new(step.x / frame.width.max(1) as f32, step.y / frame.height.max(1) as f32));
                draft.drawn_on = Some(file.to_owned());
            }
        }

        let hint = match (draft.picking, draft.rect()) {
            (Some(Pick::Full), _) => "Click the bar's full part on the image (Esc: stop).",
            (Some(Pick::Empty), _) => "Click the bar's empty part on the image (Esc: stop).",
            (Some(Pick::Tier(_)), _) => "Click the bar's part in this tier's color on the image (Esc: stop).",
            (None, None) if !can_draw => "Choose what the new indicator reads, above, then draw it on the image.",
            (None, None) if indicator_name.is_some() => "Pick a zone above, or click or drag it on the image; or draw a new zone.",
            (None, None) if draft.kind == IndicatorKind::Gauge => "Drag a rectangle along the bar, from its empty end to its full end.",
            (None, None) => "Drag a rectangle around fixed parts of the element (not text that changes).",
            (None, Some(_)) => "Drag its edges to resize it, its inside to move it; arrow keys nudge it (Shift: 10 px). Esc leaves it.",
        };
        ui.label(muted(hint).size(12.0));

        // The indicator's settings, shared by its zones.
        if can_draw {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Indicator").strong());
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut draft.name).hint_text("battle_menu").desired_width(130.0).font(egui::TextStyle::Monospace))
                    .on_hover_text("Renaming renames all its zones, and keeps the phase it is a sign of");
                if draft.indicator.is_some() {
                    ui.label(muted(draft.kind.label())).on_hover_text("Set when the indicator was made: for another kind, make a new indicator");
                }
                if draft.kind == IndicatorKind::Gauge {
                    egui::ComboBox::from_id_salt("zone-direction").selected_text(draft.direction.label()).show_ui(ui, |ui| {
                        for d in Direction::ALL {
                            ui.selectable_value(&mut draft.direction, d, d.label());
                        }
                    });
                }
                // The phases this indicator is a sure sign of (saved right away).
                if let Some(name) = indicator_name.as_ref().filter(|_| !inputs.phases.is_empty()) {
                    let of: Vec<&PhaseDef> = inputs.phases.iter().filter(|sc| sc.indicators.contains(name)).collect();
                    let phases: Vec<&str> = of.iter().map(|sc| sc.name.as_str()).collect();
                    let signs: Vec<String> = of.iter().map(|sc| format!("{}: {}", sc.name, sc.indicators.join(" + "))).collect();
                    ui.label("Sure sign of").on_hover_text(
                        "While this indicator is shown (any of its zones), GameViber is sure of the phase, right away (a \
                         battle menu: battle). A phase can need several indicators shown together (in the Phases tab), \
                         and an indicator can be part of the signs of several phases: the sign of the most indicators \
                         shown wins. How long the phase is kept once it hides is set with the phase, in the Phases tab.",
                    );
                    let label = if phases.is_empty() { "no phase".to_owned() } else { phases.join(", ") };
                    let mut toggled = None;
                    egui::ComboBox::from_id_salt("indicator-phase")
                        .selected_text(label)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show_ui(ui, |ui| {
                            for (k, phase) in inputs.phases.iter().enumerate() {
                                let mut on = phase.indicators.contains(name);
                                if ui.checkbox(&mut on, &phase.name).changed() {
                                    toggled = Some((k, on));
                                }
                            }
                        })
                        .response
                        .on_hover_text(signs.join("\n"));
                    if let Some((k, on)) = toggled {
                        let mut g = inputs.clone();
                        let phase = &mut g.phases[k];
                        if on {
                            phase.indicators.push(name.clone());
                            phase.otherwise = false;
                        } else {
                            phase.indicators.retain(|z| z != name);
                        }
                        linked = Some(g);
                    }
                }
            });
            if draft.kind == IndicatorKind::Gauge {
                ui.horizontal(|ui| {
                    ui.label("Read by");
                    ui.selectable_value(&mut draft.by_look, false, "Colors");
                    ui.selectable_value(&mut draft.by_look, true, "Look");
                });
                let why = if draft.by_look {
                    "Look: for bars a color does not describe — a gradient (red to green), segments, hearts, stripes. \
                     GameViber learns how the bar looks full and empty along its length, from captures; the bar must stay in place."
                } else {
                    "Colors: for a bar of one color over an empty part of another (filled again in another color once \
                     full: add a tier). It may move inside the zone. For a gradient, segments, hearts or stripes, choose Look."
                };
                ui.label(muted(why).size(12.0));
            }
            if draft.kind == IndicatorKind::Gauge {
                read_when(ui, draft, inputs);
            }
            if draft.kind == IndicatorKind::Gauge && draft.by_look {
                let rect = draft.rect();
                let fits = |draft: &Draft| rect.is_some_and(|r| draft.look.as_ref().is_some_and(|l| l.fits(r, draft.direction)));
                let moved = rect.is_some() && draft.look.is_some() && !fits(draft);
                let seen = draft.look.as_ref().filter(|_| fits(draft)).map_or(0.0, |l| indicators::empty_seen(&Zone { empty_look: l.empty.clone(), ..Zone::default() }));
                // 1. The rectangle.
                step(ui, 1, rect.is_some(), |ui| {
                    ui.label("Draw the rectangle exactly on the bar's track, from its empty end to its full end, and set which way it fills. \
                              Leave out its icon (a heart before the bar) and its frame.");
                });
                // 2. The bar full.
                step(ui, 2, fits(draft), |ui| {
                    ui.label("Open a capture where the bar is");
                    ui.label(RichText::new("full").strong());
                    ui.label("(left column), then");
                    if ui.add_enabled(rect.is_some(), egui::Button::new("Use it as the full bar")).clicked() {
                        if let Some(rect) = rect {
                            // Empty parts already seen stay when the rectangle did not change.
                            let empty = draft.look.as_ref().filter(|_| fits(draft)).map(|l| l.empty.clone()).unwrap_or_default();
                            let full = indicators::look(frame, rect, draft.direction);
                            draft.look = Some(Look { rect, horizontal: horizontal(draft.direction), full, empty });
                        }
                    }
                    if moved {
                        ui.label(RichText::new("The rectangle moved or turned since: do it again.").color(WARN));
                    }
                });
                // 3. The bar empty.
                step(ui, 3, seen >= 0.95, |ui| {
                    ui.label("Open captures where the bar is");
                    ui.label(RichText::new("low or empty").strong());
                    ui.label(", then");
                    let help = "The part of the bar past its end on this capture is how it looks empty. Add captures at \
                                different levels until all of it is seen.";
                    if ui.add_enabled(fits(draft), egui::Button::new("Add its empty part")).on_hover_text(help).clicked() {
                        if let (Some(rect), Some(look)) = (rect, draft.look.as_mut()) {
                            let zone = Zone {
                                kind: IndicatorKind::Gauge,
                                rect,
                                direction: draft.direction,
                                tolerance: draft.tolerance,
                                full_look: look.full.clone(),
                                empty_look: look.empty.clone(),
                                ..Zone::default()
                            };
                            look.empty = indicators::add_empty_look(&zone, frame);
                        }
                    }
                    ui.label(muted(format!("{:.0}% of the bar seen empty", seen * 100.0)));
                    if seen > 0.0 && ui.small_button("Clear").on_hover_text("Forget how it looks empty").clicked() {
                        if let Some(look) = draft.look.as_mut() {
                            look.empty.clear();
                        }
                    }
                });
                if fits(draft) && seen < 0.95 {
                    ui.label(
                        muted("Recommended: until all of it is seen empty, the reading is rougher, and a bar gone from the screen \
                               (a menu) reads empty instead of unknown (nil).")
                        .size(12.0),
                    );
                }
                // 4. Checking it.
                step(ui, 4, false, |ui| {
                    ui.label("Check it: under the rectangle, green is the part read as full, red as empty; each capture on the left shows its reading.");
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut draft.tolerance, 10.0..=150.0).text("tolerance")).on_hover_text(
                        "How different from its looks the bar may be. Raise it when the bar shines or blinks, lower it when \
                         its empty part reads full.",
                    );
                });
            } else if draft.kind == IndicatorKind::Gauge {
                ui.horizontal_wrapped(|ui| {
                    let mut remove = None;
                    let first = if draft.tiers.is_empty() { "Full" } else { "Tier 1" };
                    for (pick, label) in [(Pick::Full, first), (Pick::Empty, "Empty")] {
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
                        _ => {}
                    }
                    ui.add(egui::Slider::new(&mut draft.tolerance, 10.0..=150.0).text("tolerance")).on_hover_text("How far from the colors a pixel may be");
                });
                // Its next tiers: the bar filled again over itself in other colors.
                let mut remove_tier = None;
                for k in 0..draft.tiers.len() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("Tier {}", k + 2));
                        if draft.tiers[k].is_empty() {
                            swatch(ui, None);
                        }
                        let mut remove = None;
                        for (c, color) in draft.tiers[k].iter().enumerate() {
                            if color_button(ui, *color).on_hover_text("Click to remove this color").clicked() {
                                remove = Some(c);
                            }
                        }
                        if let Some(c) = remove {
                            draft.tiers[k].remove(c);
                        }
                        let picking = draft.picking == Some(Pick::Tier(k));
                        let text = if draft.tiers[k].is_empty() { "🖊 Pick" } else { "🖊 +" };
                        if ui.selectable_label(picking, text).on_hover_text("Then click the bar's part in this tier's color on the image").clicked() {
                            draft.picking = if picking { None } else { Some(Pick::Tier(k)) };
                        }
                        if ui.small_button("✕").on_hover_text("Remove this tier").clicked() {
                            remove_tier = Some(k);
                        }
                    });
                }
                if let Some(k) = remove_tier {
                    draft.tiers.remove(k);
                    draft.picking = None;
                }
                ui.horizontal_wrapped(|ui| {
                    let help = "For a bar filled again over itself in another color once full (green up to half, then yellow over \
                                the green). Each tier is an equal share of the value: with two, the first color reads 0 to 50%, \
                                the second 50 to 100%.";
                    if ui.small_button("+ Tier").on_hover_text(help).clicked() {
                        draft.tiers.push(Vec::new());
                        draft.picking = Some(Pick::Tier(draft.tiers.len() - 1));
                    }
                    if !draft.tiers.is_empty() {
                        let share = 100.0 / (draft.tiers.len() + 1) as f32;
                        ui.label(muted(format!("Each tier is {share:.0}% of the value; under the rectangle, green is the highest tier's part.")).size(12.0));
                    }
                });
                if draft.empty.is_empty() {
                    ui.label(RichText::new("Pick the empty color too: the bar is then found inside the zone, and reads unknown (nil) when not on screen.").color(WARN).size(12.0));
                }
            }
        }

        // The zone's own settings.
        if draft.rect().is_some() {
            let saved = draft.editing.and_then(|i| inputs.zones.get(i));
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Zone").strong());
                if draft.kind == IndicatorKind::Visibility {
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
            if draft.kind == IndicatorKind::Visibility {
                if draft.editing.is_some() && draft.drawn_on.is_some() {
                    ui.label(muted("Its look is taken again from this capture.").size(12.0));
                } else if saved.is_some_and(|z| indicators::measure(z, frame).is_none_or(|m| m < z.threshold)) {
                    ui.label(
                        RichText::new("Hidden on this capture: moving it here would take its look from an image without it.").color(WARN).size(12.0),
                    );
                }
            }
            if let Some(zone) = tested.as_ref().filter(|z| z.kind == IndicatorKind::Visibility) {
                // Whether it shows on this capture, marked by the player.
                ui.horizontal_wrapped(|ui| {
                    ui.label(muted("On this capture:").size(12.0));
                    if zone.capture.as_deref() == Some(file) {
                        ui.label(RichText::new("👁 shown (drawn here)").color(OK).size(12.0));
                    } else {
                        let (shown, hidden) = (draft.shown_on.iter().any(|f| f == file), draft.hidden_on.iter().any(|f| f == file));
                        let mark = |ui: &mut egui::Ui, on: bool, text: &str, help: &str| ui.selectable_label(on, RichText::new(text).size(12.0)).on_hover_text(help).clicked();
                        if mark(ui, shown, "👁 Shown", "It is on this capture, in this zone (click again to unmark)") {
                            draft.hidden_on.retain(|f| f != file);
                            if shown { draft.shown_on.retain(|f| f != file) } else { draft.shown_on.push(file.to_owned()) }
                        }
                        if mark(ui, hidden, "⊘ Not shown", "It is not on this capture (click again to unmark)") {
                            draft.shown_on.retain(|f| f != file);
                            if hidden { draft.hidden_on.retain(|f| f != file) } else { draft.hidden_on.push(file.to_owned()) }
                        }
                    }
                    if !zone.shown_on.is_empty() || !zone.hidden_on.is_empty() {
                        ui.label(muted(format!("marked: {} shown, {} not", zone.shown_on.len() + zone.capture.is_some() as usize, zone.hidden_on.len())).size(12.0));
                    }
                });
                // Where it is shown and where not: the captures marked so (the
                // one it was drawn on shown); none marked, those of its phase and of the others.
                let marked = !zone.shown_on.is_empty() || !zone.hidden_on.is_empty();
                let shown_on = |c: &package::Capture| -> Option<bool> {
                    if marked {
                        if zone.capture.as_ref() == Some(&c.file) || zone.shown_on.contains(&c.file) {
                            Some(true)
                        } else {
                            zone.hidden_on.contains(&c.file).then_some(false)
                        }
                    } else {
                        let phase = zone.phase.as_ref()?;
                        (!c.phase.is_empty()).then(|| c.phase == *phase)
                    }
                };
                let (mut shown_in, mut others) = (Vec::new(), Vec::new());
                let (mut shown_frames, mut other_frames) = (Vec::new(), Vec::new());
                for capture in &inputs.captures {
                    let (Some(shown), Some((capture_frame, _))) = (shown_on(capture), st.captures.get(&capture.file)) else { continue };
                    let m = indicators::measure(zone, capture_frame).unwrap_or(-1.0);
                    if shown {
                        shown_in.push(m);
                        shown_frames.push(&**capture_frame);
                    } else {
                        others.push(m);
                        other_frames.push(&**capture_frame);
                    }
                }
                let (shown_text, hidden_text) = match (&zone.phase, marked) {
                    (Some(phase), false) => (format!("the {phase} captures"), "those of the other phases".to_owned()),
                    _ => ("the captures marked shown".to_owned(), "those marked not shown".to_owned()),
                };
                // Its cells that tell, for an element whose inside changes (a minimap).
                ui.horizontal_wrapped(|ui| {
                    let help = format!(
                        "For an element whose inside changes (a minimap's map): compares only the parts of the zone that stay the \
                         same on {shown_text} and differ on {hidden_text} (its frame). Needs {} captures where it is shown or more, \
                         the more varied the better. When it shows in some phases and not others, or a phase mixes screens \
                         (cutscenes and menus), mark the captures: 👁 Shown or ⊘ Not shown, above.",
                        indicators::WEIGHT_CAPTURES
                    );
                    if indicators::has_weights(zone) {
                        let share = indicators::weighted_share(zone) * 100.0;
                        ui.label(RichText::new(format!("✔ compared on what stays ({share:.0}% of the zone)")).color(OK).size(12.0)).on_hover_text(&help);
                        if ui.small_button("Learn again").on_hover_text(format!("From the captures as they are marked now\n\n{help}")).clicked() {
                            draft.weights = indicators::learn_weights(zone, &shown_frames, &other_frames).map(|w| (zone.rect, w)).or(draft.weights.take());
                        }
                        if ui.small_button("Compare it all").on_hover_text("Compare the whole zone again").clicked() {
                            draft.weights = None;
                        }
                    } else if shown_frames.len() < indicators::WEIGHT_CAPTURES {
                        let text = format!("Its inside changes? Mark {} captures or more where it is shown to compare only what stays.", indicators::WEIGHT_CAPTURES);
                        ui.label(muted(text).size(12.0)).on_hover_text(&help);
                    } else if ui.button("Compare only what stays").on_hover_text(&help).clicked() {
                        draft.weights = indicators::learn_weights(zone, &shown_frames, &other_frames).map(|w| (zone.rect, w));
                    }
                });
                // A threshold telling where it is shown from where it is not.
                ui.horizontal(|ui| match indicators::suggest_threshold(&shown_in, &others) {
                    Some(t) if (t - draft.threshold).abs() > 0.01 => {
                        if ui.button(format!("Use threshold {t:.2}")).on_hover_text(format!("Tells {shown_text} from {hidden_text}")).clicked() {
                            draft.threshold = t;
                        }
                    }
                    Some(_) => {
                        ui.label(RichText::new(format!("✔ tells {shown_text} from {hidden_text}")).color(OK).size(12.0));
                    }
                    None if !shown_in.is_empty() && !others.is_empty() => {
                        ui.label(RichText::new("No threshold tells them apart: draw it tighter, on fixed parts, or compare only what stays").color(WARN).size(12.0));
                    }
                    None => {}
                });
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
                let label = match (&indicator_name, draft.editing, draft.rect()) {
                    (None, ..) => "Save the indicator",
                    (Some(_), None, Some(_)) => "Save the zone",
                    _ => "Save",
                };
                if ui.add_enabled(dirty && problem.is_none(), primary(label)).clicked() {
                    save_now = true;
                }
                if dirty && ui.button("Cancel").on_hover_text("Forget the changes").clicked() {
                    cancel = true;
                }
                if let Some(name) = &indicator_name {
                    if st.confirm_delete {
                        let really = egui::Button::new(RichText::new(format!("Delete {name} and its {} zone(s)", indicator_zones.len())).color(Color32::WHITE)).fill(DANGER);
                        if ui.add(really).clicked() {
                            delete_indicator = true;
                        }
                        if ui.button("Keep it").clicked() {
                            st.confirm_delete = false;
                        }
                    } else if ui.button(RichText::new("Delete the indicator").color(DANGER_TEXT)).clicked() {
                        st.confirm_delete = true;
                    }
                    if let Some(i) = draft.editing.filter(|_| indicator_zones.len() > 1) {
                        if ui.button(RichText::new("Delete this zone").color(DANGER_TEXT)).on_hover_text("Also the Delete key").clicked() {
                            delete_zone = Some(i);
                        }
                    }
                }
            });
        });

        if let Some(capture) = show_capture {
            st.selected = Some(capture);
        }
        if let Some(i) = delete_zone {
            let mut g = inputs.clone();
            g.zones.remove(i);
            if let Some(name) = &indicator_name {
                st.select_indicator(name, &g);
            }
            return Some(g);
        }
        if delete_indicator {
            if let Some(name) = &indicator_name {
                let mut g = inputs.clone();
                g.zones.retain(|z| z.indicator != *name);
                g.zones.iter_mut().for_each(|z| z.read_when.retain(|c| c.indicator != *name));
                g.phases.iter_mut().for_each(|sc| sc.indicators.retain(|z| z != name));
                st.draft = Draft { zoom: st.draft.zoom, ..Draft::default() };
                st.confirm_delete = false;
                return Some(g);
            }
        }
        if save_now {
            if let Some((g, place)) = st.applied(inputs) {
                let name = st.draft.name.clone();
                match place {
                    Some(i) => st.open_zone(i, &g, false),
                    None => st.select_indicator(&name, &g),
                }
                st.saved_at = now;
                return Some(g);
            }
        }
        if cancel {
            match (indicator_name, st.draft.editing) {
                (Some(_), Some(i)) => st.open_zone(i, inputs, false),
                (Some(name), None) => st.select_indicator(&name, inputs),
                (None, _) => st.draft = Draft { zoom: st.draft.zoom, ..Draft::default() },
            }
            return None;
        }
        if let Some(target) = target {
            return st.go(target, inputs).or(linked);
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
