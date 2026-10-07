//! The community (`crate::community`): the Community page (games, their
//! modes, a mode's page to install or update it, a mode by its share code),
//! what a game's page and the library say of it, and publishing a mode from
//! its Sharing tab. Calls to the server run in threads (`Remote`).

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Margin, RichText, Vec2};

use super::theme::*;
use super::{App, GameView as View, Page, Route};
use crate::community::{self, Account, Client, GameView, ModeDetail, ModeSummary, Origin, Source, Usage};
use crate::config::ModeEntry;
use crate::engine::{Command, Shared};
use crate::game::Game;

/// Below this width the mode's page goes under the list.
const TWO_COLUMNS_WIDTH: f32 = 900.0;
/// A game's modes, the best rated first (few votes counting little): a game has few.
const SORT: &str = "rating";
/// Searching waits for the player to stop typing this long.
const SEARCH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(300);
const REASONS: [(&str, &str); 3] = [("broken", "Broken by a game update"), ("content", "Should not be there"), ("other", "Something else")];

/// What a call to the server gives, once it came.
pub(super) enum Remote<T> {
    Idle,
    Loading(Receiver<Result<T, String>>),
    Ready(T),
    Failed(String),
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Remote::Idle
    }
}

impl<T: Send + 'static> Remote<T> {
    fn start(&mut self, ctx: &egui::Context, job: impl FnOnce() -> anyhow::Result<T> + Send + 'static) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job().map_err(|e| format!("{e:#}")));
            ctx.request_repaint();
        });
        *self = Remote::Loading(rx);
    }

    /// Takes the answer if it came.
    fn poll(&mut self) {
        if let Remote::Loading(rx) = self {
            match rx.try_recv() {
                Ok(Ok(value)) => *self = Remote::Ready(value),
                Ok(Err(e)) => *self = Remote::Failed(e),
                Err(mpsc::TryRecvError::Disconnected) => *self = Remote::Failed("interrupted".into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    fn loading(&self) -> bool {
        matches!(self, Remote::Loading(_))
    }
}

/// What an action did: said to the player, and what the engine reads again.
struct Done {
    message: String,
    /// A game and its mode were imported.
    game: Option<String>,
    /// The mode to make the active one.
    select: Option<String>,
}

/// The form publishing a mode, for the mode it was opened on.
#[derive(Default)]
struct PublishForm {
    mode: String,
    name: String,
    description: String,
    public: bool,
    changelog: String,
    accepted: bool,
    /// Captures not sent, by file name.
    left_out: HashSet<String>,
}

#[derive(Default)]
pub struct State {
    search: String,
    /// What the games shown were searched with.
    searched: String,
    games: Remote<Vec<GameView>>,
    /// A game added to the library to make a mode for: the AI request opens once it is there.
    making_for: Option<String>,
    /// The game whose modes are shown.
    game: Option<GameView>,
    /// When the search was last typed in, to search once typing stops.
    typed: Option<std::time::Instant>,
    modes: Remote<Vec<ModeSummary>>,
    /// The mode whose page is shown, and how it was reached.
    selected: Option<Source>,
    detail: Remote<ModeDetail>,
    code: String,
    action: Remote<Done>,
    message: Option<(bool, String)>,
    /// The modes installed from (or published to) the community: their id on
    /// the server, their local id and origin; and the user modes looked at.
    installed: HashMap<String, (String, Origin)>,
    installed_for: Vec<String>,
    /// The community's modes the player has as their own (the same script): id on the server, local id.
    linked: HashMap<String, String>,
    /// What the server has for the games of the library (by local id)...
    matches: HashMap<String, Remote<Option<(GameView, Vec<ModeSummary>)>>>,
    /// ...and of the modes installed from it (by id on the server), to offer updates.
    latest: HashMap<String, Remote<ModeDetail>>,
    /// The author account: None until read from disk.
    account: Option<Option<Account>>,
    pseudo: String,
    password: String,
    signing_in: Remote<Account>,
    /// The player's published modes, as the server has them.
    mine: Remote<Vec<ModeDetail>>,
    publish: PublishForm,
    /// A report being written: the mode (on the server), its reason, details.
    report: Option<(String, usize, String)>,
    /// How long the player's modes were played, read again now and then (by local id).
    usage: HashMap<String, (std::time::Instant, community::Usage)>,
    /// The first-launch question was answered (until the engine saved it)...
    consent_answered: bool,
    /// ...or shown anyway (the screenshot tour).
    pub(super) consent_preview: bool,
}

impl State {
    /// Shows `game`'s modes on the Community page.
    pub(super) fn open_game(&mut self, game: GameView) {
        self.game = Some(game);
        self.modes = Remote::Idle;
        self.selected = None;
        self.detail = Remote::Idle;
    }

    pub(super) fn busy(&self) -> bool {
        self.action.loading()
    }
}

impl App {
    /// The author account signed in on the server, read from disk once.
    fn account(&mut self) -> Option<Account> {
        self.community.account.get_or_insert_with(|| Account::load(community::URL)).clone()
    }

    fn client(&mut self) -> Client {
        let account = self.account();
        Client::new(community::URL, account.as_ref())
    }

    /// Takes the answers that came.
    fn community_poll(&mut self, s: &Shared) {
        let c = &mut self.community;
        c.games.poll();
        c.modes.poll();
        c.detail.poll();
        c.signing_in.poll();
        c.mine.poll();
        c.matches.values_mut().for_each(Remote::poll);
        c.latest.values_mut().for_each(Remote::poll);
        if let Remote::Ready(account) = std::mem::take(&mut c.signing_in) {
            if let Err(e) = account.save() {
                c.message = Some((false, format!("Cannot keep the account: {e:#}")));
            }
            c.message = Some((true, format!("Signed in as {}.", account.pseudo)));
            c.account = Some(Some(account));
            c.password.clear();
            c.mine = Remote::Idle;
        } else if let Remote::Failed(e) = &c.signing_in {
            c.message = Some((false, e.clone()));
            c.signing_in = Remote::Idle;
        }
        c.action.poll();
        match std::mem::take(&mut c.action) {
            Remote::Ready(done) => {
                c.message = Some((true, done.message));
                c.installed_for.clear();
                c.latest.clear();
                c.mine = Remote::Idle;
                c.detail = Remote::Idle;
                c.matches.clear();
                if let Some(game) = done.game {
                    self.send(Command::Imported { game });
                } else {
                    self.send(Command::RefreshModes);
                }
                if let Some(mode) = done.select {
                    self.send(Command::SelectMode(mode));
                }
            }
            Remote::Failed(e) => c.message = Some((false, e)),
            other => c.action = other,
        }
        self.scan_installed(s);
    }

    /// Where the user modes come from, read again when they change.
    fn scan_installed(&mut self, s: &Shared) {
        let modes: Vec<String> = s.modes.iter().filter(|e| !e.builtin && e.variant.is_none()).map(|e| e.id.clone()).collect();
        if modes == self.community.installed_for {
            return;
        }
        self.community.installed = modes
            .iter()
            .filter_map(|id| Origin::of(&ModeEntry::from_id(id)).map(|o| (o.id.clone(), (id.clone(), o))))
            .collect();
        self.community.linked = community::links().into_iter().filter(|(_, local)| modes.contains(local)).collect();
        self.community.installed_for = modes;
    }

    /// The origin of the user mode `id` (or of the mode it is a variant of).
    fn origin_of(&self, id: &str) -> Option<&Origin> {
        let main = ModeEntry::from_id(id).main_id();
        self.community.installed.values().find(|(local, _)| *local == main).map(|(_, o)| o)
    }

    fn community_message(&mut self, ui: &mut egui::Ui) {
        let Some((ok, text)) = &self.community.message else { return };
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
            self.community.message = None;
        }
    }

    // --- the Community page

    pub(super) fn community_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.community_poll(s);
        self.open_pending_generator(s);
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.sharing_ui(ui);
            self.community_message(ui);
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    heading(ui, "Community");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!self.sharing.busy(), egui::Button::new("Import a file")).clicked() {
                            self.import_mode();
                        }
                        let open = ui.add_enabled(!self.community.code.trim().is_empty(), egui::Button::new("Open"));
                        let field = ui.add(egui::TextEdit::singleline(&mut self.community.code).hint_text("GV-XXXX-XXXX").desired_width(130.0));
                        if open.clicked() || (field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                            let code = self.community.code.trim().to_owned();
                            self.select_mode_page(ui.ctx(), Source::Code(code));
                        }
                        ui.label(muted("A mode shared with you:"));
                    });
                });
                ui.add_space(8.0);
                match self.community.game.clone() {
                    Some(game) => self.community_game(ui, s, &game),
                    None => self.community_games(ui, s),
                }
            });
        });
    }

    fn community_games(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let mut search = false;
        card(PANEL).inner_margin(Margin::symmetric(18, 16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Which game do you play?").strong().size(17.0));
            ui.label(muted("Modes other players made for it, ready in a click."));
            ui.add_space(6.0);
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.community.search)
                    .hint_text("Elden Ring, Hades II...")
                    .font(egui::TextStyle::Heading)
                    .desired_width(ui.available_width()),
            );
            if field.changed() {
                self.community.typed = Some(std::time::Instant::now());
            }
            // The library's games, one click away.
            if !s.games.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(muted("Your games:"));
                    for game in &s.games {
                        if ui.small_button(&game.name).clicked() {
                            self.community.search = game.name.clone();
                            search = true;
                        }
                    }
                });
            }
        });
        ui.add_space(12.0);
        // Searched once typing stops for a moment: not on every key.
        if let Some(typed) = self.community.typed {
            if typed.elapsed() >= SEARCH_DEBOUNCE {
                self.community.typed = None;
                search = self.community.search.trim() != self.community.searched;
            } else {
                ui.ctx().request_repaint_after(SEARCH_DEBOUNCE - typed.elapsed());
            }
        }
        if search || matches!(self.community.games, Remote::Idle) {
            let (client, text) = (self.client(), self.community.search.trim().to_owned());
            self.community.searched = text.clone();
            self.community.games.start(ui.ctx(), move || client.games(&text));
        }
        // A mode reached by its code shows here, without a game.
        if self.community.selected.is_some() {
            card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.community_mode(ui, s);
            });
            ui.add_space(12.0);
        }
        match &self.community.games {
            Remote::Idle | Remote::Loading(_) => {
                ui.spinner();
            }
            Remote::Failed(e) => {
                let e = e.clone();
                self.unreachable(ui, &e);
            }
            Remote::Ready(games) if games.is_empty() => {
                let searched = self.community.searched.clone();
                self.make_your_own(ui, s, &searched);
            }
            Remote::Ready(games) => {
                let mine: Vec<String> = s.games.iter().map(|g| crate::game::slug(&g.name)).collect();
                let mut games = games.clone();
                // The player's games first.
                games.sort_by_key(|g| !mine.contains(&crate::game::slug(&g.name)));
                let mut open = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::splat(12.0);
                    for game in &games {
                        let yours = mine.contains(&crate::game::slug(&game.name));
                        let response = card(PANEL).inner_margin(Margin::same(14)).show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(220.0);
                                ui.label(RichText::new(&game.name).strong().size(15.0));
                                ui.label(muted(format!("{} mode{}", game.modes, if game.modes == 1 { "" } else { "s" })));
                                if yours {
                                    pill(ui, "In your games", ON_ACCENT, OK);
                                }
                            });
                        });
                        if response.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            open = Some(game.clone());
                        }
                    }
                });
                if let Some(game) = open {
                    self.community.open_game(game);
                }
            }
        }
    }

    /// Nothing found: the player makes the first mode for their game, then shares it.
    fn make_your_own(&mut self, ui: &mut egui::Ui, s: &Shared, game: &str) {
        let mut make = false;
        card(SELECTED_BG).stroke(egui::Stroke::new(1.0, ACCENT)).inner_margin(Margin::symmetric(18, 16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let title = if game.is_empty() { "No mode published yet".to_owned() } else { format!("No mode for “{game}” yet") };
            ui.label(RichText::new(title).strong().size(17.0));
            ui.label(
                "Be the first: an AI assistant writes a mode made for your game in a couple of minutes, from the rumble, \
                 your buttons and what GameViber hears and sees of it. Play it, tune it, then share it: the next players \
                 of this game will find it here.",
            );
            ui.add_space(8.0);
            if !game.is_empty() {
                make = ui.add(primary(&format!("Create a mode for {game}"))).clicked();
            }
            ui.label(muted("Already have one? Publish it from its page, Sharing tab.").size(12.0));
        });
        if make {
            match s.games.iter().find(|g| crate::game::slug(&g.name) == crate::game::slug(game)) {
                Some(local) => self.open_generator_for(&local.clone(), crate::mode::prompt::Depth::Quick),
                None => {
                    self.send(Command::CreateGame { name: game.to_owned(), executable: None });
                    self.community.making_for = Some(game.to_owned());
                }
            }
        }
    }

    /// The game added to make a mode for is in the library: its AI request opens.
    fn open_pending_generator(&mut self, s: &Shared) {
        let Some(name) = &self.community.making_for else { return };
        if let Some(game) = s.games.iter().find(|g| crate::game::slug(&g.name) == crate::game::slug(name)).cloned() {
            self.community.making_for = None;
            self.open_generator_for(&game, crate::mode::prompt::Depth::Quick);
        }
    }

    fn unreachable(&mut self, ui: &mut egui::Ui, error: &str) {
        card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("The community is out of reach").strong());
            ui.label(muted(format!("{error} ({}).", community::URL)));
            if ui.button("Try again").clicked() {
                self.community.games = Remote::Idle;
            }
        });
    }

    fn community_game(&mut self, ui: &mut egui::Ui, s: &Shared, game: &GameView) {
        ui.horizontal(|ui| {
            if ui.button("‹ All games").clicked() {
                self.community.game = None;
                self.community.selected = None;
            }
            ui.label(RichText::new(&game.name).strong().size(18.0));
        });
        ui.add_space(8.0);
        if matches!(self.community.modes, Remote::Idle) {
            let (client, id) = (self.client(), game.id);
            self.community.modes.start(ui.ctx(), move || client.modes(id, SORT));
        }
        let list = |app: &mut Self, ui: &mut egui::Ui| match &app.community.modes {
            Remote::Ready(modes) => {
                let modes = modes.clone();
                for mode in &modes {
                    app.mode_row(ui, mode);
                    ui.add_space(6.0);
                }
                if modes.is_empty() {
                    ui.label(muted("No public mode for this game anymore."));
                }
            }
            Remote::Failed(e) => {
                let e = e.clone();
                app.unreachable(ui, &e);
            }
            _ => {
                ui.spinner();
            }
        };
        let detail = |app: &mut Self, ui: &mut egui::Ui| {
            if app.community.selected.is_some() {
                card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    app.community_mode(ui, s);
                });
            }
        };
        if ui.available_width() >= TWO_COLUMNS_WIDTH {
            let gap = 16.0;
            let side = (ui.available_width() * 0.42).min(460.0);
            let main = ui.available_width() - side - gap;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let layout = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(Vec2::new(main, 0.0), layout, |ui| {
                    ui.set_width(main);
                    list(self, ui);
                });
                ui.allocate_ui_with_layout(Vec2::new(side, 0.0), layout, |ui| {
                    ui.set_width(side);
                    detail(self, ui);
                });
            });
        } else {
            list(self, ui);
            ui.add_space(12.0);
            detail(self, ui);
        }
    }

    fn mode_row(&mut self, ui: &mut egui::Ui, mode: &ModeSummary) {
        let selected = self.community.selected.as_ref() == Some(&Source::Id(mode.id.clone()));
        let response = card(if selected { SELECTED_BG } else { PANEL }).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&mode.name).strong().size(15.0));
                    let mut line = format!("by {} · v{}", mode.author, mode.version);
                    let median = (mode.figures.median_minutes > 0).then(|| format!("{} min median", mode.figures.median_minutes));
                    for figure in [mode.figures.liked(), players(mode.figures.players), median].into_iter().flatten() {
                        line.push_str(&format!(" · {figure}"));
                    }
                    ui.label(muted(line).size(12.0));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match self.community.installed.get(&mode.id) {
                        None if self.community.linked.contains_key(&mode.id) => pill(ui, "Installed", ON_ACCENT, OK),
                        Some((_, origin)) if origin.own => pill(ui, "Yours", ON_ACCENT, OK),
                        Some((_, origin)) if origin.version < mode.version => pill(ui, &format!("Update to v{}", mode.version), ON_ACCENT, WARN),
                        Some(_) => pill(ui, "Installed", ON_ACCENT, OK),
                        None if !community::APIS.contains(&mode.api) => pill(ui, "Needs a newer GameViber", MUTED, RAISED),
                        None => {}
                    }
                });
            });
        });
        if response.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.select_mode_page(ui.ctx(), Source::Id(mode.id.clone()));
        }
    }

    fn select_mode_page(&mut self, ctx: &egui::Context, source: Source) {
        let client = self.client();
        let job = source.clone();
        self.community.detail.start(ctx, move || client.mode(&job));
        self.community.selected = Some(source);
        self.community.report = None;
    }

    /// The page of the mode selected: what it is, its versions, installing or updating it, reporting it.
    fn community_mode(&mut self, ui: &mut egui::Ui, s: &Shared) {
        // Read again after an action (an install, a vote...).
        if let (Remote::Idle, Some(source)) = (&self.community.detail, self.community.selected.clone()) {
            let client = self.client();
            self.community.detail.start(ui.ctx(), move || client.mode(&source));
        }
        let detail = match &self.community.detail {
            Remote::Ready(detail) => detail.clone(),
            Remote::Failed(e) => {
                ui.label(RichText::new(e).color(DANGER_TEXT));
                return;
            }
            _ => {
                ui.spinner();
                return;
            }
        };
        let source = self.community.selected.clone().unwrap_or(Source::Id(detail.id.clone()));
        ui.label(RichText::new(&detail.name).strong().size(18.0));
        ui.label(muted(format!("by {} · {} · {}", detail.author, detail.game.name, downloads(detail.downloads))));
        let f = &detail.figures;
        if f.players > 0 || f.likes + f.dislikes > 0 {
            ui.horizontal_wrapped(|ui| {
                for (value, what) in [
                    (f.players.to_string(), "players, 30 days".to_owned()),
                    (format!("{} min", f.median_minutes), "median play time".to_owned()),
                    (format!("{:.0} %", f.came_back * 100.0), "came back 3+ times".to_owned()),
                    (f.liked().map_or("-".to_owned(), |l| l.replace(" liked", "")), format!("liked · {} votes", f.likes + f.dislikes)),
                ] {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(value).strong().size(17.0));
                        ui.label(muted(what).size(11.5));
                    });
                    ui.add_space(12.0);
                }
            });
        }
        if !detail.description.is_empty() {
            ui.add_space(6.0);
            ui.label(&detail.description);
        }
        ui.add_space(8.0);
        let latest = detail.latest().cloned();
        let installed = self.community.installed.get(&detail.id).cloned();
        ui.horizontal_wrapped(|ui| {
            let busy = self.community.busy();
            match (&installed, &latest) {
                (_, None) => {
                    ui.label(muted("No version yet."));
                }
                (_, Some(v)) if !community::APIS.contains(&v.api) => {
                    ui.label(muted(format!("Needs a newer GameViber (mode API {}).", v.api)));
                }
                (Some((local, origin)), Some(v)) => {
                    let update = !origin.own && origin.version < v.number;
                    if update && ui.add_enabled(!busy, primary(&format!("Update to v{}", v.number))).clicked() {
                        self.update_mode(ui.ctx(), s, local.clone());
                    }
                    let open = if update { egui::Button::new("Open") } else { primary("Open") };
                    if ui.add(open).on_hover_text("Its page, in Games").clicked() {
                        self.open_installed(s, local);
                    }
                    let what = if origin.own { "Yours".to_owned() } else { format!("Installed · v{}", origin.version) };
                    pill(ui, &what, ON_ACCENT, OK);
                }
                (None, Some(_)) if self.community.linked.contains_key(&detail.id) => {
                    let local = self.community.linked[&detail.id].clone();
                    if ui.add(primary("Open")).on_hover_text("Its page, in Games").clicked() {
                        self.open_installed(s, &local);
                    }
                    pill(ui, "You have it already", ON_ACCENT, OK);
                }
                (None, Some(_)) => {
                    if ui.add_enabled(!busy, primary("Install")).clicked() {
                        let (client, detail) = (self.client(), detail.clone());
                        self.community.action.start(ui.ctx(), move || {
                            let imported = community::install(&client, &source, &detail)?;
                            let message = if imported.mode_existed {
                                format!("You have {} already, as a mode of yours: Open leads to it.", detail.name)
                            } else {
                                format!("{} installed for {}.", detail.name, imported.name)
                            };
                            Ok(Done { message, game: Some(imported.game), select: None })
                        });
                    }
                }
            }
            if busy {
                ui.spinner();
            }
        });
        if let Some((local, origin)) = installed.as_ref().filter(|(_, o)| !o.own) {
            ui.add_space(6.0);
            self.vote_ui(ui, s, local, origin);
        }
        ui.add_space(8.0);
        eyebrow(ui, "Versions");
        for version in detail.versions.iter().take(5) {
            let date = version.created_at.get(..10).unwrap_or(&version.created_at);
            let notes = if version.changelog.is_empty() { String::new() } else { format!(" · {}", version.changelog) };
            ui.label(RichText::new(format!("v{} · {date}{notes}", version.number)).size(12.5));
        }
        ui.add_space(8.0);
        self.report_ui(ui, &detail.id);
    }

    /// Liked or not, for a mode installed from the community and played.
    fn vote_ui(&mut self, ui: &mut egui::Ui, s: &Shared, local: &str, origin: &Origin) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Did you like it?");
            if s.settings.share_stats != Some(true) {
                ui.label(muted("Votes are sent with your stats: share them in Settings to vote.").size(12.0));
                return;
            }
            if Usage::of(&ModeEntry::from_id(local)).played_secs < 60.0 {
                ui.label(muted("Play it first, then say what you think.").size(12.0));
                return;
            }
            for (value, label) in [(1i8, "👍 Liked it"), (-1, "👎 Not for me")] {
                let chosen = origin.vote == value;
                if ui.add_enabled(!self.community.busy(), egui::Button::new(label).selected(chosen)).clicked() {
                    // Clicked again: taken back.
                    let value = if chosen { 0 } else { value };
                    let (client, installation, id, local, origin) =
                        (self.client(), s.settings.installation_id.clone(), origin.id.clone(), local.to_owned(), origin.clone());
                    self.community.action.start(ui.ctx(), move || {
                        client.vote(&installation, &id, value)?;
                        Origin { vote: value, ..origin }.save(&ModeEntry::from_id(&local))?;
                        let message = if value == 0 { "Vote taken back." } else { "Thanks: your vote counts." };
                        Ok(Done { message: message.into(), game: None, select: None })
                    });
                }
            }
        });
    }

    fn report_ui(&mut self, ui: &mut egui::Ui, id: &str) {
        match &mut self.community.report {
            Some((mode, reason, details)) if mode == id => {
                let mut send = false;
                let mut cancel = false;
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("report-reason").selected_text(REASONS[*reason].1).show_ui(ui, |ui| {
                        for (i, (_, label)) in REASONS.iter().enumerate() {
                            ui.selectable_value(reason, i, *label);
                        }
                    });
                    ui.add(egui::TextEdit::singleline(details).hint_text("What happens (optional)").desired_width(220.0));
                    send = ui.button("Send").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                let report = send.then(|| (REASONS[*reason].0, details.clone()));
                if send || cancel {
                    self.community.report = None;
                }
                if let Some((reason, details)) = report {
                    let (client, id) = (self.client(), id.to_owned());
                    self.community.action.start(ui.ctx(), move || {
                        client.report(&id, reason, &details)?;
                        Ok(Done { message: "Thanks: the report was sent.".into(), game: None, select: None })
                    });
                }
            }
            _ => {
                if ui.small_button("Report this mode").clicked() {
                    self.community.report = Some((id.to_owned(), 0, String::new()));
                }
            }
        }
    }

    /// Opens the page of the installed mode `local` (playing its game).
    fn open_installed(&mut self, s: &Shared, local: &str) {
        let game = s.games.iter().find(|g| g.modes.iter().any(|m| m == local));
        if let Some(game) = game {
            if s.game.as_ref().is_none_or(|g| g.id != game.id) {
                self.send(Command::SelectGame(Some(game.id.clone())));
            }
            self.route = Route::Game { id: game.id.clone(), view: View::Mode };
        }
        self.send(Command::SelectMode(local.to_owned()));
        self.page = Page::Games;
    }

    fn update_mode(&mut self, ctx: &egui::Context, s: &Shared, local: String) {
        let client = self.client();
        let active = super::main_of(&s.mode.id) == local;
        self.community.action.start(ctx, move || {
            let message = match community::update(&client, &local)? {
                Some(_) => "Updated beside your changed version: both are in the game's modes.".to_owned(),
                None => "Updated.".to_owned(),
            };
            // The active mode reads its new script and inputs.
            Ok(Done { message, game: None, select: active.then_some(local) })
        });
    }

    // --- a game's page and the library

    /// What the server has for `game`: started once per game.
    fn community_match(&mut self, ctx: &egui::Context, game: &Game) -> Option<&Remote<Option<(GameView, Vec<ModeSummary>)>>> {
        if !self.community.matches.contains_key(&game.id) {
            let (client, name, app) = (self.client(), game.name.clone(), game.steam_app_id);
            let mut remote = Remote::Idle;
            remote.start(ctx, move || {
                let Some(found) = client.game_match(&name, app)? else { return Ok(None) };
                let modes = client.modes(found.id, SORT)?;
                Ok(Some((found, modes)))
            });
            self.community.matches.insert(game.id.clone(), remote);
        }
        self.community.matches.get(&game.id)
    }

    /// The card on a game's page: the community's modes for it.
    pub(super) fn community_card(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        self.community_poll(s);
        let found = match self.community_match(ui.ctx(), game) {
            Some(Remote::Ready(found)) => found.clone(),
            Some(Remote::Failed(_)) => {
                ui.label(muted("The community is out of reach (Settings › Community server)."));
                return;
            }
            _ => return,
        };
        let mut open = None;
        card(RAISED).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match &found {
                None => {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("From the community").strong());
                        ui.label(muted(format!("No mode for {} yet: publish yours from its Sharing tab.", game.name)));
                    });
                }
                Some((remote, modes)) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("From the community").strong());
                        ui.label(muted(format!("{} mode{}", remote.modes, if remote.modes == 1 { "" } else { "s" })));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("See all ›").clicked() {
                                open = Some((remote.clone(), None));
                            }
                        });
                    });
                    for mode in modes.iter().take(3) {
                        ui.horizontal(|ui| {
                            let link = ui.add(egui::Label::new(RichText::new(&mode.name).color(ACCENT_TEXT)).sense(egui::Sense::click()));
                            if link.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                open = Some((remote.clone(), Some(mode.id.clone())));
                            }
                            ui.label(muted(format!("by {} · {}", mode.author, downloads(mode.downloads))).size(12.0));
                            if self.community.installed.contains_key(&mode.id) || self.community.linked.contains_key(&mode.id) {
                                pill(ui, "Installed", ON_ACCENT, OK);
                            }
                        });
                    }
                }
            }
        });
        if let Some((remote, mode)) = open {
            self.community.open_game(remote);
            if let Some(mode) = mode {
                self.select_mode_page(ui.ctx(), Source::Id(mode));
            }
            self.page = Page::Community;
        }
    }

    /// Above the library: the game being played has modes in the community, and none of the player's.
    pub(super) fn community_banner(&mut self, ui: &mut egui::Ui, s: &Shared) {
        self.community_poll(s);
        let Some(game) = &s.game else { return };
        let own = game.modes.iter().any(|m| !ModeEntry::from_id(m).builtin);
        if own {
            return;
        }
        let Some(Remote::Ready(Some((remote, _)))) = self.community_match(ui.ctx(), game) else { return };
        let remote = remote.clone();
        let mut open = false;
        card(SELECTED_BG).stroke(egui::Stroke::new(1.0, ACCENT)).inner_margin(Margin::symmetric(16, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(format!("{} has {} community mode{}", game.name, remote.modes, if remote.modes == 1 { "" } else { "s" })).strong());
                    ui.label(muted("You have no mode of your own for it: try one made by other players."));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| open = ui.add(primary("See them ›")).clicked());
            });
        });
        ui.add_space(10.0);
        if open {
            self.community.open_game(remote);
            self.page = Page::Community;
        }
    }

    /// In a game's modes list: the update of a mode installed from the community, if one came.
    pub(super) fn community_update_button(&mut self, ui: &mut egui::Ui, s: &Shared, local: &str) {
        let Some(origin) = self.origin_of(local).cloned() else { return };
        if origin.own || origin.pinned {
            return;
        }
        if !self.community.latest.contains_key(&origin.id) {
            let (client, source) = (self.client(), origin.source());
            let mut remote = Remote::Idle;
            remote.start(ui.ctx(), move || client.mode(&source));
            self.community.latest.insert(origin.id.clone(), remote);
        }
        let Some(Remote::Ready(detail)) = self.community.latest.get(&origin.id) else { return };
        let Some(latest) = detail.latest().filter(|v| v.number > origin.version && community::APIS.contains(&v.api)).cloned() else { return };
        let hint = if latest.changelog.is_empty() { "A new version".to_owned() } else { latest.changelog.clone() };
        if ui.add_enabled(!self.community.busy(), egui::Button::new(format!("Update to v{}", latest.number))).on_hover_text(hint).clicked() {
            self.update_mode(ui.ctx(), s, local.to_owned());
        }
    }

    /// Under a mode of the player's that works (played a while, unchanged):
    /// a suggestion to publish it, never while playing it.
    pub(super) fn publish_suggestion(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, local: &str) {
        let entry = ModeEntry::from_id(local);
        if entry.builtin || self.origin_of(local).is_some() {
            return;
        }
        let stale = self.community.usage.get(local).is_none_or(|(read, _)| read.elapsed().as_secs() >= 10);
        if stale {
            self.community.usage.insert(local.to_owned(), (std::time::Instant::now(), Usage::of(&entry)));
        }
        let usage = self.community.usage[local].1.clone();
        let playing = s.running_executable.is_some() && super::main_of(&s.mode.id) == local;
        if !usage.suggest() || playing {
            return;
        }
        let name = s.catalog.get(local).and_then(|r| r.as_ref().ok()).map_or(entry.key.clone(), |i| i.name.clone());
        let (mut later, mut never, mut publish) = (false, false, false);
        egui::Frame::new().fill(RAISED).inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let hours = (usage.played_secs / 3600.0).floor();
                ui.label(RichText::new(format!("{hours} h of play, no change since.")).strong());
                ui.label(muted(format!("{name} seems to work well: share it with other players of {}?", game.name)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    publish = ui.add(primary("Publish")).clicked();
                    later = ui.button("Later").clicked();
                    never = ui.button("Don't ask for this mode").clicked();
                });
            });
        });
        if later || never {
            let usage = Usage { remind_at_secs: usage.played_secs + community::SUGGEST_AFTER_SECS, dismissed: never, ..usage };
            usage.save(&entry);
            self.community.usage.remove(local);
        }
        if publish {
            if s.game.as_ref().is_none_or(|g| g.id != game.id) {
                self.send(Command::SelectGame(Some(game.id.clone())));
            }
            self.send(Command::SelectMode(local.to_owned()));
            self.route = Route::Game { id: game.id.clone(), view: View::Sharing };
        }
    }

    // --- the Sharing tab: publishing

    fn account_ui(&mut self, ui: &mut egui::Ui) -> Option<Account> {
        if let Some(account) = self.account() {
            ui.horizontal(|ui| {
                ui.label(muted(format!("Publishing as {}", account.pseudo)));
                if ui.small_button("Sign out").clicked() {
                    Account::sign_out();
                    self.community.account = Some(None);
                    self.community.mine = Remote::Idle;
                }
            });
            return Some(account);
        }
        ui.label(RichText::new("You, as an author").strong());
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.community.pseudo).hint_text("Name shown to players").desired_width(180.0));
            ui.add(egui::TextEdit::singleline(&mut self.community.password).password(true).hint_text("Password (8 characters or more)").desired_width(200.0));
            let ready = !self.community.pseudo.trim().is_empty() && !self.community.password.is_empty() && !self.community.signing_in.loading();
            for (new, label) in [(true, "Create the account"), (false, "Sign in")] {
                if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
                    let (client, pseudo, password) = (self.client(), self.community.pseudo.trim().to_owned(), self.community.password.clone());
                    self.community.signing_in.start(ui.ctx(), move || client.sign_in(&pseudo, &password, new));
                }
            }
            if self.community.signing_in.loading() {
                ui.spinner();
            }
        });
        ui.label(RichText::new("No email: a lost password cannot be recovered. Linking Discord or an email comes later.").color(WARN).size(12.0));
        None
    }

    /// The Sharing tab's community part, for the active mode of `game`.
    pub(super) fn community_sharing(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game) {
        self.community_poll(s);
        self.community_message(ui);
        let local = super::main_of(&s.mode.id);
        let origin = self.origin_of(&local).cloned();
        card(RAISED).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("In the community").strong().size(16.0));
            match &origin {
                Some(origin) if !origin.own => self.installed_panel(ui, s, &local, origin),
                _ => {
                    let Some(account) = self.account_ui(ui) else { return };
                    ui.add_space(8.0);
                    match &origin {
                        Some(origin) => self.published_panel(ui, game, &local, origin, &account),
                        None => self.publish_form(ui, s, game, &local),
                    }
                }
            }
        });
    }

    /// A mode installed from the community: where from, its updates, keeping this version, reporting it.
    fn installed_panel(&mut self, ui: &mut egui::Ui, s: &Shared, local: &str, origin: &Origin) {
        let how = if origin.code.is_some() { "shared with you by a code" } else { "from the community" };
        ui.label(format!("Installed {how} · version {}.", origin.version));
        if origin.changed(&ModeEntry::from_id(local)) {
            ui.label(muted("You changed it since: an update installs beside it, yours stays."));
        }
        ui.horizontal(|ui| {
            self.community_update_button(ui, s, local);
            let mut pinned = origin.pinned;
            if ui.checkbox(&mut pinned, "Keep this version").on_hover_text("No update is offered").changed() {
                let entry = ModeEntry::from_id(local);
                if let Err(e) = (Origin { pinned, ..origin.clone() }).save(&entry) {
                    self.community.message = Some((false, format!("{e:#}")));
                }
                self.community.installed_for.clear();
            }
        });
        ui.add_space(6.0);
        self.vote_ui(ui, s, local, origin);
        self.report_ui(ui, &origin.id);
    }

    /// The player's published mode: who gets it, its code, its next version, withdrawing it.
    fn published_panel(&mut self, ui: &mut egui::Ui, game: &Game, local: &str, origin: &Origin, _account: &Account) {
        if matches!(self.community.mine, Remote::Idle) {
            let client = self.client();
            self.community.mine.start(ui.ctx(), move || client.my_modes());
        }
        let detail = match &self.community.mine {
            Remote::Ready(mine) => mine.iter().find(|m| m.id == origin.id).cloned(),
            Remote::Failed(e) => {
                ui.label(RichText::new(e).color(DANGER_TEXT));
                return;
            }
            _ => {
                ui.spinner();
                return;
            }
        };
        let Some(detail) = detail else {
            ui.label(muted("This mode was published from another account: sign in with it to manage it."));
            return;
        };
        if let Some(reason) = &detail.withdrawn_reason {
            ui.label(RichText::new(format!("Withdrawn: {reason}.")).color(WARN));
            return;
        }
        let busy = self.community.busy();
        let id = detail.id.clone();
        ui.horizontal_wrapped(|ui| {
            let who = if detail.is_public() { "Everyone (listed in Community)" } else { "People with the code" };
            ui.label(format!("Published · v{} · {} · {who}", origin.version, downloads(detail.downloads)));
            let (label, public) = if detail.is_public() { ("Make it private", false) } else { ("Publish for everyone", true) };
            if ui.add_enabled(!busy, egui::Button::new(label)).clicked() {
                let (client, id) = (self.client(), id.clone());
                self.community.action.start(ui.ctx(), move || {
                    client.change(&id, None, None, Some(public))?;
                    Ok(Done { message: if public { "Listed for everyone.".into() } else { "Only people with the code get it now.".into() }, game: None, select: None })
                });
            }
        });
        if let Some(code) = &detail.share_code {
            ui.horizontal(|ui| {
                ui.label("Tester code");
                ui.label(RichText::new(code).monospace().strong().size(16.0));
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(code.clone());
                }
                if ui.add_enabled(!busy, egui::Button::new("New code").small()).on_hover_text("The current one stops working").clicked() {
                    let (client, id) = (self.client(), id.clone());
                    self.community.action.start(ui.ctx(), move || {
                        client.new_share_code(&id)?;
                        Ok(Done { message: "A new code: the old one no longer works.".into(), game: None, select: None })
                    });
                }
            });
        }
        ui.add_space(6.0);
        let changed = origin.changed(&ModeEntry::from_id(local));
        ui.horizontal_wrapped(|ui| {
            if changed {
                ui.label(RichText::new("Changed since you published it.").color(WARN));
            }
            ui.add(egui::TextEdit::singleline(&mut self.community.publish.changelog).hint_text("What changed").desired_width(240.0));
            let next = format!("Publish v{}", origin.version + 1);
            if ui.add_enabled(!busy && !self.community.publish.changelog.trim().is_empty(), primary(&next)).clicked() {
                let (client, game, mode, changelog) = (self.client(), game.id.clone(), local.to_owned(), self.community.publish.changelog.clone());
                let left_out: Vec<String> = self.community.publish.left_out.iter().cloned().collect();
                self.community.action.start(ui.ctx(), move || {
                    let detail = community::publish(&client, &game, &mode, "", "", false, &changelog, &left_out)?;
                    Ok(Done { message: format!("Version {} published.", detail.latest().map_or(1, |v| v.number)), game: None, select: None })
                });
                self.community.publish.changelog.clear();
            }
        });
        ui.add_space(6.0);
        if ui.add_enabled(!busy, egui::Button::new(RichText::new("Withdraw from the community").color(DANGER_TEXT))).clicked() {
            let client = self.client();
            self.community.action.start(ui.ctx(), move || {
                client.withdraw(&id)?;
                Ok(Done { message: "Withdrawn: nobody can get it anymore.".into(), game: None, select: None })
            });
        }
    }

    /// Publishing the mode for the first time.
    fn publish_form(&mut self, ui: &mut egui::Ui, s: &Shared, game: &Game, local: &str) {
        let (name, description) = s.mode.info.as_ref().map_or_else(Default::default, |info| {
            (info.name.clone(), if info.help.is_empty() { info.description.clone() } else { info.help.clone() })
        });
        let form = &mut self.community.publish;
        if form.mode != local {
            *form = PublishForm {
                mode: local.to_owned(),
                name,
                description,
                changelog: "First version".into(),
                ..Default::default()
            };
        }
        egui::Grid::new("publish-form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(320.0));
            ui.end_row();
            ui.label("Description");
            ui.add(egui::TextEdit::multiline(&mut form.description).desired_rows(3).desired_width(420.0));
            ui.end_row();
            ui.label("Who gets it");
            ui.vertical(|ui| {
                ui.radio_value(&mut form.public, false, "People with the code: let a few testers try it first");
                ui.radio_value(&mut form.public, true, format!("Everyone: listed in Community under {}", game.name));
            });
            ui.end_row();
        });
        ui.add_space(6.0);
        self.captures_to_send(ui, s);
        let form = &mut self.community.publish;
        ui.checkbox(&mut form.accepted, "Shared under the MIT license: anyone can copy and change it, keeping your name.");
        let ready = form.accepted && !form.name.trim().is_empty() && !self.community.action.loading();
        if ui.add_enabled(ready, primary("Publish")).clicked() {
            let client = self.client();
            let (game, mode) = (game.id.clone(), local.to_owned());
            let form = &self.community.publish;
            let (name, description, public, changelog) = (form.name.trim().to_owned(), form.description.trim().to_owned(), form.public, form.changelog.clone());
            let left_out: Vec<String> = form.left_out.iter().cloned().collect();
            self.community.action.start(ui.ctx(), move || {
                let detail = community::publish(&client, &game, &mode, &name, &description, public, &changelog, &left_out)?;
                let message = match (&detail.share_code, public) {
                    (Some(code), false) => format!("Published: share the code {code} with your testers."),
                    _ => "Published: it is listed in Community.".to_owned(),
                };
                Ok(Done { message, game: None, select: None })
            });
        }
    }

    /// The captures that will be sent, to look at and leave some out.
    fn captures_to_send(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let Some(inputs) = s.mode_inputs.clone() else { return };
        let captures: Vec<_> = inputs.captures.iter().filter(|c| !c.phase.is_empty()).cloned().collect();
        if captures.is_empty() {
            return;
        }
        self.load_captures(ui.ctx(), Some(&inputs));
        ui.label(RichText::new(format!("Captures that will be sent · {}", captures.len() - self.community.publish.left_out.len())).strong());
        ui.label(muted("Images of your screen: look for your name, chat messages or notifications, and leave out those you would rather keep.").size(12.0));
        // As many as fit on a row.
        let columns = ((ui.available_width() + 8.0) / 128.0).floor().max(1.0) as usize;
        egui::Grid::new("captures-to-send").spacing([8.0, 8.0]).show(ui, |ui| {
            for (i, capture) in captures.iter().enumerate() {
                let left_out = self.community.publish.left_out.contains(&capture.file);
                ui.vertical(|ui| {
                    if let Some(texture) = self.screen.capture_texture(&capture.file) {
                        let tint = if left_out { egui::Color32::from_gray(70) } else { egui::Color32::WHITE };
                        ui.add(egui::Image::new(&texture).fit_to_exact_size(Vec2::new(120.0, 68.0)).tint(tint));
                    }
                    let mut send = !left_out;
                    if ui.checkbox(&mut send, RichText::new(&capture.phase).size(11.5)).changed() {
                        if send {
                            self.community.publish.left_out.remove(&capture.file);
                        } else {
                            self.community.publish.left_out.insert(capture.file.clone());
                        }
                    }
                });
                if (i + 1) % columns == 0 {
                    ui.end_row();
                }
            }
        });
        ui.add_space(6.0);
    }
}

