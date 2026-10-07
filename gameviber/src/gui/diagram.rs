//! "How a mode works": a diagram of what a mode reads (rumble and buttons,
//! sound, image, indicators, other programs), the phases told apart from
//! them, the script turning it all into vibrations and the toys. Shown in
//! full when creating a mode, and from the Creator's help button.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};

use super::theme::*;

/// The diagram's size at full scale.
const SIZE: Vec2 = Vec2::new(1060.0, 464.0);
/// What is needed: the script.
const NEEDED: Color32 = ACCENT;
/// What is recommended (the phases), and what tells them apart.
const RECOMMENDED: Color32 = GAME;
const ARROW: Color32 = IDLE;

/// A box of the diagram.
#[derive(PartialEq, Clone, Copy)]
pub enum Part {
    Rumble,
    Sound,
    Image,
    Indicators,
    Programs,
    Phases,
    Script,
    Toys,
}

struct Box {
    part: Part,
    rect: [f32; 4],
    title: &'static str,
    tag: &'static str,
    tag_color: Color32,
    border: Color32,
    fill: Color32,
    dashed: bool,
}

const BOXES: [Box; 8] = [
    Box { part: Part::Rumble, rect: [16.0, 24.0, 224.0, 64.0], title: "Rumble & buttons", tag: "always · nothing to do", tag_color: OK, border: LINE, fill: PANEL, dashed: false },
    Box { part: Part::Sound, rect: [16.0, 112.0, 224.0, 64.0], title: "Sound", tag: "automatic", tag_color: OK, border: LINE, fill: PANEL, dashed: false },
    Box { part: Part::Image, rect: [16.0, 200.0, 224.0, 64.0], title: "Image", tag: "captures · optional", tag_color: MUTED, border: LINE, fill: PANEL, dashed: false },
    Box { part: Part::Indicators, rect: [16.0, 288.0, 224.0, 64.0], title: "Indicators", tag: "optional", tag_color: MUTED, border: LINE, fill: PANEL, dashed: false },
    Box { part: Part::Programs, rect: [16.0, 376.0, 224.0, 64.0], title: "Other programs", tag: "optional · experts", tag_color: MUTED, border: LINE, fill: PANEL, dashed: false },
    Box { part: Part::Phases, rect: [332.0, 192.0, 208.0, 80.0], title: "Phases", tag: "recommended", tag_color: RECOMMENDED, border: RECOMMENDED, fill: PANEL, dashed: false },
    Box { part: Part::Script, rect: [680.0, 24.0, 176.0, 416.0], title: "Script", tag: "needed", tag_color: NEEDED, border: NEEDED, fill: RAISED, dashed: false },
    Box { part: Part::Toys, rect: [924.0, 192.0, 120.0, 80.0], title: "Your toys", tag: "capped", tag_color: MUTED, border: LINE, fill: BG, dashed: true },
];

/// The arrows, as the points they go through: the last one is the tip.
const ARROWS: [(&[[f32; 2]], Color32); 9] = [
    (&[[240.0, 56.0], [680.0, 56.0]], ARROW),
    (&[[240.0, 133.0], [680.0, 133.0]], ARROW),
    (&[[240.0, 155.0], [410.0, 155.0], [410.0, 192.0]], RECOMMENDED),
    (&[[240.0, 232.0], [332.0, 232.0]], RECOMMENDED),
    (&[[240.0, 309.0], [470.0, 309.0], [470.0, 272.0]], RECOMMENDED),
    (&[[240.0, 331.0], [680.0, 331.0]], ARROW),
    (&[[240.0, 408.0], [680.0, 408.0]], ARROW),
    (&[[540.0, 232.0], [680.0, 232.0]], RECOMMENDED),
    (&[[856.0, 232.0], [924.0, 232.0]], ARROW),
];

/// Draws the diagram across the width available (smaller in a narrow
/// window), `here` outlined as the part being edited.
pub fn mode_diagram(ui: &mut egui::Ui, here: Option<Part>) {
    let scale = (ui.available_width() / SIZE.x).clamp(0.55, 1.0);
    let (area, _) = ui.allocate_exact_size(SIZE * scale, egui::Sense::hover());
    let painter = ui.painter_at(area);
    let at = |x: f32, y: f32| area.min + Vec2::new(x, y) * scale;
    for (points, color) in ARROWS {
        let points: Vec<Pos2> = points.iter().map(|[x, y]| at(*x, *y)).collect();
        arrow(&painter, &points, color, scale);
    }
    for b in &BOXES {
        let [x, y, w, h] = b.rect;
        let rect = Rect::from_min_size(at(x, y), Vec2::new(w, h) * scale);
        let current = here == Some(b.part);
        painter.rect_filled(rect, 10.0 * scale, b.fill);
        if b.dashed {
            let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
            painter.extend(egui::Shape::dashed_line(&corners, Stroke::new(1.0, b.border), 6.0 * scale, 4.0 * scale));
        } else {
            let stroke = if current { Stroke::new(2.5, TEXT) } else { Stroke::new(1.0, b.border) };
            painter.rect_stroke(rect, 10.0 * scale, stroke, egui::StrokeKind::Inside);
        }
        // Inputs read left-aligned, the rest centred.
        let left = rect.width() > 200.0 * scale && b.part != Part::Phases;
        let (anchor, align) = if left { (rect.left_center() + Vec2::new(12.0 * scale, 0.0), Align2::LEFT_CENTER) } else { (rect.center(), Align2::CENTER_CENTER) };
        let line = 20.0 * scale;
        let mut lines: Vec<(&str, f32, Color32)> = vec![(b.title, 15.5, TEXT), (b.tag, 12.0, b.tag_color)];
        if b.part == Part::Script {
            lines.extend([("", 12.0, MUTED), ("turns what it reads", 12.0, MUTED), ("into vibrations", 12.0, MUTED)]);
        }
        if current {
            lines.insert(0, ("you are here", 11.0, TEXT));
        }
        let top = anchor.y - line * (lines.len() as f32 - 1.0) / 2.0;
        for (i, (text, size, color)) in lines.iter().enumerate() {
            let pos = Pos2::new(anchor.x, top + line * i as f32);
            painter.text(pos, align, *text, FontId::proportional(size * scale), *color);
        }
    }
    painter.text(at(436.0, 36.0), Align2::CENTER_CENTER, "the peaks", FontId::proportional(13.0 * scale), MUTED);
    painter.text(at(610.0, 214.0), Align2::CENTER_CENTER, "the mood", FontId::proportional(13.0 * scale), RECOMMENDED);
}

/// A line through `points`, with a head on its last point.
fn arrow(painter: &egui::Painter, points: &[Pos2], color: Color32, scale: f32) {
    let stroke = Stroke::new(2.0, color);
    let [.., from, tip] = points else { return };
    let direction = (*tip - *from).normalized();
    let head = 9.0 * scale;
    let mut line = points.to_vec();
    if let Some(last) = line.last_mut() {
        *last = *tip - direction * head;
    }
    painter.add(egui::Shape::line(line, stroke));
    let side = direction.rot90() * head * 0.55;
    let base = *tip - direction * head;
    painter.add(egui::Shape::convex_polygon(vec![*tip, base + side, base - side], color, Stroke::NONE));
}
