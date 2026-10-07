//! Setting up a gamepad's buttons (Gamepad page), in a popup: a drawn gamepad
//! shows the button to press, with its Xbox or PlayStation name; any button
//! clicked on it is set again, and the buttons pressed light up through the
//! setup so far, so that a mistake shows at once.

use std::collections::BTreeSet;
use std::f32::consts::PI;

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, Vec2};

use super::theme::*;
use super::App;
use crate::engine::Command;
use crate::gamepad::mapping::{step_of, Element, Learner, Mapping, PadOutput, Target, STEPS};
use crate::source::PadInfo;

/// The names the drawing and the steps use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Labels {
    #[default]
    Xbox,
    PlayStation,
}

pub struct PadSetup {
    pub(super) guid: String,
    name: String,
    /// None until the player, hands off the gamepad, starts.
    learner: Option<Learner>,
    /// The step being learned; None: every step done, checking.
    step: Option<usize>,
    binds: Vec<(Target, Element)>,
    skipped: BTreeSet<usize>,
    /// What happened with the last press, when worth saying.
    note: Option<String>,
    labels: Labels,
}

impl PadSetup {
    /// From scratch, or from the gamepad's mapping (only the wrong buttons to set again).
    pub fn new(pad: &PadInfo, labels: Labels) -> Self {
        let binds: Vec<_> = pad
            .mapping
            .iter()
            .flat_map(|m| m.binds.iter().copied())
            .filter(|(target, _)| step_of(target.name()).is_some_and(|i| STEPS[i].target == *target))
            .collect();
        let mut setup = Self {
            guid: pad.guid.clone(),
            name: pad.name.clone(),
            learner: None,
            step: Some(0),
            binds,
            skipped: BTreeSet::new(),
            note: None,
            labels,
        };
        if !setup.binds.is_empty() {
            setup.skipped = (0..STEPS.len()).filter(|&i| setup.bind(i).is_none()).collect();
            setup.step = None;
        }
        setup
    }

    /// Half done, for the screenshot tour.
    pub fn preview(pad: &PadInfo, labels: Labels) -> Self {
        let mut setup = Self::new(pad, labels);
        setup.learner = Some(Learner::new(pad.raw.clone()));
        for (i, element) in [(0, Element::Button(0)), (1, Element::Button(1)), (2, Element::Button(3)), (3, Element::Button(4))] {
            setup.learned(i, element);
        }
        setup.skipped.insert(4);
        setup.step = Some(5);
        setup
    }

    fn bind(&self, step: usize) -> Option<Element> {
        self.binds.iter().find(|(t, _)| *t == STEPS[step].target).map(|(_, e)| *e)
    }

    /// The next step neither set nor skipped, after `from` first.
    fn next(&self, from: usize) -> Option<usize> {
        (from + 1..STEPS.len()).chain(0..=from).find(|&i| self.bind(i).is_none() && !self.skipped.contains(&i))
    }

    fn go_to(&mut self, step: usize) {
        self.step = Some(step);
        self.skipped.remove(&step);
        self.note = None;
        if let Some(learner) = self.learner.as_mut() {
            learner.reset();
        }
    }

    /// A press was learned for the current step.
    fn learned(&mut self, step: usize, element: Element) {
        let target = STEPS[step].target;
        // Already another button's: it moves here, and that one is to set again.
        let taken = self.binds.iter().position(|(t, e)| *e == element && *t != target);
        let freed = taken.map(|i| self.binds.remove(i).0);
        self.binds.retain(|(t, _)| *t != target);
        self.binds.push((target, element));
        match freed.and_then(|t| step_of(t.name())) {
            Some(other) => {
                self.note = Some(format!(
                    "That was {}: it is {} now. Set {} again.",
                    label(STEPS[other].target.name(), self.labels),
                    label(target.name(), self.labels),
                    label(STEPS[other].target.name(), self.labels),
                ));
                self.step = Some(other);
            }
            None => {
                self.note = None;
                self.step = self.next(step);
            }
        }
    }

    fn mapping(&self) -> Mapping {
        Mapping { guid: self.guid.clone(), name: self.name.clone(), binds: self.binds.clone() }
    }
}

