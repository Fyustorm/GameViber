//! The Creator's Sessions tab: recording play sessions, the recordings, and
//! the session player — a recording replayed offline into the active mode
//! and watched like a video: the game's images, its rumble, the buttons, what
//! the mode's inputs said and what the mode sent to the toys at each moment,
//! seekable, with the images to add to the mode's captures. While it plays,
//! the toys play what the mode sent (`Command::PlaySession`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

use tokio::sync::mpsc::UnboundedSender as Sender;

use eframe::egui::{self, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Vec2};

use super::live::row;
use super::theme::*;
use super::{pad_view, App};
use crate::audio::AudioLevels;
use crate::config::ModeEntry;
use crate::engine::{Command, SessionOutputs, Shared, HISTORY_SECS, RECENT_SECS};
use crate::mode::report::{self, Simulation, Tick};
use crate::mode::{IndicatorValue, ParamValue};
use crate::package::Inputs;
use crate::screen::ScreenLevels;
use crate::session::{Change, RecordingInfo, Session, FRAME_RATE, FRAME_RATES};

/// From this width the player shows the inputs beside the image.
const WIDE_PLAYER: f32 = 980.0;
/// Images shown on each side of the current one, at most.
const STRIP_SIDE: usize = 4;
const THUMB: Vec2 = Vec2::new(112.0, 63.0);
/// Room left under the image for the transport and the timeline.
const UNDER_IMAGE: f32 = 134.0;
/// Images kept decoded (as textures) at most.
const TEXTURES: usize = 48;
/// What happened this many seconds before the moment shown is listed.
const RECENT_EVENTS_SECS: f64 = 2.0;
const SPEEDS: [f64; 4] = [0.25, 0.5, 1.0, 2.0];
/// Seconds Shift + an arrow moves by.
const JUMP_SECS: f64 = 5.0;
/// The player's keyboard shortcuts, as shown to players.
const SHORTCUTS: &str = "Space: play or pause\n\
    Left / Right: previous or next image\n\
    Shift + Left / Right: 5 seconds back or forward\n\
    Home / End: start or end\n\
    M / Shift + M: next or previous moment you marked\n\
    + −: faster or slower\n\
    P: pick the image shown (or unpick it)\n\
    A: add the images picked (else the one shown) to the captures\n\
    Esc: unpick all";
/// About what an image of the game takes as JPEG, for the estimates.
const IMAGE_BYTES: f64 = 35_000.0;

#[derive(Default)]
pub struct State {
    /// Recording whose deletion is being confirmed.
    deleting: Option<PathBuf>,
    /// The recording open in the player.
    player: Option<Player>,
}

impl State {
    /// Opens a recording half way through (the screenshot tour).
    pub(super) fn preview(&mut self, info: &RecordingInfo) {
        let mut player = Player::open(info.clone());
        player.position = info.header.duration / 2.0;
        player.picked.insert(1);
        self.player = Some(player);
    }
}

/// What a session is replayed into: the mode, its script and its settings.
#[derive(Clone, PartialEq)]
struct ModeKey {
    id: String,
    source: String,
    values: BTreeMap<String, ParamValue>,
}

/// A recording open in the player.
struct Player {
    info: RecordingInfo,
    /// The session, once read (in a thread).
    session: Option<Arc<Session>>,
    reading: Option<mpsc::Receiver<Result<Session, String>>>,
    error: Option<String>,
    /// The session replayed into the mode, the mode it was replayed into, and
    /// the replay under way (in a thread).
    simulation: Option<Result<Simulation, String>>,
    simulated: Option<ModeKey>,
    simulating: Option<(ModeKey, mpsc::Receiver<Result<Simulation, String>>)>,
    /// The channels of the replay, for the toys.
    outputs: Option<SessionOutputs>,
    /// Seconds into the session.
    position: f64,
    playing: bool,
    speed: f64,
    /// What the toys were asked to play (the channels, at what speed), whether
    /// their playing was seen since, and whether the player moved elsewhere since.
    sent: Option<(SessionOutputs, f64)>,
    seen: bool,
    jumped: bool,
    cursor: Cursor,
    /// Images decoded, by index.
    textures: HashMap<usize, egui::TextureHandle>,
    /// Images picked to add to the captures, by index, and the phase they go under.
    picked: BTreeSet<usize>,
    phase: String,
    message: Option<(bool, String)>,
    /// The images are to be added to the captures (A), by the filmstrip.
    adding: bool,
}

