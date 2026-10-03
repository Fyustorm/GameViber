//! Overlay page: installing the in-game overlay (a Vulkan layer), enabling it
//! per game, its appearance and the games showing it now.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::config::{Corner, OverlaySettings};
use crate::engine::{Command, Shared};
use crate::overlay::{self, InstallState};

const LAUNCH_OPTION: &str = "GAMEVIBER_OVERLAY=1 %command%";

#[derive(Default)]
pub struct State {
    /// Checked when the page is shown and after each install action.
    install: Option<InstallState>,
    error: Option<String>,
    copied: bool,
}

impl State {
    pub fn forget_install_state(&mut self) {
        self.install = None;
    }
}

impl App {
    pub(super) fn overlay_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "In-game overlay");
                ui.label(muted(
                    "A small panel drawn over the game: the active mode, how strong your toys run, the mode's \
                     gauges and what it detects (\"Parry!\"), and warnings such as a lost toy.",
                ));
                ui.add_space(8.0);
                self.overlay_install(ui, s);
                ui.add_space(8.0);
                if self.overlay.install.is_some_and(|i| i != InstallState::NotInstalled) {
                    if !s.settings.overlay.all_games {
                        launch_options(ui, &mut self.overlay.copied);
                        ui.add_space(8.0);
                    }
                    if let Some(new) = appearance(ui, &s.settings.overlay) {
                        self.send(Command::SetOverlay(new));
                    }
                    ui.add_space(8.0);
                    connected_games(ui, s);
                }
            });
        });
    }

    fn overlay_install(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let install = *self.overlay.install.get_or_insert_with(overlay::install_state);
        let mut settings = s.settings.overlay.clone();
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (color, text) = match install {
                    InstallState::NotInstalled => (IDLE, "Not installed"),
                    InstallState::Outdated => (WARN, "Installed, update available"),
                    InstallState::Installed => (OK, "Installed"),
                };
                ui.label(RichText::new("Overlay").strong());
                dot(ui, color);
                ui.label(text);
            });
            ui.label(muted(
                "Works with Vulkan games, which includes every Windows game run through Proton (Steam, \
                 Lutris, Heroic). OpenGL and 32-bit games are not supported yet.",
            ));
            ui.add_space(4.0);
            let mut all_games = settings.all_games;
            ui.radio_value(&mut all_games, false, "Only in games I enable it for (recommended)");
            ui.radio_value(&mut all_games, true, "In every Vulkan game and program");
            let scope_changed = all_games != settings.all_games;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = match install {
                    InstallState::NotInstalled => "Install",
                    InstallState::Outdated => "Update",
                    InstallState::Installed => "Reinstall",
                };
                let reinstall = scope_changed && install != InstallState::NotInstalled;
                if ui.add(primary(label)).clicked() || reinstall {
                    self.overlay.error = overlay::install(all_games).err().map(|e| e.to_string());
                    self.overlay.install = None;
                }
                if install != InstallState::NotInstalled && ui.button("Remove").clicked() {
                    self.overlay.error = overlay::uninstall().err().map(|e| e.to_string());
                    self.overlay.install = None;
                }
            });
            if let Some(e) = &self.overlay.error {
                ui.label(RichText::new(e).color(DANGER_TEXT));
            }
            ui.label(muted("Games started before installing need a restart.").size(12.0));
            if scope_changed {
                settings.all_games = all_games;
                self.send(Command::SetOverlay(settings));
            }
        });
    }
}

fn launch_options(ui: &mut egui::Ui, copied: &mut bool) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Enable it in a game").strong());
        ui.label(muted("Steam: right-click the game › Properties › Launch options, and paste:"));
        ui.horizontal(|ui| {
            ui.label(RichText::new(LAUNCH_OPTION).monospace().color(ACCENT_TEXT));
            if ui.button("📋 Copy").clicked() {
                ui.ctx().copy_text(LAUNCH_OPTION.to_owned());
                *copied = true;
            }
            if *copied {
                ui.label(RichText::new("✔ Copied").color(OK));
            }
        });
        ui.label(muted(
            "If the game already has launch options, add GAMEVIBER_OVERLAY=1 before them. Lutris, Heroic and \
             others: set the environment variable GAMEVIBER_OVERLAY to 1 in the game's settings.",
        ));
    });
}

/// Returns the new settings when the user changed one.
fn appearance(ui: &mut egui::Ui, current: &OverlaySettings) -> Option<OverlaySettings> {
    let mut s = current.clone();
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Appearance").strong());
        ui.checkbox(&mut s.visible, "Show the overlay");
        ui.horizontal(|ui| {
            ui.label("Position");
            egui::ComboBox::from_id_salt("overlay-corner").selected_text(s.corner.label()).show_ui(ui, |ui| {
                for corner in Corner::ALL {
                    ui.selectable_value(&mut s.corner, corner, corner.label());
                }
            });
        });
        ui.add(egui::Slider::new(&mut s.scale, 0.5..=2.0).text("Size").step_by(0.05).fixed_decimals(2));
        ui.add(egui::Slider::new(&mut s.opacity, 0.0..=1.0).text("Background opacity").step_by(0.05).fixed_decimals(2));
    });
    (s != *current).then_some(s)
}

fn connected_games(ui: &mut egui::Ui, s: &Shared) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Games showing it now").strong());
        if s.overlay_clients.is_empty() {
            ui.label(muted("None. Start a game with the overlay enabled; it appears within a second."));
        }
        for client in &s.overlay_clients {
            ui.horizontal(|ui| {
                dot(ui, OK);
                ui.label(&client.exe);
                ui.label(muted(format!("{} · pid {}", client.api, client.pid)).size(12.0));
            });
        }
        ui.add_space(4.0);
        ui.label(muted("Mode authors: hud() and hud_event() add gauges and messages to it.").size(12.0));
    });
}