impl App {
    /// The popup, while a setup runs.
    pub(super) fn pad_setup_ui(&mut self, ctx: &egui::Context, pad: &PadInfo) {
        let Some(setup) = self.pad_setup.as_mut() else { return };
        ctx.request_repaint();
        if let (Some(learner), Some(step)) = (setup.learner.as_mut(), setup.step) {
            if let Some(element) = learner.update(&pad.raw, &STEPS[step].target) {
                setup.learned(step, element);
            }
        }
        let preview = setup.mapping().apply(&pad.raw);
        let (mut save, mut cancel) = (false, false);
        egui::Modal::new(egui::Id::new("pad-setup")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.horizontal(|ui| {
                heading(ui, &format!("Buttons of {}", setup.name));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.selectable_value(&mut setup.labels, Labels::PlayStation, "PlayStation");
                    ui.selectable_value(&mut setup.labels, Labels::Xbox, "Xbox");
                    ui.label(muted("Names"));
                });
            });
            ui.add_space(4.0);
            if let Some(step) = draw_pad(ui, setup, &preview) {
                setup.go_to(step);
            }
            ui.add_space(6.0);
            match (&setup.learner, setup.step) {
                (None, _) => {
                    ui.label(RichText::new("Let go of the gamepad: sticks centered, triggers released.").size(16.0).strong());
                    ui.label(muted("GameViber notes how it rests, then asks for one button at a time."));
                }
                (Some(_), Some(step)) => {
                    ui.label(muted(format!("{} of {} set: press the blinking one", setup.binds.len(), STEPS.len())));
                    ui.label(RichText::new(prompt(step, setup.labels)).size(16.0).strong().color(ACCENT_TEXT));
                }
                (Some(_), None) => {
                    ui.label(RichText::new("Check it").size(16.0).strong());
                    ui.label("Press buttons and move the sticks: what they light up on the drawing is what games will get. \
                              Click a wrong one to set it again.");
                }
            }
            if let Some(note) = &setup.note {
                ui.label(RichText::new(note).color(WARN));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                match (&setup.learner, setup.step) {
                    (None, _) => {
                        if ui.add(primary("Start")).clicked() {
                            setup.learner = Some(Learner::new(pad.raw.clone()));
                        }
                    }
                    (Some(_), Some(step)) => {
                        let skip = if STEPS[step].optional { "My gamepad has none" } else { "Skip" };
                        if ui.button(skip).clicked() {
                            setup.skipped.insert(step);
                            setup.binds.retain(|(t, _)| *t != STEPS[step].target);
                            setup.note = None;
                            setup.step = setup.next(step);
                            if let Some(learner) = setup.learner.as_mut() {
                                learner.reset();
                            }
                        }
                    }
                    (Some(_), None) => {
                        save = ui.add(primary("Save")).clicked();
                    }
                }
                cancel = ui.button("Cancel").clicked();
                if setup.learner.is_some() {
                    ui.label(muted("Click a button on the drawing to set it again."));
                }
            });
        });
        if save {
            let mapping = setup.mapping();
            self.send(Command::SaveMapping(mapping));
        }
        if save || cancel {
            self.pad_setup = None;
        }
    }
}

/// What to press for a step, in the names chosen.
fn prompt(step: usize, labels: Labels) -> String {
    let name = STEPS[step].target.name();
    let what = label(name, labels);
    match name {
        "LT" | "RT" => format!("Pull {what} all the way, then let it go"),
        "LX" => "Push the left stick to the right, then let it go".into(),
        "LY" => "Push the left stick down, then let it go".into(),
        "RX" => "Push the right stick to the right, then let it go".into(),
        "RY" => "Push the right stick down, then let it go".into(),
        "LS" | "RS" => format!("Click {what} (press the stick in)"),
        "DPAD_UP" => "Press up on the d-pad".into(),
        "DPAD_DOWN" => "Press down on the d-pad".into(),
        "DPAD_LEFT" => "Press left on the d-pad".into(),
        "DPAD_RIGHT" => "Press right on the d-pad".into(),
        _ => format!("Press {what}"),
    }
}

/// A button's name on an Xbox or a PlayStation gamepad.
fn label(name: &str, labels: Labels) -> String {
    let (xbox, ps) = match name {
        "A" => ("A", "Cross ✕"),
        "B" => ("B", "Circle ○"),
        "X" => ("X", "Square □"),
        "Y" => ("Y", "Triangle △"),
        "LB" => ("LB", "L1"),
        "RB" => ("RB", "R1"),
        "LT" => ("LT", "L2"),
        "RT" => ("RT", "R2"),
        "BACK" => ("View (Back)", "Share (Select)"),
        "START" => ("Menu (Start)", "Options (Start)"),
        "GUIDE" => ("the Xbox button", "the PS button"),
        "LS" => ("the left stick (LS)", "the left stick (L3)"),
        "RS" => ("the right stick (RS)", "the right stick (R3)"),
        "LX" | "LY" => ("the left stick", "the left stick"),
        "RX" | "RY" => ("the right stick", "the right stick"),
        "DPAD_UP" => ("up", "up"),
        "DPAD_DOWN" => ("down", "down"),
        "DPAD_LEFT" => ("left", "left"),
        "DPAD_RIGHT" => ("right", "right"),
        other => (other, other),
    };
    match labels {
        Labels::Xbox => xbox.to_owned(),
        Labels::PlayStation => ps.to_owned(),
    }
}

