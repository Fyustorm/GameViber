//! What a mode did during a recorded session, for an AI assistant asked to
//! fix a mode that does not feel right: the session is replayed offline into
//! a fresh copy of the mode, and the game's vibrations, the player's presses,
//! what was heard of the game's sound, the mode's overlay messages and a
//! timeline of its outputs and `plot()` values are written out as Markdown.

use std::collections::BTreeMap;
use std::fmt::Write;

use super::rumble_events::{RumbleEvent, RumbleTracker};
use super::phases::Sense;
use super::{ModeEvent, ModeRuntime, ParamValue, IndicatorValue};
use crate::models::{self, Model};
use crate::package::Inputs;
use crate::gamepad::PadState;
use crate::screen::indicators::IndicatorReader;
use crate::session::{Change, Player, Progress, Session};

const DT: f64 = 0.02;
/// The timeline has at most this many rows (before identical rows are dropped).
const MAX_ROWS: usize = 300;
const MIN_ROW_SECS: f64 = 0.25;
const MAX_VIBRATIONS: usize = 150;
const MAX_PRESSES: usize = 200;
const MAX_PLOTS: usize = 6;

/// What the mode did during one step.
pub struct Tick {
    pub t: f64,
    pub rumble: f64,
    pub held: Vec<&'static str>,
    /// Loudness of the game's sound, when it was captured.
    pub sound: Option<f64>,
    /// Action on the game's screen, when it was copied.
    pub image: Option<f64>,
    pub phase: Option<String>,
    /// How likely each phase is (averaged), when the mode has phases.
    pub phases: Vec<(String, f64)>,
    pub channels: BTreeMap<String, f64>,
    pub plots: Vec<(String, f64)>,
}

