//! Luau syntax highlighting for the Creator's editor: keywords, strings,
//! comments, numbers, and the mode API (functions, callbacks, `P`, `input`)
//! so that what GameViber provides stands out from the script's own names.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontId};

use super::theme::*;

const KEYWORD: Color32 = ACCENT_TEXT;
const STRING: Color32 = Color32::from_rgb(0x9f, 0xd6, 0x8c);
const NUMBER: Color32 = Color32::from_rgb(0xe6, 0xb6, 0x73);
const COMMENT: Color32 = Color32::from_rgb(0x74, 0x7c, 0x8c);
const API: Color32 = GAME;
const CALLBACK: Color32 = Color32::from_rgb(0xc7, 0x9b, 0xf2);
/// Background of the line an error points to.
const ERROR_LINE: Color32 = Color32::from_rgb(0x4a, 0x1f, 0x24);

const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "else", "elseif", "end", "export", "for", "function", "if", "in", "local",
    "not", "or", "repeat", "return", "then", "type", "until", "while",
];
const CONSTANTS: &[&str] = &["true", "false", "nil"];
/// Globals of the mode API (docs/spec-modes.md) and of the Luau sandbox.
const GLOBALS: &[&str] = &[
    "mode", "number", "bool", "choice", "button_param", "set", "pulse", "play", "stop_all", "pattern", "hud",
    "hud_event", "plot", "log", "print", "after", "every", "clamp", "lerp", "map", "random", "P", "input", "persist",
    "math", "string", "table", "bit32", "utf8", "tostring", "tonumber", "pairs", "ipairs", "type", "typeof",
    "select", "error", "assert", "pcall", "setmetatable", "getmetatable", "rawget", "rawset", "unpack",
];
const CALLBACKS: &[&str] = &[
    "tick", "on_start", "on_stop", "on_rumble", "on_rumble_start", "on_rumble_end", "on_button",
    "on_param_changed", "on_device",
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Plain,
    Keyword,
    Constant,
    String,
    Number,
    Comment,
    Api,
    Callback,
}

/// Splits `text` into (byte range, kind) covering all of it.
fn tokens(text: &str) -> Vec<(std::ops::Range<usize>, Kind)> {
    let bytes = text.as_bytes();
    let mut out: Vec<(std::ops::Range<usize>, Kind)> = Vec::new();
    let mut i = 0;
    // Last non-blank byte, to tell a field (`input.time`) from a global.
    let mut previous = b' ';
    while i < bytes.len() {
        let start = i;
        let c = bytes[i];
        let kind = if c == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i += 2;
            match long_bracket(bytes, i) {
                Some(level) => i = close_long_bracket(bytes, i, level),
                None => i = line_end(bytes, i),
            }
            Kind::Comment
        } else if c == b'[' && long_bracket(bytes, i).is_some() {
            i = close_long_bracket(bytes, i, long_bracket(bytes, i).unwrap());
            Kind::String
        } else if c == b'"' || c == b'\'' || c == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != c && bytes[i] != b'\n' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(bytes.len());
            Kind::String
        } else if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            while i < bytes.len() {
                let b = bytes[i];
                let exponent_sign = (b == b'+' || b == b'-') && matches!(bytes[i - 1], b'e' | b'E');
                if !(b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || exponent_sign) {
                    break;
                }
                i += 1;
            }
            Kind::Number
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &text[start..i];
            if KEYWORDS.contains(&word) {
                Kind::Keyword
            } else if CONSTANTS.contains(&word) {
                Kind::Constant
            } else if previous == b'.' || previous == b':' {
                Kind::Plain
            } else if CALLBACKS.contains(&word) {
                Kind::Callback
            } else if GLOBALS.contains(&word) {
                Kind::Api
            } else {
                Kind::Plain
            }
        } else {
            i += text[i..].chars().next().map_or(1, char::len_utf8);
            Kind::Plain
        };
        if !text[start..i].trim().is_empty() {
            previous = text.as_bytes()[i - 1];
        }
        match out.last_mut() {
            Some((range, last)) if *last == kind && kind == Kind::Plain => range.end = i,
            _ => out.push((start..i, kind)),
        }
    }
    out
}

/// Level of a long bracket (`[[` = 0, `[==[` = 2) opening at `i`.
fn long_bracket(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'[') {
        return None;
    }
    let level = bytes[i + 1..].iter().take_while(|b| **b == b'=').count();
    (bytes.get(i + 1 + level) == Some(&b'[')).then_some(level)
}

