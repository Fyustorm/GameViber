//! Moving around: the path to the active mode (Library › its game › the mode,
//! Change mode playing another one), and going back and forth between the places
//! seen, with the mouse's side buttons or Alt+←/→.

use eframe::egui::{self, RichText};

use super::library::{groups, mode_game, Filter};
use super::theme::*;
use super::{community, creator, main_of, setup, App, Page, Route};
use crate::config::ModeEntry;
use crate::engine::Shared;
use crate::game::Game;

/// Places kept to go back to.
const KEPT: usize = 50;

/// A place in the app, as the history keeps it.
#[derive(Clone, PartialEq)]
pub(super) struct Location {
    page: Page,
    route: Route,
    /// The mode's page shows the fix request.
    fixing: bool,
    creator: creator::Tab,
    setup: setup::Tab,
    community: community::Place,
}

#[derive(Default)]
pub struct State {
    /// The places shown before the one shown now, and those left by going back.
    back: Vec<Location>,
    forward: Vec<Location>,
    /// The place shown at the end of the last frame.
    last: Option<Location>,
    /// What the menu of modes is searched with.
    search: String,
}

/// One step of the path.
enum Crumb {
    Library(Filter),
    ModePage,
}

impl App {
    fn location(&self) -> Location {
        Location {
            page: self.page,
            route: self.route,
            fixing: self.route == Route::Mode && self.feedback.open,
            creator: self.creator.tab,
            setup: self.setup_tab,
            community: self.community.place(),
        }
    }

    fn go_to(&mut self, s: &Shared, to: Location) {
        if to.page == Page::Setup && (self.page != Page::Setup || to.setup == setup::Tab::Overlay) {
            // Check the installed overlay files again.
            self.overlay.forget_install_state();
        }
        self.page = to.page;
        self.route = to.route;
        if to.fixing {
            self.open_feedback(s);
        } else {
            self.feedback.open = false;
        }
        self.creator.tab = to.creator;
        self.setup_tab = to.setup;
        self.community.go_to(to.community);
    }

    /// Goes back or forth when asked: the mouse's side buttons, Alt+←/→ (not while typing).
    pub(super) fn history_buttons(&mut self, ctx: &egui::Context, s: &Shared) {
        let typing = ctx.egui_wants_keyboard_input();
        let (back, forward) = ctx.input_mut(|i| {
            let alt = |i: &mut egui::InputState, key| !typing && i.consume_key(egui::Modifiers::ALT, key);
            let back = i.pointer.button_pressed(egui::PointerButton::Extra1)
                | alt(i, egui::Key::ArrowLeft);
            let forward = i.pointer.button_pressed(egui::PointerButton::Extra2)
                | alt(i, egui::Key::ArrowRight);
            (back, forward)
        });
        let here = self.location();
        let h = &mut self.navigation;
        let to = if back {
            h.back.pop().inspect(|_| h.forward.push(here))
        } else if forward {
            h.forward.pop().inspect(|_| h.back.push(here))
        } else {
            None
        };
        if let Some(to) = to {
            self.navigation.last = Some(to.clone());
            self.go_to(s, to);
        }
    }

    /// Keeps the place left this frame, to go back to it.
    pub(super) fn remember_location(&mut self) {
        let here = self.location();
        let h = &mut self.navigation;
        if h.last.as_ref() == Some(&here) {
            return;
        }
        if let Some(last) = h.last.replace(here) {
            h.back.push(last);
            if h.back.len() > KEPT {
                h.back.remove(0);
            }
            h.forward.clear();
        }
    }

