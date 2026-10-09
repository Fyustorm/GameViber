//! `gameviber://` links, the website's "Open in GameViber": the desktop
//! starts GameViber with the link (`linux`: its desktop entry,
//! `platform::linux::desktop`), and one instance runs at a time. A second
//! start hands its link (or nothing: show yourself) to the first one and quits,
//! rather than run a second engine on the same gamepad.
//!
//! - `gameviber://mode/<id>`: a public mode of the community, by its id;
//! - `gameviber://shared/<code>`: a mode by the code its author shares;
//! - `gameviber://`: GameViber itself.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{claim, Instance};
#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::{claim, Instance};

pub const SCHEME: &str = "gameviber";
/// Longest link taken: ids and codes are short.
const MAX_LINK: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// Only bring GameViber to the front.
    Open,
    /// A public mode, by its id on the community server.
    Mode(String),
    /// A mode by its share code.
    Shared(String),
}

impl Link {
    /// `gameviber://...`, as the website writes it; None for anything else.
    pub fn parse(text: &str) -> Option<Link> {
        let text = text.trim();
        if text.len() > MAX_LINK {
            return None;
        }
        let (scheme, rest) = text.split_once(':')?;
        if !scheme.eq_ignore_ascii_case(SCHEME) {
            return None;
        }
        let rest = rest.trim_start_matches('/');
        let rest = rest.split(['?', '#']).next().unwrap_or_default();
        let parts: Vec<&str> = rest.split('/').filter(|p| !p.is_empty()).collect();
        match parts.as_slice() {
            [] => Some(Link::Open),
            ["mode", id] if valid(id) => Some(Link::Mode(id.to_ascii_lowercase())),
            ["shared", code] if valid(code) => Some(Link::Shared(code.to_ascii_uppercase())),
            _ => None,
        }
    }
}

/// Ids and codes are a few letters, digits and dashes.
fn valid(part: &str) -> bool {
    (1..=32).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// How this start went.
pub enum Claim {
    /// No other GameViber runs: this one does, and hears the next starts' links.
    First(Instance),
    /// Another one runs, and was handed the link.
    Forwarded,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_are_read_as_the_website_writes_them() {
        assert_eq!(Link::parse("gameviber://mode/k3x9m2q7ab"), Some(Link::Mode("k3x9m2q7ab".into())));
        assert_eq!(Link::parse("GameViber://mode/K3X9M2Q7AB/"), Some(Link::Mode("k3x9m2q7ab".into())));
        assert_eq!(Link::parse("gameviber://shared/gv-7kq2-m9xd"), Some(Link::Shared("GV-7KQ2-M9XD".into())));
        assert_eq!(Link::parse("gameviber:shared/GV-7KQ2-M9XD?from=site#top"), Some(Link::Shared("GV-7KQ2-M9XD".into())));
        assert_eq!(Link::parse(" gameviber:// "), Some(Link::Open));
        assert_eq!(Link::parse("gameviber:"), Some(Link::Open));
    }

    #[test]
    fn anything_else_is_refused() {
        for text in [
            "",
            "https://gameviber.fyustorm.ovh/modes/k3x9m2q7ab",
            "gameviber://mode",
            "gameviber://mode/a/b",
            "gameviber://delete/k3x9m2q7ab",
            "gameviber://mode/../etc",
            "gameviber://mode/a%2Fb",
            "gameviber://mode/abc def",
            &format!("gameviber://mode/{}", "a".repeat(33)),
            &format!("gameviber://{}", "/".repeat(300)),
        ] {
            assert_eq!(Link::parse(text), None, "{text}");
        }
    }
}
