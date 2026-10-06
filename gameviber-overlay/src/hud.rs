//! Lays out the overlay panel from an [`OverlayState`] with epaint, the
//! painting library behind the GameViber window, and tessellates it into
//! meshes for the renderer.

use epaint::text::{FontDefinitions, Fonts, FontsView, LayoutJob, TextOptions, TextWrapping};
use epaint::{
    emath::Align2, ClippedPrimitive, ClippedShape, Color32, CornerRadius, FontId, ImageDelta, Pos2, Rect, Shape, Stroke,
    StrokeKind, TessellationOptions, Tessellator, Vec2,
};
use gameviber_common::overlay::{Corner, OverlayState};

// Colors of the GameViber theme.
const PANEL: Color32 = Color32::from_rgb(0x16, 0x18, 0x1e);
const LINE: Color32 = Color32::from_rgb(0x2b, 0x30, 0x3c);
const TEXT: Color32 = Color32::from_rgb(0xe7, 0xe9, 0xee);
const MUTED: Color32 = Color32::from_rgb(0xa3, 0xaa, 0xb8);
const ACCENT: Color32 = Color32::from_rgb(0xf0, 0x6d, 0x94);
const ACCENT_TEXT: Color32 = Color32::from_rgb(0xff, 0x9d, 0xb8);
const GAME: Color32 = Color32::from_rgb(0x6a, 0xa8, 0xff);
const WARN: Color32 = Color32::from_rgb(0xe9, 0xb0, 0x4a);
const DANGER: Color32 = Color32::from_rgb(0xc4, 0x34, 0x3b);

/// Panel size in points; points are 1080p pixels times the user scale.
const WIDTH: f32 = 240.0;
const PADDING: f32 = 12.0;
const MARGIN: f32 = 20.0;
/// The mode name is highlighted this long after a change.
const MODE_HIGHLIGHT_SECS: f32 = 4.0;
/// Events stay this long, the last part fading out.
const EVENT_SECS: f32 = 2.5;
const EVENT_FADE_SECS: f32 = 1.0;
const MAX_EVENTS: usize = 3;

pub struct Frame {
    pub primitives: Vec<ClippedPrimitive>,
    /// Change to the font texture (managed texture 0) since the last frame.
    pub texture: Option<ImageDelta>,
    pub pixels_per_point: f32,
}

pub struct Hud {
    fonts: Fonts,
}

impl Hud {
    pub fn new() -> Self {
        Self { fonts: Fonts::new(TextOptions::default(), FontDefinitions::default()) }
    }

    /// Meshes of the overlay for a `width` x `height` pixels image.
    pub fn build(&mut self, state: &OverlayState, width: u32, height: u32) -> Frame {
        let ppp = state.scale.clamp(0.25, 4.0) * (height as f32 / 1080.0).max(0.5);
        self.fonts.begin_pass(TextOptions::default());
        let screen = Vec2::new(width as f32, height as f32) / ppp;
        let shapes = if state.visible { panel(&mut self.fonts.with_pixels_per_point(ppp), state, screen) } else { Vec::new() };
        let clip = Rect::from_min_size(Pos2::ZERO, screen);
        let clipped = shapes.into_iter().map(|shape| ClippedShape { clip_rect: clip, shape }).collect();
        let mut tessellator = Tessellator::new(
            ppp,
            TessellationOptions::default(),
            self.fonts.font_image_size(),
            self.fonts.texture_atlas().prepared_discs(),
        );
        let primitives = tessellator.tessellate_shapes(clipped);
        Frame { primitives, texture: self.fonts.font_image_delta(), pixels_per_point: ppp }
    }
}