/// The drawing's size, scaled to the popup's width.
const CANVAS: Vec2 = Vec2::new(560.0, 330.0);

/// Draws the gamepad: the step being learned stands out, those set are
/// outlined, those pressed now (through the setup so far) are filled. Returns
/// the step of a part clicked.
fn draw_pad(ui: &mut egui::Ui, setup: &PadSetup, preview: &PadOutput) -> Option<usize> {
    let width = ui.available_width();
    let scale = width / CANVAS.x;
    let (rect, _) = ui.allocate_exact_size(CANVAS * scale, Sense::hover());
    let at = |x: f32, y: f32| rect.min + Vec2::new(x, y) * scale;
    let painter = ui.painter_at(rect);
    let time = ui.input(|i| i.time);
    let pulse = (0.5 + 0.5 * (time * 4.0).sin()) as f32;
    // A ring leaving the button to press, once a second.
    let wave = (time % 1.0) as f32;

    // The body: two grips and what joins them.
    let body = Color32::from_rgb(0x2a, 0x2f, 0x3a);
    for (x, y, r) in [(160.0, 215.0, 78.0), (400.0, 215.0, 78.0)] {
        painter.circle_filled(at(x, y), r * scale, body);
    }
    painter.rect_filled(Rect::from_min_max(at(110.0, 82.0), at(450.0, 220.0)), 60.0 * scale, body);

    let mut clicked = None;
    let mut part = |ui: &mut egui::Ui, name: &'static str, hit: Rect, draw: &dyn Fn(Look)| {
        let Some(step) = step_of(name) else { return };
        let look = Look {
            current: setup.learner.is_some() && setup.step == Some(step),
            set: setup.bind(step).is_some(),
            skipped: setup.skipped.contains(&step),
            pressed: pressed(name, preview),
            pulse,
        };
        if look.current {
            let ring = hit.expand((4.0 + 14.0 * wave) * scale);
            let color = ACCENT.gamma_multiply(1.0 - wave);
            painter.rect_stroke(ring, ring.height() / 2.0, Stroke::new(3.0 * scale, color), egui::StrokeKind::Middle);
        }
        draw(look);
        let response = ui.interact(hit, ui.id().with(("pad-part", name)), Sense::click());
        if setup.learner.is_some() && response.clicked() {
            clicked = Some(step);
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hover(step, setup));
    };

    // Triggers and bumpers.
    for (name, x) in [("LT", 125.0), ("RT", 355.0)] {
        let r = Rect::from_min_size(at(x, 8.0), Vec2::new(80.0, 30.0) * scale);
        let level = preview.axes[name] as f32;
        part(ui, name, r, &|look| {
            painter.rect_filled(r, 8.0 * scale, look.fill());
            if level > 0.02 {
                let filled = Rect::from_min_max(Pos2::new(r.min.x, r.max.y - r.height() * level), r.max);
                painter.rect_filled(filled, 8.0 * scale, PRESSED);
            }
            painter.rect_stroke(r, 8.0 * scale, look.stroke(scale), egui::StrokeKind::Inside);
            text(&painter, r.center(), short(name, setup.labels), 13.0 * scale, look.text());
        });
    }
    for (name, x) in [("LB", 115.0), ("RB", 345.0)] {
        let r = Rect::from_min_size(at(x, 46.0), Vec2::new(100.0, 22.0) * scale);
        part(ui, name, r, &|look| {
            painter.rect_filled(r, 10.0 * scale, look.fill());
            painter.rect_stroke(r, 10.0 * scale, look.stroke(scale), egui::StrokeKind::Inside);
            text(&painter, r.center(), short(name, setup.labels), 12.0 * scale, look.text());
        });
    }

    // Sticks: clicked in (LS, RS) and their axes (arrows right and down).
    for (stick, (x, y), (ax, ay)) in [("LS", (165.0, 130.0), ("LX", "LY")), ("RS", (330.0, 200.0), ("RX", "RY"))] {
        let center = at(x, y);
        let radius = 30.0 * scale;
        let hit = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
        let knob = center + Vec2::new(preview.axes[ax] as f32, preview.axes[ay] as f32) * 12.0 * scale;
        part(ui, stick, hit, &|look| {
            painter.circle_filled(center, radius, Color32::from_rgb(0x1b, 0x1e, 0x25));
            painter.circle_filled(knob, radius * 0.72, look.fill());
            painter.circle_stroke(knob, radius * 0.72, look.stroke(scale));
            text(&painter, knob, short(stick, setup.labels), 12.0 * scale, look.text());
        });
        for (axis, dir) in [(ax, Vec2::X), (ay, Vec2::Y)] {
            let tip = center + dir * (radius + 22.0 * scale);
            let hit = Rect::from_center_size(center + dir * (radius + 13.0 * scale), Vec2::splat(22.0 * scale));
            let moved = preview.axes[axis].abs() > 0.5;
            part(ui, axis, hit, &|look| {
                let look = Look { pressed: moved, ..look };
                arrow(&painter, center + dir * (radius + 4.0 * scale), tip, scale, look);
            });
        }
    }

    // D-pad.
    let pad_center = at(230.0, 200.0);
    for (name, dir) in [("DPAD_UP", -Vec2::Y), ("DPAD_DOWN", Vec2::Y), ("DPAD_LEFT", -Vec2::X), ("DPAD_RIGHT", Vec2::X)] {
        let r = Rect::from_center_size(pad_center + dir * 20.0 * scale, Vec2::splat(20.0 * scale));
        part(ui, name, r, &|look| {
            painter.rect_filled(r, 3.0 * scale, look.fill());
            painter.rect_stroke(r, 3.0 * scale, look.stroke(scale), egui::StrokeKind::Inside);
            let a = r.center() + dir * 5.0 * scale;
            let side = Vec2::new(dir.y, dir.x) * 4.0 * scale;
            painter.add(Shape::convex_polygon(vec![a, a - dir * 7.0 * scale + side, a - dir * 7.0 * scale - side], look.text(), Stroke::NONE));
        });
    }

    // Back, Home, Start.
    for (name, (x, y), w) in [("BACK", (240.0, 125.0), 28.0), ("START", (320.0, 125.0), 28.0)] {
        let r = Rect::from_center_size(at(x, y), Vec2::new(w, 16.0) * scale);
        part(ui, name, r, &|look| {
            painter.rect_filled(r, 8.0 * scale, look.fill());
            painter.rect_stroke(r, 8.0 * scale, look.stroke(scale), egui::StrokeKind::Inside);
        });
        text(&painter, at(x, y + 20.0), short(name, setup.labels), 10.5 * scale, MUTED);
    }
    let home = at(280.0, 95.0);
    part(ui, "GUIDE", Rect::from_center_size(home, Vec2::splat(30.0 * scale)), &|look| {
        painter.circle_filled(home, 15.0 * scale, look.fill());
        painter.circle_stroke(home, 15.0 * scale, look.stroke(scale));
        text(&painter, home, if setup.labels == Labels::Xbox { "⊗" } else { "PS" }, 12.0 * scale, look.text());
    });

    // Face buttons, in their colours.
    let face = at(400.0, 130.0);
    for (name, dir) in [("Y", -Vec2::Y), ("A", Vec2::Y), ("X", -Vec2::X), ("B", Vec2::X)] {
        let center = face + dir * 27.0 * scale;
        let radius = 15.0 * scale;
        let hit = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
        part(ui, name, hit, &|look| {
            painter.circle_filled(center, radius, look.fill());
            painter.circle_stroke(center, radius, look.stroke(scale));
            let color = if look.pressed { ON_ACCENT } else { face_color(name, setup.labels) };
            match setup.labels {
                Labels::Xbox => text(&painter, center, name, 15.0 * scale, color),
                Labels::PlayStation => ps_symbol(&painter, name, center, radius * 0.5, Stroke::new(2.0 * scale, color)),
            }
        });
    }
    clicked
}