/// The mode replayed on a session.
pub struct Simulation {
    duration: f64,
    ticks: Vec<Tick>,
    /// Start, duration and peak of each vibration of the game.
    vibrations: Vec<(f64, f64, f64)>,
    /// Press time, button, how long it was held (None: still held at the end).
    presses: Vec<(f64, &'static str, Option<f64>)>,
    hud_events: Vec<(f64, String)>,
    /// Moments the player marked as feeling wrong.
    marks: Vec<f64>,
    /// The mode stopped on a runtime error.
    error: Option<(f64, String)>,
    /// Sounds heard as hits.
    audio_hits: usize,
    /// Flashes of the game's image.
    flashes: usize,
    /// When an indicator changed, and to what.
    indicator_changes: Vec<(f64, String, IndicatorValue)>,
    /// The indicators were read again from the session's images (their zones changed since).
    indicators_reread: bool,
    /// Events other programs sent, and how many values they set.
    external: Vec<(f64, String)>,
    external_values: usize,
    /// The mode declares phases.
    has_phases: bool,
    /// Why the phases could not be recognized.
    phases_unavailable: Option<String>,
    /// When the recognized phase changed, and to what.
    phase_changes: Vec<(f64, Option<String>)>,
}

/// Replays `session` into a freshly loaded mode with `params` and the inputs
/// the player set up for it (`inputs`: its phases, captures and indicators, as
/// they are now: indicators whose zones changed since the recording are read
/// again from its images). Tells how far it got in `progress`, and gives up once it is stopped.
pub fn simulate(
    chunk_name: &str,
    source: &str,
    params: &BTreeMap<String, ParamValue>,
    inputs: &Inputs,
    session: Session,
    progress: &Progress,
) -> Result<Simulation, String> {
    let mut rt = ModeRuntime::load(chunk_name, source, params, None)?;
    rt.start()?;
    let info = rt.info().clone();
    let duration = session.header.duration;
    let marks = session.changes.iter().filter(|(_, c)| *c == Change::Mark).map(|(t, _)| *t).collect();
    let reread = !session.frames.is_empty() && session.header.zones.as_ref() != Some(&inputs.zones);
    let frames = if reread { session.frames.clone() } else { Vec::new() };
    let mut reader = IndicatorReader::default();
    let mut next_frame = 0;
    let mut player = Player::new(session, 0.0);
    let mut pad = PadState::default();
    let mut tracker = RumbleTracker::new(info.rumble_threshold, info.rumble_release, 0.0);
    let mut sim = Simulation {
        duration,
        ticks: Vec::new(),
        vibrations: Vec::new(),
        presses: Vec::new(),
        hud_events: Vec::new(),
        marks,
        error: None,
        audio_hits: 0,
        flashes: 0,
        indicator_changes: Vec::new(),
        indicators_reread: reread,
        external: Vec::new(),
        external_values: 0,
        has_phases: !info.phases.is_empty(),
        phases_unavailable: None,
        phase_changes: Vec::new(),
    };
    rt.set_game_phases(&inputs.phase_decls());
    rt.set_phase_references(Sense::Examples, inputs.example_centroids());
    sim.has_phases = !rt.phase_decls().is_empty();
    let mut unavailable = Vec::new();
    for (sense, model) in [(Sense::Sound, Model::Sound), (Sense::Screen, Model::Image)] {
        let descriptions = rt.phase_descriptions(sense);
        if descriptions.is_empty() {
            continue;
        }
        let texts: Vec<String> = descriptions.iter().map(|(_, d)| d.clone()).collect();
        match models::text_embeddings(model, &texts) {
            Ok(embeddings) => rt.set_phase_references(sense, descriptions.into_iter().map(|(n, _)| n).zip(embeddings).collect()),
            Err(e) => unavailable.push(format!("{e:#}")),
        }
    }
    if !unavailable.is_empty() {
        sim.phases_unavailable = Some(unavailable.join("; "));
    }

    let mut phase = None;
    let mut vibration_start = 0.0;
    let mut step = 0;
    loop {
        let t = step as f64 * DT;
        if t > duration {
            break;
        }
        if progress.stopped() {
            return Err("stopped".into());
        }
        if step % 50 == 0 {
            progress.set(t / duration);
        }
        step += 1;
        let buttons = player.advance(t, &mut pad);
        for b in &buttons {
            if b.pressed {
                sim.presses.push((t, b.name, None));
            } else if let Some(press) = sim.presses.iter_mut().rev().find(|p| p.1 == b.name && p.2.is_none()) {
                press.2 = Some(t - press.0);
            }
        }
        for event in tracker.update(player.rumble, t) {
            match event {
                RumbleEvent::Start(_) => vibration_start = t,
                RumbleEvent::End { peak, duration } => sim.vibrations.push((vibration_start, duration, peak)),
                RumbleEvent::Changed(_) => {}
            }
        }
        let mut replayed = player.take_events();
        if reread {
            replayed.retain(|e| !matches!(e, ModeEvent::Indicator { .. }));
            while let Some(frame) = frames.get(next_frame).filter(|f| f.t <= t) {
                next_frame += 1;
                if inputs.zones.is_empty() {
                    continue;
                }
                match frame.data.load() {
                    Ok(image) => {
                        let read = reader.update(&inputs.zones, &image);
                        replayed.extend(read.into_iter().map(|(name, value)| ModeEvent::Indicator { name, value }));
                    }
                    Err(e) => log::warn!("cannot read an image of the session: {e:#}"),
                }
            }
        }
        for event in &replayed {
            match event {
                ModeEvent::AudioHit(_) => sim.audio_hits += 1,
                ModeEvent::ScreenFlash(_) => sim.flashes += 1,
                ModeEvent::Indicator { name, value } => {
                    if !sim.indicator_changes.iter().rev().find(|(_, n, _)| n == name).is_some_and(|(_, _, v)| v == value) {
                        sim.indicator_changes.push((t, name.clone(), *value));
                    }
                }
                ModeEvent::ExternalEvent { name, .. } => sim.external.push((t, name.clone())),
                ModeEvent::ExternalValue { .. } => sim.external_values += 1,
                _ => {}
            }
        }
        let events: Vec<ModeEvent> = buttons.into_iter().map(ModeEvent::Button).chain(replayed).collect();
        rt.set_audio(player.audio);
        rt.set_screen(player.screen);
        let out = match rt.step(DT, player.rumble, &pad, pad.input_idle(t), &events) {
            Ok(out) => out,
            Err(e) => {
                sim.error = Some((t, e));
                break;
            }
        };
        sim.hud_events.extend(out.hud_events.into_iter().map(|e| (t, e)));
        let (current, likelihoods) = if sim.has_phases { rt.phase_state() } else { (None, Vec::new()) };
        if current != phase {
            sim.phase_changes.push((t, current.clone()));
            phase = current;
        }
        sim.ticks.push(Tick {
            t,
            rumble: player.rumble.level(),
            held: pad.held().iter().copied().collect(),
            sound: player.audio.map(|a| a.level),
            image: player.screen.map(|l| l.action as f64),
            phase: phase.clone(),
            phases: likelihoods,
            channels: out.channels,
            plots: out.plots,
        });
    }
    Ok(sim)
}

impl Simulation {
    /// What the mode did, step by step (every 20 ms).
    pub fn ticks(&self) -> &[Tick] {
        &self.ticks
    }

