//! A mode's page (Library › game › mode): its name and what to do with it on
//! top, then its tabs: its settings (presets, the main ones, the others to
//! search), its variants (played, renamed, deleted, made), its game and what
//! it reads; and tiles of modes, for the setup guide. Sharing it is in the
//! Creator, where it is made.

use std::collections::BTreeMap;

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{display_name, main_of, mode_icon, App, Page, Route};
use crate::config::ModeEntry;
use crate::models::Model;
use crate::engine::{Command, ModeView, Shared};
use crate::gamepad::BUTTONS;
use crate::mode::{ModeInfo, ParamDef, ParamKind, ParamValue};

const TILE_MIN_WIDTH: f32 = 220.0;
const TILE_HEIGHT: f32 = 104.0;
/// Below this width the mode page shows its settings in a single column.
const TWO_COLUMNS_WIDTH: f32 = 720.0;
/// From this many other settings, they can be searched.
const SEARCH_FROM: usize = 6;

/// The mode page's tabs.
#[derive(PartialEq, Clone, Copy, Default)]
pub(super) enum ModeTab {
    #[default]
    Settings,
    Variants,
    /// Its game, and what it reads.
    Game,
}

#[derive(Default)]
pub struct State {
    pub(super) tab: ModeTab,
    new_preset_name: String,
    /// Preset whose deletion waits for confirmation.
    confirm_delete: Option<String>,
    /// Mode whose deletion waits for confirmation.
    confirm_delete_mode: Option<String>,
    /// The other settings searched, and only those changed from their default shown.
    search: String,
    changed_only: bool,
    /// The name of a new variant being typed, and whether it starts from the defaults rather than the settings played.
    new_variant: String,
    variant_defaults: bool,
    /// The variant being renamed and its name typed, and why it could not be.
    renaming: Option<(String, String)>,
    rename_error: Option<String>,
    /// Variant whose deletion waits for confirmation.
    confirm_delete_variant: Option<String>,
}