/// How a part of the drawing looks.
#[derive(Clone, Copy)]
struct Look {
    current: bool,
    set: bool,
    skipped: bool,
    pressed: bool,
    pulse: f32,
}

const PRESSED: Color32 = GAME;

impl Look {
    fn fill(&self) -> Color32 {
        if self.pressed {
            PRESSED
        } else if self.current {
            RAISED.lerp_to_gamma(ACCENT, 0.25 + 0.35 * self.pulse)
        } else {
            RAISED
        }
    }

    fn stroke(&self, scale: f32) -> Stroke {
        if self.current {
            Stroke::new(3.0 * scale, ACCENT)
        } else if self.set {
            Stroke::new(1.5 * scale, OK)
        } else if self.skipped {
            Stroke::new(1.0 * scale, LINE)
        } else {
            Stroke::new(1.0 * scale, MUTED)
        }
    }

    fn text(&self) -> Color32 {
        if self.pressed {
            ON_ACCENT
        } else if self.skipped && !self.current {
            IDLE
        } else {
            TEXT
        }
    }
}

/// Whether a part is pressed now, through the setup so far.
fn pressed(name: &str, preview: &PadOutput) -> bool {
    match name {
        "LT" | "RT" => preview.axes[name] > 0.5,
        "LX" | "LY" | "RX" | "RY" => preview.axes[name].abs() > 0.5,
        _ => preview.held.contains(name),
    }
}

