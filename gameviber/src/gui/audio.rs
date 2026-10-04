//! Sound page: which sound GameViber listens to, what it hears right now,
//! and the scene model that lets modes recognize battles, calm moments...
//! from the game's music.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::audio::clap::{ModelState, DOWNLOAD_SIZE};
use crate::config::AudioSource;
use crate::engine::{AudioView, Command, Shared};

/// A hit stays lit this long.
const HIT_SECS: f64 = 0.3;

impl App {
    pub(super) fn audio_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Game sound");
                ui.label(muted(
                    "Modes can react to the game's sound: impacts, loudness, and scenes such as a battle or a calm \
                     walk, recognized from the music. The sound is analysed on your computer and never saved.",
                ));
                ui.add_space(8.0);
                if let Some(command) = source_picker(ui, s) {
                    self.send(command);
                }
                ui.add_space(8.0);
                heard_now(ui, s);
                ui.add_space(8.0);
                if let Some(command) = scene_model(ui, &s.audio, s.mode.info.as_ref().map(|i| i.name.as_str())) {
                    self.send(command);
                }
            });
        });
    }
}

fn source_picker(ui: &mut egui::Ui, s: &Shared) -> Option<Command> {
    let audio = &s.audio;
    let current = &s.settings.audio;
    let mut command = None;
    card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Listen to");
        let mut pick = |ui: &mut egui::Ui, source: AudioSource, label: &str, hint: &str| {
            let response = ui.radio(*current == source, label).on_hover_text(hint);
            if response.clicked() && *current != source {
                command = Some(Command::SetAudio(source));
            }
        };
        pick(
            ui,
            AudioSource::Auto,
            "Automatic",
            "The game showing the in-game overlay, or else everything the computer plays",
        );
        let mut apps = audio.status.streams.clone();
        if let AudioSource::App(name) = current {
            if !apps.contains(name) {
                apps.push(name.clone());
            }
        }
        for app in apps {
            pick(ui, AudioSource::App(app.clone()), &app, "Only this application");
        }
        pick(ui, AudioSource::Off, "Off", "Modes hear nothing");
        ui.add_space(4.0);
        let (color, text) = match (&audio.status.target, current, &audio.status.error) {
            (_, _, Some(error)) => (DANGER, format!("Cannot capture the sound: {error}")),
            (Some(target), _, None) => (OK, format!("Listening to {target}")),
            (None, AudioSource::Off, None) => (IDLE, "Off".to_owned()),
            (None, AudioSource::App(app), None) => (WARN, format!("Waiting for {app} to play sound")),
            (None, AudioSource::Auto, None) => (WARN, "Starting...".to_owned()),
        };
        ui.horizontal(|ui| {
            dot(ui, color);
            ui.label(text);
        });
        if *current == AudioSource::Auto && audio.status.target.as_deref().is_some_and(|t| t.starts_with("everything")) {
            ui.label(muted(
                "Music or voice chat playing next to the game is heard too. Pick the game in the list, or show the \
                 in-game overlay so that GameViber finds it.",
            ));
        }
    });
    command
}

fn heard_now(ui: &mut egui::Ui, s: &Shared) {
    let audio = &s.audio;
    card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Heard right now");
        let Some(levels) = audio.levels else {
            ui.label(muted("Nothing: start a game, or check the choice above."));
            return;
        };
        egui::Grid::new("audio-levels").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for (label, value, hint) in [
                ("Loudness", levels.level, "input.audio.level"),
                ("Bass", levels.low, "input.audio.low"),
                ("Mids", levels.mid, "input.audio.mid"),
                ("Treble", levels.high, "input.audio.high"),
                ("Intensity", levels.intensity, "input.audio.intensity: loudness and hits over the last seconds"),
            ] {
                ui.label(muted(label)).on_hover_text(hint);
                meter(ui, 240.0, value, GAME);
                ui.end_row();
            }
        });
        ui.horizontal(|ui| match audio.last_hit {
            Some((t, hit)) if s.time - t < HIT_SECS => {
                pill(ui, &format!("Hit {:.0}% · {}", hit.strength * 100.0, hit.band.name()), ON_ACCENT, ACCENT)
            }
            Some(_) => pill(ui, "Hits detected", MUTED, RAISED),
            None => {
                ui.label(muted("No hit heard yet."));
            }
        });
    });
}

fn scene_model(ui: &mut egui::Ui, audio: &AudioView, mode: Option<&str>) -> Option<Command> {
    let mut command = None;
    card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, "Scene recognition");
        match &audio.model {
            ModelState::Missing | ModelState::Failed(_) => {
                ui.label(
                    "Recognizing scenes needs a sound model (CLAP, by LAION). It is downloaded once and runs on your \
                     computer, using a little processor time while a mode uses scenes.",
                );
                if let ModelState::Failed(error) = &audio.model {
                    ui.label(RichText::new(format!("Download failed: {error}")).color(DANGER_TEXT).size(12.0));
                }
                let label = format!("Download the scene model ({} MB)", DOWNLOAD_SIZE / 1_000_000);
                if ui.add(primary(&label)).clicked() {
                    command = Some(Command::DownloadAudioModel);
                }
            }
            ModelState::Downloading { done, total } => {
                ui.label("Downloading the scene model...");
                let fraction = *done as f32 / (*total).max(1) as f32;
                ui.add(egui::ProgressBar::new(fraction).desired_width(320.0).text(format!(
                    "{} / {} MB",
                    done / 1_000_000,
                    total / 1_000_000
                )));
            }
            ModelState::Ready => {
                ui.horizontal(|ui| {
                    dot(ui, OK);
                    ui.label("Scene model ready");
                });
            }
        }
        ui.add_space(6.0);
        if audio.scenes.is_empty() {
            ui.label(muted(format!(
                "{} does not use scenes. Modes written for a game by an AI assistant can.",
                mode.unwrap_or("The active mode")
            )));
            return;
        }
        ui.label(RichText::new(format!("Scenes of {}", mode.unwrap_or("the mode"))).strong());
        if audio.model == ModelState::Ready && !audio.scenes_ready {
            ui.label(muted("Preparing..."));
        }
        egui::Grid::new("audio-scenes").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
            for (name, p) in &audio.scenes {
                let current = audio.scene.as_deref() == Some(name.as_str());
                let label = RichText::new(name.as_str());
                ui.label(if current { label.strong().color(ACCENT_TEXT) } else { label.color(MUTED) });
                meter(ui, 200.0, *p, if current { ACCENT } else { GAME });
                ui.label(muted(format!("{:.0}%", p * 100.0)));
                ui.end_row();
            }
        });
        if audio.scenes_ready && audio.scene.is_none() {
            ui.label(muted("No scene recognized yet: it takes about 10 s of sound."));
        }
    });
    command
}
