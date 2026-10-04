//! Play page: tiles of the player's modes (or the built-in ones), then a page
//! for the chosen mode with its explanation and settings. It opens on the
//! mode of the last session.

use std::collections::BTreeMap;

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{mode_icon, App, Page};
use crate::audio::clap::ModelState;
use crate::config::NEW_MODE_TEMPLATE;
use crate::engine::{Command, ModeView, Shared};
use crate::gamepad::BUTTONS;
use crate::mode::{ModeInfo, ParamDef, ParamKind, ParamValue};

const TILE_MIN_WIDTH: f32 = 220.0;
const TILE_HEIGHT: f32 = 104.0;
/// Below this width the mode page shows its settings in a single column.
const TWO_COLUMNS_WIDTH: f32 = 720.0;

#[derive(Default)]
pub struct State {
    /// The list shows the built-in modes instead of the player's.
    builtin: bool,
    /// The list of modes is shown instead of the active mode's page.
    list: bool,
    new_preset_name: String,
    /// Preset whose deletion waits for confirmation.
    confirm_delete: Option<String>,
    /// Mode whose deletion waits for confirmation.
    confirm_delete_mode: Option<String>,
}

impl State {
    /// Shows the active mode's page.
    pub fn show_mode(&mut self) {
        self.list = false;
    }
}

