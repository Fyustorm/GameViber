//! Images loaded in threads: games' banners, as the website shows them (the
//! header image of their Steam store page, for the games whose Steam app id is
//! known, kept in GameViber's data directory once downloaded), and the images
//! showing modes (a community mode's cover from the server, a mode of the
//! library's from its package).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use anyhow::Context;
use eframe::egui::{self, Align2, ColorImage, CornerRadius, FontId, TextureHandle, TextureOptions, Vec2};

use super::theme::*;
use crate::config;

/// Width over height of Steam's header images (460 × 215).
pub(super) const BANNER_RATIO: f32 = 460.0 / 215.0;
/// Larger images are shrunk to this width: they are only shown small.
const MAX_WIDTH: u32 = 640;
/// A capture with an indicator drawn on it is shared whole: a few MB at most.
const MAX_BYTES: u64 = 16 << 20;

/// Where an image comes from.
enum Source {
    /// Downloaded, and kept in a file when one is given.
    Web { url: String, keep: Option<PathBuf> },
    File(PathBuf),
}

enum Slot {
    Loading(Receiver<Option<ColorImage>>),
    Ready(TextureHandle),
    /// None there, or out of reach: asked again at the next start.
    Missing,
}

/// The images asked for, by URL or path.
#[derive(Default)]
pub struct Images(HashMap<String, Slot>);

impl Images {
    /// The banner of the Steam app `app`, once it is loaded.
    pub(super) fn banner(&mut self, ctx: &egui::Context, app: u32) -> Option<TextureHandle> {
        let url = format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{app}/header.jpg");
        let keep = config::data_dir().join("banners").join(format!("{app}.jpg"));
        self.get(ctx, Source::Web { url, keep: Some(keep) })
    }

    /// The image at `url`, once it is loaded (none when there is none).
    pub(super) fn web(&mut self, ctx: &egui::Context, url: &str) -> Option<TextureHandle> {
        self.get(ctx, Source::Web { url: url.to_owned(), keep: None })
    }

    /// The image in the file `path`, once it is loaded.
    pub(super) fn file(&mut self, ctx: &egui::Context, path: &Path) -> Option<TextureHandle> {
        self.get(ctx, Source::File(path.to_owned()))
    }

    fn get(&mut self, ctx: &egui::Context, source: Source) -> Option<TextureHandle> {
        let key = match &source {
            Source::Web { url, .. } => url.clone(),
            Source::File(path) => path.to_string_lossy().into_owned(),
        };
        let slot = self.0.entry(key.clone()).or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let image = load(&source).inspect_err(|e| log::debug!("image {key}: {e:#}")).ok();
                let _ = tx.send(image);
                ctx.request_repaint();
            });
            Slot::Loading(rx)
        });
        if let Slot::Loading(rx) = slot {
            match rx.try_recv() {
                Ok(Some(image)) => *slot = Slot::Ready(ctx.load_texture("image", image, TextureOptions::LINEAR)),
                Ok(None) | Err(TryRecvError::Disconnected) => *slot = Slot::Missing,
                Err(TryRecvError::Empty) => {}
            }
        }
        match slot {
            Slot::Ready(texture) => Some(texture.clone()),
            _ => None,
        }
    }

    /// The banner of the game `name`, `width` wide; its initial while there is none.
    pub(super) fn show_banner(&mut self, ui: &mut egui::Ui, app: Option<u32>, name: &str, width: f32) -> egui::Response {
        let size = Vec2::new(width, (width / BANNER_RATIO).round());
        let corners = CornerRadius::same(8);
        if let Some(texture) = app.and_then(|app| self.banner(ui.ctx(), app)) {
            return ui.add(egui::Image::new(&texture).fit_to_exact_size(size).corner_radius(corners));
        }
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, corners, RAISED);
        let initial = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        painter.text(rect.center(), Align2::CENTER_CENTER, initial, FontId::proportional(size.y * 0.4), MUTED);
        response
    }
}

/// The image kept on disk, else downloaded (and kept).
fn load(source: &Source) -> anyhow::Result<ColorImage> {
    let (url, keep) = match source {
        Source::File(path) => return decode(&std::fs::read(path)?),
        Source::Web { url, keep } => (url, keep),
    };
    if let Some(image) = keep.as_ref().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| decode(&bytes).ok()) {
        return Ok(image);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let mut bytes = Vec::new();
    agent.get(url).call().with_context(|| format!("downloading {url}"))?.body_mut().as_reader().take(MAX_BYTES).read_to_end(&mut bytes)?;
    let image = decode(&bytes)?;
    if let Some(path) = keep {
        let written = path.parent().map_or(Ok(()), config::create_dir).and_then(|()| std::fs::write(path, &bytes));
        if let Err(e) = written {
            log::debug!("cannot keep {}: {e:#}", path.display());
        }
    }
    Ok(image)
}

fn decode(bytes: &[u8]) -> anyhow::Result<ColorImage> {
    let mut image = image::load_from_memory(bytes)?;
    if image.width() > MAX_WIDTH {
        image = image.thumbnail(MAX_WIDTH, u32::MAX);
    }
    let image = image.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()))
}