/// Builds the panel top-down at the origin, then moves it to its corner.
fn panel(fonts: &mut FontsView<'_>, state: &OverlayState, screen: Vec2) -> Vec<Shape> {
    let mut p = Painter { fonts, shapes: Vec::new(), y: PADDING };
    let inner = WIDTH - 2.0 * PADDING;

    // The mode and preset on one short line, so that the phase stands out.
    let fresh = state.mode_age < MODE_HIGHLIGHT_SECS;
    let (size, color) = if fresh { (15.0, ACCENT_TEXT) } else { (12.0, MUTED) };
    let title = match &state.preset {
        Some(preset) => format!("{} · {preset}", state.mode),
        None => state.mode.clone(),
    };
    p.line(&title, size, color, inner);
    if let Some(phase) = &state.phase {
        p.phase(phase);
    }
    p.y += 6.0;

    if state.panic {
        let rect = Rect::from_min_size(Pos2::new(PADDING, p.y), Vec2::new(inner, 28.0));
        p.shapes.push(Shape::rect_filled(rect, CornerRadius::same(6), DANGER));
        p.text_at(rect.center(), Align2::CENTER_CENTER, "STOPPED", 15.0, Color32::WHITE);
        p.y = rect.bottom() + 8.0;
    } else {
        p.bar("Output", state.output, 1.0, Some(state.cap), ACCENT, inner);
    }
    for gauge in &state.gauges {
        p.bar(&gauge.label, gauge.value, gauge.max, None, GAME, inner);
    }

    let recent = state.events.iter().filter(|e| e.age < EVENT_SECS);
    let recent: Vec<_> = recent.collect();
    for event in recent.iter().rev().take(MAX_EVENTS).rev() {
        let alpha = ((EVENT_SECS - event.age) / EVENT_FADE_SECS).clamp(0.0, 1.0);
        p.text(&event.text, 17.0, ACCENT_TEXT.gamma_multiply(alpha), inner);
    }
    for alert in &state.alerts {
        p.text(alert, 12.5, WARN, inner);
    }

    let size = Vec2::new(WIDTH, p.y + PADDING - 4.0);
    let mut shapes = vec![
        Shape::rect_filled(Rect::from_min_size(Pos2::ZERO, size), CornerRadius::same(10), PANEL.gamma_multiply(state.opacity.clamp(0.0, 1.0))),
        Shape::rect_stroke(Rect::from_min_size(Pos2::ZERO, size), CornerRadius::same(10), Stroke::new(1.0, LINE), StrokeKind::Inside),
    ];
    shapes.append(&mut p.shapes);
    let origin = match state.corner {
        Corner::TopLeft => Vec2::new(MARGIN, MARGIN),
        Corner::TopRight => Vec2::new(screen.x - MARGIN - size.x, MARGIN),
        Corner::BottomLeft => Vec2::new(MARGIN, screen.y - MARGIN - size.y),
        Corner::BottomRight => screen - Vec2::splat(MARGIN) - size,
    };
    for shape in &mut shapes {
        shape.translate(origin);
    }
    shapes
}

struct Painter<'a, 'b> {
    fonts: &'a mut FontsView<'b>,
    shapes: Vec<Shape>,
    /// Top of the next row.
    y: f32,
}

impl Painter<'_, '_> {
    /// A wrapped text row.
    fn text(&mut self, text: &str, size: f32, color: Color32, width: f32) {
        let galley = self.fonts.layout(text.to_owned(), FontId::proportional(size), color, width);
        let height = galley.size().y;
        self.shapes.push(Shape::galley(Pos2::new(PADDING, self.y), galley, color));
        self.y += height + 2.0;
    }

    /// A single text row, cut with "…" when too long.
    fn line(&mut self, text: &str, size: f32, color: Color32, width: f32) {
        let mut job = LayoutJob::simple_singleline(text.to_owned(), FontId::proportional(size), color);
        job.wrap = TextWrapping { max_width: width, max_rows: 1, break_anywhere: true, ..TextWrapping::default() };
        let galley = self.fonts.layout_job(job);
        let height = galley.size().y;
        self.shapes.push(Shape::galley(Pos2::new(PADDING, self.y), galley, color));
        self.y += height + 2.0;
    }