    /// The indicators the mode was given at `t` seconds.
    pub fn indicators_at(&self, t: f64) -> BTreeMap<String, IndicatorValue> {
        let to = self.indicator_changes.partition_point(|(at, _, _)| *at <= t);
        self.indicator_changes[..to].iter().map(|(_, name, value)| (name.clone(), *value)).collect()
    }

    /// When an indicator changed, and to what, as the mode was given them.
    pub fn indicator_changes(&self) -> &[(f64, String, IndicatorValue)] {
        &self.indicator_changes
    }

    /// The mode declares phases, or was given some.
    pub fn has_phases(&self) -> bool {
        self.has_phases
    }

    /// The indicators were read again from the session's images.
    pub fn indicators_reread(&self) -> bool {
        self.indicators_reread
    }

    /// Messages the mode showed with `hud_event`.
    pub fn hud_events(&self) -> &[(f64, String)] {
        &self.hud_events
    }

    /// When and why the mode stopped on a runtime error.
    pub fn error(&self) -> Option<&(f64, String)> {
        self.error.as_ref()
    }

    /// Why the phases could not be recognized.
    pub fn phases_unavailable(&self) -> Option<&str> {
        self.phases_unavailable.as_deref()
    }

    /// The simulation as Markdown sections for an AI assistant.
    pub fn report(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "Session length: {:.1} s.\n", self.duration);
        if let Some((t, error)) = &self.error {
            let _ = writeln!(out, "**The mode stopped on an error at {t:.2} s:** `{}`\n", error.trim());
        }

        if !self.marks.is_empty() {
            let _ = writeln!(out, "### Moments the player marked as feeling wrong ({})\n", self.marks.len());
            out.push_str("Look at what happened in the seconds before each mark first.\n\n");
            for t in &self.marks {
                let _ = writeln!(out, "- {t:.2} s");
            }
            out.push('\n');
        }
        let _ = writeln!(out, "### Vibrations sent by the game ({})\n", self.vibrations.len());
        if self.vibrations.is_empty() {
            out.push_str("None.\n");
        }
        for (start, length, peak) in self.vibrations.iter().take(MAX_VIBRATIONS) {
            let _ = writeln!(out, "- {start:.2} s: {length:.2} s long, peak {peak:.2}");
        }
        more(&mut out, self.vibrations.len(), MAX_VIBRATIONS);

        let _ = writeln!(out, "\n### Button presses ({})\n", self.presses.len());
        if self.presses.is_empty() {
            out.push_str("None.\n");
        }
        for (t, button, held) in self.presses.iter().take(MAX_PRESSES) {
            let held = held.map_or("held until the end".to_owned(), |h| format!("held {h:.2} s"));
            let _ = writeln!(out, "- {t:.2} s: {button} ({held})");
        }
        more(&mut out, self.presses.len(), MAX_PRESSES);

        out.push_str(&self.sound());

        if !self.hud_events.is_empty() {
            let _ = writeln!(out, "\n### Messages the mode showed with hud_event ({})\n", self.hud_events.len());
            for (t, text) in self.hud_events.iter().take(MAX_PRESSES) {
                let _ = writeln!(out, "- {t:.2} s: {text}");
            }
            more(&mut out, self.hud_events.len(), MAX_PRESSES);
        }

        out.push('\n');
        out.push_str(&self.timeline());
        out
    }

