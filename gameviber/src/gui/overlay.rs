//! Overlay page: installing the in-game overlay (a Vulkan layer), enabling it
//! per game, its look (everywhere, in a game, in its phases) and the games
//! showing it now.

use eframe::egui::{self, Color32, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use gameviber_common::overlay::Hello;

use super::theme::*;
use super::App;
use crate::config::{OverlayLook, OverlaySettings};
use crate::engine::{Command, Shared};
use crate::game::{Game, GameOverlay};
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
    /// The game whose look is set (by id); None: the one played.
    pub game: Option<String>,
    /// The phase whose own look is open.
    phase: Option<String>,
}

impl State {
    pub fn forget_install_state(&mut self) {
        self.install = None;
    }
}

/// How the panel looks in a phase of a game.
#[derive(Clone, Copy, PartialEq)]
enum PhaseLook {
    LikeTheGame,
    Hidden,
    Own,
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
                    if let Some(new) = appearance(ui, &s.settings.overlay, &s.overlay_clients) {
                        self.send(Command::SetOverlay(new));
                    }
                    ui.add_space(8.0);
                    self.game_look(ui, s);
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
fn appearance(ui: &mut egui::Ui, current: &OverlaySettings, clients: &[Hello]) -> Option<OverlaySettings> {
    let mut s = current.clone();
    card(PANEL).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("Appearance").strong());
        ui.label(muted("In every game, unless a game has a look of its own (below)."));
        older_overlays(ui, clients);
        ui.checkbox(&mut s.look.visible, "Show the overlay");
        if s.look.visible {
            look_editor(ui, "everywhere", &mut s.look, false);
        }
        ui.add_space(4.0);
        ui.checkbox(&mut s.mode_hides, "Hide it when the mode says it may (cutscenes, dialogues)");
        ui.label(muted("Modes made for a game can tell when nothing happens on screen worth showing it.").size(12.0));
    });
    (s != *current).then_some(s)
}

impl App {
    /// The look in a game (the one played by default), and in each of its phases.
    fn game_look(&mut self, ui: &mut egui::Ui, s: &Shared) {
        if s.games.is_empty() {
            return;
        }
        let chosen = self.overlay.game.as_ref().and_then(|id| s.games.iter().find(|g| &g.id == id));
        let Some(game) = chosen.or(s.game.as_ref()).or(s.games.first()) else { return };
        let mut overlay = game.overlay.clone();
        card(PANEL).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("In a game").strong());
                egui::ComboBox::from_id_salt("overlay-game").selected_text(&game.name).width(240.0).show_ui(ui, |ui| {
                    for g in &s.games {
                        if ui.selectable_label(g.id == game.id, &g.name).clicked() {
                            self.overlay.game = Some(g.id.clone());
                            self.overlay.phase = None;
                        }
                    }
                });
            });
            let mut own = overlay.look.is_some();
            ui.horizontal(|ui| {
                ui.radio_value(&mut own, false, "Like every game");
                ui.radio_value(&mut own, true, "A look of its own");
            });
            match (own, &mut overlay.look) {
                (true, None) => overlay.look = Some(s.settings.overlay.look),
                (false, Some(_)) => overlay.look = None,
                (true, Some(look)) => {
                    ui.checkbox(&mut look.visible, format!("Show the overlay in {}", game.name));
                    if look.visible {
                        look_editor(ui, ("game", &game.id), look, false);
                    }
                }
                (false, None) => {}
            }
            ui.add_space(6.0);
            self.phase_looks(ui, s, game, &mut overlay);
        });
        if overlay != game.overlay {
            self.send(Command::SetGameOverlay { game: game.id.clone(), overlay });
        }
    }

    /// Each phase like the game, hidden (a cutscene) or with a look of its own.
    fn phase_looks(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, overlay: &mut GameOverlay) {
        ui.label(RichText::new("In its phases").strong());
        // The phases the active mode recognizes when it is one of the game's, and those set before.
        let playing = super::library::mode_game(s).is_some_and(|g| g.id == game.id);
        let mut phases: Vec<String> = if playing { s.phases.phases.iter().map(|(name, _)| name.clone()).collect() } else { Vec::new() };
        phases.extend(overlay.phases.keys().filter(|p| !phases.contains(p)).cloned().collect::<Vec<_>>());
        phases.sort();
        if phases.is_empty() {
            ui.label(muted(
                "Its phases show here while one of its modes, with phases, is the active mode. The panel can then \
                 move, fade or hide in some of them (a cutscene).",
            ));
            return;
        }
        let game_look = overlay.look.unwrap_or(s.settings.overlay.look);
        for phase in &phases {
            let current = match overlay.phases.get(phase) {
                None => PhaseLook::LikeTheGame,
                Some(look) if !look.visible => PhaseLook::Hidden,
                Some(_) => PhaseLook::Own,
            };
            let mut chosen = current;
            ui.horizontal(|ui| {
                ui.add_sized([160.0, 18.0], egui::Label::new(phase.as_str()).truncate());
                let label = |l: PhaseLook| match l {
                    PhaseLook::LikeTheGame => "Like the game",
                    PhaseLook::Hidden => "Hidden",
                    PhaseLook::Own => "A look of its own",
                };
                egui::ComboBox::from_id_salt(("overlay-phase", phase)).selected_text(label(current)).width(150.0).show_ui(ui, |ui| {
                    for option in [PhaseLook::LikeTheGame, PhaseLook::Hidden, PhaseLook::Own] {
                        ui.selectable_value(&mut chosen, option, label(option));
                    }
                });
                if current == PhaseLook::Own {
                    let open = self.overlay.phase.as_ref() == Some(phase);
                    if ui.selectable_label(open, "✏ Edit").clicked() {
                        self.overlay.phase = (!open).then(|| phase.clone());
                    }
                }
                if playing && s.phases.phase.as_ref() == Some(phase) {
                    ui.label(RichText::new("● now").color(OK).size(12.0));
                }
            });
            if chosen != current {
                match chosen {
                    PhaseLook::LikeTheGame => {
                        overlay.phases.remove(phase);
                    }
                    PhaseLook::Hidden => {
                        let look = overlay.phases.get(phase).copied().unwrap_or(game_look);
                        overlay.phases.insert(phase.clone(), OverlayLook { visible: false, ..look });
                    }
                    PhaseLook::Own => {
                        let look = overlay.phases.get(phase).copied().unwrap_or(game_look);
                        overlay.phases.insert(phase.clone(), OverlayLook { visible: true, ..look });
                        self.overlay.phase = Some(phase.clone());
                    }
                }
            }
            if let Some(look) = overlay.phases.get_mut(phase).filter(|l| l.visible && self.overlay.phase.as_ref() == Some(phase)) {
                ui.indent(("overlay-phase-look", phase), |ui| look_editor(ui, ("phase", &game.id, phase), look, true));
            }
        }
        ui.label(muted("A phase's look is the game's for every mode of the game recognizing a phase of that name.").size(12.0));
    }
}