impl Player {
    fn open(info: RecordingInfo) -> Self {
        let (tx, rx) = mpsc::channel();
        let path = info.path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Session::open(&path).map_err(|e| format!("{e:#}")));
        });
        Self {
            info,
            session: None,
            reading: Some(rx),
            error: None,
            simulation: None,
            simulated: None,
            simulating: None,
            outputs: None,
            position: 0.0,
            playing: false,
            speed: 1.0,
            sent: None,
            seen: false,
            jumped: false,
            cursor: Cursor::default(),
            textures: HashMap::new(),
            picked: BTreeSet::new(),
            phase: String::new(),
            message: None,
            adding: false,
        }
    }

    fn duration(&self) -> f64 {
        self.info.header.duration
    }

    /// Reads what the threads finished, and replays the session again when the mode changed.
    fn poll(&mut self, key: Option<ModeKey>) {
        if let Some(rx) = &self.reading {
            if let Ok(read) = rx.try_recv() {
                self.reading = None;
                match read {
                    Ok(session) => self.session = Some(Arc::new(session)),
                    Err(e) => self.error = Some(e),
                }
            }
        }
        if let Some((key, rx)) = &self.simulating {
            if let Ok(simulation) = rx.try_recv() {
                self.simulated = Some(key.clone());
                self.outputs = simulation.as_ref().ok().map(|sim| Arc::new(sim.ticks().iter().map(|k| (k.t, k.channels.clone())).collect()));
                self.simulation = Some(simulation);
                self.simulating = None;
            }
        }
        let (Some(session), Some(key)) = (&self.session, key) else { return };
        let wanted = Some(&key) != self.simulated.as_ref() && self.simulating.as_ref().is_none_or(|(k, _)| *k != key);
        if wanted {
            let (tx, rx) = mpsc::channel();
            let session = Session::clone(session);
            let run = key.clone();
            std::thread::spawn(move || {
                let entry = ModeEntry::from_id(&run.id);
                let _ = tx.send(report::simulate(&entry.chunk_name(), &run.source, &run.values, &Inputs::of(&entry), session));
            });
            self.simulating = Some((key, rx));
        }
    }

    /// What the mode did at the moment shown.
    fn tick(&self) -> Option<&Tick> {
        let ticks = self.simulation.as_ref()?.as_ref().ok()?.ticks();
        ticks.partition_point(|k| k.t <= self.position).checked_sub(1).map(|i| &ticks[i])
    }

    /// The image shown at the moment shown, by index.
    fn frame_index(&self) -> Option<usize> {
        let frames = &self.session.as_ref()?.frames;
        frames.partition_point(|f| f.t <= self.position + 1e-6).checked_sub(1).or((!frames.is_empty()).then_some(0))
    }

    /// An image as a texture, decoded the first time.
    fn texture(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.textures.get(&i) {
            return Some(texture.clone());
        }
        let frame = self.session.as_ref()?.frames.get(i)?;
        let image = match frame.data.load() {
            Ok(image) => image,
            Err(e) => {
                log::warn!("cannot read an image of the session: {e:#}");
                return None;
            }
        };
        if self.textures.len() >= TEXTURES {
            self.textures.retain(|k, _| k.abs_diff(i) <= TEXTURES / 2);
        }
        let pixels = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.pixels);
        let texture = ctx.load_texture(format!("session-{i}"), pixels, egui::TextureOptions::LINEAR);
        self.textures.insert(i, texture.clone());
        Some(texture)
    }

    fn seek(&mut self, t: f64) {
        self.position = t.clamp(0.0, self.duration());
        self.jumped = true;
    }

    fn toggle(&mut self) {
        self.playing = !self.playing;
        if self.playing && self.position >= self.duration() {
            self.seek(0.0);
        }
    }

    /// Has the toys play the replay from where the player is while it plays
    /// (not while a session is recorded), and stop when it stops.
    fn sync(&mut self, commands: &Sender<Command>, info: &RecordingInfo, recording: bool) {
        let send = |command| {
            let _ = commands.send(command);
        };
        match (&self.outputs, &self.sent) {
            (Some(outputs), sent) if self.playing && !recording => {
                let changed = sent.as_ref().is_none_or(|(o, speed)| !Arc::ptr_eq(o, outputs) || *speed != self.speed);
                if changed || self.jumped {
                    send(Command::PlaySession { path: info.path.clone(), outputs: outputs.clone(), from: self.position, speed: self.speed });
                    self.sent = Some((outputs.clone(), self.speed));
                    self.seen = false;
                }
            }
            (_, Some(_)) if !self.playing || recording => {
                send(Command::StopSession);
                self.sent = None;
                self.seen = false;
            }
            _ => {}
        }
        self.jumped = false;
    }

    /// Moves to the previous (-1) or next (1) image.
    fn step(&mut self, by: i64) {
        let Some(session) = &self.session else { return };
        let frames = &session.frames;
        let target = match self.frame_index() {
            Some(i) if !frames.is_empty() => frames.get((i as i64 + by).clamp(0, frames.len() as i64 - 1) as usize).map(|f| f.t),
            _ => Some(self.position + by as f64 / FRAME_RATE),
        };
        if let Some(t) = target {
            self.seek(t);
        }
    }

    /// Moves to the previous (-1) or next (1) marked moment.
    fn to_mark(&mut self, by: i64) {
        let Some(session) = &self.session else { return };
        let marks = session.changes.iter().filter(|(_, c)| *c == Change::Mark).map(|(t, _)| *t);
        // A little before the mark: what led to it.
        let target = if by < 0 {
            marks.filter(|t| t + 0.6 < self.position).last()
        } else {
            marks.into_iter().find(|t| *t - 0.4 > self.position)
        };
        if let Some(t) = target {
            self.seek(t - 0.5);
        }
    }
}

/// What the recording says at a moment, kept as the moment moves forward.
#[derive(Default)]
struct Cursor {
    /// Changes applied so far.
    next: usize,
    moment: Moment,
}

#[derive(Default, Clone)]
struct Moment {
    strong: f64,
    weak: f64,
    held: BTreeSet<String>,
    axes: BTreeMap<String, f64>,
    audio: Option<AudioLevels>,
    screen: Option<ScreenLevels>,
    indicators: BTreeMap<String, IndicatorValue>,
    external: BTreeMap<String, serde_json::Value>,
}

