//! Colors, egui style and the small drawing helpers shared by the pages.

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

pub const BG: Color32 = Color32::from_rgb(0x12, 0x14, 0x19);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x16, 0x18, 0x1e);
pub const PANEL: Color32 = Color32::from_rgb(0x1a, 0x1d, 0x24);
pub const RAISED: Color32 = Color32::from_rgb(0x22, 0x26, 0x2f);
pub const LINE: Color32 = Color32::from_rgb(0x2b, 0x30, 0x3c);
pub const TEXT: Color32 = Color32::from_rgb(0xe7, 0xe9, 0xee);
pub const MUTED: Color32 = Color32::from_rgb(0xa3, 0xaa, 0xb8);
pub const ACCENT: Color32 = Color32::from_rgb(0xf0, 0x6d, 0x94);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xff, 0x9d, 0xb8);
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x2a, 0x0a, 0x14);
pub const SELECTED_BG: Color32 = Color32::from_rgb(0x21, 0x1a, 0x20);
pub const OK: Color32 = Color32::from_rgb(0x46, 0xc0, 0x8a);
pub const WARN: Color32 = Color32::from_rgb(0xe9, 0xb0, 0x4a);
pub const DANGER: Color32 = Color32::from_rgb(0xc4, 0x34, 0x3b);
pub const DANGER_TEXT: Color32 = Color32::from_rgb(0xff, 0x7a, 0x7f);
pub const GAME: Color32 = Color32::from_rgb(0x6a, 0xa8, 0xff);
pub const IDLE: Color32 = Color32::from_rgb(0x5b, 0x62, 0x73);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.extreme_bg_color = SIDEBAR;
    visuals.faint_bg_color = PANEL;
    visuals.code_bg_color = SIDEBAR;
    visuals.hyperlink_color = ACCENT_TEXT;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, ON_ACCENT);
    visuals.slider_trailing_fill = true;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    for (state, fill) in [
        (&mut widgets.inactive, RAISED),
        (&mut widgets.hovered, Color32::from_rgb(0x2c, 0x31, 0x3c)),
        (&mut widgets.active, Color32::from_rgb(0x33, 0x39, 0x46)),
        (&mut widgets.open, RAISED),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::same(8);
    }
    widgets.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x36, 0x3c, 0x4a));
    widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 6.0);
        style.spacing.interact_size.y = 26.0;
    });
}

pub fn card(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
}

/// Small uppercase section title.
pub fn eyebrow(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(11.0).strong().color(MUTED));
}

pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(19.0).strong().color(TEXT));
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).strong().color(ON_ACCENT)).fill(ACCENT).min_size(Vec2::new(0.0, 34.0))
}

pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// Rounded badge.
pub fn pill(ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
    egui::Frame::new().fill(bg).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(11.5).strong().color(fg));
    });
}

/// Horizontal level bar.
pub fn meter(ui: &mut egui::Ui, width: f32, level: f64, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(4), LINE);
    let mut filled = rect;
    filled.set_width(rect.width() * level.clamp(0.0, 1.0) as f32);
    if filled.width() > 0.5 {
        painter.rect_filled(filled, CornerRadius::same(4), color);
    }
}

/// Gamepad control lit by `level` (0..1): a button, or a trigger's travel.
pub fn pad_chip(ui: &mut egui::Ui, label: &str, level: f64) {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(11.5), TEXT);
    let size = Vec2::new((galley.size().x + 14.0).max(26.0), 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    let level = level.clamp(0.0, 1.0) as f32;
    painter.rect_filled(rect, CornerRadius::same(6), RAISED);
    if level > 0.0 {
        let mut filled = rect;
        filled.set_top(rect.bottom() - rect.height() * level);
        painter.rect_filled(filled, CornerRadius::same(6), ACCENT);
    }
    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    let color = if level >= 0.5 { ON_ACCENT } else { MUTED };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(11.5), color);
}

/// Stick position (-1..1 on both axes, y down) in a small circle.
pub fn stick(ui: &mut egui::Ui, label: &str, x: f64, y: f64) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
    let painter = ui.painter();
    let radius = rect.width() / 2.0 - 1.0;
    painter.circle(rect.center(), radius, RAISED, Stroke::new(1.0, LINE));
    let moved = x.abs() > 0.0 || y.abs() > 0.0;
    let offset = Vec2::new(x.clamp(-1.0, 1.0) as f32, y.clamp(-1.0, 1.0) as f32) * (radius - 4.0);
    painter.circle_filled(rect.center() + offset, 4.0, if moved { ACCENT } else { MUTED });
    response.on_hover_text(label);
}

