//! The indicator selected checked on every image of its game's recordings
//! (the Captures column's In recordings view): an indicator set up on a few
//! dozen captures can read wrong on scenes none of them show. How far each
//! image reads from the threshold, as a histogram; the images of the bars
//! picked (at first those nearest the threshold), each of which can be added
//! to the captures, marked as showing the indicator or not.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::{color_image, drawn_capture, on_image, outline_line, tag, Scored};
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::gui::sessions::{clock, game_recordings};
use crate::gui::theme::*;
use crate::gui::App;
use crate::package::{IndicatorKind, Inputs, Outline, Reading, Zone};
use crate::screen::indicators;
use crate::session::{self, FrameData};

/// Bars of the histogram, half of them on each side of the threshold.
const BARS: i32 = 24;
/// Images shown at most for the bars picked, spread over them.
const SHOWN: usize = 24;
/// Height of the histogram.
const HISTOGRAM_HEIGHT: f32 = 64.0;
/// Images kept loaded beyond those shown.
const KEPT_TEXTURES: usize = 120;
/// The zones are shown with this much of the image around them, each way (shares of their size).
const AROUND: f32 = 0.4;

#[derive(Default)]
pub(in crate::gui) struct Check {
    /// What `images` were read with...
    key: Option<Key>,
    /// ...the recordings' titles...
    recordings: Vec<String>,
    /// ...and each image read.
    images: Vec<Image>,
    /// The reading under way.
    job: Option<Job>,
    /// The bars picked (from, to), numbered from the threshold (0: just above
    /// it, -1: just below); None: the four nearest it.
    bars: Option<(i32, i32)>,
    /// Images loaded to be shown, by file, and those being loaded.
    textures: HashMap<PathBuf, egui::TextureHandle>,
    loading: Option<mpsc::Receiver<Vec<(PathBuf, egui::ColorImage)>>>,
}

/// The zones read (without their thresholds and marks: they change no
/// reading) and the recordings, with how many images each had.
#[derive(Clone, PartialEq)]
struct Key {
    zones: Vec<Zone>,
    recordings: Vec<(PathBuf, u32)>,
}

/// An image of a recording, and what each zone read on it.
struct Image {
    recording: usize,
    path: PathBuf,
    t: f64,
    measures: Vec<f32>,
}