impl App {
    /// The active mode's name and what to do with it, the tabs, and the variant
    /// played; false when no mode is active.
    pub(super) fn mode_header(&mut self, ui: &mut egui::Ui, s: &Shared) -> bool {
        let mode = &s.mode;
        let entry = s.modes.iter().find(|e| e.id == mode.id);
        if mode.id.is_empty() {
            ui.label(muted("No mode is active."));
            return false;
        }
        let main = main_of(&mode.id);
        ui.horizontal(|ui| {
            let icon = entry.map(mode_icon).unwrap_or("🎮");
            egui::Frame::new().fill(RAISED).corner_radius(9).inner_margin(Margin::same(8)).show(ui, |ui| {
                ui.label(RichText::new(icon).size(18.0).color(ACCENT));
            });
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(display_name(s, &main)).size(20.0).strong());
                    if let Some(variant) = entry.and_then(|e| e.variant.as_deref()) {
                        pill(ui, &format!("variant {variant}"), ON_ACCENT, GAME);
                    }
                });
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
                } else if self.play.confirm_delete_mode.as_deref() == Some(main.as_str()) {
                    if ui.button("Cancel").clicked() {
                        self.play.confirm_delete_mode = None;
                    }
                    let delete = egui::Button::new(RichText::new("Delete for good").color(egui::Color32::WHITE)).fill(DANGER);
                    if ui.add(delete).clicked() {
                        self.send(Command::DeleteMode(main.clone()));
                        self.play.confirm_delete_mode = None;
                        self.route = Route::Library;
                    }
                    ui.label(RichText::new("Delete this mode, its variants, settings and presets?").color(DANGER_TEXT));
                    return;
                } else {
                    if ui.button("🗑").on_hover_text("Delete this mode, its variants, settings and presets").clicked() {
                        self.play.confirm_delete_mode = Some(main.clone());
                    }
                    if ui.button("Duplicate").on_hover_text("A new mode from the script played now").clicked() {
                        self.duplicate_mode(&mode.id);
                    }
                    if ui.button("Share ›").on_hover_text("As a file, or in the community: in the Creator").clicked() {
                        self.page = Page::Creator;
                        self.creator.tab = super::creator::Tab::Share;
                    }
                    if ui.button("Edit in Creator ›").clicked() {
                        self.page = Page::Creator;
                    }
                    self.community_update_button(ui, s, &main);
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
        ui.add_space(8.0);
        if let Some(game) = super::library::mode_game(s).cloned() {
            self.publish_suggestion(ui, s, &game, &main);
        }
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

        // The tabs, and the variant played on their right.
        let variants: Vec<&ModeEntry> = s.modes.iter().filter(|e| e.variant.is_some() && e.main_id() == main).collect();
        ui.horizontal(|ui| {
            let tabs = [
                (ModeTab::Settings, "Settings".to_owned()),
                (ModeTab::Variants, if variants.is_empty() { "Variants".to_owned() } else { format!("Variants · {}", variants.len()) }),
                (ModeTab::Game, "Game & inputs".to_owned()),
            ];
            for (tab, label) in tabs {
                let on = self.play.tab == tab;
                if ui.selectable_label(on, RichText::new(label).size(15.0).color(if on { TEXT } else { MUTED })).clicked() {
                    self.play.tab = tab;
                }
            }
            if !variants.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = |e: &ModeEntry| e.variant.clone().unwrap_or_else(|| "Main".to_owned());
                    let played = entry.map_or_else(|| "Main".to_owned(), label);
                    egui::ComboBox::from_id_salt("variant-played").selected_text(played).show_ui(ui, |ui| {
                        let main_entry = ModeEntry::from_id(&main);
                        for e in std::iter::once(&main_entry).chain(variants.iter().copied()) {
                            if ui.selectable_label(e.id == mode.id, label(e)).clicked() && e.id != mode.id {
                                self.send(Command::SelectMode(e.id.clone()));
                            }
                        }
                    });
                    ui.label(muted("Playing")).on_hover_text("The script played: the mode, or one of its variants");
                });
            }
        });
        ui.separator();
        true
    }

    /// The Settings tab: what is missing, how the mode works, its presets and settings.
    pub(super) fn mode_settings(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mode = &s.mode;
        self.readiness_card(ui, s);
        let Some(info) = &mode.info else {
            ui.label(muted("This mode does not load. Fix it in Creator, or pick another one."));
            return;
        };
        let missing = [
            (s.phases.sound.0 && !Model::Sound.ready(), "music"),
            (s.phases.screen.0 && !Model::Image.ready(), "image"),
        ];
        for (_, what) in missing.into_iter().filter(|m| m.0) {
            card(RAISED).inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("Phases are recognized from the game's {what}. It needs a phase model:"));
                    if ui.button("Get it in the Creator's Phases").clicked() {
                        self.page = Page::Creator;
                    }
                });
            });
            ui.add_space(8.0);
        }

        let (main, others): (Vec<&ParamDef>, Vec<&ParamDef>) = if info.main_params.is_empty() {
            (Vec::new(), info.params.iter().collect())
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
                ui.add_space(4.0);
                ui.separator();
                app.preset_management(ui, mode);
            });
        };
        let right = |app: &mut Self, ui: &mut egui::Ui| {
            if others.is_empty() {
                return;
            }
            card(PANEL).inner_margin(Margin::same(14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let title = if main.is_empty() { "Settings" } else { "More settings" };
                let changed = others.iter().filter(|def| mode.values.get(&def.name).is_some_and(|v| *v != def.default)).count();
                ui.horizontal(|ui| {
                    eyebrow(ui, &format!("{title} ({})", others.len()));
                    if changed > 0 {
                        ui.label(muted(format!("· {changed} changed")).size(12.0));
                    }
                });
                if others.len() >= SEARCH_FROM {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut app.play.search).hint_text("🔍 Search").desired_width(180.0));
                        if !app.play.search.is_empty() && ui.small_button("✕").on_hover_text("Clear the search").clicked() {
                            app.play.search.clear();
                        }
                        ui.toggle_value(&mut app.play.changed_only, "Changed only").on_hover_text("Only the settings changed from their default");
                    });
                }
                ui.add_space(4.0);
                let search = app.play.search.trim().to_lowercase();
                let shown: Vec<&&ParamDef> = others
                    .iter()
                    .filter(|def| search.is_empty() || def.label.to_lowercase().contains(&search) || def.name.to_lowercase().contains(&search))
                    .filter(|def| !app.play.changed_only || mode.values.get(&def.name).is_some_and(|v| *v != def.default))
                    .collect();
                if shown.is_empty() {
                    ui.label(muted("No setting matches."));
                }
                for def in shown {
                    app.param(ui, def, mode);
                }
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

    /// The Variants tab: the mode and its variants (other scripts reading the
    /// same inputs), each played, renamed or deleted, and making a new one.
    pub(super) fn mode_variants(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let entry = ModeEntry::from_id(&s.mode.id);
        card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Variants").strong().size(17.0));
            ui.label(muted(
                "Other scripts of this mode, reading the same phases, indicators and captures: a calmer one, one for boss fights. \
                 Each has its own settings and presets, and is changed in the Creator while it is played. They are shared with the mode.",
            ));
            if entry.builtin {
                ui.label("Built-in modes have no variants: duplicate it to make your own.");
                return;
            }
            if entry.dir().is_none() {
                ui.label("This mode is a file of its own: only the modes in your library have variants.");
                return;
            }
            ui.add_space(8.0);
            let main = ModeEntry::from_id(&entry.main_id());
            let variants: Vec<&ModeEntry> = s.modes.iter().filter(|e| e.variant.is_some() && e.main_id() == main.id).collect();
            let mut rename = None;
            egui::Grid::new("variants").num_columns(3).spacing([16.0, 8.0]).show(ui, |ui| {
                for e in std::iter::once(&main).chain(variants.iter().copied()) {
                    let playing = e.id == s.mode.id;
                    match (&mut self.play.renaming, &e.variant) {
                        (Some((id, name)), Some(_)) if *id == e.id => {
                            let field = ui.add(egui::TextEdit::singleline(name).desired_width(160.0));
                            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                rename = Some((e.id.clone(), name.clone(), playing));
                            }
                        }
                        (_, variant) => {
                            ui.label(RichText::new(variant.as_deref().unwrap_or("Main")).strong());
                        }
                    }
                    if playing {
                        pill(ui, "▶ Playing", ON_ACCENT, OK);
                    } else if ui.button("Play").clicked() {
                        self.send(Command::SelectMode(e.id.clone()));
                    }
                    ui.horizontal(|ui| {
                        if e.variant.is_none() {
                            ui.label(muted("the mode itself"));
                            return;
                        }
                        let editing = self.play.renaming.as_ref().is_some_and(|(id, _)| *id == e.id);
                        if editing {
                            if ui.button("Save").clicked() {
                                let name = self.play.renaming.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                                rename = Some((e.id.clone(), name, playing));
                            }
                            if ui.button("Cancel").clicked() {
                                (self.play.renaming, self.play.rename_error) = (None, None);
                            }
                        } else if self.play.confirm_delete_variant.as_deref() == Some(e.id.as_str()) {
                            let delete = egui::Button::new(RichText::new("Delete for good").color(egui::Color32::WHITE)).fill(DANGER);
                            if ui.add(delete).clicked() {
                                self.send(Command::DeleteMode(e.id.clone()));
                                self.play.confirm_delete_variant = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.play.confirm_delete_variant = None;
                            }
                        } else {
                            if ui.button("Rename").clicked() {
                                self.play.renaming = Some((e.id.clone(), e.variant.clone().unwrap_or_default()));
                                self.play.rename_error = None;
                            }
                            if ui.button(RichText::new("🗑 Delete").color(DANGER_TEXT)).on_hover_text("Delete this variant, its settings and presets").clicked() {
                                self.play.confirm_delete_variant = Some(e.id.clone());
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            if let Some((id, name, playing)) = rename {
                match self.rename_variant(&id, &name, playing) {
                    Ok(()) => (self.play.renaming, self.play.rename_error) = (None, None),
                    Err(e) => self.play.rename_error = Some(e),
                }
            }
            if let Some(error) = &self.play.rename_error {
                ui.label(RichText::new(error).color(DANGER_TEXT).size(12.0));
            }

            ui.add_space(12.0);
            ui.label(RichText::new("New variant").strong());
            let played = entry.variant.as_deref().map_or_else(|| "the mode's script".to_owned(), |v| format!("the variant {v}"));
            ui.label(muted(format!("A copy of {played}, played now, to change in the Creator.")));
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.play.new_variant).hint_text("boss-only").desired_width(160.0));
                let mut keep = !self.play.variant_defaults;
                if ui.checkbox(&mut keep, "With the settings played now").on_hover_text("Otherwise it starts from the script's defaults").changed() {
                    self.play.variant_defaults = !keep;
                }
                let name = self.play.new_variant.trim().to_owned();
                if ui.add_enabled(!name.is_empty(), primary("Create")).clicked() {
                    let values = keep.then_some(&s.mode.values);
                    self.create_variant(&s.mode.id, &name, values);
                    self.play.new_variant.clear();
                }
            });
        });
    }

    pub(super) fn param(&self, ui: &mut egui::Ui, def: &ParamDef, mode: &ModeView) {
        if let Some(value) = mode.values.get(&def.name) {
            if let Some(new) = param_widget(ui, def, value) {
                self.send(Command::SetParam(def.name.clone(), new));
            }
        }
    }

    pub(super) fn preset_picker(&self, ui: &mut egui::Ui, info: &ModeInfo, mode: &ModeView) {
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
        ui.label(muted("Save these settings under a name: a soft one, an intense one."));
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
            image: None,
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
/// A setting changed from its default reads so, with a button putting it back.
pub(super) fn param_widget(ui: &mut egui::Ui, def: &ParamDef, value: &ParamValue) -> Option<ParamValue> {
    let changed = *value != def.default;
    let label = if changed { RichText::new(&def.label).color(TEXT).strong() } else { RichText::new(&def.label) };
    match (&def.kind, value) {
        (ParamKind::Number { min, max, step }, ParamValue::Number(v)) => {
            let mut v = *v;
            let mut new = None;
            ui.horizontal(|ui| {
                ui.label(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| new = reset(ui, def, changed));
            });
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(80.0);
            let mut slider = egui::Slider::new(&mut v, *min..=*max);
            if let Some(step) = step {
                // As many decimals as the step has: 1 -> 0, 0.05 -> 2.
                let decimals = (-step.log10()).ceil().max(0.0) as usize;
                slider = slider.step_by(*step).fixed_decimals(decimals);
            }
            let moved = ui.add(slider).changed();
            ui.add_space(4.0);
            new.or(moved.then_some(ParamValue::Number(v)))
        }
        (ParamKind::Bool, ParamValue::Bool(b)) => {
            let mut b = *b;
            let mut new = None;
            ui.horizontal(|ui| {
                if ui.checkbox(&mut b, label).changed() {
                    new = Some(ParamValue::Bool(b));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| new = new.take().or(reset(ui, def, changed)));
            });
            new
        }
        (ParamKind::Choice(options), ParamValue::Text(current)) => {
            combo(ui, def, label, current, changed, options.iter().map(String::as_str))
        }
        (ParamKind::Button, ParamValue::Text(current)) => combo(ui, def, label, current, changed, BUTTONS.iter().copied()),
        _ => None,
    }
}

/// Puts a changed setting back to its default.
fn reset(ui: &mut egui::Ui, def: &ParamDef, changed: bool) -> Option<ParamValue> {
    let default = match &def.default {
        ParamValue::Number(n) => format!("{n}"),
        ParamValue::Bool(b) => if *b { "on" } else { "off" }.to_owned(),
        ParamValue::Text(t) => t.clone(),
    };
    let button = egui::Button::new(RichText::new("↺").color(ACCENT_TEXT)).small().frame(false);
    (changed && ui.add(button).on_hover_text(format!("Back to its default: {default}")).clicked()).then(|| def.default.clone())
}

fn combo<'a>(
    ui: &mut egui::Ui,
    def: &ParamDef,
    label: RichText,
    current: &str,
    changed: bool,
    options: impl Iterator<Item = &'a str>,
) -> Option<ParamValue> {
    let mut selected = current.to_owned();
    let mut new = None;
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt(&def.name).selected_text(&selected).show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(&mut selected, option.to_owned(), option);
                }
            });
            new = reset(ui, def, changed);
        });
    });
    ui.add_space(2.0);
    new.or((selected != current).then_some(ParamValue::Text(selected)))
}