/// Line chart of `points` ((seconds ago <= 0, value 0..1)) over the last `span` seconds.
pub fn sparkline(ui: &mut egui::Ui, size: Vec2, span: f64, points: &[(f64, f64)], color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), BG);
    let to_screen = |(t, v): (f64, f64)| {
        let x = rect.right() + (t / span) as f32 * rect.width();
        let y = rect.bottom() - 2.0 - v.clamp(0.0, 1.0) as f32 * (rect.height() - 4.0);
        Pos2::new(x, y)
    };
    let line: Vec<Pos2> = points.iter().map(|p| to_screen(*p)).collect();
    let area = color.gamma_multiply(0.18);
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let quad = vec![Pos2::new(a.x, rect.bottom()), a, b, Pos2::new(b.x, rect.bottom())];
        painter.add(egui::Shape::convex_polygon(quad, area, Stroke::NONE));
    }
    if line.len() > 1 {
        painter.line(line, Stroke::new(1.6, color));
    }
}

/// A clickable card with an icon, a title, a subtitle and wrapped text below.
pub struct Tile<'a> {
    pub icon: &'a str,
    pub title: &'a str,
    pub subtitle: &'a str,
    pub body: &'a str,
    pub selected: bool,
    pub badge: Option<&'a str>,
}

impl Tile<'_> {
    pub fn show(self, ui: &mut egui::Ui, size: Vec2) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click());
        let painter = ui.painter_at(rect.expand(1.0));
        let (fill, stroke) = match (self.selected, response.hovered()) {
            (true, _) => (SELECTED_BG, Stroke::new(2.0, ACCENT)),
            (false, true) => (PANEL, Stroke::new(1.0, MUTED)),
            (false, false) => (PANEL, Stroke::new(1.0, LINE)),
        };
        painter.rect(rect, CornerRadius::same(12), fill, stroke, StrokeKind::Inside);

        let inner = rect.shrink(14.0);
        let icon = Rect::from_min_size(inner.min, Vec2::splat(34.0));
        painter.rect_filled(icon, CornerRadius::same(9), RAISED);
        painter.text(icon.center(), egui::Align2::CENTER_CENTER, self.icon, FontId::proportional(17.0), ACCENT);

        let text_left = icon.right() + 10.0;
        let mut title_width = inner.right() - text_left;
        if let Some(badge) = self.badge {
            let galley = painter.layout_no_wrap(badge.to_owned(), FontId::proportional(11.0), ACCENT_TEXT);
            let badge_rect = Rect::from_min_size(
                Pos2::new(inner.right() - galley.size().x - 16.0, inner.top() + 2.0),
                galley.size() + Vec2::new(16.0, 6.0),
            );
            painter.rect_filled(badge_rect, CornerRadius::same(9), Color32::from_rgb(0x3a, 0x1d, 0x28));
            painter.galley(badge_rect.min + Vec2::new(8.0, 3.0), galley, ACCENT_TEXT);
            title_width -= badge_rect.width() + 6.0;
        }
        let title = painter.layout(self.title.to_owned(), FontId::proportional(15.0), TEXT, title_width.max(20.0));
        painter.galley(Pos2::new(text_left, inner.top() - 1.0), title, TEXT);
        let subtitle = painter.layout(self.subtitle.to_owned(), FontId::proportional(12.0), MUTED, title_width.max(20.0));
        painter.galley(Pos2::new(text_left, inner.top() + 18.0), subtitle, MUTED);

        if !self.body.is_empty() {
            let top = icon.bottom() + 8.0;
            let mut job = egui::text::LayoutJob::simple(self.body.to_owned(), FontId::proportional(12.5), MUTED, inner.width());
            job.wrap.max_rows = ((inner.bottom() - top) / 16.0).floor().max(1.0) as usize;
            painter.galley(Pos2::new(inner.left(), top), painter.layout_job(job), MUTED);
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
}

/// Lays out tiles in a grid filling the available width.
pub fn tile_grid(ui: &mut egui::Ui, count: usize, min_width: f32, height: f32, mut tile: impl FnMut(&mut egui::Ui, usize, Vec2)) {
    let gap = 12.0;
    let width = ui.available_width();
    let columns = (((width + gap) / (min_width + gap)).floor() as usize).clamp(1, count.max(1));
    let tile_width = (width - gap * (columns - 1) as f32) / columns as f32;
    for row in (0..count).collect::<Vec<_>>().chunks(columns) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for &i in row {
                let size = Vec2::new(tile_width, height);
                ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Min), |ui| tile(ui, i, size));
            }
        });
        ui.add_space(gap - ui.spacing().item_spacing.y);
    }
}
