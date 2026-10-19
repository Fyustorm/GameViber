//! Games' banners, as the website shows them: the header image of their Steam
//! store page, for the games whose Steam app id is known. Downloaded once in a
//! thread, then kept in GameViber's data directory.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use anyhow::Context;
use eframe::egui::{self, Align2, ColorImage, CornerRadius, FontId, TextureHandle, TextureOptions, Vec2};

use super::theme::*;
use crate::config;

/// Width over height of Steam's header images (460 × 215).
pub(super) const RATIO: f32 = 460.0 / 215.0;
/// A header weighs some 50 kB: anything much larger is not one.
const MAX_BYTES: u64 = 4 << 20;

enum Banner {
    Loading(Receiver<Option<ColorImage>>),
    Ready(TextureHandle),
    /// No header on Steam, or out of reach: asked again at the next start.
    Missing,
}

/// The banners asked for, by Steam app id.
#[derive(Default)]
pub struct Banners(HashMap<u32, Banner>);

impl Banners {
    /// The banner of the Steam app `app`, once it is loaded.
    pub(super) fn get(&mut self, ctx: &egui::Context, app: u32) -> Option<TextureHandle> {
        let banner = self.0.entry(app).or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let image = load(app).inspect_err(|e| log::debug!("Steam banner of app {app}: {e:#}")).ok();
                let _ = tx.send(image);
                ctx.request_repaint();
            });
            Banner::Loading(rx)
        });
        if let Banner::Loading(rx) = banner {
            match rx.try_recv() {
                Ok(Some(image)) => *banner = Banner::Ready(ctx.load_texture(format!("steam-banner-{app}"), image, TextureOptions::LINEAR)),
                Ok(None) | Err(TryRecvError::Disconnected) => *banner = Banner::Missing,
                Err(TryRecvError::Empty) => {}
            }
        }
        match banner {
            Banner::Ready(texture) => Some(texture.clone()),
            _ => None,
        }
    }

    /// The banner of the game `name`, `width` wide; its initial while there is none.
    pub(super) fn show(&mut self, ui: &mut egui::Ui, app: Option<u32>, name: &str, width: f32) -> egui::Response {
        let size = Vec2::new(width, (width / RATIO).round());
        let corners = CornerRadius::same(8);
        if let Some(texture) = app.and_then(|app| self.get(ui.ctx(), app)) {
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

fn dir() -> PathBuf {
    config::data_dir().join("banners")
}

/// The banner kept on disk, else downloaded from Steam and kept.
fn load(app: u32) -> anyhow::Result<ColorImage> {
    let path = dir().join(format!("{app}.jpg"));
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(image) = decode(&bytes) {
            return Ok(image);
        }
    }
    let url = format!("https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps/{app}/header.jpg");
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
    let mut bytes = Vec::new();
    agent.get(&url).call().with_context(|| format!("downloading {url}"))?.body_mut().as_reader().take(MAX_BYTES).read_to_end(&mut bytes)?;
    let image = decode(&bytes)?;
    if let Err(e) = config::create_dir(&dir()).and_then(|()| std::fs::write(&path, &bytes)) {
        log::debug!("cannot keep the Steam banner of app {app}: {e:#}");
    }
    Ok(image)
}

fn decode(bytes: &[u8]) -> anyhow::Result<ColorImage> {
    let image = image::load_from_memory(bytes)?.to_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied([image.width() as usize, image.height() as usize], image.as_raw()))
}