/// End of the long bracket of `level` opened at `i` (or of the text).
fn close_long_bracket(bytes: &[u8], i: usize, level: usize) -> usize {
    let close: Vec<u8> = std::iter::once(b']').chain(std::iter::repeat_n(b'=', level)).chain([b']']).collect();
    let body = i + level + 2;
    bytes[body.min(bytes.len())..]
        .windows(close.len())
        .position(|w| w == close.as_slice())
        .map_or(bytes.len(), |p| body + p + close.len())
}

fn line_end(bytes: &[u8], i: usize) -> usize {
    bytes[i..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |p| i + p)
}

/// Highlighted layout of a mode's source; `error_line` (1-based) gets a red background.
pub fn highlight(text: &str, font: FontId, error_line: Option<usize>) -> LayoutJob {
    let mut job = LayoutJob::default();
    let error_range = error_line.and_then(|line| {
        let start = if line == 1 { 0 } else { text.match_indices('\n').nth(line - 2)?.0 + 1 };
        Some(start..line_end(text.as_bytes(), start))
    });
    for (range, kind) in tokens(text) {
        let color = match kind {
            Kind::Plain => TEXT,
            Kind::Keyword => KEYWORD,
            Kind::Constant | Kind::Number => NUMBER,
            Kind::String => STRING,
            Kind::Comment => COMMENT,
            Kind::Api => API,
            Kind::Callback => CALLBACK,
        };
        // Tokens are cut at the error line's edges so only that line is tinted.
        let mut cuts = vec![range.start, range.end];
        if let Some(e) = &error_range {
            cuts.extend([e.start, e.end].into_iter().filter(|c| range.contains(c) && *c > range.start));
        }
        cuts.sort_unstable();
        cuts.dedup();
        for part in cuts.windows(2) {
            let in_error = error_range.as_ref().is_some_and(|e| part[0] >= e.start && part[1] <= e.end);
            let format = TextFormat {
                font_id: font.clone(),
                color,
                background: if in_error { ERROR_LINE } else { Color32::TRANSPARENT },
                italics: kind == Kind::Comment,
                ..Default::default()
            };
            job.append(&text[part[0]..part[1]], 0.0, format);
        }
    }
    job
}

/// Line (1-based) of the first `<chunk>:<line>:` location in an error message.
pub fn error_line(error: &str, chunk: &str) -> Option<usize> {
    let pattern = format!("{chunk}:");
    error.match_indices(&pattern).find_map(|(at, _)| {
        let rest = &error[at + pattern.len()..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        rest[digits.len()..].starts_with(':').then(|| digits.parse().ok()).flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, Kind)> {
        tokens(text).into_iter().filter(|(_, k)| *k != Kind::Plain).map(|(r, k)| (&text[r], k)).collect()
    }

    #[test]
    fn tokens_cover_the_whole_text() {
        let text = "local x = 1 -- é\nfunction tick(dt, input) set(\"a\\\"b\") end";
        let joined: String = tokens(text).into_iter().map(|(r, _)| &text[r]).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn mode_api_and_lua_syntax_are_recognised() {
        assert_eq!(
            kinds("local v = input.time + P.gain * 0.5e-2 -- note\nfunction tick(dt) set(v, 'main') end"),
            [
                ("local", Kind::Keyword),
                ("input", Kind::Api),
                ("P", Kind::Api),
                ("0.5e-2", Kind::Number),
                ("-- note", Kind::Comment),
                ("function", Kind::Keyword),
                ("tick", Kind::Callback),
                ("set", Kind::Api),
                ("'main'", Kind::String),
                ("end", Kind::Keyword),
            ]
        );
        // A field named like an API function is not one.
        assert_eq!(kinds("t.set = nil"), [("nil", Kind::Constant)]);
    }

    #[test]
    fn long_brackets() {
        assert_eq!(kinds("--[[ a\nb ]] x = [==[ s ]] ]==]"), [("--[[ a\nb ]]", Kind::Comment), ("[==[ s ]] ]==]", Kind::String)]);
        assert_eq!(kinds("--[[ open"), [("--[[ open", Kind::Comment)]);
    }

    #[test]
    fn error_lines_are_found() {
        assert_eq!(error_line("combo.luau:12: attempt to call a nil value", "combo.luau"), Some(12));
        assert_eq!(error_line("reload failed: x.luau:3: oops", "x.luau"), Some(3));
        assert_eq!(error_line("other.luau:3: oops", "x.luau"), None);
        assert_eq!(error_line("mode { ... } was never called", "x.luau"), None);
    }

    #[test]
    fn error_line_is_tinted() {
        let job = highlight("a = 1\nb = 2\nc = 3", FontId::monospace(12.0), Some(2));
        let tinted: String = job
            .sections
            .iter()
            .filter(|s| s.format.background == ERROR_LINE)
            .map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0])
            .collect();
        assert_eq!(tinted, "b = 2");
    }
}
