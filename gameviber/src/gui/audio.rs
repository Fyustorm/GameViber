//! Setup › Sound: which sound GameViber listens to by default (a game can
//! pick its own in its Signals), what it hears right now, and the sound scene
//! model. The model and scene cards are shared with a game's Signals.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::config::AudioSource;
use crate::engine::{Command, Shared};
use crate::models::{Model, ModelState};

/// A hit stays lit this long.
const HIT_SECS: f64 = 0.3;

impl App {
    pub(super) fn audio_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Sound, by default");
                ui.label(muted(
                    "Modes react to the game's sound: impacts, loudness, and scenes recognized from the music. This is \
                     what GameViber listens to for games that pick nothing else in their Signals. The sound is analysed \
                     on your computer and never saved.",
                ));
                ui.add_space(8.0);
                if let Some(command) = source_picker(ui, s) {
                    self.send(command);
                }
                ui.add_space(8.0);
                heard_now(ui, s);
                ui.add_space(8.0);
                if let Some(command) = model_card(ui, Model::Sound, &s.audio.model) {
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
        pick(
            ui,
            AudioSource::Everything,
            "Everything the computer plays",
            "Every sound, the game's and any other (music, voice chat...)",
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
            (None, AudioSource::Auto | AudioSource::Everything, None) => (WARN, "Starting...".to_owned()),
        };
        ui.horizontal(|ui| {
            dot(ui, color);
            ui.label(text);
        });
        let silent = audio.levels.is_some_and(|l| l.level == 0.0 && l.intensity < 0.01);
        if matches!(current, AudioSource::App(_)) && audio.status.target.is_some() && silent {
            ui.label(muted(
                "Nothing heard from it yet. If the game is playing sound, pick \"Everything the computer plays\".",
            ));
        }
        if *current != AudioSource::Off && audio.status.target.as_deref().is_some_and(|t| t.starts_with("everything")) {
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

/// Download of a scene model, or its state.
pub(super) fn model_card(ui: &mut egui::Ui, model: Model, state: &ModelState) -> Option<Command> {
    let mut command = None;
    let (title, what) = match model {
        Model::Sound => ("Scene recognition from the sound", "a sound model (CLAP, by LAION)"),
        Model::Image => ("Scene recognition from the image", "an image model (CLIP, by OpenAI)"),
    };
    card(PANEL).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        eyebrow(ui, title);
        match state {
            ModelState::Missing | ModelState::Failed(_) => {
                ui.label(format!(
                    "Recognizing scenes needs {what}. It is downloaded once and runs on your computer, using a \
                     little processor time while a mode uses scenes."
                ));
                if let ModelState::Failed(error) = state {
                    ui.label(RichText::new(format!("Download failed: {error}")).color(DANGER_TEXT).size(12.0));
                }
                let label = format!("Download the scene model ({} MB)", model.size() / 1_000_000);
                if ui.add(primary(&label)).clicked() {
                    command = Some(Command::DownloadModel(model));
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
    });
    command
}