    /// What was heard of the game's sound, seen of its image, sent by other
    /// programs, and the phases recognized.
    fn sound(&self) -> String {
        let mut out = String::new();
        if self.ticks.iter().any(|t| t.sound.is_some()) {
            let _ = writeln!(out, "\n### The game's sound\n\n{} hits heard (on_audio_hit).", self.audio_hits);
        }
        if self.ticks.iter().any(|t| t.image.is_some()) {
            let _ = writeln!(out, "\n### The game's image\n\n{} flashes seen (on_impact).", self.flashes);
            if !self.indicator_changes.is_empty() {
                let _ = writeln!(out, "\nIndicator changes ({}):\n", self.indicator_changes.len());
                for (t, name, value) in self.indicator_changes.iter().take(MAX_PRESSES) {
                    let value = match value {
                        IndicatorValue::Visibility(shown) => if *shown { "shown" } else { "hidden" }.to_owned(),
                        IndicatorValue::Gauge(fill) => format!("{fill:.2}"),
                        IndicatorValue::Unknown => "unknown (not on screen)".to_owned(),
                    };
                    let _ = writeln!(out, "- {t:.2} s: {name} {value}");
                }
                more(&mut out, self.indicator_changes.len(), MAX_PRESSES);
            }
        }
        if !self.external.is_empty() || self.external_values > 0 {
            let _ = writeln!(out, "\n### Sent by other programs\n\n{} values set (input.external).", self.external_values);
            for (t, name) in self.external.iter().take(MAX_PRESSES) {
                let _ = writeln!(out, "- {t:.2} s: event {name}");
            }
            more(&mut out, self.external.len(), MAX_PRESSES);
        }
        if !self.has_phases {
            return out;
        }
        out.push_str("\n### Phases\n\n");
        if let Some(why) = &self.phases_unavailable {
            let _ = writeln!(out, "Some phases could not be recognized: {why}.\n");
        }
        if !self.ticks.iter().any(|t| t.sound.is_some() || t.image.is_some()) {
            out.push_str("Neither the sound nor the image was captured during this session: no phase was recognized.\n");
            return out;
        }
        let _ = writeln!(out, "Phase changes ({}):\n", self.phase_changes.len());
        if self.phase_changes.is_empty() {
            out.push_str("None: no phase was recognized.\n");
        }
        let mut previous: Option<&Option<String>> = None;
        for (t, phase) in self.phase_changes.iter().take(MAX_PRESSES) {
            let was = previous.map_or(String::new(), |p| format!(" (was {})", p.as_deref().unwrap_or("none")));
            let _ = writeln!(out, "- {t:.2} s: {}{was}", phase.as_deref().unwrap_or("none"));
            previous = Some(phase);
        }
        more(&mut out, self.phase_changes.len(), MAX_PRESSES);
        out
    }

    /// Table of the game rumble, held buttons, outputs (max over each row) and
    /// plot values (last of each row).
    fn timeline(&self) -> String {
        let row_secs = (self.duration / MAX_ROWS as f64).max(MIN_ROW_SECS);
        let channels: Vec<String> = self.ticks.iter().flat_map(|t| t.channels.keys().cloned()).fold(Vec::new(), uniq);
        let plots: Vec<String> =
            self.ticks.iter().flat_map(|t| t.plots.iter().map(|(n, _)| n.clone())).fold(Vec::new(), uniq);
        let plots = &plots[..plots.len().min(MAX_PLOTS)];

        let mut out = String::new();
        let _ = writeln!(
            out,
            "### Timeline\n\nOne row per {row_secs:.2} s: the game rumble, the sound's loudness and mode outputs are \
             the maximum over the row, held buttons, the screen's action, the phase and plot() values are taken at its end. Rows equal to the previous one are left \
             out.\n"
        );
        let marked = !self.marks.is_empty();
        let sound = self.ticks.iter().any(|t| t.sound.is_some());
        let image = self.ticks.iter().any(|t| t.image.is_some());
        let mut header = vec!["t (s)".to_owned(), "game rumble".into(), "held".into()];
        if marked {
            header.push("marked".into());
        }
        if sound {
            header.push("sound".into());
        }
        if image {
            header.push("image action".into());
        }
        if self.has_phases {
            header.push("phase".into());
        }
        header.extend(channels.iter().map(|c| format!("out {c}")));
        header.extend(plots.iter().map(|p| format!("plot {p}")));
        let _ = writeln!(out, "| {} |", header.join(" | "));
        let _ = writeln!(out, "|{}", "---|".repeat(header.len()));

        let mut last_values: Option<Vec<String>> = None;
        let mut plot_values: BTreeMap<&str, f64> = BTreeMap::new();
        for row in self.ticks.chunk_by(|a, b| (a.t / row_secs).floor() == (b.t / row_secs).floor()) {
            let end = row.last().unwrap();
            for tick in row {
                for (name, value) in &tick.plots {
                    plot_values.insert(name, *value);
                }
            }
            let rumble = row.iter().map(|t| t.rumble).fold(0.0, f64::max);
            let held = if end.held.is_empty() { "-".to_owned() } else { end.held.join(" ") };
            let mut values = vec![format!("{rumble:.2}"), held];
            if marked {
                let (from, to) = (row[0].t, end.t + DT);
                values.push(if self.marks.iter().any(|m| (from..to).contains(m)) { "⚑" } else { "" }.to_owned());
            }
            if sound {
                let loudest = row.iter().filter_map(|t| t.sound).reduce(f64::max);
                values.push(loudest.map_or("-".to_owned(), |l| format!("{l:.2}")));
            }
            if image {
                values.push(end.image.map_or("-".to_owned(), |a| format!("{a:.2}")));
            }
            if self.has_phases {
                values.push(end.phase.clone().unwrap_or_else(|| "-".to_owned()));
            }
            for c in &channels {
                let max = row.iter().filter_map(|t| t.channels.get(c)).copied().fold(0.0, f64::max);
                values.push(format!("{max:.2}"));
            }
            for p in plots {
                values.push(plot_values.get(p.as_str()).map_or("-".to_owned(), |v| format!("{v:.3}")));
            }
            if last_values.as_ref() == Some(&values) {
                continue;
            }
            let start = (row[0].t / row_secs).floor() * row_secs;
            let _ = writeln!(out, "| {start:.2} | {} |", values.join(" | "));
            last_values = Some(values);
        }
        out
    }
}