impl App {
    pub(super) fn play_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            if self.feedback.open {
                self.feedback_page(ui, s);
            } else if self.play.list || s.mode.id.is_empty() {
                self.mode_list(ui, s);
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| self.mode_page(ui, s));
            }
        });
    }

    fn mode_list(&mut self, ui: &mut egui::Ui, s: &Shared) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                heading(ui, "What are you playing?");
                ui.label(muted("Pick a mode to see how it works and tune it."));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.play.builtin, true, "Built-in");
                ui.selectable_value(&mut self.play.builtin, false, "My modes");
            });
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.play.builtin {
                card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(muted(
                            "Built-in modes suit a whole genre but know nothing about your game: use them to try \
                             GameViber quickly.",
                        ));
                        if ui.link("Make one for your game instead").clicked() {
                            self.open_generator();
                        }
                    });
                });
                ui.add_space(8.0);
            } else {
                self.generator_banner(ui);
                ui.add_space(8.0);
            }
            if let Some(id) = mode_tiles(ui, s, !self.play.builtin) {
                if id != s.mode.id {
                    self.send(Command::SelectMode(id));
                }
                self.play.list = false;
            }
            if !self.play.builtin {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("➕ New mode").clicked() {
                        self.create_mode("my-mode", &NEW_MODE_TEMPLATE.replace("NAME", "My mode"));
                    }
                    ui.label(muted("Modes are small Lua scripts, edited in Creator."));
                });
            }
        });
    }

    /// Puts per-game modes forward: the built-in ones are fallbacks.
    fn generator_banner(&mut self, ui: &mut egui::Ui) {
        card(SELECTED_BG).stroke(egui::Stroke::new(1.0, ACCENT)).inner_margin(Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("✨ Get a mode made for your game").size(15.0).strong());
                    ui.label(muted(
                        "An AI assistant (ChatGPT, Claude...) writes one tailored to your game's controls and \
                         mechanics in a minute.",
                    ));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary("Make a mode for my game")).clicked() {
                        self.open_generator();
                    }
                });
            });
        });
    }

    /// The active mode: what it does, its presets and settings, what to do when it feels wrong.
    fn mode_page(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mode = &s.mode;
        let entry = s.modes.iter().find(|e| e.id == mode.id);
        ui.horizontal(|ui| {
            if ui.button("⏴ All modes").clicked() {
                self.play.list = true;
                self.play.builtin = entry.is_some_and(|e| e.builtin);
            }
            ui.add_space(8.0);
            let icon = entry.map(mode_icon).unwrap_or("🎮");
            egui::Frame::new().fill(RAISED).corner_radius(9).inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.label(RichText::new(icon).size(18.0).color(ACCENT));
            });
            ui.vertical(|ui| {
                let name = mode.info.as_ref().map_or(entry.map_or("", |e| e.key.as_str()), |i| i.name.as_str());
                ui.label(RichText::new(name).size(20.0).strong());
                let mut sub = mode.info.as_ref().map(|i| i.category.clone()).unwrap_or_default();
                if let Some(info) = mode.info.as_ref().filter(|i| !i.author.is_empty() || !i.version.is_empty()) {
                    let by = format!("by {} {}", info.author, info.version);
                    sub = if sub.is_empty() { by.trim().to_owned() } else { format!("{sub} · {}", by.trim()) };
                }
                if !sub.is_empty() {
                    ui.label(muted(sub));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if entry.is_some_and(|e| e.builtin) {
                    if ui.button("Duplicate and edit").on_hover_text("Copy this mode to change how it works").clicked() {
                        self.duplicate_mode(&mode.id);
                    }
                } else if self.play.confirm_delete_mode.as_deref() == Some(mode.id.as_str()) {
                    if ui.button("Cancel").clicked() {
                        self.play.confirm_delete_mode = None;
                    }
                    let delete = egui::Button::new(RichText::new("Delete for good").color(egui::Color32::WHITE)).fill(DANGER);
                    if ui.add(delete).clicked() {
                        self.send(Command::DeleteMode(mode.id.clone()));
                        self.play.confirm_delete_mode = None;
                        self.play.list = true;
                        self.play.builtin = false;
                    }
                    ui.label(RichText::new("Delete this mode, its settings and presets?").color(DANGER_TEXT));
                    return;
                } else {
                    if ui.button("🗑 Delete").on_hover_text("Delete this mode, its settings and presets").clicked() {
                        self.play.confirm_delete_mode = Some(mode.id.clone());
                    }
                    if ui.button("Edit in Creator").clicked() {
                        self.page = Page::Creator;
                    }
                    if ui.button("Duplicate").clicked() {
                        self.duplicate_mode(&mode.id);
                    }
                }
                if mode.info.is_some()
                    && ui
                        .add(primary("Doesn't feel right?"))
                        .on_hover_text("Get the mode fixed by an AI assistant, from what you felt and a recorded session")
                        .clicked()
                {
                    self.open_feedback(s);
                }
            });
        });
        ui.add_space(12.0);
        if let Some(error) = &mode.error {
            card(PANEL).stroke(egui::Stroke::new(1.0, DANGER)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(error).color(DANGER_TEXT).monospace().size(12.0));
                if mode.suspended && ui.button("Resume").clicked() {
                    self.send(Command::Resume);
                }
            });
            ui.add_space(8.0);
        }
        let Some(info) = &mode.info else {
            ui.label(muted("This mode does not load. Fix it in Creator, or pick another one."));
            return;
        };
        if !info.audio_scenes.is_empty() && s.audio.model != ModelState::Ready {
            card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label("This mode recognizes scenes from the game's music. It needs the scene model:");
                    if ui.button("Get it on the Sound page").clicked() {
                        self.page = Page::Audio;
                    }
                });
            });
            ui.add_space(8.0);
        }

        let (main, others): (Vec<&ParamDef>, Vec<&ParamDef>) = if info.main_params.is_empty() {
            (info.params.iter().collect(), Vec::new())
        } else {
            info.params.iter().partition(|def| info.main_params.contains(&def.name))
        };
        let help = if info.help.is_empty() { &info.description } else { &info.help };
        let left = |app: &mut Self, ui: &mut egui::Ui| {
            if !help.is_empty() {
                card(RAISED).inner_margin(Margin::same(14)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    eyebrow(ui, "How it works");
                    ui.label(help);
                });
                ui.add_space(12.0);
            }
            card(PANEL).inner_margin(Margin::same(14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                app.preset_picker(ui, info, mode);
                ui.add_space(6.0);
                for def in &main {
                    app.param(ui, def, mode);
                }
            });
        };
        let right = |app: &mut Self, ui: &mut egui::Ui| {
            card(PANEL).inner_margin(Margin::same(14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !others.is_empty() {
                    eyebrow(ui, &format!("More settings ({})", others.len()));
                    ui.add_space(4.0);
                    for def in &others {
                        app.param(ui, def, mode);
                    }
                    ui.add_space(8.0);
                }
                eyebrow(ui, "Presets");
                app.preset_management(ui, mode);
            });
        };
        if ui.available_width() >= TWO_COLUMNS_WIDTH {
            let gap = 16.0;
            let width = (ui.available_width() - gap) / 2.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let layout = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), layout, |ui| {
                    ui.set_width(width);
                    left(self, ui);
                });
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), layout, |ui| {
                    ui.set_width(width);
                    right(self, ui);
                });
            });
        } else {
            left(self, ui);
            ui.add_space(12.0);
            right(self, ui);
        }
    }

    fn param(&self, ui: &mut egui::Ui, def: &ParamDef, mode: &ModeView) {
        if let Some(value) = mode.values.get(&def.name) {
            if let Some(new) = param_widget(ui, def, value) {
                self.send(Command::SetParam(def.name.clone(), new));
            }
        }
    }

    fn preset_picker(&self, ui: &mut egui::Ui, info: &ModeInfo, mode: &ModeView) {
        let presets = &mode.presets.presets;
        let active = mode.presets.active.as_deref().filter(|name| presets.contains_key(*name));
        let modified = active.is_some_and(|name| !matches_values(info, &presets[name], &mode.values));
        let current = match active {
            Some(name) if modified => format!("{name} (modified)"),
            Some(name) => name.to_owned(),
            None if matches_values(info, &BTreeMap::new(), &mode.values) => "Defaults".to_owned(),
            None => "Custom".to_owned(),
        };
        ui.horizontal(|ui| {
            eyebrow(ui, "Preset");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("preset").width(180.0).selected_text(current).show_ui(ui, |ui| {
                    if ui.selectable_label(false, "Defaults").clicked() {
                        self.send(Command::ResetParams);
                    }
                    for name in presets.keys() {
                        // Selecting the active preset again reverts its unsaved changes.
                        if ui.selectable_label(Some(name.as_str()) == active, name).clicked() {
                            self.send(Command::LoadPreset(name.clone()));
                        }
                    }
                });
            });
        });
    }

    fn preset_management(&mut self, ui: &mut egui::Ui, mode: &ModeView) {
        let presets = &mode.presets.presets;
        let active = mode.presets.active.as_deref().filter(|name| presets.contains_key(*name));
        ui.label(muted("Save these settings under a name, e.g. one per game."));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.play.new_preset_name).hint_text("preset name").desired_width(150.0));
            let name = self.play.new_preset_name.trim().to_owned();
            let label = if presets.contains_key(&name) { "Replace" } else { "Save" };
            if ui.add_enabled(!name.is_empty(), egui::Button::new(label)).clicked() {
                self.send(Command::SavePreset(name));
                self.play.new_preset_name.clear();
            }
        });
        ui.horizontal(|ui| {
            if let Some(name) = active {
                if ui.button(format!("Update \"{name}\"")).clicked() {
                    self.send(Command::SavePreset(name.to_owned()));
                }
                if self.play.confirm_delete.as_deref() == Some(name) {
                    if ui.button(RichText::new("Really delete?").color(DANGER_TEXT)).clicked() {
                        self.send(Command::DeletePreset(name.to_owned()));
                        self.play.confirm_delete = None;
                    }
                    if ui.button("Cancel").clicked() {
                        self.play.confirm_delete = None;
                    }
                } else if ui.button("Delete").clicked() {
                    self.play.confirm_delete = Some(name.to_owned());
                }
            }
            if ui.button("Defaults").on_hover_text("Put every setting back to its default").clicked() {
                self.send(Command::ResetParams);
            }
        });
    }
}

