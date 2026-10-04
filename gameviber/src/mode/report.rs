//! What a mode did during a recorded session, for an AI assistant asked to
//! fix a mode that does not feel right: the session is replayed offline into
//! a fresh copy of the mode, and the game's vibrations, the player's presses,
//! what was heard of the game's sound, the mode's overlay messages and a
//! timeline of its outputs and `plot()` values are written out as Markdown.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::Path;

use super::rumble_events::{RumbleEvent, RumbleTracker};
use super::scenes::Sense;
use super::{ModeEvent, ModeRuntime, ParamValue, ZoneValue};
use crate::models::{self, Model};
use crate::game::Game;
use crate::gamepad::PadState;
use crate::session::{Change, Player, Session};

const DT: f64 = 0.02;
/// The timeline has at most this many rows (before identical rows are dropped).
const MAX_ROWS: usize = 300;
const MIN_ROW_SECS: f64 = 0.25;
const MAX_VIBRATIONS: usize = 150;
const MAX_PRESSES: usize = 200;
const MAX_PLOTS: usize = 6;

struct Tick {
    t: f64,
    rumble: f64,
    held: Vec<&'static str>,
    /// Loudness of the game's sound, when it was captured.
    sound: Option<f64>,
    /// Action on the game's screen, when it was copied.
    image: Option<f64>,
    scene: Option<String>,
    channels: BTreeMap<String, f64>,
    plots: Vec<(String, f64)>,
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
    /// When a zone changed, and to what.
    zone_changes: Vec<(f64, String, ZoneValue)>,
    /// Events other programs sent, and how many values they set.
    external: Vec<(f64, String)>,
    custom_values: usize,
    /// The mode declares scenes.
    has_scenes: bool,
    /// Why the scenes could not be recognized.
    scenes_unavailable: Option<String>,
    /// When the recognized scene changed, and to what.
    scene_changes: Vec<(f64, Option<String>)>,
}

