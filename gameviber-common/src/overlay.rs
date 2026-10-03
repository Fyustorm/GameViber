//! Protocol between GameViber and the in-game overlay (gameviber-overlay).
//!
//! GameViber binds a datagram socket in the abstract namespace, which games
//! reach even from the Steam Runtime or Flatpak containers since they share
//! the network namespace. Each overlay instance (one per game process) sends
//! a JSON [`Hello`] every second; GameViber answers with the JSON
//! [`OverlayState`] about 25 times per second and forgets overlays it has
//! not heard from for a few seconds.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
/// Overlays send a hello this often.
pub const HELLO_SECS: f64 = 1.0;
/// GameViber forgets an overlay after this long without a hello.
pub const CLIENT_TIMEOUT_SECS: f64 = 3.0;
/// The overlay hides itself after this long without a state.
pub const STATE_TIMEOUT_SECS: f64 = 2.0;
/// Datagrams never exceed this size.
pub const MAX_DATAGRAM: usize = 16 * 1024;

/// Abstract socket name GameViber listens on, for the user `uid`.
pub fn server_name(uid: u32) -> String {
    format!("gameviber-overlay-{uid}")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hello {
    pub version: u32,
    pub pid: u32,
    /// Executable name of the game process.
    pub exe: String,
    /// Graphics API the overlay draws with ("vulkan", "opengl").
    pub api: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 4] = [Corner::TopLeft, Corner::TopRight, Corner::BottomLeft, Corner::BottomRight];

    pub fn label(self) -> &'static str {
        match self {
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }
}

/// A gauge a mode shows with `hud(label, value, max)`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Gauge {
    pub label: String,
    pub value: f32,
    pub max: f32,
}

/// A short message a mode shows with `hud_event(text)`, fading out with age.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Event {
    pub text: String,
    /// Seconds since it was raised.
    pub age: f32,
}

/// Everything the overlay draws. GameViber computes it; the overlay only lays it out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlayState {
    pub version: u32,
    pub visible: bool,
    pub corner: Corner,
    /// Size multiplier (1 = default size at 1080p).
    pub scale: f32,
    /// Background opacity, 0..1.
    pub opacity: f32,
    /// Strongest toy output after the safety layer, 0..1.
    pub output: f32,
    /// Global intensity cap, 0..1.
    pub cap: f32,
    /// The panic stop is engaged.
    pub panic: bool,
    pub mode: String,
    pub preset: Option<String>,
    /// Seconds since the mode or preset changed (the overlay shows them larger for a while).
    pub mode_age: f32,
    pub gauges: Vec<Gauge>,
    pub events: Vec<Event>,
    /// Problems the player should know about (toy lost, mode error...).
    pub alerts: Vec<String>,
}

impl Default for OverlayState {
    fn default() -> Self {
        Self {
            version: PROTOCOL_VERSION,
            visible: true,
            corner: Corner::default(),
            scale: 1.0,
            opacity: 0.75,
            output: 0.0,
            cap: 1.0,
            panic: false,
            mode: String::new(),
            preset: None,
            // JSON has no infinity.
            mode_age: 1e6,
            gauges: Vec::new(),
            events: Vec::new(),
            alerts: Vec::new(),
        }
    }
}