/// Tiles of the built-in (or user) modes; returns the id of a clicked mode.
pub(super) fn mode_tiles(ui: &mut egui::Ui, s: &Shared, user_modes: bool) -> Option<String> {
    let entries: Vec<_> = s.modes.iter().filter(|e| e.builtin != user_modes).collect();
    if entries.is_empty() {
        ui.label(muted("You have not made any mode yet."));
        return None;
    }
    let mut clicked = None;
    tile_grid(ui, entries.len(), TILE_MIN_WIDTH, TILE_HEIGHT, |ui, i, size| {
        let entry = entries[i];
        let active = entry.id == s.mode.id;
        let (title, subtitle, body) = match s.catalog.get(&entry.id) {
            Some(Ok(info)) => {
                let subtitle = if info.category.is_empty() { entry.key.clone() } else { info.category.clone() };
                (info.name.clone(), subtitle, info.description.clone())
            }
            Some(Err(e)) => (entry.key.clone(), "does not load".to_owned(), e.clone()),
            None => (entry.key.clone(), String::new(), String::new()),
        };
        let tile = Tile {
            icon: mode_icon(entry),
            title: &title,
            subtitle: &subtitle,
            body: &body,
            selected: active,
            badge: active.then_some("Active"),
        };
        if tile.show(ui, size).clicked() && !active {
            clicked = Some(entry.id.clone());
        }
    });
    clicked
}