/// Replays `session` into a freshly loaded mode with `params`.
pub fn simulate(
    chunk_name: &str,
    source: &str,
    params: &BTreeMap<String, ParamValue>,
    session: Session,
) -> Result<Simulation, String> {
    let mut rt = ModeRuntime::load(chunk_name, source, params, None)?;
    rt.start()?;
    let info = rt.info().clone();
    let duration = session.header.duration;
    let game = session.header.game.clone();
    let marks = session.changes.iter().filter(|(_, c)| *c == Change::Mark).map(|(t, _)| *t).collect();
    let mut player = Player::new(session, Path::new(""), 0.0);
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
        zone_changes: Vec::new(),
        external: Vec::new(),
        custom_values: 0,
        has_scenes: !info.scenes.is_empty(),
        scenes_unavailable: None,
        scene_changes: Vec::new(),
    };
    // The game the session was played in: its scenes and captures, as they are now.
    let played = game.as_ref().and_then(|name| Game::list().into_iter().find(|g| g.name == *name || g.runs_as(name)));
    if let Some(played) = &played {
        rt.set_game_scenes(&played.scene_decls());
        rt.set_scene_references(Sense::Examples, played.example_centroids());
    }
    sim.has_scenes = !rt.scene_decls().is_empty();
    let mut unavailable = Vec::new();
    for (sense, model) in [(Sense::Sound, Model::Sound), (Sense::Screen, Model::Image)] {
        let descriptions = rt.scene_descriptions(sense);
        if descriptions.is_empty() {
            continue;
        }
        let texts: Vec<String> = descriptions.iter().map(|(_, d)| d.clone()).collect();
        match models::text_embeddings(model, &texts) {
            Ok(embeddings) => rt.set_scene_references(sense, descriptions.into_iter().map(|(n, _)| n).zip(embeddings).collect()),
            Err(e) => unavailable.push(format!("{e:#}")),
        }
    }
    if !unavailable.is_empty() {
        sim.scenes_unavailable = Some(unavailable.join("; "));
    }

    let mut scene = None;
    let mut vibration_start = 0.0;
    let mut step = 0;
    loop {
        let t = step as f64 * DT;
        if t > duration {
            break;
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
        let replayed = player.take_events();
        for event in &replayed {
            match event {
                ModeEvent::AudioHit(_) => sim.audio_hits += 1,
                ModeEvent::ScreenFlash(_) => sim.flashes += 1,
                ModeEvent::Zone { name, value } => {
                    if !sim.zone_changes.iter().rev().find(|(_, n, _)| n == name).is_some_and(|(_, _, v)| v == value) {
                        sim.zone_changes.push((t, name.clone(), *value));
                    }
                }
                ModeEvent::External { name, .. } => sim.external.push((t, name.clone())),
                ModeEvent::Custom { .. } => sim.custom_values += 1,
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
        let current = rt.scene_state().0;
        if current != scene {
            sim.scene_changes.push((t, current.clone()));
            scene = current;
        }
        sim.ticks.push(Tick {
            t,
            rumble: player.rumble.level(),
            held: pad.held().iter().copied().collect(),
            sound: player.audio.map(|a| a.level),
            image: player.screen.map(|l| l.action as f64),
            scene: scene.clone(),
            channels: out.channels,
            plots: out.plots,
        });
    }
    Ok(sim)
}

impl Simulation {
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
    /// programs, and the scenes recognized.
    fn sound(&self) -> String {
        let mut out = String::new();
        if self.ticks.iter().any(|t| t.sound.is_some()) {
            let _ = writeln!(out, "\n### The game's sound\n\n{} hits heard (on_audio_hit).", self.audio_hits);
        }
        if self.ticks.iter().any(|t| t.image.is_some()) {
            let _ = writeln!(out, "\n### The game's image\n\n{} flashes seen (on_impact).", self.flashes);
            if !self.zone_changes.is_empty() {
                let _ = writeln!(out, "\nZone changes ({}):\n", self.zone_changes.len());
                for (t, name, value) in self.zone_changes.iter().take(MAX_PRESSES) {
                    let value = match value {
                        ZoneValue::Visible(shown) => if *shown { "shown" } else { "hidden" }.to_owned(),
                        ZoneValue::Bar(fill) => format!("{fill:.2}"),
                        ZoneValue::Unknown => "unknown (not on screen)".to_owned(),
                    };
                    let _ = writeln!(out, "- {t:.2} s: {name} {value}");
                }
                more(&mut out, self.zone_changes.len(), MAX_PRESSES);
            }
        }
        if !self.external.is_empty() || self.custom_values > 0 {
            let _ = writeln!(out, "\n### Sent by other programs\n\n{} values set (input.custom).", self.custom_values);
            for (t, name) in self.external.iter().take(MAX_PRESSES) {
                let _ = writeln!(out, "- {t:.2} s: event {name}");
            }
            more(&mut out, self.external.len(), MAX_PRESSES);
        }
        if !self.has_scenes {
            return out;
        }
        out.push_str("\n### Scenes\n\n");
        if let Some(why) = &self.scenes_unavailable {
            let _ = writeln!(out, "Some scenes could not be recognized: {why}.\n");
        }
        if !self.ticks.iter().any(|t| t.sound.is_some() || t.image.is_some()) {
            out.push_str("Neither the sound nor the image was captured during this session: no scene was recognized.\n");
            return out;
        }
        let _ = writeln!(out, "Scene changes ({}):\n", self.scene_changes.len());
        if self.scene_changes.is_empty() {
            out.push_str("None: no scene was recognized.\n");
        }
        let mut previous: Option<&Option<String>> = None;
        for (t, scene) in self.scene_changes.iter().take(MAX_PRESSES) {
            let was = previous.map_or(String::new(), |p| format!(" (was {})", p.as_deref().unwrap_or("none")));
            let _ = writeln!(out, "- {t:.2} s: {}{was}", scene.as_deref().unwrap_or("none"));
            previous = Some(scene);
        }
        more(&mut out, self.scene_changes.len(), MAX_PRESSES);
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
             the maximum over the row, held buttons, the screen's action, the scene and plot() values are taken at its end. Rows equal to the previous one are left \
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
        if self.has_scenes {
            header.push("scene".into());
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
            if self.has_scenes {
                values.push(end.scene.clone().unwrap_or_else(|| "-".to_owned()));
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
            header: Header { version: 1, started: "now".into(), game: None, mode: "T".into(), duration: 2.0, marks: 1 },
            changes: vec![
                (0.5, Change::Button { name: "LT".into(), pressed: true }),
                (0.6, Change::Rumble { strong: 0.8, weak: 0.0 }),
                (0.7, Change::Button { name: "LT".into(), pressed: false }),
                (0.9, Change::Rumble { strong: 0.0, weak: 0.0 }),
                (1.2, Change::Mark),
            ],
        };
        let params = [("gain".to_owned(), ParamValue::Number(0.5))].into_iter().collect();
        let sim = simulate("t.luau", source, &params, session).unwrap();
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
mode { api = 1, name = "T", scenes = { battle = { sound = "intense battle music" }, calm = { sound = "calm music" } } }
function on_audio_hit(ev) pulse(ev.strength, 0.1) end
function tick(dt, input) end
"#;
        let session = Session {
            header: Header { version: 2, started: "now".into(), game: None, mode: "T".into(), duration: 1.0, marks: 0 },
            changes: vec![
                (0.1, Change::Audio { level: 0.6, low: 0.5, mid: 0.2, high: 0.1, intensity: 0.3 }),
                (0.3, Change::AudioHit { strength: 0.9, band: crate::audio::Band::Low }),
                (0.6, Change::NoAudio),
            ],
        };
        let report = simulate("t.luau", source, &BTreeMap::new(), session).unwrap().report();
        assert!(report.contains("### The game's sound\n\n1 hits heard"), "{report}");
        assert!(report.contains("| sound | scene |"), "{report}");
        assert!(report.contains("| 0.25 | 0.00 | - | 0.60 | - | 0.90 |"), "{report}");
        // Without the downloaded model the scenes are explained, not silently missing.
        if !Model::Sound.ready() {
            assert!(report.contains("Some scenes could not be recognized"), "{report}");
        }
    }

    #[test]
    fn runtime_errors_are_reported() {
        let source = "mode { api = 1, name = 'T' } function tick(dt, input) if input.time > 0.1 then error('boom') end end";
        let session = Session {
            header: Header { version: 1, started: "now".into(), game: None, mode: "T".into(), duration: 1.0, marks: 0 },
            changes: Vec::new(),
        };
        let report = simulate("t.luau", source, &BTreeMap::new(), session).unwrap().report();
        assert!(report.contains("stopped on an error"), "{report}");
        assert!(report.contains("boom"), "{report}");
    }
}
