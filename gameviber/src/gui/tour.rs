//! A tour of every page for development: with `GAMEVIBER_SCREENSHOTS=<dir>`,
//! the GUI opens each page (and its dialogs) in turn and saves a screenshot
//! of it in `<dir>`, to check the layout without clicking through it.

use std::path::PathBuf;

use eframe::egui;

use super::{setup, App, GameView, Page, Route};
use crate::engine::Shared;
use crate::mode::prompt::Depth;

/// Frames to wait on a page before its screenshot (data loads, layout settles).
const SETTLE_FRAMES: u32 = 30;

pub struct Tour {
    dir: PathBuf,
    step: usize,
    frames: u32,
    requested: bool,
}

#[derive(Clone, Copy)]
enum Stop {
    Page(Page),
    Games(fn(&Shared) -> Option<Route>),
    Setup(setup::Tab),
    AddGame,
    Generator,
    /// The captures and zones page, editing a bar zone (else the first zone).
    ZoneEdit,
}

const STOPS: [(&str, Stop); 18] = [
    ("games", Stop::Games(|_| Some(Route::Library))),
    ("add-game", Stop::AddGame),
    ("builtin-modes", Stop::Games(|_| Some(Route::BuiltIn))),
    ("game-modes", Stop::Games(|s| game(s, GameView::Modes))),
    ("generator", Stop::Generator),
    ("mode", Stop::Games(|s| game(s, GameView::Mode))),
    ("signals", Stop::Games(|s| game(s, GameView::Signals))),
    ("captures-zones", Stop::Games(|s| game(s, GameView::Screen))),
    ("zone-edit", Stop::ZoneEdit),
    ("sessions", Stop::Games(|s| game(s, GameView::Sessions))),
    ("toys", Stop::Page(Page::Toys)),
    ("setup-gamepad", Stop::Setup(setup::Tab::Gamepad)),
    ("setup-combos", Stop::Setup(setup::Tab::Combos)),
    ("setup-overlay", Stop::Setup(setup::Tab::Overlay)),
    ("setup-sound", Stop::Setup(setup::Tab::Sound)),
    ("setup-programs", Stop::Setup(setup::Tab::Programs)),
    ("creator", Stop::Page(Page::Creator)),
    ("settings", Stop::Page(Page::Settings)),
];

/// The first game's page, in `view`.
fn game(s: &Shared, view: GameView) -> Option<Route> {
    let id = s.game.as_ref().or(s.games.first())?.id.clone();
    Some(Route::Game { id, view })
}

impl Tour {
    pub fn from_env() -> Option<Self> {
        let dir = PathBuf::from(std::env::var_os("GAMEVIBER_SCREENSHOTS")?);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self { dir, step: 0, frames: 0, requested: false })
    }
}

impl App {
    /// Shows the tour's current stop, and saves its screenshot once it settled.
    pub(super) fn tour(&mut self, ctx: &egui::Context, s: &Shared) {
        let Some(tour) = &mut self.tour else { return };
        let Some((name, stop)) = STOPS.get(tour.step).copied() else {
            log::info!("screenshot tour done: {}", tour.dir.display());
            self.tour = None;
            return;
        };
        // A screenshot taken: save it and go on.
        let shot = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = shot {
            let path = tour.dir.join(format!("{:02}-{name}.png", tour.step + 1));
            if let Err(e) = save_png(&path, &image) {
                log::error!("cannot save {}: {e}", path.display());
            }
            tour.step += 1;
            tour.frames = 0;
            tour.requested = false;
            self.generator.open = false;
            self.games.close_dialog();
            return;
        }
        tour.frames += 1;
        let request = tour.frames >= SETTLE_FRAMES && !tour.requested;
        if request {
            tour.requested = true;
        }
        match stop {
            Stop::Page(page) => self.page = page,
            Stop::Games(route) => {
                self.page = Page::Games;
                if let Some(route) = route(s) {
                    self.route = route;
                }
            }
            Stop::Setup(tab) => {
                self.page = Page::Setup;
                self.setup_tab = tab;
            }
            Stop::AddGame => {
                self.page = Page::Games;
                self.route = Route::Library;
                self.games.open_dialog();
            }
            Stop::ZoneEdit => {
                self.page = Page::Games;
                if let Some(game) = s.game.as_ref().or(s.games.first()).cloned() {
                    self.route = Route::Game { id: game.id.clone(), view: GameView::Screen };
                    if tour.frames == 1 {
                        let zone = game.zones.iter().position(|z| z.kind == crate::game::ZoneKind::Bar).unwrap_or(0);
                        self.screen.edit_zone(zone, &game);
                    }
                }
            }
            Stop::Generator => {
                if !self.generator.open {
                    if let Some(game) = s.game.as_ref().or(s.games.first()).cloned() {
                        self.open_generator_for(&game, Depth::Advanced);
                    }
                }
            }
        }
        if request {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }
}

fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.size[0] as u32, image.size[1] as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    let pixels: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
    writer.write_image_data(&pixels).map_err(std::io::Error::other)?;
    writer.finish().map_err(std::io::Error::other)
}