/// Games running an overlay older than this GameViber, which puts the panel
/// in the corner nearest to its place and ignores the panel opacity.
fn older_overlays(ui: &mut egui::Ui, clients: &[Hello]) {
    let older: Vec<&str> = clients.iter().filter(|c| !c.places_freely()).map(|c| c.exe.as_str()).collect();
    if older.is_empty() {
        return;
    }
    ui.label(
        RichText::new(format!(
            "⚠ {} runs an older overlay: it puts the panel in the nearest corner and ignores the panel opacity. \
             Update the overlay (or GameViber's package), then restart the game.",
            older.join(", ")
        ))
        .color(WARN)
        .size(12.5),
    );
}

/// Panel size on a 1080p screen at size 1 (`gameviber-overlay`'s), its
/// height with a phase and a gauge.
const PANEL_SIZE: Vec2 = Vec2::new(240.0, 130.0);
const PANEL_MARGIN: f32 = 20.0;

/// Where the panel sits, its size and opacity, on a picture of the screen
/// it is dragged on. `compact`: a smaller picture.
fn look_editor(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, look: &mut OverlayLook, compact: bool) {
    let id = egui::Id::new(id);
    ui.horizontal(|ui| {
        let width = if compact { 280.0_f32 } else { 400.0 }.min(ui.available_width() - 120.0).max(160.0);
        screen_picture(ui, id, look, width);
        ui.vertical(|ui| {
            ui.label(muted("Drag it, or:").size(12.0));
            egui::Grid::new(id.with("spots")).spacing([4.0, 4.0]).show(ui, |ui| {
                for y in [0.0, 0.5, 1.0] {
                    for x in [0.0, 0.5, 1.0] {
                        if spot_button(ui, [x, y], look.position == [x, y]).clicked() {
                            look.position = [x, y];
                        }
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            let mut percent = look.position.map(|f| f * 100.0);
            for (axis, label) in [(0, "Across"), (1, "Down")] {
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut percent[axis]).range(0.0..=100.0).speed(0.25).suffix(" %").fixed_decimals(1));
                    ui.label(muted(label).size(12.0));
                });
            }
            look.position = percent.map(|p| p / 100.0);
        });
    });
    ui.add(egui::Slider::new(&mut look.scale, 0.5..=2.0).text("Size").step_by(0.05).fixed_decimals(2));
    ui.add(egui::Slider::new(&mut look.opacity, 0.0..=1.0).text("Background opacity").step_by(0.05).fixed_decimals(2));
    ui.add(egui::Slider::new(&mut look.alpha, 0.1..=1.0).text("Panel opacity (text too)").step_by(0.05).fixed_decimals(2));
}

