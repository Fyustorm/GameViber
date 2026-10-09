//! Windows sources: only the "proxy" method, a virtual Xbox 360 controller
//! made with ViGEmBus; games keep their gamepad otherwise (no kernel probe).

mod hid;
mod proxy;

use super::{ActiveSource, EventSender, HideOption, Method, SourceOptions};
use crate::config::SourceChoice;
use proxy::ProxySource;

/// SDL_GameControllerDB's Windows mappings (`gamepad::mapping`).
pub const MAPPINGS: &str = include_str!("../../../gamepads/gamecontrollerdb-windows.txt");
/// Written at the end of the mapping lines GameViber saves.
pub const MAPPING_PLATFORM: &str = "Windows";

pub const METHODS: &[Method] = &[Method {
    choice: SourceChoice::Proxy,
    name: "Standard",
    badge: "Recommended",
    summary: "GameViber shows games a virtual Xbox 360 controller (ViGEmBus) and listens to what they send it.",
    pros: &["Works with nearly every game", "Your Xbox controller can still vibrate too", "Other gamepads get the Xbox layout"],
    cons: &["Needs ViGEmBus (the installer offers it)", "Start GameViber before the game", "Games may see two controllers: hide the real one below"],
}];

pub const HIDE: HideOption = HideOption {
    label: "Hide the real gamepad from games (HidHide)",
    hover: "Games only see the virtual controller. Needs HidHide (the installer offers it); Windows asks you to allow it",
};

pub struct Sources;

impl Sources {
    pub fn new() -> Self {
        Self
    }

    /// Must be called from within a tokio runtime. `SourceChoice::None` is not a source.
    pub fn start(&self, choice: SourceChoice, opts: &SourceOptions, tx: EventSender) -> anyhow::Result<Box<dyn ActiveSource>> {
        match choice {
            SourceChoice::Proxy => Ok(Box::new(ProxySource::start(opts, tx)?)),
            SourceChoice::Ebpf => anyhow::bail!("the kernel probe only exists on Linux: pick the standard method"),
            SourceChoice::None => anyhow::bail!("no source selected"),
        }
    }

    /// Gives back a gamepad a GameViber that stopped badly left hidden.
    pub fn shutdown(&self) {
        let left = crate::platform::windows::hidhide::left_hidden();
        if !left.is_empty() {
            if let Err(e) = crate::platform::windows::hidhide::unhide(&left) {
                log::warn!("cannot give the hidden gamepad back to games: {e:#}");
            }
        }
    }
}