    /// The current phase, with a dot, larger than the mode name.
    fn phase(&mut self, phase: &str) {
        let mut name: String = phase.chars().take(1).flat_map(char::to_uppercase).collect();
        name.extend(phase.chars().skip(1).map(|c| if c == '_' { ' ' } else { c }));
        self.y += 2.0;
        self.shapes.push(Shape::circle_filled(Pos2::new(PADDING + 4.0, self.y + 10.0), 4.0, GAME));
        let mut job = LayoutJob::simple_singleline(name, FontId::proportional(17.0), TEXT);
        job.wrap = TextWrapping { max_width: WIDTH - 2.0 * PADDING - 14.0, max_rows: 1, break_anywhere: true, ..TextWrapping::default() };
        let galley = self.fonts.layout_job(job);
        let height = galley.size().y;
        self.shapes.push(Shape::galley(Pos2::new(PADDING + 14.0, self.y), galley, TEXT));
        self.y += height + 2.0;
    }

    fn text_at(&mut self, pos: Pos2, anchor: Align2, text: &str, size: f32, color: Color32) {
        let shape = Shape::text(self.fonts, pos, anchor, text, FontId::proportional(size), color);
        self.shapes.push(shape);
    }

    /// A labelled level bar, with an optional marker (the global cap).
    fn bar(&mut self, label: &str, value: f32, max: f32, marker: Option<f32>, color: Color32, width: f32) {
        let fraction = if max > 0.0 { (value / max).clamp(0.0, 1.0) } else { 0.0 };
        self.text_at(Pos2::new(PADDING, self.y), Align2::LEFT_TOP, label, 12.0, MUTED);
        let shown = if max == 1.0 { format!("{:.0}%", fraction * 100.0) } else { format!("{value:.0} / {max:.0}") };
        self.text_at(Pos2::new(PADDING + width, self.y), Align2::RIGHT_TOP, &shown, 12.0, TEXT);
        self.y += 17.0;
        let track = Rect::from_min_size(Pos2::new(PADDING, self.y), Vec2::new(width, 8.0));
        self.shapes.push(Shape::rect_filled(track, CornerRadius::same(4), LINE));
        if fraction > 0.0 {
            let fill = Rect::from_min_size(track.min, Vec2::new(width * fraction, track.height()));
            self.shapes.push(Shape::rect_filled(fill, CornerRadius::same(4), color));
        }
        if let Some(marker) = marker.filter(|m| *m < 1.0) {
            let x = track.left() + width * marker.clamp(0.0, 1.0);
            self.shapes.push(Shape::line_segment([Pos2::new(x, track.top() - 2.0), Pos2::new(x, track.bottom() + 2.0)], Stroke::new(2.0, TEXT)));
        }
        self.y = track.bottom() + 8.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gameviber_common::overlay::{Event, Gauge};

    #[test]
    fn panel_fits_in_its_corner() {
        let mut hud = Hud::new();
        let state = OverlayState {
            mode: "Surge".into(),
            preset: Some("Lost Crown with a very long preset name that cannot fit".into()),
            phase: Some("battle".into()),
            gauges: vec![Gauge { label: "Surge gauge".into(), value: 40.0, max: 100.0 }],
            events: vec![Event { text: "Parry!".into(), age: 0.5 }],
            alerts: vec!["No toy connected".into()],
            corner: Corner::BottomRight,
            ..OverlayState::default()
        };
        let frame = hud.build(&state, 1920, 1080);
        assert!(frame.texture.is_some(), "font atlas uploaded on the first frame");
        let bounds = frame.primitives.iter().fold(Rect::NOTHING, |r, p| match &p.primitive {
            epaint::Primitive::Mesh(m) => r.union(m.calc_bounds()),
            epaint::Primitive::Callback(_) => r,
        });
        assert!(bounds.max.x <= 1920.0 - MARGIN + 0.5 && bounds.max.y <= 1080.0 - MARGIN + 0.5, "{bounds:?}");
        assert!(bounds.width() > WIDTH - 1.0);
        let hidden = hud.build(&OverlayState { visible: false, ..state }, 1920, 1080);
        assert!(hidden.primitives.is_empty());
    }
}