impl Cursor {
    fn seek(&mut self, changes: &[(f64, Change)], t: f64) -> &Moment {
        if self.next > 0 && changes.get(self.next - 1).is_some_and(|(at, _)| *at > t) {
            *self = Cursor::default();
        }
        while let Some((at, change)) = changes.get(self.next) {
            if *at > t {
                break;
            }
            let m = &mut self.moment;
            match change {
                Change::Rumble { strong, weak } => (m.strong, m.weak) = (*strong, *weak),
                Change::Button { name, pressed: true } => {
                    m.held.insert(name.clone());
                }
                Change::Button { name, pressed: false } => {
                    m.held.remove(name);
                }
                Change::Axis { name, value } => {
                    m.axes.insert(name.clone(), *value);
                }
                Change::Audio { level, low, mid, high, intensity } => {
                    m.audio = Some(AudioLevels { level: *level, low: *low, mid: *mid, high: *high, intensity: *intensity })
                }
                Change::NoAudio => m.audio = None,
                Change::Screen { brightness, motion, action } => {
                    m.screen = Some(ScreenLevels { brightness: *brightness as f32, motion: *motion as f32, action: *action as f32 })
                }
                Change::NoScreen => m.screen = None,
                Change::Indicator { name, value } => {
                    m.indicators.insert(name.clone(), *value);
                }
                Change::ExternalValue { name, value } if value.is_null() => {
                    m.external.remove(name);
                }
                Change::ExternalValue { name, value } => {
                    m.external.insert(name.clone(), value.clone());
                }
                Change::Mark
                | Change::AudioHit { .. }
                | Change::AudioClip { .. }
                | Change::Flash { .. }
                | Change::ScreenClip { .. }
                | Change::ExternalEvent { .. } => {}
            }
            self.next += 1;
        }
        &self.moment
    }
}

/// What reaches the toys from a step of the mode, within the ceiling.
fn output(tick: &Tick, cap: f64) -> f64 {
    tick.channels.values().copied().fold(0.0, f64::max).min(cap)
}

impl App {
    /// Recording real play sessions, and the recordings; one of them open in the player.
    pub(super) fn sessions(&mut self, ui: &mut egui::Ui, s: &Shared) {
        if self.creator.sessions.player.is_some() {
            return self.player_ui(ui, s);
        }
        egui::ScrollArea::vertical().show(ui, |ui| self.recordings(ui, s));
    }