struct Job {
    key: Key,
    titles: Vec<String>,
    done: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    rx: mpsc::Receiver<Vec<Image>>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Key {
    fn of(zones: &Scored, recordings: Vec<(PathBuf, u32)>) -> Self {
        let zones = zones
            .iter()
            .map(|(_, z)| Zone { threshold: 0.0, shown_on: Vec::new(), hidden_on: Vec::new(), capture: None, phase: None, ..z.clone() })
            .collect();
        Key { zones, recordings }
    }
}

/// Reads `key`'s zones on every image of its recordings, on all cores but one.
fn start(key: Key, titles: Vec<String>, ctx: &egui::Context) -> Job {
    let (tx, rx) = mpsc::channel();
    let (done, total, stop) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)), Arc::new(AtomicBool::new(false)));
    let (zones, paths) = (key.zones.clone(), key.recordings.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>());
    let (job_done, job_total, job_stop, ctx) = (done.clone(), total.clone(), stop.clone(), ctx.clone());
    std::thread::spawn(move || {
        let listed: Vec<(usize, session::SessionFrame)> =
            paths.iter().enumerate().flat_map(|(i, path)| session::images(path).into_iter().map(move |f| (i, f))).collect();
        job_total.store(listed.len(), Ordering::Relaxed);
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1).max(1));
        let chunk = listed.len().div_ceil(threads).max(1);
        let images: Vec<Image> = std::thread::scope(|scope| {
            let workers: Vec<_> = listed
                .chunks(chunk)
                .map(|part| {
                    let (zones, done, stop) = (&zones, &job_done, &job_stop);
                    scope.spawn(move || {
                        part.iter()
                            .filter_map(|(recording, image)| {
                                if stop.load(Ordering::Relaxed) {
                                    return None;
                                }
                                done.fetch_add(1, Ordering::Relaxed);
                                let FrameData::File(path) = &image.data else { return None };
                                let frame = image.data.load().ok()?;
                                let measures = zones.iter().map(|z| indicators::measure(z, &frame).unwrap_or(0.0)).collect();
                                Some(Image { recording: *recording, path: path.clone(), t: image.t, measures })
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
        });
        if !job_stop.load(Ordering::Relaxed) {
            let _ = tx.send(images);
        }
        ctx.request_repaint();
    });
    Job { key, titles, done, total, stop, rx }
}

/// The capture an image of a recording becomes: named after the recording and its time.
fn capture_name(recording: &Path, t: f64) -> String {
    let stem = recording.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let stem = stem.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    format!("{stem}-{:09}.png", (t * 1000.0).round().max(0.0) as u64)
}

/// The bar of the histogram a margin falls in, bars being `width` wide.
fn bar_of(margin: f32, width: f32) -> i32 {
    ((margin / width).floor() as i32).clamp(-BARS / 2, BARS / 2 - 1)
}

/// The part of the image (shares of it) the zones are shown in: around them,
/// square on an image `aspect` times as wide as high.
fn crop(zones: &Scored, aspect: f32) -> [f32; 4] {
    let (mut x0, mut y0, mut x1, mut y1) = (1f32, 1f32, 0f32, 0f32);
    for (_, z) in zones {
        let [x, y, w, h] = z.rect;
        (x0, y0, x1, y1) = (x0.min(x - w * AROUND), y0.min(y - h * AROUND), x1.max(x + w * (1.0 + AROUND)), y1.max(y + h * (1.0 + AROUND)));
    }
    let side = ((x1 - x0) * aspect).max(y1 - y0).clamp(0.05, 1.0);
    let (w, h) = ((side / aspect).min(1.0), side);
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    [(cx - w / 2.0).clamp(0.0, 1.0 - w), (cy - h / 2.0).clamp(0.0, 1.0 - h), w, h]
}

impl App {
    /// The Captures column's In recordings view: `scored` read on every image of the game's recordings.
    pub(in crate::gui) fn recordings_check(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, inputs: &Inputs, scored: &Scored) {
        let Some((_, first)) = scored.first() else {
            ui.label(muted("Open a visibility indicator (on the right) to see what it reads on every image of your recordings."));
            return;
        };
        if first.kind != IndicatorKind::Visibility {
            ui.label(muted("Only visibility indicators are checked on the recordings for now."));
            return;
        }
        let recorded: Vec<(PathBuf, u32, String)> =
            game_recordings(s, &game.name).map(|r| (r.path.clone(), r.header.frames, r.title().to_owned())).collect();
        if recorded.is_empty() {
            ui.label(muted(format!(
                "No recording of {} with images yet: record a few sessions (Sessions tab), playing through as many scenes as you can.",
                game.name
            )));
            return;
        }
        ui.label(
            muted("What the indicator reads on every image of your recordings of this game, not only on the captures: the scenes it reads wrong show up here.")
                .size(11.5),
        );
        let ctx = ui.ctx().clone();
        let key = Key::of(scored, recorded.iter().map(|(p, n, _)| (p.clone(), *n)).collect());
        let check = &mut self.screen.check;
        // The reading ended, or another is needed (the latest wins: a change cancels the one under way).
        if let Some(job) = &check.job {
            match job.rx.try_recv() {
                Ok(images) => {
                    let job = check.job.take().expect("a job");
                    (check.key, check.recordings, check.images) = (Some(job.key.clone()), job.titles.clone(), images);
                }
                Err(mpsc::TryRecvError::Disconnected) => check.job = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let current = check.key.as_ref() == Some(&key);
        if !current && check.job.as_ref().is_none_or(|j| j.key != key) {
            check.job = Some(start(key.clone(), recorded.iter().map(|(_, _, t)| t.clone()).collect(), &ctx));
        }
        if let Some(job) = &check.job {
            let (done, total) = (job.done.load(Ordering::Relaxed), job.total.load(Ordering::Relaxed).max(1));
            ui.add(egui::ProgressBar::new(done as f32 / total as f32).desired_height(14.0).text(RichText::new(format!("Reading {done} of {total} images...")).size(11.0)));
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        // Results read with other zones (one added, removed) cannot be shown with these.
        if check.images.is_empty() || check.images[0].measures.len() != scored.len() {
            return;
        }
        let zones: Vec<&Zone> = scored.iter().map(|(_, z)| z).collect();
        let margins: Vec<(f32, usize)> = check.images.iter().map(|i| indicators::margin(&zones, &i.measures)).collect();
        let shown = margins.iter().filter(|(m, _)| *m >= 0.0).count();
        ui.label(
            RichText::new(format!(
                "{} images of {} recordings: {shown} read shown, {} hidden.",
                check.images.len(),
                check.recordings.len(),
                check.images.len() - shown
            ))
            .size(12.0),
        );

        // The histogram: how far from the threshold each image reads.
        let widest = margins.iter().map(|(m, _)| m.abs()).fold(0.01f32, f32::max);
        let width = widest / (BARS / 2) as f32;
        let mut counts = vec![0usize; BARS as usize];
        for (m, _) in &margins {
            counts[(bar_of(*m, width) + BARS / 2) as usize] += 1;
        }
        let picked = check.bars.unwrap_or((-2, 1));
        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HISTOGRAM_HEIGHT), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, BG);
        let most = counts.iter().copied().max().unwrap_or(1).max(1) as f32;
        let step = rect.width() / BARS as f32;
        let bar_at = |x: f32| ((((x - rect.left()) / step).floor() as i32).clamp(0, BARS - 1)) - BARS / 2;
        for (k, &n) in counts.iter().enumerate() {
            let b = k as i32 - BARS / 2;
            let left = rect.left() + k as f32 * step;
            let on = (picked.0..=picked.1).contains(&b);
            if on {
                painter.rect_filled(Rect::from_min_max(Pos2::new(left, rect.top()), Pos2::new(left + step, rect.bottom())), 0.0, SELECTED_BG);
            }
            if n > 0 {
                // A square root: a few images beside thousands still show.
                let h = ((n as f32 / most).sqrt() * (rect.height() - 8.0)).max(2.0);
                let color = if b >= 0 { ACCENT } else { MUTED };
                let bar = Rect::from_min_max(Pos2::new(left + 1.0, rect.bottom() - 4.0 - h), Pos2::new(left + step - 1.0, rect.bottom() - 4.0));
                painter.rect_filled(bar, 1.0, if on { color } else { color.gamma_multiply(0.45) });
            }
        }
        painter.vline(rect.center().x, rect.y_range(), Stroke::new(1.5, TEXT));
        painter.text(rect.left_top() + Vec2::new(5.0, 3.0), egui::Align2::LEFT_TOP, "read hidden", egui::FontId::proportional(10.5), MUTED);
        painter.text(rect.right_top() + Vec2::new(-5.0, 3.0), egui::Align2::RIGHT_TOP, "read shown", egui::FontId::proportional(10.5), ACCENT_TEXT);
        if let Some(p) = response.hover_pos() {
            let b = bar_at(p.x);
            let n = counts[(b + BARS / 2) as usize];
            response.clone().on_hover_text(format!(
                "{n} images, {:+.2} to {:+.2} from the threshold. Click to show them; Shift + click to show several bars.",
                b as f32 * width,
                (b + 1) as f32 * width
            ));
        }
        if let Some(p) = response.interact_pointer_pos().filter(|_| response.clicked()) {
            let b = bar_at(p.x);
            check.bars = Some(if ui.input(|i| i.modifiers.shift) { (picked.0.min(b), picked.1.max(b)) } else { (b, b) });
        }

        // The images of the bars picked, spread over them, by how they read.
        let mut chosen: Vec<usize> = (0..check.images.len()).filter(|&i| (picked.0..=picked.1).contains(&bar_of(margins[i].0, width))).collect();
        chosen.sort_by(|&a, &b| margins[a].0.total_cmp(&margins[b].0));
        let in_bars = chosen.len();
        if chosen.len() > SHOWN {
            chosen = (0..SHOWN).map(|k| chosen[k * (in_bars - 1) / (SHOWN - 1)]).collect();
        }
        let marking = self.screen.draft.kind == IndicatorKind::Visibility && self.screen.draft.rect().is_some();
        let phase = s.capture_phase.clone();
        let mut into = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let what = if in_bars > chosen.len() { format!("{} of the {in_bars} images", chosen.len()) } else { format!("The {in_bars} images") };
            ui.label(muted(format!("{what} of the bars picked. Marked, an image is added to the captures under")).size(11.5));
            let label = |p: &str| if p.is_empty() { "to sort".to_owned() } else { p.to_owned() };
            egui::ComboBox::from_id_salt("check-phase").selected_text(RichText::new(label(&phase)).size(11.5)).width(90.0).show_ui(ui, |ui| {
                for p in std::iter::once(String::new()).chain(inputs.phase_names()) {
                    if ui.selectable_label(p == phase, label(&p)).clicked() {
                        into = Some(p);
                    }
                }
            });
        });
        if let Some(p) = into {
            self.send(Command::SetCapturePhase(p));
        }
        if !marking {
            ui.label(RichText::new("Open one of the indicator's zones (on the right) to mark images.").color(WARN).size(11.5));
        }
        self.load_textures(&ctx, &chosen);

        // Three per row, each zoomed on the zones.
        let check = &self.screen.check;
        // The recordings as they were when the images were read.
        let read_from: Vec<PathBuf> = check.key.as_ref().map_or_else(Vec::new, |k| k.recordings.iter().map(|(p, _)| p.clone()).collect());
        let drawn = drawn_capture(&self.screen.draft, inputs);
        let several = scored.len() > 1;
        let mut marks: Vec<(usize, bool)> = Vec::new();
        egui::ScrollArea::vertical().id_salt("check-images").auto_shrink([false, false]).show(ui, |ui| {
            let gap = 6.0;
            let side = ((ui.available_width() - 2.0 * gap) / 3.0).floor();
            for row in chosen.chunks(3) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for &i in row {
                        let image = &check.images[i];
                        let (margin, k) = margins[i];
                        let name = capture_name(&read_from[image.recording], image.t);
                        let d = &self.screen.draft;
                        let mark = if d.shown_on.contains(&name) || drawn.as_ref() == Some(&name) {
                            Some(true)
                        } else {
                            d.hidden_on.contains(&name).then_some(false)
                        };
                        ui.allocate_ui_with_layout(Vec2::new(side, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_width(side);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
                            let texture = check.textures.get(&image.path);
                            match texture {
                                Some(texture) => {
                                    let [w, h] = texture.size();
                                    let c = crop(scored, w as f32 / h.max(1) as f32);
                                    let uv = Rect::from_min_size(Pos2::new(c[0], c[1]), Vec2::new(c[2], c[3]));
                                    egui::Image::new(texture).uv(uv).corner_radius(4.0).paint_at(ui, rect);
                                    // The whole image, laid so that the part shown fills the square.
                                    let whole = Rect::from_min_size(
                                        rect.min - Vec2::new(c[0] / c[2] * rect.width(), c[1] / c[3] * rect.height()),
                                        Vec2::new(rect.width() / c[2], rect.height() / c[3]),
                                    );
                                    let painter = ui.painter_at(rect);
                                    for (_, z) in scored {
                                        let stroke = Stroke::new(1.0, Color32::from_rgb(0xff, 0xe0, 0x4a).gamma_multiply(0.8));
                                        if indicators::has_path(z) {
                                            let points = z.path.iter().map(|p| whole.min + Vec2::new(p[0] * whole.width(), p[1] * whole.height())).collect();
                                            painter.add(egui::Shape::line(points, stroke));
                                        } else if z.reading != Reading::Look || z.ring {
                                            let outline = if z.ring { Outline::Ellipse } else { z.outline };
                                            painter.add(egui::Shape::closed_line(outline_line(whole, z.rect, outline, 1.0), stroke));
                                        } else {
                                            painter.rect_stroke(on_image(whole, z.rect), 0.0, stroke, StrokeKind::Outside);
                                        }
                                    }
                                    let title = &check.recordings[image.recording];
                                    response.on_hover_ui(|ui| {
                                        ui.label(RichText::new(format!("{title} · {}", clock(image.t))).size(12.0));
                                        let size = Vec2::new(420.0, 420.0 * h as f32 / w.max(1) as f32);
                                        ui.add(egui::Image::new(texture).fit_to_exact_size(size));
                                    });
                                }
                                None => {
                                    ui.painter().rect_filled(rect, 4.0, BG);
                                }
                            }
                            if let Some(mark) = mark {
                                let wrong = mark != (margin >= 0.0);
                                let text = match (mark, wrong) {
                                    (true, false) => "👁",
                                    (true, true) => "👁 reads hidden",
                                    (false, false) => "⊘",
                                    (false, true) => "⊘ reads shown",
                                };
                                tag(ui, rect.left_top() + Vec2::new(3.0, 3.0), egui::Align2::LEFT_TOP, text.to_owned(), if wrong { DANGER_TEXT } else if mark { OK } else { MUTED });
                            }
                            let word = if margin >= 0.0 { "shown" } else { "hidden" };
                            let measure = image.measures[k];
                            let text = if several { format!("{word} {measure:.2} · zone {}", k + 1) } else { format!("{word} {measure:.2}") };
                            ui.horizontal_wrapped(|ui| {
                                ui.spacing_mut().item_spacing = Vec2::new(3.0, 0.0);
                                ui.label(RichText::new(text).color(if margin >= 0.0 { ACCENT_TEXT } else { MUTED }).size(11.0).strong());
                                ui.add_enabled_ui(marking, |ui| {
                                    if ui.selectable_label(mark == Some(true), RichText::new("👁").size(11.0)).on_hover_text("Shown here: added to the captures, marked").clicked() {
                                        marks.push((i, true));
                                    }
                                    if ui.selectable_label(mark == Some(false), RichText::new("⊘").size(11.0)).on_hover_text("Not shown here: added to the captures, marked").clicked() {
                                        marks.push((i, false));
                                    }
                                });
                            });
                        });
                    }
                });
                ui.add_space(4.0);
            }
        });

        // Marked: added to the captures (once), under the name the mark is kept by.
        for (i, shown) in marks {
            let image = &self.screen.check.images[i];
            let name = capture_name(&read_from[image.recording], image.t);
            if !inputs.captures.iter().any(|c| c.file == name) {
                match FrameData::File(image.path.clone()).load() {
                    Ok(frame) => self.send(Command::AddCaptureAs { dir: inputs.dir.clone(), phase: phase.clone(), file: name.clone(), frame }),
                    Err(e) => {
                        log::warn!("cannot read {}: {e:#}", image.path.display());
                        continue;
                    }
                }
            }
            self.screen.draft.mark(&[name], Some(shown), drawn.as_deref());
        }
    }

    /// Loads the images `chosen` that are not, one batch at a time; forgets
    /// those not shown when too many are kept.
    fn load_textures(&mut self, ctx: &egui::Context, chosen: &[usize]) {
        let check = &mut self.screen.check;
        if let Some(rx) = &check.loading {
            match rx.try_recv() {
                Ok(loaded) => {
                    for (path, image) in loaded {
                        let texture = ctx.load_texture(path.to_string_lossy(), image, egui::TextureOptions::LINEAR);
                        check.textures.insert(path, texture);
                    }
                    check.loading = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => check.loading = None,
                Err(mpsc::TryRecvError::Empty) => return,
            }
        }
        let wanted: Vec<PathBuf> = chosen.iter().map(|&i| check.images[i].path.clone()).collect();
        if check.textures.len() > KEPT_TEXTURES {
            let keep: HashSet<&PathBuf> = wanted.iter().collect();
            check.textures.retain(|path, _| keep.contains(path));
        }
        let missing: Vec<PathBuf> = wanted.into_iter().filter(|p| !check.textures.contains_key(p)).collect();
        if missing.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let loaded = missing.into_iter().filter_map(|path| Some((path.clone(), color_image(&FrameData::File(path).load().ok()?)))).collect();
            let _ = tx.send(loaded);
            ctx.request_repaint();
        });
        check.loading = Some(rx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_named_after_their_recording_and_time() {
        let path = Path::new("/recordings/2026-10-14 20-21-46 The Witcher 3.jsonl");
        assert_eq!(capture_name(path, 123.4561), "2026-10-14-20-21-46-The-Witcher-3-000123456.png");
    }

    #[test]
    fn margins_fall_in_bars_around_the_threshold() {
        assert_eq!(bar_of(0.0, 0.1), 0);
        assert_eq!(bar_of(-0.01, 0.1), -1);
        assert_eq!(bar_of(5.0, 0.1), BARS / 2 - 1);
        assert_eq!(bar_of(-5.0, 0.1), -BARS / 2);
    }

    #[test]
    fn the_zones_are_shown_square_inside_the_image() {
        let zone = Zone { rect: [0.87, 0.04, 0.11, 0.19], ..Zone::default() };
        let [x, y, w, h] = crop(&vec![("zone 1".to_owned(), zone)], 16.0 / 9.0);
        assert!(x >= 0.0 && y >= 0.0 && x + w <= 1.0 + 1e-6 && y + h <= 1.0 + 1e-6);
        assert!((w * 16.0 / 9.0 - h).abs() < 1e-5, "square on the screen");
        assert!(x <= 0.87 && x + w >= 0.98 - 1e-6, "the zone inside");
    }
}
