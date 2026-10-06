//! Sharing a mode with its game (`crate::sharing`): exported to a
//! `.gameviber` file from the mode's page, imported from the library. The
//! file dialog and the copy run in a thread; what they did shows above the
//! Games page.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::{App, GameView};
use crate::config::ModeEntry;
use crate::engine::{Command, Shared};
use crate::game::Game;
use crate::sharing::{self, Imported, EXTENSION};

const KIND: &str = "GameViber modes";

enum Outcome {
    Exported(PathBuf),
    Imported(Imported),
    Cancelled,
    Failed(String),
}

#[derive(Default)]
pub struct State {
    job: Option<Receiver<Outcome>>,
    /// What the last export or import did: true when it went well.
    message: Option<(bool, String)>,
}

impl State {
    /// A file dialog is open, or a file is being written or read.
    pub(super) fn busy(&self) -> bool {
        self.job.is_some()
    }
}

impl App {
    /// Asks where to save `mode` with `game`, and writes it there.
    pub(super) fn export_mode(&mut self, game: &Game, mode: &str) {
        let (game, mode, name) = (game.id.clone(), mode.to_owned(), sharing::file_name(game, mode));
        self.start_sharing(move || match crate::platform::save_file("Export a mode", KIND, EXTENSION, &name) {
            Ok(Some(mut path)) => {
                if path.extension().is_none() {
                    path.set_extension(EXTENSION);
                }
                match sharing::export(&game, &mode, &path, &[]) {
                    Ok(()) => Outcome::Exported(path),
                    Err(e) => Outcome::Failed(format!("Cannot export the mode: {e:#}")),
                }
            }
            Ok(None) => Outcome::Cancelled,
            Err(e) => Outcome::Failed(format!("Cannot export the mode: {e:#}")),
        });
    }

    /// The active mode's Sharing tab: as a file now, in the community later.
    pub(super) fn mode_sharing(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        if ModeEntry::from_id(&s.mode.id).builtin {
            ui.label(muted("Built-in modes come with every GameViber: there is no need to share them."));
            return;
        }
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("As a file").strong().size(16.0));
            ui.label(format!(
                "A .gameviber file holds the mode, its variants, its inputs (phases, indicators, captures, external inputs) \
                 and {} to join it to. Captures are images of your screen: look at them before sending the file.",
                game.name
            ));
            ui.add_space(6.0);
            if ui.add_enabled(!self.sharing.busy(), primary("Export")).clicked() {
                self.export_mode(game, &s.mode.id);
            }
        });
        ui.add_space(10.0);
        self.community_sharing(ui, s, game);
    }

    /// Asks for a `.gameviber` file, and imports it.
    pub(super) fn import_mode(&mut self) {
        let commands = self.commands.clone();
        self.start_sharing(move || match crate::platform::open_file("Import a mode", KIND, EXTENSION) {
            Ok(Some(path)) => match sharing::import(&path) {
                Ok(imported) => {
                    let _ = commands.send(Command::Imported { game: imported.game.clone() });
                    Outcome::Imported(imported)
                }
                Err(e) => Outcome::Failed(format!("Cannot import {}: {e:#}", path.display())),
            },
            Ok(None) => Outcome::Cancelled,
            Err(e) => Outcome::Failed(format!("Cannot import a mode: {e:#}")),
        });
    }

    fn start_sharing(&mut self, job: impl FnOnce() -> Outcome + Send + 'static) {
        if self.sharing.busy() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.sharing.job = Some(rx);
        self.sharing.message = None;
        std::thread::spawn(move || {
            let _ = tx.send(job());
        });
    }

    /// Collects what a finished export or import did, and shows it.
    pub(super) fn sharing_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(outcome) = self.sharing.job.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.sharing.job = None;
            self.sharing.message = match outcome {
                Outcome::Exported(path) => {
                    Some((true, format!("Exported to {}: send this file to share the mode with its inputs.", path.display())))
                }
                Outcome::Imported(imported) => {
                    // Opened once the engine lists it.
                    self.games.open_when_listed(&imported.name, GameView::Modes);
                    let what = match (imported.new_game, imported.mode_existed) {
                        (true, _) => format!("{} was added with its mode.", imported.name),
                        (false, false) => format!("The mode joined {}, with its inputs.", imported.name),
                        (false, true) => format!("{} had this mode already: its inputs stay as they are.", imported.name),
                    };
                    Some((true, format!("Imported: {what}")))
                }
                Outcome::Cancelled => None,
                Outcome::Failed(error) => {
                    log::error!("{error}");
                    Some((false, error))
                }
            };
        }
        let Some((ok, text)) = &self.sharing.message else { return };
        let mut close = false;
        card(PANEL).stroke(egui::Stroke::new(1.0, LINE)).inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                dot(ui, if *ok { OK } else { DANGER });
                ui.add(egui::Label::new(RichText::new(text).color(if *ok { TEXT } else { DANGER_TEXT })).wrap());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| close = ui.small_button("✕").clicked());
            });
        });
        ui.add_space(8.0);
        if close {
            self.sharing.message = None;
        }
    }
}