    /// Recording sessions, and the recordings.
    fn recordings(&mut self, ui: &mut egui::Ui, s: &Shared) {
        card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Without the game").strong().size(15.0));
            self.simulator(ui);
        });
        ui.add_space(10.0);
        ui.label(muted(
            "Record the game's rumble, your buttons, what the mode's inputs say and images of the game while you \
             play, then watch it again here, replayed into the mode as it is now: tune it on a real fight without \
             playing it again, and pick images for its captures.",
        ));
        ui.add_space(4.0);
        if let Some(secs) = s.recording {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("⏺ Recording {}", clock(secs))).strong().color(DANGER_TEXT));
                if ui.add(primary("⏹ Stop and save")).clicked() {
                    self.send(Command::StopRecording);
                }
            });
        } else {
            ui.horizontal(|ui| {
                if ui.add(primary("⏺ Record a session")).on_hover_text("Start it, then play the game").clicked() {
                    self.send(Command::StartRecording);
                }
                let mut images = s.settings.recording_images;
                egui::ComboBox::from_id_salt("recording-images").width(80.0).selected_text(format!("{images} images/s")).show_ui(ui, |ui| {
                    for rate in FRAME_RATES {
                        ui.selectable_value(&mut images, rate, format!("{rate} images/s"));
                    }
                });
                if images != s.settings.recording_images {
                    self.send(Command::SetRecordingImages(images));
                }
                ui.label(muted(format!("about {} an hour", bytes(images * IMAGE_BYTES * 3600.0))).size(12.0))
                    .on_hover_text("Images are written to disk as they come, never kept in memory; a recording stops after an hour");
                let save = egui::Button::new(format!("Save the last {:.0} min", RECENT_SECS / 60.0));
                if ui.add(save).on_hover_text("GameViber always keeps the last minutes of play in memory").clicked() {
                    self.send(Command::SaveRecent);
                }
            });
            if !s.settings.screen {
                ui.label(
                    muted("Images of the game are recorded while it shows the in-game overlay; the last minutes keep them only while modes see the image.")
                        .size(12.0),
                );
            }
        }
        ui.separator();
        eyebrow(ui, "Recordings");
        if s.recordings.is_empty() {
            ui.label(muted("No recording yet."));
        }
        let mut command = None;
        let mut open = None;
        for info in &s.recordings {
            card(PANEL).inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(title(info)).strong());
                        let mut details = format!("{} · {} · mode {}", info.header.started, clock(info.header.duration), info.header.mode);
                        if info.header.frames > 0 {
                            details.push_str(&format!(" · {} images", info.header.frames));
                        }
                        if info.header.marks > 0 {
                            details.push_str(&format!(" · {} marked", info.header.marks));
                        }
                        details.push_str(&format!(" · {}", bytes(info.size as f64)));
                        ui.label(muted(details).size(12.0));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.creator.sessions.deleting.as_ref() == Some(&info.path) {
                            if ui.button(RichText::new("Delete").color(DANGER_TEXT)).clicked() {
                                command = Some(Command::DeleteRecording(info.path.clone()));
                                self.creator.sessions.deleting = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.creator.sessions.deleting = None;
                            }
                        } else if ui.button("🗑").on_hover_text("Delete this recording").clicked() {
                            self.creator.sessions.deleting = Some(info.path.clone());
                        }
                        if ui.add(primary("▶ Watch")).on_hover_text("Replayed into the active mode, as it is now").clicked() {
                            open = Some(info.clone());
                        }
                    });
                });
            });
        }
        if let Some(command) = command {
            self.send(command);
        }
        if let Some(info) = open {
            self.creator.sessions.player = Some(Player::open(info));
        }
    }

    /// The recording open, replayed into the active mode; the toys play it while it plays.
    fn player_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let key = self.replayed_mode(s);
        let ctx = ui.ctx().clone();
        let cap = s.settings.global_cap;
        let commands = self.commands.clone();
        let Some(p) = self.creator.sessions.player.as_mut() else { return };
        p.poll(key);
        if p.playing {
            // The toys' clock leads once they play from where the player is.
            match s.replay.as_ref().filter(|r| r.path == p.info.path && p.sent.is_some()) {
                Some(r) if p.seen || (r.position - p.position).abs() < 0.3 => {
                    p.seen = true;
                    p.position = r.position.min(p.duration());
                }
                None if p.seen => p.playing = false,
                _ => {
                    let dt = ui.input(|i| i.stable_dt).min(0.1) as f64;
                    p.position = (p.position + dt * p.speed).min(p.duration());
                }
            }
            if p.position >= p.duration() {
                p.playing = false;
            }
        }
        if p.playing || p.reading.is_some() || p.simulating.is_some() {
            ctx.request_repaint();
        }
        if ctx.memory(|m| m.focused().is_none()) {
            shortcuts(ui, p);
        }

        let mut close = false;
        let info = p.info.clone();
        ui.horizontal_wrapped(|ui| {
            close = ui.button("‹ Recordings").clicked();
            ui.label(RichText::new(title(&info)).size(17.0).strong());
            ui.label(muted(format!("{} · recorded with {}", info.header.started, info.header.mode)));
        });
        let replayed = match (&p.simulating, &p.simulation, &s.mode.info) {
            (Some(_), _, _) => muted("Replaying it into the mode...").color(WARN),
            (None, Some(Err(e)), _) => RichText::new(format!("The mode cannot be replayed: {e}")).color(DANGER_TEXT),
            (None, Some(Ok(sim)), Some(mode)) => match sim.error() {
                Some((t, e)) => RichText::new(format!("{} stopped on an error at {}: {e}", mode.name, clock(*t))).color(DANGER_TEXT),
                None if s.recording.is_some() => muted(format!("What {} does with it; the toys play it once the recording stops.", mode.name)),
                None => muted(format!("What {} does with it, as it is now (its script and settings): the toys play it while it plays.", mode.name)),
            },
            _ => muted(""),
        };
        ui.label(replayed.size(12.5));
        if let Some(e) = &p.error {
            ui.label(RichText::new(format!("Cannot read the recording: {e}")).color(DANGER_TEXT));
        }
        if close {
            if p.sent.is_some() {
                let _ = commands.send(Command::StopSession);
            }
            self.creator.sessions.player = None;
            return;
        }
        let Some(session) = p.session.clone() else {
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(muted("Reading the recording..."));
            });
            return;
        };
        ui.add_space(6.0);

        let moment = p.cursor.seek(&session.changes, p.position).clone();
        let mut add = None;
        // The gamepad, the images and the player keep their size, on the left;
        // the inputs grow with what the mode reads, on the right.
        if ui.available_width() >= WIDE_PLAYER {
            let gap = 12.0;
            let width = ((ui.available_width() - gap) * 0.66).floor();
            let height = ui.available_height();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_width(width);
                    add = left_column(ui, &ctx, p, &session, s, &moment, cap, true);
                });
                ui.vertical(|ui| {
                    egui::ScrollArea::vertical().id_salt("session-inputs").auto_shrink([false, true]).show(ui, |ui| {
                        inputs_card(ui, p, &moment, &session);
                    });
                });
            });
        } else {
            egui::ScrollArea::vertical().id_salt("session-player").show(ui, |ui| {
                add = left_column(ui, &ctx, p, &session, s, &moment, cap, false);
                ui.add_space(10.0);
                inputs_card(ui, p, &moment, &session);
            });
        }
        p.adding = false;
        p.sync(&commands, &info, s.recording.is_some());
        if let Some((dir, phase, frames)) = add {
            let count = frames.len();
            let _ = commands.send(Command::AddCaptures { dir, phase: phase.clone(), frames });
            p.picked.clear();
            let under = if phase.is_empty() { "to sort".to_owned() } else { format!("under {phase}") };
            p.message = Some((true, format!("{count} image{} added to the captures, {under}.", if count == 1 { "" } else { "s" })));
        }
    }

    /// The active mode as it would be replayed; None while its script has unsaved changes elsewhere.
    fn replayed_mode(&self, s: &Shared) -> Option<ModeKey> {
        if s.mode.id.is_empty() {
            return None;
        }
        let editor = &self.creator.editor;
        // The script as saved: the editor holds it when it is this mode's.
        let source = if editor.id == s.mode.id && !editor.text.is_empty() {
            if editor.dirty {
                return self.creator.sessions.player.as_ref().and_then(|p| p.simulated.clone());
            }
            editor.text.clone()
        } else {
            ModeEntry::from_id(&s.mode.id).source().ok()?
        };
        Some(ModeKey { id: s.mode.id.clone(), source, values: s.mode.values.clone() })
    }
}

/// The player's keyboard shortcuts, listed by `SHORTCUTS`.
fn shortcuts(ui: &mut egui::Ui, p: &mut Player) {
    use egui::{Key, Modifiers};
    let (none, shift) = (Modifiers::NONE, Modifiers::SHIFT);
    let key = |modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
    if key(none, Key::Space) {
        p.toggle();
    }
    for (k, by) in [(Key::ArrowLeft, -1), (Key::ArrowRight, 1)] {
        if key(none, k) {
            p.playing = false;
            p.step(by);
        }
        if key(shift, k) {
            p.seek(p.position + by as f64 * JUMP_SECS);
        }
    }
    if key(none, Key::Home) {
        p.seek(0.0);
    }
    if key(none, Key::End) {
        p.playing = false;
        p.seek(p.duration());
    }
    if key(none, Key::M) {
        p.to_mark(1);
    }
    if key(shift, Key::M) {
        p.to_mark(-1);
    }
    let faster = key(none, Key::Plus) || key(none, Key::Equals) || key(shift, Key::Equals);
    let slower = key(none, Key::Minus);
    if faster || slower {
        let at = SPEEDS.iter().position(|s| *s == p.speed).unwrap_or(2);
        let at = if faster { (at + 1).min(SPEEDS.len() - 1) } else { at.saturating_sub(1) };
        p.speed = SPEEDS[at];
    }
    if key(none, Key::P) {
        if let Some(i) = p.frame_index() {
            if !p.picked.remove(&i) {
                p.picked.insert(i);
            }
        }
    }
    if key(none, Key::Escape) {
        p.picked.clear();
    }
    if key(none, Key::A) {
        p.adding = true;
    }
}

