//! Overlay page: installing the in-game overlay (a Vulkan layer), enabling it
//! per game, its appearance and the games showing it now.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::config::{Corner, OverlaySettings};
use crate::engine::{Command, Shared};
use crate::overlay::{self, Arch, InstallState};

const LAUNCH_OPTION: &str = "GAMEVIBER_OVERLAY=1 %command%";
/// The overlay runs inside the game's process, which anti-cheats watch for;
/// the rest of GameViber never touches the game.
const ANTICHEAT_WARNING: &str = "⚠ The overlay runs inside the game, like MangoHud or the Steam overlay. \
     An anti-cheat could mistake it for a cheat: do not enable it in online games with an anti-cheat \
     (EasyAntiCheat, BattlEye, VAC). Rumble and toys work without it and never touch the game.";

#[derive(Default)]
pub struct State {
    /// Checked when the page is shown, after each install action and when the
    /// scope (the bool: every game) changes.
    install: Option<(bool, Vec<(Arch, InstallState)>)>,
    error: Option<String>,
    /// Launch option last copied.
    copied: Option<String>,
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
                if overlay::WINDOW_CAPTURE {
                    window_capture(ui, s);
                    return;
                }
                required(ui);
                ui.add_space(8.0);
                self.overlay_install(ui, s);
                ui.add_space(8.0);
                if self.overlay.install.as_ref().is_some_and(|(_, archs)| overall(archs) != InstallState::NotInstalled) {
                    launch_options(ui, s.settings.overlay.all_games, &mut self.overlay.copied);
                    ui.add_space(8.0);
                    if let Some(new) = appearance(ui, &s.settings.overlay) {
                        self.send(Command::SetOverlay(new));
                    }
                    ui.add_space(8.0);
                    connected_games(ui, s);
                }
            });
        });
    }

    /// The overlay's files, checked again only when forgotten or the scope changed.
    fn overlay_archs(&mut self, s: &Shared) -> Vec<(Arch, InstallState)> {
        let all_games = s.settings.overlay.all_games;
        if self.overlay.install.as_ref().is_none_or(|(all, _)| *all != all_games) {
            self.overlay.install = Some((all_games, overlay::install_state(all_games)));
        }
        self.overlay.install.as_ref().map(|(_, a)| a.clone()).unwrap_or_default()
    }

    /// The overlay is installed, even outdated.
    pub(super) fn overlay_installed(&mut self, s: &Shared) -> bool {
        overall(&self.overlay_archs(s)) != InstallState::NotInstalled
    }

    fn overlay_install(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mut settings = s.settings.overlay.clone();
        let archs = self.overlay_archs(s);
        let install = overall(&archs);
        // Installed with GameViber's package: nothing to install or remove.
        let packaged = overlay::packaged();
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (color, text) = match install {
                    InstallState::NotInstalled | InstallState::NotBuilt => (IDLE, "Not installed"),
                    InstallState::Outdated => (WARN, "Installed, update available"),
                    InstallState::Installed if packaged => (OK, "Installed with GameViber"),
                    InstallState::Installed => (OK, "Installed"),
                };
                ui.label(RichText::new("Overlay").strong());
                dot(ui, color);
                ui.label(text);
            });
            ui.label(muted(
                "Works with Vulkan and OpenGL games, 64-bit and 32-bit, which includes every Windows game \
                 run through Proton (Steam, Lutris, Heroic).",
            ));
            for (arch, state) in &archs {
                let text = match state {
                    InstallState::NotBuilt if packaged => "not included in this package",
                    InstallState::NotBuilt => "not included in this build of GameViber",
                    InstallState::NotInstalled => "not installed",
                    InstallState::Outdated => "update available",
                    InstallState::Installed => "installed",
                };
                ui.label(muted(format!("{} games: {text}", arch.label())).size(12.0));
            }
            ui.add_space(4.0);
            ui.label(RichText::new(ANTICHEAT_WARNING).color(WARN).size(12.5));
            ui.add_space(4.0);
            let mut all_games = settings.all_games;
            ui.radio_value(&mut all_games, false, "Only in games I enable it for (recommended)");
            ui.radio_value(&mut all_games, true, "In every Vulkan game and program");
            if all_games {
                ui.label(
                    RichText::new(
                        "The overlay then loads into every Vulkan game, including online games with an \
                         anti-cheat. Set DISABLE_GAMEVIBER_OVERLAY=1 in those games' launch options.",
                    )
                    .color(WARN)
                    .size(12.5),
                );
            }
            let scope_changed = all_games != settings.all_games;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = match install {
                    InstallState::Outdated if packaged => "Repair",
                    InstallState::NotInstalled | InstallState::NotBuilt => "Install",
                    InstallState::Outdated => "Update",
                    InstallState::Installed => "Reinstall",
                };
                let reinstall = scope_changed && install != InstallState::NotInstalled;
                let shown = !packaged || install == InstallState::Outdated;
                let install_now = shown && ui.add(primary(label)).clicked();
                if install_now || reinstall {
                    self.overlay.error = overlay::install(all_games).err().map(|e| e.to_string());
                    self.overlay.install = None;
                }
                if !packaged && install != InstallState::NotInstalled && ui.button("Remove").clicked() {
                    self.overlay.error = overlay::uninstall().err().map(|e| e.to_string());
                    self.overlay.install = None;
                }
            });
            if let Some(e) = &self.overlay.error {
                ui.label(RichText::new(e).color(DANGER_TEXT));
            }
            if !packaged {
                ui.label(muted("Games started before installing need a restart.").size(12.0));
            }
            if scope_changed {
                settings.all_games = all_games;
                self.send(Command::SetOverlay(settings));
            }
        });
    }
}