    /// The path to the active mode: Library › its game › the mode, then `here`
    /// (the Creator) when the mode's page is not the one shown; Change mode at its end.
    pub(super) fn mode_path(&mut self, ui: &mut egui::Ui, s: &Shared, here: Option<&str>) {
        let entry = ModeEntry::from_id(&s.mode.id);
        let name = s.mode.info.as_ref().map_or_else(|| if entry.key.is_empty() { "Mode".to_owned() } else { entry.key.clone() }, |i| i.name.clone());
        let game = match mode_game(s) {
            Some(g) => Some((g.name.clone(), Filter::Game(g.id.clone()))),
            None if entry.builtin => Some(("Any game".to_owned(), Filter::AnyGame)),
            None => None,
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if crumb_link(ui, "Library") {
                self.follow(Crumb::Library(Filter::All));
            }
            if let Some((game, filter)) = game {
                ui.label(muted("›"));
                if crumb_link(ui, &game) {
                    self.follow(Crumb::Library(filter));
                }
            }
            ui.label(muted("›"));
            match here {
                Some(_) => {
                    if crumb_link(ui, &name) {
                        self.follow(Crumb::ModePage);
                    }
                }
                None => {
                    ui.label(RichText::new(&name).color(TEXT));
                }
            }
            if let Some(here) = here {
                ui.label(muted("›"));
                ui.label(RichText::new(here).color(TEXT));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| self.mode_menu(ui, s));
        });
        ui.add_space(6.0);
    }

    fn follow(&mut self, crumb: Crumb) {
        self.page = Page::Library;
        self.feedback.open = false;
        match crumb {
            Crumb::Library(filter) => {
                self.route = Route::Library;
                self.library.filter = filter;
            }
            Crumb::ModePage => self.route = Route::Mode,
        }
    }

    /// The menu playing another mode: searched, by game (the one played first), the built-in ones last.
    fn mode_menu(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let config = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let button = egui::Button::new("⇄ Change mode");
        let menu = egui::containers::menu::MenuButton::from_button(button).config(config).ui(ui, |ui| {
            ui.set_min_width(260.0);
            ui.add(egui::TextEdit::singleline(&mut self.navigation.search).hint_text("🔍 Search a mode or a game").desired_width(f32::INFINITY));
            ui.add_space(4.0);
            let search = self.navigation.search.trim().to_lowercase();
            let mut picked: Option<(Option<Game>, String)> = None;
            egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                let mut builtin: Vec<String> = s.modes.iter().filter(|e| e.builtin).map(|e| e.id.clone()).collect();
                builtin.sort_by_key(|id| mode_name(s, id).to_lowercase());
                let mut shown = false;
                for (game, modes) in groups(s).into_iter().chain([(None, builtin)]) {
                    let title = match &game {
                        Some(g) => g.name.clone(),
                        None if modes.first().is_some_and(|id| ModeEntry::from_id(id).builtin) => "Any game".to_owned(),
                        None => "In no game".to_owned(),
                    };
                    let whole = title.to_lowercase().contains(&search);
                    // A mode, then its variants, indented.
                    let rows: Vec<(String, String, bool)> = modes
                        .iter()
                        .flat_map(|main| {
                            let variants = s.modes.iter().filter(move |e| e.variant.is_some() && e.main_id() == *main);
                            std::iter::once((main.clone(), mode_name(s, main), false))
                                .chain(variants.map(|e| (e.id.clone(), e.variant.clone().unwrap_or_default(), true)))
                        })
                        .filter(|(id, label, _)| whole || label.to_lowercase().contains(&search) || mode_name(s, &main_of(id)).to_lowercase().contains(&search))
                        .collect();
                    if rows.is_empty() {
                        continue;
                    }
                    if shown {
                        ui.add_space(4.0);
                    }
                    shown = true;
                    ui.label(muted(title).size(12.5));
                    for (id, label, variant) in rows {
                        let row = ui.horizontal(|ui| {
                            if variant {
                                ui.add_space(14.0);
                            }
                            ui.selectable_label(id == s.mode.id, label)
                        });
                        if row.inner.clicked() {
                            picked = Some((game.clone(), id));
                        }
                    }
                }
                if !shown {
                    ui.label(muted("No mode found."));
                }
            });
            ui.separator();
            if ui.selectable_label(false, "+ Create a mode").clicked() {
                self.navigation.search.clear();
                self.open_create(None);
                ui.close();
            }
            if let Some((game, id)) = picked {
                self.navigation.search.clear();
                self.play_mode(s, game.as_ref(), &id);
                ui.close();
            }
        });
        menu.0.on_hover_text("Play another mode, or create one");
    }
}

/// A step of a path, clicked.
fn crumb_link(ui: &mut egui::Ui, label: &str) -> bool {
    let link = ui.add(egui::Label::new(muted(label)).sense(egui::Sense::click()));
    link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn mode_name(s: &Shared, id: &str) -> String {
    s.catalog.get(id).and_then(|r| r.as_ref().ok()).map_or_else(|| ModeEntry::from_id(id).key, |i| i.name.clone())
}