/// The gamepad, the images, the image shown and the transport; returns the
/// images to add to the captures (`filmstrip`).
#[allow(clippy::too_many_arguments)]
fn left_column(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    p: &mut Player,
    session: &Session,
    s: &Shared,
    moment: &Moment,
    cap: f64,
    fill: bool,
) -> Option<(PathBuf, String, Vec<crate::screen::Frame>)> {
    gamepad_card(ui, moment);
    ui.add_space(8.0);
    let mut add = None;
    if !session.frames.is_empty() {
        add = filmstrip(ui, ctx, p, session, s);
        ui.add_space(8.0);
    }
    image_view(ui, ctx, p, session, fill);
    transport(ui, p, cap, session);
    add
}

/// The gamepad at the moment shown, on one line.
fn gamepad_card(ui: &mut egui::Ui, moment: &Moment) {
    card(PANEL).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            pad_view(ui, |b| moment.held.contains(b), |a| moment.axes.get(a).copied().unwrap_or(0.0));
            ui.add_space(10.0);
            ui.label(muted("rumble").size(11.5));
            meter(ui, 50.0, moment.strong, GAME);
            meter(ui, 50.0, moment.weak, GAME);
        })
        .response
        .on_hover_text("The buttons held, the sticks, and the game's rumble (strong and weak motors)");
    });
}

/// The game's image at the moment shown; `fill`: as high as the room left
/// above the transport allows.
fn image_view(ui: &mut egui::Ui, ctx: &egui::Context, p: &mut Player, session: &Session, fill: bool) {
    let width = ui.available_width();
    let full = width * 9.0 / 16.0;
    let height = if fill { (ui.available_height() - UNDER_IMAGE).clamp(160.0, full) } else { full.min(420.0) };
    let texture = p.frame_index().and_then(|i| p.texture(ctx, i));
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(8), egui::Color32::BLACK);
    match texture {
        Some(texture) => {
            // The image fit in the frame, its aspect ratio kept.
            let size = texture.size_vec2();
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let shown = Rect::from_center_size(rect.center(), size * scale);
            egui::Image::new(&texture).corner_radius(6.0).paint_at(ui, shown);
        }
        None => {
            let text = if session.frames.is_empty() {
                "No images in this recording.\nThey are recorded while the game shows the in-game overlay."
            } else {
                "Reading the image..."
            };
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(14.0), MUTED);
        }
    }
    if !p.playing && response.hovered() && !session.frames.is_empty() {
        painter.text(rect.right_top() + Vec2::new(-10.0, 10.0), egui::Align2::RIGHT_TOP, "▶", egui::FontId::proportional(18.0), TEXT);
    }
    if response.clicked() {
        p.toggle();
    }
}

/// Play, pause, speed, the moment shown and what the toys get then, and the timeline to seek in.
fn transport(ui: &mut egui::Ui, p: &mut Player, cap: f64, session: &Session) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("⏮").on_hover_text("Back to the start (Home)").clicked() {
            p.seek(0.0);
        }
        if ui.button("⏴").on_hover_text("Previous image (Left)").clicked() {
            p.playing = false;
            p.step(-1);
        }
        let play = if p.playing { "⏸ Pause" } else { "▶ Play" };
        if ui.add(primary(play)).on_hover_text("Space: the toys play it too").clicked() {
            p.toggle();
        }
        if ui.button("⏵").on_hover_text("Next image (Right)").clicked() {
            p.playing = false;
            p.step(1);
        }
        egui::ComboBox::from_id_salt("session-speed").width(64.0).selected_text(format!("×{}", p.speed)).show_ui(ui, |ui| {
            for speed in SPEEDS {
                ui.selectable_value(&mut p.speed, speed, format!("×{speed}"));
            }
        })
        .response
        .on_hover_text("Speed (+ −)");
        ui.label(RichText::new(format!("{} / {}", clock_tenths(p.position), clock(p.duration()))).monospace());
        if session.changes.iter().any(|(_, c)| *c == Change::Mark) {
            ui.add_space(8.0);
            if ui.small_button("⚑ ‹").on_hover_text("Previous moment you marked (Shift + M)").clicked() {
                p.to_mark(-1);
            }
            if ui.small_button("› ⚑").on_hover_text("Next moment you marked (M)").clicked() {
                p.to_mark(1);
            }
        }
        if let Some(tick) = p.tick() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0}%", output(tick, cap) * 100.0)).size(18.0).strong().color(ACCENT_TEXT))
                    .on_hover_text("Sent to the toys at this moment, after the safety ceiling");
                ui.label(muted("toys").size(12.0));
            });
        }
    });
    if let Some(t) = timeline(ui, p, cap, session) {
        p.seek(t);
    }
    ui.horizontal(|ui| {
        legend(ui, GAME, "game rumble");
        legend(ui, ACCENT, "sent to the toys");
        ui.label(RichText::new("⚑ marked").size(11.0).color(WARN));
        if !p.picked.is_empty() {
            legend(ui, OK, "picked images");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(muted("⌨ Shortcuts").size(11.0)).on_hover_text(SHORTCUTS);
        });
    });
}

