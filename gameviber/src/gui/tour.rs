//! A tour of every page for development: with `GAMEVIBER_SCREENSHOTS=<dir>`,
//! the GUI opens each page (and its dialogs) in turn and saves a screenshot
//! of it in `<dir>`, to check the layout without clicking through it.

use std::path::PathBuf;

use eframe::egui;

use super::creator::Tab;
use super::pad_setup::{Labels, PadSetup};
use super::{setup, App, Page, Route};
use crate::engine::Shared;

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
    Library(Route),
    /// The Creator on a tab, how a mode works shown or not.
    Creator(Tab, bool),
    Setup(setup::Tab),
    /// The newest recording open in the session player, half way through.
    SessionPlayer,
    /// The captures and indicators tab, editing a bar indicator (else the first indicator)...
    IndicatorEdit,
    /// ...and a new visibility indicator, not drawn yet.
    IndicatorNew,
    /// The Community page on its first game, and that game's first mode.
    CommunityGame,
    /// The Community page searching a game nobody made a mode for.
    CommunityNothing,
    /// The first-launch question about sharing stats.
    StatsConsent,
    /// Setting up the gamepad's buttons, half done (when the proxy reads one).
    PadSetup(Labels),
}

const STOPS: [(&str, Stop); 28] = [
    ("library", Stop::Library(Route::Library)),
    ("create-mode", Stop::Library(Route::Create)),
    ("mode", Stop::Library(Route::Mode)),
    ("creator-assistant", Stop::Creator(Tab::Assistant, false)),
    ("creator-phases", Stop::Creator(Tab::Phases, false)),
    ("creator-help", Stop::Creator(Tab::Phases, true)),
    ("creator-sessions", Stop::Creator(Tab::Sessions, false)),
    ("session-player", Stop::SessionPlayer),
    ("creator-captures-indicators", Stop::Creator(Tab::Screen, false)),
    ("indicator-edit", Stop::IndicatorEdit),
    ("indicator-new", Stop::IndicatorNew),
    ("creator-programs", Stop::Creator(Tab::Programs, false)),
    ("creator-script", Stop::Creator(Tab::Script, false)),
    ("creator-logs", Stop::Creator(Tab::Logs, false)),
    ("community", Stop::Page(Page::Community)),
    ("community-game", Stop::CommunityGame),
    ("community-nothing", Stop::CommunityNothing),
    ("stats-consent", Stop::StatsConsent),
    ("live", Stop::Page(Page::Live)),
    ("toys", Stop::Page(Page::Toys)),
    ("setup-gamepad", Stop::Setup(setup::Tab::Gamepad)),
    ("pad-setup", Stop::PadSetup(Labels::Xbox)),
    ("pad-setup-playstation", Stop::PadSetup(Labels::PlayStation)),
    ("setup-combos", Stop::Setup(setup::Tab::Combos)),
    ("setup-overlay", Stop::Setup(setup::Tab::Overlay)),
    ("setup-sound", Stop::Setup(setup::Tab::Sound)),
    ("setup-programs", Stop::Setup(setup::Tab::Programs)),
    ("settings", Stop::Page(Page::Settings)),
];

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
            self.pad_setup = None;
            return;
        }
        tour.frames += 1;
        let request = tour.frames >= SETTLE_FRAMES && !tour.requested;
        if request {
            tour.requested = true;
        }
        // The consent window shows on its own stop only.
        self.community.consent_preview = matches!(stop, Stop::StatsConsent);
        match stop {
            Stop::Page(page) => self.page = page,
            Stop::Library(route) => {
                self.page = Page::Library;
                self.route = route;
            }
            Stop::Creator(tab, help) => {
                self.page = Page::Creator;
                self.creator.tab = tab;
                self.creator.help = help;
            }
            Stop::Setup(tab) => {
                self.page = Page::Setup;
                self.setup_tab = tab;
            }
            Stop::IndicatorEdit => {
                self.page = Page::Creator;
                self.creator.tab = Tab::Screen;
                self.creator.help = false;
                if let Some(inputs) = s.mode_inputs.as_ref().filter(|_| tour.frames == 1) {
                    let zone = inputs.zones.iter().position(|z| z.kind == crate::package::IndicatorKind::Gauge).unwrap_or(0);
                    self.screen.edit_zone(zone, inputs);
                }
            }
            Stop::IndicatorNew => {
                self.page = Page::Creator;
                self.creator.tab = Tab::Screen;
                if tour.frames == 1 {
                    self.screen.new_indicator();
                }
            }
            Stop::SessionPlayer => {
                self.page = Page::Creator;
                self.creator.tab = Tab::Sessions;
                if let Some(info) = s.recordings.first().filter(|_| tour.frames == 1) {
                    self.creator.sessions.preview(info);
                }
            }
            Stop::StatsConsent => {}
            Stop::PadSetup(labels) => {
                self.page = Page::Setup;
                self.setup_tab = setup::Tab::Gamepad;
                if let Some(pad) = s.pad.as_ref().filter(|_| self.pad_setup.is_none()) {
                    self.pad_setup = Some(PadSetup::preview(pad, labels));
                }
            }
            Stop::CommunityNothing => {
                self.page = Page::Community;
                super::community::tour_search(self, "Hollow Knight: Silksong");
            }
            Stop::CommunityGame => {
                self.page = Page::Community;
                super::community::tour_first_game(self, ctx);
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