/// On Linux, the game's image comes only through the overlay.
fn required(ui: &mut egui::Ui) {
    card(PANEL).stroke(egui::Stroke::new(1.5, DANGER)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("⚠ Required to play modes made for a game").strong().size(15.0).color(DANGER_TEXT));
        ui.label(
            "GameViber sees the game's image only through the overlay. Without it, the indicators a mode reads \
             (health, gauges...) and the phases recognized from the image stay empty: most modes made for a game \
             then barely work. Install it, then enable it in each game below.",
        );
    });
}

/// The state to show for the whole overlay: the 64-bit layer decides whether it
/// is installed; any architecture left behind makes it outdated.
fn overall(archs: &[(Arch, InstallState)]) -> InstallState {
    let main = archs.iter().find(|(a, _)| *a == Arch::X86_64).map(|(_, s)| *s).unwrap_or_default();
    if matches!(main, InstallState::NotInstalled | InstallState::NotBuilt) {
        return InstallState::NotInstalled;
    }
    let behind = archs.iter().any(|(_, s)| matches!(s, InstallState::Outdated | InstallState::NotInstalled));
    if behind {
        InstallState::Outdated
    } else {
        InstallState::Installed
    }
}

/// How to turn the overlay on in a game: the environment variable is enough for
/// Vulkan (unless the layer is on everywhere), OpenGL needs the launcher.
fn launch_options(ui: &mut egui::Ui, all_games: bool, copied: &mut Option<String>) {
    let launcher = overlay::launcher_path().display().to_string();
    let launcher = if launcher.contains(' ') { format!("\"{launcher}\"") } else { launcher };
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Enable it in a game").strong());
        ui.label(muted("Steam: right-click the game › Properties › Launch options, and paste:"));
        let mut option = |ui: &mut egui::Ui, title: &str, text: String| {
            ui.label(RichText::new(title).size(12.5));
            ui.horizontal(|ui| {
                ui.label(RichText::new(&text).monospace().color(ACCENT_TEXT));
                if ui.button("📋 Copy").clicked() {
                    ui.ctx().copy_text(text.clone());
                    *copied = Some(text.clone());
                }
                if copied.as_deref() == Some(text.as_str()) {
                    ui.label(RichText::new("✔ Copied").color(OK));
                }
            });
        };
        if !all_games {
            option(ui, "Windows games (Proton) and Vulkan games:", LAUNCH_OPTION.to_owned());
        }
        option(ui, "Native Linux games using OpenGL (also works for any game):", format!("{launcher} %command%"));
        ui.label(muted(
            "If the game already has launch options, put this before %command%. Lutris, Heroic and others: \
             set the environment variable GAMEVIBER_OVERLAY to 1, or use the launcher as a command prefix.",
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

/// Without an overlay (Windows): the game's image comes from its window.
fn window_capture(ui: &mut egui::Ui, s: &Shared) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Overlay").strong());
            dot(ui, IDLE);
            ui.label("Not available on this system yet");
        });
        ui.label(muted(
            "Modes still see the game's image: GameViber reads the window of the game in front, fullscreen or \
             borderless (or any window of a game from Steam, Epic, GOG or Xbox), without getting into the game. \
             A game in exclusive fullscreen may give a black image: switch it to borderless.",
        ));
    });
    ui.add_space(8.0);
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Game read now").strong());
        if s.overlay_clients.is_empty() {
            ui.label(muted("None. Bring the game to the front."));
        }
        for client in &s.overlay_clients {
            ui.horizontal(|ui| {
                dot(ui, if client.frames { OK } else { IDLE });
                ui.label(&client.exe);
                ui.label(muted(format!("pid {}{}", client.pid, if client.frames { " · image read" } else { "" })).size(12.0));
            });
        }
    });
}

fn connected_games(ui: &mut egui::Ui, s: &Shared) {
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Games showing it now").strong());
        if s.overlay_unavailable {
            ui.label(
                RichText::new(
                    "Another program holds the overlay connection (another GameViber?): games show its panel, \
                     not this one. Close it; GameViber takes over within a few seconds.",
                )
                .color(DANGER_TEXT),
            );
        } else if s.overlay_clients.is_empty() {
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