/// The whole session: the game's rumble and what the mode sent, marked moments,
/// images picked, and the moment shown; returns where it was clicked or dragged to.
fn timeline(ui: &mut egui::Ui, p: &Player, cap: f64, session: &Session) -> Option<f64> {
    let duration = p.duration().max(0.001);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(6), BG);
    painter.rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, LINE), StrokeKind::Inside);
    let x_of = |t: f64| rect.left() + (t / duration).clamp(0.0, 1.0) as f32 * rect.width();
    // Under the phases' band.
    let y_of = |v: f64| rect.bottom() - 3.0 - v.clamp(0.0, 1.0) as f32 * (rect.height() - 19.0);
    if let Some(Ok(sim)) = &p.simulation {
        // The phase recognized, as a band along the top.
        let ticks = sim.ticks();
        let names = ticks.last().map(|k| k.phases.as_slice()).unwrap_or_default();
        let band = |from: f64, to: f64, phase: &str| {
            let r = Rect::from_x_y_ranges(x_of(from)..=x_of(to), rect.top() + 2.0..=rect.top() + 13.0);
            painter.rect_filled(r, 2.0, phase_color(names, phase).gamma_multiply(0.45));
            let text = painter.layout_no_wrap(phase.to_owned(), egui::FontId::proportional(10.0), TEXT);
            if text.size().x + 6.0 < r.width() {
                painter.galley(Pos2::new(r.left() + 3.0, r.center().y - text.size().y / 2.0), text, TEXT);
            }
        };
        let mut start: Option<(f64, &str)> = None;
        for k in ticks {
            if start.map(|(_, name)| name) != k.phase.as_deref() {
                if let Some((from, name)) = start {
                    band(from, k.t, name);
                }
                start = k.phase.as_deref().map(|name| (k.t, name));
            }
        }
        if let Some((from, name)) = start {
            band(from, duration, name);
        }
        // The highest value of each column of pixels.
        let columns = (rect.width() / 2.0).max(1.0) as usize;
        let mut rumble = Vec::with_capacity(columns);
        let mut sent = Vec::with_capacity(columns);
        let mut start = 0;
        for c in 0..columns {
            let t1 = duration * (c + 1) as f64 / columns as f64;
            let end = start + ticks[start..].partition_point(|k| k.t < t1);
            let bucket = &ticks[start..end];
            if !bucket.is_empty() {
                let x = rect.left() + (c as f32 + 0.5) * rect.width() / columns as f32;
                rumble.push(Pos2::new(x, y_of(bucket.iter().map(|k| k.rumble).fold(0.0, f64::max))));
                sent.push(Pos2::new(x, y_of(bucket.iter().map(|k| output(k, cap)).fold(0.0, f64::max))));
            }
            start = end;
        }
        painter.line(rumble, Stroke::new(1.2, GAME));
        painter.line(sent, Stroke::new(1.4, ACCENT));
    }
    for (t, change) in &session.changes {
        if *change == Change::Mark {
            let x = x_of(*t);
            painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(1.0, WARN.gamma_multiply(0.6)));
            painter.text(Pos2::new(x + 2.0, rect.top() + 1.0), egui::Align2::LEFT_TOP, "⚑", egui::FontId::proportional(11.0), WARN);
        }
    }
    for &i in &p.picked {
        if let Some(frame) = session.frames.get(i) {
            let x = x_of(frame.t);
            painter.rect_filled(Rect::from_min_size(Pos2::new(x - 1.0, rect.bottom() - 6.0), Vec2::new(3.0, 6.0)), 0.0, OK);
        }
    }
    let x = x_of(p.position);
    painter.line_segment([Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())], Stroke::new(2.0, TEXT));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let hover = response.hover_pos().map(|pos| ((pos.x - rect.left()) / rect.width()) as f64 * duration);
    let response = match hover {
        Some(t) => response.on_hover_text_at_pointer(clock_tenths(t.clamp(0.0, duration))),
        None => response,
    };
    let seek = response.clicked() || response.dragged();
    seek.then(|| response.interact_pointer_pos()).flatten().map(|pos| ((pos.x - rect.left()) / rect.width()) as f64 * duration)
}

