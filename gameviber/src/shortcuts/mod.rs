//! Keyboard shortcuts for the gamepad combos' actions (stop, mark a moment,
//! capture the screen), through the desktop (`linux`: its global shortcuts
//! portal).

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::Shortcuts;
#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::Shortcuts;

/// The id the desktop knows GameViber by (Linux: its desktop entry, which the
/// shortcuts portal needs, else it files them under the terminal GameViber started from).
pub const APP_ID: &str = "io.github.gameviber.GameViber";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Panic,
    Mark,
    Capture,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Panic, Action::Mark, Action::Capture];

    fn id(self) -> &'static str {
        match self {
            Action::Panic => "panic",
            Action::Mark => "mark",
            Action::Capture => "capture",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Action::Panic => "GameViber: stop every toy",
            Action::Mark => "GameViber: mark a moment that felt wrong",
            Action::Capture => "GameViber: capture the game's screen",
        }
    }

    /// Keys suggested to the desktop (it may pick others).
    fn preferred(self) -> &'static str {
        match self {
            Action::Panic => "CTRL+ALT+X",
            Action::Mark => "CTRL+ALT+M",
            Action::Capture => "CTRL+ALT+C",
        }
    }
}

/// The shortcuts as the GUI shows them.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Status {
    #[default]
    Off,
    Connecting,
    /// Bound: the keys of each action, as the desktop describes them ("" if none).
    Ready(Vec<(Action, String)>),
    Failed(String),
}