/// "3 players" (none: nothing to say).
fn players(n: u64) -> Option<String> {
    match n {
        0 => None,
        1 => Some("1 player".to_owned()),
        n => Some(format!("{n} players")),
    }
}

fn downloads(n: u64) -> String {
    if n == 1 { "1 download".to_owned() } else { format!("{n} downloads") }
}

/// For the screenshot tour: the first game with modes, and its first mode's page.
pub(super) fn tour_first_game(app: &mut App, ctx: &egui::Context) {
    if let Remote::Ready(games) = &app.community.games {
        if let Some(game) = games.first().cloned().filter(|_| app.community.game.is_none()) {
            app.community.open_game(game);
        }
    }
    if let Remote::Ready(modes) = &app.community.modes {
        if let Some(mode) = modes.first().map(|m| m.id.clone()).filter(|_| app.community.selected.is_none()) {
            app.select_mode_page(ctx, Source::Id(mode));
        }
    }
}

/// For the screenshot tour: the Community page searching `game`.
pub(super) fn tour_search(app: &mut App, game: &str) {
    app.community.game = None;
    app.community.selected = None;
    if app.community.searched != game {
        app.community.search = game.to_owned();
        app.community.searched = game.to_owned();
        app.community.games = Remote::Idle;
    }
}

// --- the first launch: sharing stats or not