/// What the mode's inputs said at the moment shown, and what just happened.
fn inputs_card(ui: &mut egui::Ui, p: &Player, moment: &Moment, session: &Session) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Inputs");
        // The phase, and how likely each is.
        let sim = p.simulation.as_ref().and_then(|s| s.as_ref().ok());
        let tick = p.tick();
        let phases = tick.map(|k| k.phases.as_slice()).unwrap_or_default();
        match (tick.and_then(|k| k.phase.as_ref()), sim.and_then(Simulation::phases_unavailable)) {
            (Some(phase), _) => {
                ui.label(RichText::new(phase).size(20.0).strong().color(phase_color(phases, phase)));
            }
            (None, Some(why)) => {
                ui.label(muted(format!("Phases not recognized: {why}")).size(12.0));
            }
            (None, None) if phases.is_empty() => {
                ui.label(muted("No phases: name them in the Phases tab."));
            }
            (None, None) => {
                ui.label(RichText::new("No phase yet").size(16.0).color(MUTED));
            }
        }
        for (name, likelihood) in phases {
            let current = tick.and_then(|k| k.phase.as_ref()) == Some(name);
            row(ui, name, |ui| {
                meter(ui, (ui.available_width() - 44.0).max(40.0), *likelihood, if current { phase_color(phases, name) } else { IDLE });
                ui.label(RichText::new(format!("{:.0}%", likelihood * 100.0)).size(12.0));
            });
        }
        if !moment.indicators.is_empty() {
            ui.add_space(6.0);
            eyebrow(ui, "Indicators");
            for (name, value) in &moment.indicators {
                row(ui, name, |ui| match value {
                    IndicatorValue::Gauge(x) => {
                        meter(ui, (ui.available_width() - 44.0).max(40.0), *x, OK);
                        ui.label(RichText::new(format!("{:.0}%", x * 100.0)).size(12.0));
                    }
                    IndicatorValue::Visibility(true) => {
                        ui.label(RichText::new("shown").color(ACCENT_TEXT));
                    }
                    IndicatorValue::Visibility(false) => {
                        ui.label(muted("hidden"));
                    }
                    IndicatorValue::Unknown => {
                        ui.label(RichText::new("unknown").color(WARN));
                    }
                });
            }
        }
        ui.add_space(6.0);
        eyebrow(ui, "Sound");
        match &moment.audio {
            Some(levels) => {
                row(ui, "level", |ui| meter(ui, ui.available_width().max(40.0), levels.level, GAME));
                row(ui, "action", |ui| meter(ui, ui.available_width().max(40.0), levels.intensity, GAME))
                    .on_hover_text("Loudness and density of hits over the last seconds");
            }
            None => {
                ui.label(muted("Not heard.").size(12.0));
            }
        }
        if let Some(levels) = &moment.screen {
            ui.add_space(6.0);
            eyebrow(ui, "Image");
            row(ui, "brightness", |ui| meter(ui, ui.available_width().max(40.0), levels.brightness as f64, GAME));
            row(ui, "motion", |ui| meter(ui, ui.available_width().max(40.0), levels.motion as f64, GAME));
            row(ui, "action", |ui| meter(ui, ui.available_width().max(40.0), levels.action as f64, GAME))
                .on_hover_text("Motion over the last seconds");
        }
        if !moment.external.is_empty() {
            ui.add_space(6.0);
            eyebrow(ui, "From other programs");
            for (name, value) in &moment.external {
                row(ui, name, |ui| ui.label(RichText::new(value.to_string()).monospace().size(12.0)));
            }
        }
        // The mode's plot() values, each scaled to its own range over the seconds before.
        if let Some(tick) = p.tick().filter(|k| !k.plots.is_empty()) {
            let ticks = p.simulation.as_ref().and_then(|s| s.as_ref().ok()).map(|s| s.ticks()).unwrap_or_default();
            let recent = &ticks[ticks.partition_point(|k| k.t < p.position - HISTORY_SECS)..ticks.partition_point(|k| k.t <= p.position)];
            ui.add_space(6.0);
            eyebrow(ui, "What the mode tracks");
            for (name, value) in &tick.plots {
                let series: Vec<(f64, f64)> =
                    recent.iter().filter_map(|k| k.plots.iter().find(|(n, _)| n == name).map(|(_, v)| (k.t - p.position, *v))).collect();
                let top = series.iter().map(|(_, v)| v.abs()).fold(0.0, f64::max).max(1e-9);
                let line: Vec<(f64, f64)> = series.iter().map(|(t, v)| (*t, v / top)).collect();
                row(ui, name, |ui| {
                    let width = (ui.available_width() - 52.0).max(40.0);
                    sparkline(ui, Vec2::new(width, 22.0), HISTORY_SECS, &line, GAME);
                    ui.label(RichText::new(format!("{value:.2}")).monospace().size(12.0));
                });
            }
        }
        ui.add_space(6.0);
        eyebrow(ui, "Just before");
        let events = recent_events(p, session);
        if events.is_empty() {
            ui.label(muted("Nothing in the last 2 s.").size(12.0));
        }
        for (ago, text, color) in events {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("-{ago:.1} s")).monospace().size(11.5).color(MUTED));
                ui.label(RichText::new(text).size(12.5).color(color));
            });
        }
    });
}