/// A 16:9 screen with the panel where it sits; dragging or clicking moves it.
fn screen_picture(ui: &mut egui::Ui, id: egui::Id, look: &mut OverlayLook, width: f32) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width * 9.0 / 16.0), Sense::click_and_drag());
    let response = response.on_hover_text("Drag the panel where it should sit in the game; Shift: without snapping");
    let unit = rect.height() / 1080.0;
    let size = PANEL_SIZE * look.scale.clamp(0.25, 4.0) * unit;
    let margin = Vec2::splat(PANEL_MARGIN * unit);
    let room = (rect.size() - 2.0 * margin - size).max(Vec2::splat(1.0));
    if let Some(pointer) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
        // The panel's center follows the pointer.
        let offset = ui.ctx().memory(|m| m.data.get_temp::<Vec2>(id)).filter(|_| !response.drag_started()).unwrap_or(size / 2.0);
        let origin = pointer - offset - rect.min - margin;
        let snap = !ui.input(|i| i.modifiers.shift);
        look.position = [origin.x / room.x, origin.y / room.y].map(|f| {
            let f = f.clamp(0.0, 1.0);
            let near = [0.0, 0.5, 1.0].into_iter().find(|s| (f - s).abs() < 0.02);
            if snap { near.unwrap_or(f) } else { f }
        });
    }
    let panel = Rect::from_min_size(rect.min + margin + Vec2::new(room.x * look.position[0], room.y * look.position[1]), size);
    if response.drag_started() {
        // Grabbed where the pointer is on it, or by its center when grabbed elsewhere.
        let grab = response.interact_pointer_pos().filter(|p| panel.contains(*p)).map(|p| p - panel.min).unwrap_or(size / 2.0);
        ui.ctx().memory_mut(|m| m.data.insert_temp(id, grab));
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, BG);
    painter.rect_stroke(rect, 4.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Game", egui::FontId::proportional(14.0), IDLE);
    let alpha = look.alpha.clamp(0.0, 1.0);
    painter.rect_filled(panel, 3.0, Color32::from_rgb(0x16, 0x18, 0x1e).gamma_multiply(look.opacity.clamp(0.0, 1.0) * alpha));
    let hovered = response.hovered() || response.dragged();
    painter.rect_stroke(panel, 3.0, Stroke::new(1.0, if hovered { ACCENT } else { LINE }.gamma_multiply(alpha)), egui::StrokeKind::Inside);
    // Its rows: the mode, the phase, the output and a gauge.
    let pad = 12.0 * unit * look.scale;
    let row = |at: f32, length: f32, height: f32, color: Color32| {
        let min = panel.min + Vec2::new(pad, panel.height() * at);
        painter.rect_filled(Rect::from_min_size(min, Vec2::new((panel.width() - 2.0 * pad) * length, height)), 1.0, color.gamma_multiply(alpha));
    };
    let thin = (3.0 * unit * look.scale).max(1.0);
    row(0.12, 0.5, thin, MUTED);
    row(0.28, 0.7, thin * 1.6, TEXT);
    row(0.55, 0.8, thin * 1.3, ACCENT);
    row(0.78, 0.5, thin * 1.3, GAME);
}

/// A small screen with a dot where the panel would sit.
fn spot_button(ui: &mut egui::Ui, [x, y]: [f32; 2], selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(26.0, 17.0), Sense::click());
    let fill = if selected { SELECTED_BG } else if response.hovered() { RAISED } else { PANEL };
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, fill);
    painter.rect_stroke(rect, 3.0, Stroke::new(1.0, if selected { ACCENT } else { LINE }), egui::StrokeKind::Inside);
    let inner = rect.shrink(4.0);
    let spot = Pos2::new(inner.left() + inner.width() * x, inner.top() + inner.height() * y);
    painter.circle_filled(spot, 2.5, if selected { ACCENT } else { MUTED });
    response
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
                if !client.places_freely() {
                    ui.label(RichText::new("older overlay: restart the game after updating it").color(WARN).size(12.0));
                }
            });
        }
        ui.add_space(4.0);
        ui.label(muted("Mode authors: hud() and hud_event() add gauges and messages to it, hud_hide() tells it may hide.").size(12.0));
    });
}