impl App {
    /// Asked once: whether play time and votes are sent to the community.
    pub(super) fn stats_consent(&mut self, ctx: &egui::Context, s: &Shared) {
        // The settings are only known after the engine's first tick.
        let asked = s.settings.share_stats.is_some() || s.time == 0.0 || self.community.consent_answered;
        if asked && !self.community.consent_preview {
            return;
        }
        let mut answer = None;
        egui::Modal::new(egui::Id::new("stats-consent")).show(ctx, |ui| {
            ui.set_width(520.0);
            heading(ui, "Help players find the best modes");
            ui.label(
                "GameViber can tell the community how long you play the modes you installed from it, and send your votes. \
                 That is what ranks modes by how much players keep playing them, rather than by downloads, and shows \
                 authors that their mode is played.",
            );
            ui.add_space(6.0);
            ui.label(RichText::new("What is sent").strong());
            ui.label(
                "An id made up for this installation, which modes you played, for how many minutes and sessions, and your \
                 votes. Never your name, your games' image or sound, your toys or how they ran; not linked to your author \
                 account.",
            );
            ui.label(muted("You can change your mind any time in Settings."));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let size = Vec2::new(180.0, 36.0);
                if ui.add(egui::Button::new("Share my stats").min_size(size)).clicked() {
                    answer = Some(true);
                }
                if ui.add(egui::Button::new("Don't share").min_size(size)).clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(share) = answer {
            self.community.consent_answered = true;
            self.send(Command::SetShareStats(share));
        }
    }
}
