//! Updates: a banner under the status bar when a new GameViber is out, and
//! the Updates card of the Settings page (check, release notes, download and
//! install when GameViber can do it for this installation, restart).

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::{App, Page};
use crate::engine::{Command, Shared};
use crate::update::{self, Installation, Phase, Updater};

impl App {
    /// Starts the update checks once the settings are known.
    pub(super) fn start_updater(&mut self, s: &Shared) {
        if self.updater.is_none() && s.time > 0.0 {
            self.updater = Some(Updater::start(s.settings.check_updates));
        }
    }

    /// Under the status bar while a new version waits, leading to the Updates
    /// card (not on the Settings page, which shows it).
    pub(super) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(updater) = &self.updater else { return };
        if self.update_banner_closed || self.page == Page::Settings {
            return;
        }
        let text = match updater.status().phase {
            Phase::Available(release) => format!("GameViber {} is available.", release.version),
            Phase::Installed(release) => format!("GameViber {} is installed: restart GameViber to use it.", release.version),
            _ => return,
        };
        let frame = egui::Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(16, 6));
        egui::Panel::top("update").frame(frame).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⬆").color(ACCENT));
                ui.label(text);
                if ui.add(primary("See the update")).clicked() {
                    self.page = Page::Settings;
                }
                if ui.button("Later").clicked() {
                    self.update_banner_closed = true;
                }
            });
        });
    }

    pub(super) fn updates_card(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let Some(updater) = &self.updater else { return };
        let status = updater.status();
        let installation = status.installation.clone();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Updates").strong().size(15.0));
            let how = installation.as_ref().map(Installation::describe).unwrap_or_default();
            ui.label(muted(format!("GameViber {} · {how}", update::current_version())));
        });
        let mut automatic = s.settings.check_updates;
        if ui.checkbox(&mut automatic, "Check for new versions automatically").changed() {
            updater.set_automatic(automatic);
            self.send(Command::SetCheckUpdates(automatic));
        }
        ui.add_space(4.0);
        let busy = matches!(status.phase, Phase::Checking | Phase::Downloading { .. } | Phase::Installing(_) | Phase::Installed(_));
        match &status.phase {
            Phase::Idle => {
                ui.label(muted("Not checked yet."));
            }
            Phase::Checking => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Looking for a new version...");
                });
            }
            Phase::UpToDate => {
                let ago = status.last_check.map(|t| t.elapsed().as_secs() / 60).unwrap_or_default();
                ui.label(RichText::new(format!("✔ Up to date (checked {ago} min ago)")).color(OK));
            }
            Phase::Available(release) => {
                ui.label(RichText::new(format!("GameViber {} is available.", release.version)).strong().color(ACCENT));
                release_notes(ui, release);
                match installation.as_ref() {
                    Some(i) if i.installs_updates() => {
                        let password = matches!(i, Installation::Package(_));
                        ui.horizontal(|ui| {
                            if ui.add(primary("Download and install")).clicked() {
                                updater.install();
                            }
                            if password {
                                ui.label(muted("GameViber asks for your password to install it."));
                            }
                        });
                    }
                    Some(Installation::Managed(by)) => {
                        ui.label(format!("Update GameViber through {by}."));
                    }
                    _ => {
                        ui.label(muted("Built from source: git pull, then cargo build --release."));
                    }
                }
            }
            Phase::Downloading { release, done, total } => {
                ui.label(format!("Downloading GameViber {}...", release.version));
                let fraction = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                ui.add(egui::ProgressBar::new(fraction).desired_width(320.0).text(format!("{} / {} MB", done >> 20, total >> 20)));
            }
            Phase::Installing(release) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let password = matches!(installation, Some(Installation::Package(_)));
                    let text = if password { "Installing: enter your password in the dialog" } else { "Installing" };
                    ui.label(format!("{text} (GameViber {})...", release.version));
                });
            }
            Phase::Installed(release) => {
                ui.label(RichText::new(format!("✔ GameViber {} is installed.", release.version)).color(OK));
                ui.horizontal(|ui| {
                    if ui.add(primary("Restart GameViber now")).clicked() {
                        update::request_restart();
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    ui.label(muted("Toys stop during the restart. Running games keep the old overlay until they restart."));
                });
            }
            Phase::Failed { release, error } => {
                ui.label(RichText::new(format!("⚠ {error}")).color(DANGER_TEXT));
                if let Some(release) = release {
                    release_notes(ui, release);
                    if installation.as_ref().is_some_and(Installation::installs_updates) && ui.add(primary("Try again")).clicked() {
                        updater.install();
                    }
                }
            }
        }
        ui.add_space(4.0);
        if ui.add_enabled(!busy, egui::Button::new("Check now")).clicked() {
            updater.check();
        }
    }
}

fn release_notes(ui: &mut egui::Ui, release: &update::Release) {
    ui.horizontal(|ui| {
        ui.hyperlink_to("Release page", &release.page);
    });
    if !release.notes.trim().is_empty() {
        egui::CollapsingHeader::new("What's new").id_salt("release-notes").show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                ui.label(release.notes.trim());
            });
        });
    }
}