/// Events of the seconds before the moment shown, the latest first: seconds ago, what, its color.
fn recent_events(p: &Player, session: &Session) -> Vec<(f64, String, egui::Color32)> {
    let changes = &session.changes;
    let from = changes.partition_point(|(t, _)| *t < p.position - RECENT_EVENTS_SECS);
    let to = changes.partition_point(|(t, _)| *t <= p.position);
    let mut events: Vec<(f64, String, egui::Color32)> = changes[from..to]
        .iter()
        .filter_map(|(t, change)| {
            let (text, color) = match change {
                Change::Button { name, pressed: true } => (format!("{name} pressed"), TEXT),
                Change::AudioHit { strength, .. } => (format!("Sound hit ({:.0}%)", strength * 100.0), GAME),
                Change::Flash { strength } => (format!("Flash ({:.0}%)", strength * 100.0), GAME),
                Change::Mark => ("⚑ You marked this moment".to_owned(), WARN),
                Change::ExternalEvent { name, .. } => (format!("Event {name}"), OK),
                _ => return None,
            };
            Some((p.position - t, text, color))
        })
        .collect();
    if let Some(Ok(sim)) = &p.simulation {
        let hud = sim.hud_events();
        let from = hud.partition_point(|(t, _)| *t < p.position - RECENT_EVENTS_SECS);
        let to = hud.partition_point(|(t, _)| *t <= p.position);
        events.extend(hud[from..to].iter().map(|(t, text)| (p.position - t, format!("Overlay: {text}"), ACCENT_TEXT)));
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    events.truncate(8);
    events
}

/// The images around the one shown, to move to or pick (the corner of an
/// image), and adding the picked ones to the active mode's captures: returns
/// the mode's package, the phase and the images when they are to be added.
fn filmstrip(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    p: &mut Player,
    session: &Session,
    s: &Shared,
) -> Option<(PathBuf, String, Vec<crate::screen::Frame>)> {
    let mut add = None;
    card(PANEL).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let current = p.frame_index().unwrap_or(0);
        ui.horizontal_wrapped(|ui| {
            eyebrow(ui, &format!("Images · {}", session.frames.len()));
            let Some(inputs) = s.mode_inputs.as_ref() else {
                ui.label(muted("Captures belong to a mode of your own: duplicate this built-in mode, or create one.").size(12.0));
                return;
            };
            let picked = p.picked.len();
            let wanted: Vec<usize> = if picked == 0 { vec![current] } else { p.picked.iter().copied().collect() };
            let label = match picked {
                0 => "📸 Add this image to the captures".to_owned(),
                1 => "📸 Add the image picked to the captures".to_owned(),
                n => format!("📸 Add the {n} images picked to the captures"),
            };
            let clicked = ui.button(label).on_hover_text("A · Pick images with the corner of each, or P for the one shown").clicked();
            ui.label("under");
            let shown = if p.phase.is_empty() { "To sort later".to_owned() } else { p.phase.clone() };
            egui::ComboBox::from_id_salt("session-capture-phase").selected_text(shown).show_ui(ui, |ui| {
                ui.selectable_value(&mut p.phase, String::new(), "To sort later");
                for phase in &inputs.phases {
                    ui.selectable_value(&mut p.phase, phase.name.clone(), &phase.name);
                }
            });
            if picked > 0 && ui.small_button("Unpick all").on_hover_text("Esc").clicked() {
                p.picked.clear();
            }
            if let Some((ok, message)) = &p.message {
                ui.label(RichText::new(message).color(if *ok { OK } else { WARN }).size(12.0));
            }
            if clicked || p.adding {
                let frames: Vec<_> = wanted.iter().filter_map(|i| session.frames.get(*i)?.data.load().ok()).collect();
                if frames.len() < wanted.len() {
                    p.message = Some((false, "Some images could not be read.".to_owned()));
                }
                if !frames.is_empty() {
                    add = Some((inputs.dir.clone(), p.phase.clone(), frames));
                }
            }
        });
        let fits = ((ui.available_width() + 6.0) / (THUMB.x + 6.0)).floor().max(1.0) as usize;
        let side = STRIP_SIDE.min(fits.saturating_sub(1) / 2);
        let first = current.saturating_sub(side).min(session.frames.len().saturating_sub(2 * side + 1));
        let last = (first + 2 * side).min(session.frames.len() - 1);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for i in first..=last {
                let (rect, response) = ui.allocate_exact_size(THUMB, Sense::click());
                match p.texture(ctx, i) {
                    Some(texture) => {
                        egui::Image::new(&texture).corner_radius(4.0).paint_at(ui, rect);
                    }
                    None => {
                        ui.painter().rect_filled(rect, 4.0, RAISED);
                    }
                }
                let painter = ui.painter();
                let picked = p.picked.contains(&i);
                // The corner picks the image; the rest of it moves to it.
                let corner = Rect::from_min_size(rect.right_top() + Vec2::new(-22.0, 2.0), Vec2::splat(20.0));
                let on_corner = response.hover_pos().is_some_and(|pos| corner.contains(pos));
                let stroke = if picked {
                    Stroke::new(2.5, OK)
                } else if i == current {
                    Stroke::new(2.0, TEXT)
                } else if response.hovered() {
                    Stroke::new(1.0, MUTED)
                } else {
                    Stroke::NONE
                };
                painter.rect_stroke(rect, 4.0, stroke, StrokeKind::Outside);
                if picked || response.hovered() {
                    let fill = if picked { OK } else if on_corner { RAISED } else { BG.gamma_multiply(0.8) };
                    painter.rect_filled(corner, 4.0, fill);
                    let mark = if picked { "✔" } else { "+" };
                    painter.text(corner.center(), egui::Align2::CENTER_CENTER, mark, egui::FontId::proportional(13.0), if picked { ON_ACCENT } else { TEXT });
                }
                let time = clock_tenths(session.frames[i].t);
                let label = Rect::from_min_size(rect.left_bottom() + Vec2::new(3.0, -16.0), Vec2::new(46.0, 14.0));
                painter.rect_filled(label, 3.0, BG.gamma_multiply(0.75));
                painter.text(label.center(), egui::Align2::CENTER_CENTER, time, egui::FontId::proportional(10.5), TEXT);
                if response.hovered() {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                let response = response.on_hover_text(if on_corner { "Pick it for the captures (P: the image shown)" } else { "See this moment" });
                if response.clicked() {
                    if on_corner {
                        if !p.picked.remove(&i) {
                            p.picked.insert(i);
                        }
                    } else {
                        p.playing = false;
                        p.seek(session.frames[i].t);
                    }
                }
            }
        });
    });
    add
}

/// The color of a phase, by its place among the mode's phases.
fn phase_color(phases: &[(String, f64)], phase: &str) -> egui::Color32 {
    const COLORS: [egui::Color32; 5] = [ACCENT, OK, WARN, GAME, DANGER_TEXT];
    phases.iter().position(|(n, _)| n == phase).map_or(ACCENT, |i| COLORS[i % COLORS.len()])
}

/// A line of `color` and what it shows, under the timeline.
fn legend(ui: &mut egui::Ui, color: egui::Color32, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 10.0), Sense::hover());
    ui.painter().line_segment([rect.left_center(), rect.right_center()], Stroke::new(2.5, color));
    ui.label(muted(text).size(11.0));
}

/// The game a recording comes from, else its mode.
fn title(info: &RecordingInfo) -> &str {
    info.header.game.as_deref().unwrap_or(&info.header.mode)
}

/// "35 MB"
fn bytes(n: f64) -> String {
    match n {
        n if n >= 1e9 => format!("{:.1} GB", n / 1e9),
        n if n >= 1e6 => format!("{:.0} MB", n / 1e6),
        n => format!("{:.0} KB", n / 1e3),
    }
}

/// "1:05"
fn clock(secs: f64) -> String {
    let secs = secs.max(0.0) as u64;
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// "1:05.3"
fn clock_tenths(secs: f64) -> String {
    let tenths = (secs.max(0.0) * 10.0) as u64;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}