fn uniq(mut list: Vec<String>, item: String) -> Vec<String> {
    if !list.contains(&item) {
        list.push(item);
    }
    list
}

fn more(out: &mut String, total: usize, shown: usize) {
    if total > shown {
        let _ = writeln!(out, "- ... and {} more", total - shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Header;

    #[test]
    fn report_shows_what_the_mode_did() {
        let source = r#"
mode { api = 1, name = "T", params = { gain = number(1, 0, 2, "Gain") } }
function on_button(ev) if ev.pressed then hud_event("Press " .. ev.button) end end
function tick(dt, input)
  set(input.rumble.level * P.gain)
  plot("level", input.rumble.level)
end
"#;
        let session = Session {
            header: Header { version: 1, started: "now".into(), game: None, mode: "T".into(), duration: 2.0, marks: 1, frames: 0, zones: None },
            changes: vec![
                (0.5, Change::Button { name: "LT".into(), pressed: true }),
                (0.6, Change::Rumble { strong: 0.8, weak: 0.0 }),
                (0.7, Change::Button { name: "LT".into(), pressed: false }),
                (0.9, Change::Rumble { strong: 0.0, weak: 0.0 }),
                (1.2, Change::Mark),
            ],
            frames: Vec::new(),
        };
        let params = [("gain".to_owned(), ParamValue::Number(0.5))].into_iter().collect();
        let sim = simulate("t.luau", source, &params, &Inputs::default(), session, &Progress::default()).unwrap();
        let report = sim.report();
        assert!(report.contains("### Vibrations sent by the game (1)"), "{report}");
        assert!(report.contains("- 0.60 s: 0.30 s long, peak 0.80"), "{report}");
        assert!(report.contains("- 0.50 s: LT (held 0.20 s)"), "{report}");
        assert!(report.contains("- 0.50 s: Press LT"), "{report}");
        assert!(report.contains("### Moments the player marked as feeling wrong (1)\n"), "{report}");
        assert!(report.contains("- 1.20 s\n"), "{report}");
        assert!(report.contains("| t (s) | game rumble | held | marked | out main | plot level |"), "{report}");
        // Gain 0.5 halves the 0.8 rumble.
        assert!(report.contains("| 0.50 | 0.80 | - |  | 0.40 | 0.800 |"), "{report}");
        assert!(report.contains("| 1.00 | 0.00 | - | ⚑ | 0.00 | 0.000 |"), "{report}");
    }

    #[test]
    fn report_shows_the_game_sound() {
        let source = r#"
mode { api = 1, name = "T", phases = { battle = { sound = "intense battle music" }, calm = { sound = "calm music" } } }
function on_audio_hit(ev) pulse(ev.strength, 0.1) end
function tick(dt, input) end
"#;
        let session = Session {
            header: Header { version: 2, started: "now".into(), game: None, mode: "T".into(), duration: 1.0, marks: 0, frames: 0, zones: None },
            changes: vec![
                (0.1, Change::Audio { level: 0.6, low: 0.5, mid: 0.2, high: 0.1, intensity: 0.3 }),
                (0.3, Change::AudioHit { strength: 0.9, band: crate::audio::Band::Low }),
                (0.6, Change::NoAudio),
            ],
            frames: Vec::new(),
        };
        let report = simulate("t.luau", source, &BTreeMap::new(), &Inputs::default(), session, &Progress::default()).unwrap().report();
        assert!(report.contains("### The game's sound\n\n1 hits heard"), "{report}");
        assert!(report.contains("| sound | phase |"), "{report}");
        assert!(report.contains("| 0.25 | 0.00 | - | 0.60 | - | 0.90 |"), "{report}");
        // Without the downloaded model the phases are explained, not silently missing.
        if !Model::Sound.ready() {
            assert!(report.contains("Some phases could not be recognized"), "{report}");
        }
    }

    /// Indicators whose zones changed since the recording are read again from its images.
    #[test]
    fn indicators_are_read_again_when_their_zones_changed() {
        use crate::package::Zone;
        use crate::screen::{indicators, Frame};
        use crate::session::{encode_frame, FrameData, SessionFrame};
        // A ring in the bottom right corner over some scenery.
        let image = |shown: bool| {
            let (w, h) = (160, 90);
            let mut pixels = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let (dx, dy) = (x as f32 - 140.0, y as f32 - 75.0);
                    let ring = (dx * dx + dy * dy).sqrt();
                    let p = if shown && (6.0..10.0).contains(&ring) { [250, 250, 250] } else { [(x * 3 % 200) as u8, 90, ((y * 5) % 160) as u8] };
                    pixels.extend([p[0], p[1], p[2], 255]);
                }
            }
            Frame { width: w, height: h, source_width: w, source_height: h, count: 1, pixels }
        };
        let rect = [120.0 / 160.0, 60.0 / 90.0, 40.0 / 160.0, 30.0 / 90.0];
        let zone = Zone { indicator: "battle_hud".into(), rect, reference: indicators::reference(&image(true), rect), ..Zone::default() };
        let inputs = Inputs { zones: vec![zone], ..Inputs::default() };
        let session = |zones: Option<Vec<Zone>>| Session {
            header: Header { version: 4, started: "now".into(), game: None, mode: "T".into(), duration: 2.0, marks: 0, frames: 2, zones },
            // Shown all along, as read in the zones of the recording.
            changes: vec![(0.0, Change::Indicator { name: "battle_hud".into(), value: IndicatorValue::Visibility(true) })],
            frames: [(0.0, true), (1.0, false)]
                .into_iter()
                .map(|(t, shown)| SessionFrame { t, data: FrameData::Jpeg(encode_frame(&image(shown)).unwrap()) })
                .collect(),
        };
        let source = "mode { api = 1, name = 'T' } function tick(dt, input) set(input.indicators.battle_hud and 1 or 0) end";
        let replay = |zones| simulate("t.luau", source, &BTreeMap::new(), &inputs, session(zones), &Progress::default()).unwrap();
        let shown = |sim: &Simulation, t: f64| sim.indicators_at(t).get("battle_hud").copied();

        let same = replay(Some(inputs.zones.clone()));
        assert!(!same.indicators_reread());
        assert_eq!(shown(&same, 1.5), Some(IndicatorValue::Visibility(true)), "as recorded");

        let changed = replay(None);
        assert!(changed.indicators_reread());
        assert_eq!(shown(&changed, 0.5), Some(IndicatorValue::Visibility(true)));
        assert_eq!(shown(&changed, 1.5), Some(IndicatorValue::Visibility(false)), "read from the image");
        let out = |t: f64| changed.ticks().iter().find(|k| k.t >= t).unwrap().channels.values().copied().fold(0.0, f64::max);
        assert_eq!((out(0.5), out(1.5)), (1.0, 0.0), "the mode is given them");
    }

    #[test]
    fn a_replay_stops_when_asked() {
        let source = "mode { api = 1, name = 'T' } function tick(dt, input) end";
        let session = Session {
            header: Header { version: 1, started: "now".into(), game: None, mode: "T".into(), duration: 1.0, marks: 0, frames: 0, zones: None },
            changes: Vec::new(),
            frames: Vec::new(),
        };
        let stopped = Progress::default();
        stopped.stop();
        assert!(simulate("t.luau", source, &BTreeMap::new(), &Inputs::default(), session, &stopped).is_err());
    }

    #[test]
    fn runtime_errors_are_reported() {
        let source = "mode { api = 1, name = 'T' } function tick(dt, input) if input.time > 0.1 then error('boom') end end";
        let session = Session {
            header: Header { version: 1, started: "now".into(), game: None, mode: "T".into(), duration: 1.0, marks: 0, frames: 0, zones: None },
            changes: Vec::new(),
            frames: Vec::new(),
        };
        let report = simulate("t.luau", source, &BTreeMap::new(), &Inputs::default(), session, &Progress::default()).unwrap().report();
        assert!(report.contains("stopped on an error"), "{report}");
        assert!(report.contains("boom"), "{report}");
    }
}
