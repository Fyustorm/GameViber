//! GameViber: intercepts the rumble games send to the gamepad, runs it
//! through a scriptable Lua mode and drives toys through Intiface Central.

// Without an OS backend (see `platform`), what only backends use is left unused.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
// Windows: no console window behind the GUI in release builds (logs: Creator › Logs).
#![cfg_attr(all(target_os = "windows", not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod community;
mod config;
mod engine;
mod funscript;
mod external;
mod gamepad;
mod gui;
mod intiface;
mod links;
mod logging;
mod models;
mod mode;
mod overlay;
mod platform;
mod game;
mod package;
mod rumble;
mod screen;
mod session;
mod sharing;
mod stroke;
mod shortcuts;
mod source;
mod update;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use clap::Parser;
use tokio::sync::mpsc;

use crate::config::{ModeEntry, BUILTIN_PREFIX};
use crate::engine::{EngineOptions, Shared, SourceChoice};

#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Run without the GUI (logs only)
    #[arg(long)]
    headless: bool,
    /// Rumble interception method (default: the saved choice, proxy at first launch)
    #[arg(long, value_enum)]
    source: Option<SourceChoice>,
    /// Gamepad to use (/dev/input/eventX). Proxy: auto-detected; ebpf: every gamepad if absent
    #[arg(long)]
    device: Option<PathBuf>,
    /// Proxy: hide the real gamepad from games (asks for authorization)
    #[arg(long)]
    hide: bool,
    /// Proxy: do not forward the rumble to the real gamepad
    #[arg(long)]
    no_passthrough: bool,
    /// Intiface server URL (default: from settings, ws://127.0.0.1:12345)
    #[arg(long)]
    url: Option<String>,
    /// Do not connect to Intiface
    #[arg(long)]
    no_intiface: bool,
    /// Mode to activate: a .luau file or a built-in name (simple, accumulation, combo, overheat, tension,
    /// engine, heartbeat, all_or_nothing, ambient)
    #[arg(long)]
    mode: Option<String>,
    /// Named preset of the mode to load at startup
    #[arg(long)]
    preset: Option<String>,
    /// Verbose logs (effects, buttons)
    #[arg(short, long)]
    verbose: bool,
    /// A gameviber:// link to open (the website's "Open in GameViber"); handed to
    /// the GameViber already running, if one is
    link: Option<String>,
}

fn mode_id(arg: &str) -> String {
    if arg.starts_with(BUILTIN_PREFIX) {
        return arg.to_owned();
    }
    let path = PathBuf::from(arg);
    if path.exists() {
        return std::fs::canonicalize(&path).unwrap_or(path).to_string_lossy().into_owned();
    }
    let builtin = format!("{BUILTIN_PREFIX}{arg}");
    if ModeEntry::from_id(&builtin).source().is_ok() {
        builtin
    } else {
        arg.to_owned()
    }
}

fn main() -> anyhow::Result<()> {
    if let Some(result) = platform::privileged_subcommand() {
        return result;
    }
    let args = Args::parse();
    let logs = logging::init(args.verbose);
    // One GameViber at a time: a second start hands its link to the first one.
    let instance = if args.headless {
        None
    } else {
        match links::claim(args.link.as_deref()) {
            links::Claim::First(instance) => Some(instance),
            links::Claim::Forwarded => return Ok(()),
        }
    };
    let link = args.link.as_deref().and_then(|text| {
        let link = links::Link::parse(text);
        if link.is_none() {
            log::warn!("not a GameViber link: {text}");
        }
        link
    });
    package::migrate();
    let opts = EngineOptions {
        source: args.source,
        device: args.device,
        hide: args.hide,
        passthrough: !args.no_passthrough,
        url: args.url,
        intiface: !args.no_intiface,
        mode: args.mode.as_deref().map(mode_id),
        preset: args.preset,
    };
    let shared = Arc::new(Mutex::new(Shared::default()));
    let (commands, commands_rx) = mpsc::unbounded_channel();

    if args.headless {
        return engine::run(opts, shared, commands_rx);
    }

    platform::window_expected();
    let engine = {
        let shared = shared.clone();
        std::thread::Builder::new().name("engine".into()).spawn(move || {
            if let Err(e) = engine::run(opts, shared, commands_rx) {
                log::error!("engine stopped: {e:#}");
            }
        })?
    };
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_title("GameViber").with_icon(eframe::icon_data::from_png_bytes(gui::ICON).unwrap_or_default()).with_app_id(shortcuts::APP_ID).with_inner_size([1180.0, 760.0]).with_min_inner_size([1180.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "GameViber",
        native,
        Box::new(move |cc| {
            platform::window_created(cc);
            Ok(Box::new(gui::App::new(&cc.egui_ctx, shared, logs, commands, engine, instance, link)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))?;
    // After an update, the engine has stopped with the GUI: start the new version.
    update::restart_if_requested();
    Ok(())
}