fn hover(step: usize, setup: &PadSetup) -> String {
    let name = label(STEPS[step].target.name(), setup.labels);
    let state = match setup.bind(step) {
        Some(_) => "set",
        None if setup.skipped.contains(&step) => "skipped",
        None => "not set yet",
    };
    format!("{name}: {state}. Click to set it again.")
}

/// The short name written on a part.
fn short(name: &'static str, labels: Labels) -> &'static str {
    match (name, labels) {
        (_, Labels::Xbox) => match name {
            "BACK" => "View",
            "START" => "Menu",
            other => other,
        },
        ("LB", _) => "L1",
        ("RB", _) => "R1",
        ("LT", _) => "L2",
        ("RT", _) => "R2",
        ("LS", _) => "L3",
        ("RS", _) => "R3",
        ("BACK", _) => "Share",
        ("START", _) => "Options",
        (other, _) => other,
    }
}

fn face_color(name: &str, labels: Labels) -> Color32 {
    match (labels, name) {
        (Labels::Xbox, "A") => Color32::from_rgb(0x6c, 0xc2, 0x4a),
        (Labels::Xbox, "B") => Color32::from_rgb(0xe8, 0x4a, 0x4a),
        (Labels::Xbox, "X") => Color32::from_rgb(0x3d, 0x8b, 0xf2),
        (Labels::Xbox, _) => Color32::from_rgb(0xf2, 0xc2, 0x3d),
        (Labels::PlayStation, "A") => Color32::from_rgb(0x7c, 0xa4, 0xf2),
        (Labels::PlayStation, "B") => Color32::from_rgb(0xf2, 0x6b, 0x6b),
        (Labels::PlayStation, "X") => Color32::from_rgb(0xe8, 0x8c, 0xd0),
        (Labels::PlayStation, _) => Color32::from_rgb(0x4a, 0xd0, 0xa8),
    }
}

/// ✕ ○ □ △, drawn (fonts may lack them).
fn ps_symbol(painter: &egui::Painter, name: &str, center: Pos2, r: f32, stroke: Stroke) {
    match name {
        "A" => {
            painter.line_segment([center + Vec2::splat(-r), center + Vec2::splat(r)], stroke);
            painter.line_segment([center + Vec2::new(-r, r), center + Vec2::new(r, -r)], stroke);
        }
        "B" => {
            painter.circle_stroke(center, r, stroke);
        }
        "X" => {
            painter.rect_stroke(Rect::from_center_size(center, Vec2::splat(r * 1.8)), 0.0, stroke, egui::StrokeKind::Middle);
        }
        _ => {
            let points = (0..3).map(|i| center + Vec2::angled(-PI / 2.0 + i as f32 * 2.0 * PI / 3.0) * r * 1.1).collect();
            painter.add(Shape::closed_line(points, stroke));
        }
    }
}

fn arrow(painter: &egui::Painter, from: Pos2, to: Pos2, scale: f32, look: Look) {
    let color = if look.pressed {
        PRESSED
    } else if look.current {
        ACCENT
    } else if look.set {
        OK
    } else {
        MUTED
    };
    let width = if look.current { 4.0 } else { 2.5 } * scale;
    let dir = (to - from).normalized();
    let side = Vec2::new(-dir.y, dir.x) * 6.0 * scale;
    painter.line_segment([from, to - dir * 6.0 * scale], Stroke::new(width, color));
    painter.add(Shape::convex_polygon(vec![to, to - dir * 9.0 * scale + side, to - dir * 9.0 * scale - side], color, Stroke::NONE));
}

fn text(painter: &egui::Painter, at: Pos2, text: &str, size: f32, color: Color32) {
    painter.text(at, egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(size), color);
}