/// True when applying `preset` would leave `values` unchanged.
fn matches_values(info: &ModeInfo, preset: &BTreeMap<String, ParamValue>, values: &BTreeMap<String, ParamValue>) -> bool {
    info.params.iter().all(|def| {
        let wanted = preset.get(&def.name).and_then(|v| def.accept(v)).unwrap_or_else(|| def.default.clone());
        values.get(&def.name) == Some(&wanted)
    })
}

/// Draws a parameter control; returns the new value when the user changed it.
pub(super) fn param_widget(ui: &mut egui::Ui, def: &ParamDef, value: &ParamValue) -> Option<ParamValue> {
    match (&def.kind, value) {
        (ParamKind::Number { min, max, step }, ParamValue::Number(v)) => {
            let mut v = *v;
            ui.label(&def.label);
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(80.0);
            let mut slider = egui::Slider::new(&mut v, *min..=*max);
            if let Some(step) = step {
                // As many decimals as the step has: 1 -> 0, 0.05 -> 2.
                let decimals = (-step.log10()).ceil().max(0.0) as usize;
                slider = slider.step_by(*step).fixed_decimals(decimals);
            }
            let changed = ui.add(slider).changed();
            ui.add_space(4.0);
            changed.then_some(ParamValue::Number(v))
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => {
            let mut b = *b;
            ui.checkbox(&mut b, &def.label).changed().then_some(ParamValue::Bool(b))
        }
        (ParamKind::Choice(options), ParamValue::Text(current)) => {
            combo(ui, def, current, options.iter().map(String::as_str))
        }
        (ParamKind::Button, ParamValue::Text(current)) => combo(ui, def, current, BUTTONS.iter().copied()),
        _ => None,
    }
}

fn combo<'a>(
    ui: &mut egui::Ui,
    def: &ParamDef,
    current: &str,
    options: impl Iterator<Item = &'a str>,
) -> Option<ParamValue> {
    let mut selected = current.to_owned();
    ui.horizontal(|ui| {
        ui.label(&def.label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt(&def.name).selected_text(&selected).show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(&mut selected, option.to_owned(), option);
                }
            });
        });
    });
    ui.add_space(2.0);
    (selected != current).then_some(ParamValue::Text(selected))
}
