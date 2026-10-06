//! Requests for an AI assistant (ChatGPT, Claude...) to write a mode for one
//! game or to fix a mode that does not feel right, and extraction of the
//! script from its answer. The request templates ship with GameViber; players
//! can override them with files in `~/.config/gameviber/prompts/`.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use super::{ParamDef, ParamValue};
use crate::config;

const SPEC: &str = include_str!("../../../docs/spec-modes.md");
/// Lines between these markers are left out of a short fix request, sent in the
/// conversation that already has them.
const FULL_START: &str = "<!-- full -->";
const FULL_END: &str = "<!-- /full -->";
/// Spec sections left out of the requests: they are about GameViber itself
/// (architecture, presets, reloading, safety, plans, built-in modes), not about
/// writing a mode.
const SPEC_LEFT_OUT: [&str; 7] = [
    "## 1. ",
    "## 2. ",
    "### 4.1 ",
    "## 11. ",
    "## 12. ",
    "## 13. ",
    "## 14. ",
];
/// Left out of quick requests too: the raw sound and image, the game's
/// profile and the advanced inputs (`Depth::Quick`).
const SPEC_ADVANCED: [&str; 3] = ["### 6.4 ", "### 6.5 ", "### 7.1 "];

/// How much of GameViber a request shows the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    /// The rumble, the buttons and what GameViber makes of the sound and the
    /// image (phases, impacts, intensity): a mode in a couple of minutes.
    #[default]
    Quick,
    /// Also the raw sound and image measures, the indicators of the screen, the
    /// example images and values other programs send (the game's profile).
    Advanced,
}

/// What the inputs set up for the mode bring to a request: their description
/// (`Inputs::describe`), if any; a quick request only gets the phases.
fn profile_text(depth: Depth, profile: Option<&str>, scenes: &[String]) -> String {
    match (depth, profile.map(str::trim).filter(|p| !p.is_empty())) {
        (Depth::Quick, _) if !scenes.is_empty() => {
            let names: Vec<String> = scenes.iter().map(|s| format!("`{s}`")).collect();
            format!(
                "GameViber already recognizes this game's phases: {}. Read them with `input.phase` and \
                 `on_phase`, and do not declare `phases` in the mode. Otherwise use the rumble, the buttons and the \
                 high-level inputs (impacts, intensity).",
                names.join(", ")
            )
        }
        (Depth::Quick, _) => "Nothing more: use the rumble, the buttons and the high-level inputs (phases, impacts, \
             intensity)."
            .to_owned(),
        (Depth::Advanced, Some(profile)) => format!(
            "The player set up inputs for this game in GameViber (§6.5). Use what helps:\n\n{profile}"
        ),
        (Depth::Advanced, None) => "The player has not set up anything for this game yet. If an indicator of the screen \
             would help (an interface shown only in battles, a health bar), tell the player which indicators to draw in \
             the mode's Inputs: a name, its kind (visibility or gauge) and where to draw it. Read them defensively: \
             `input.indicators.<name>` is nil until the indicator exists."
            .to_owned(),
    }
}

/// A request template, as shipped or as overridden by the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// A mode for one game.
    NewMode,
    /// A fix for a mode that does not feel right.
    FixFeel,
    /// What makes a mode feel good and the script rules, shared by both.
    Rules,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::NewMode, Template::FixFeel, Template::Rules];

    pub fn title(self) -> &'static str {
        match self {
            Template::NewMode => "A mode for a game",
            Template::FixFeel => "Fix a mode",
            Template::Rules => "Shared rules",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Template::NewMode => "new-mode.md",
            Template::FixFeel => "fix-feel.md",
            Template::Rules => "rules.md",
        }
    }

    pub fn builtin(self) -> &'static str {
        match self {
            Template::NewMode => include_str!("../../prompts/new-mode.md"),
            Template::FixFeel => include_str!("../../prompts/fix-feel.md"),
            Template::Rules => include_str!("../../prompts/rules.md"),
        }
    }

    /// Placeholders the template should keep, so the request holds what it needs.
    pub fn placeholders(self) -> &'static [&'static str] {
        match self {
            Template::NewMode => &["{{GAME}}", "{{LANGUAGE}}", "{{PROFILE}}", "{{RULES}}", "{{SPEC}}"],
            Template::FixFeel => &[
                "{{GAME}}", "{{LANGUAGE}}", "{{PROBLEMS}}", "{{HISTORY}}", "{{NAME}}", "{{PARAMS}}",
                "{{SOURCE}}", "{{SESSION}}", "{{PROFILE}}", "{{RULES}}", "{{SPEC}}",
            ],
            Template::Rules => &[],
        }
    }

    /// The player's version of the template.
    pub fn path(self) -> PathBuf {
        config::config_dir().join("prompts").join(self.file_name())
    }

    /// The player's version, or the shipped one.
    pub fn text(self) -> String {
        std::fs::read_to_string(self.path()).unwrap_or_else(|_| self.builtin().to_owned())
    }

    pub fn customized(self) -> bool {
        self.path().exists()
    }

    /// Saves the player's version; the shipped text removes it.
    pub fn save(self, text: &str) -> io::Result<()> {
        if text == self.builtin() {
            return self.reset();
        }
        config::write_file(&self.path(), text)
    }

    pub fn reset(self) -> io::Result<()> {
        match std::fs::remove_file(self.path()) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

/// The templates a request is built from.
pub struct Templates {
    pub new_mode: String,
    pub fix_feel: String,
    pub rules: String,
}

impl Templates {
    /// The player's templates (the shipped ones where they changed nothing).
    pub fn load() -> Self {
        Self { new_mode: Template::NewMode.text(), fix_feel: Template::FixFeel.text(), rules: Template::Rules.text() }
    }

    #[cfg(test)]
    pub fn builtin() -> Self {
        Self {
            new_mode: Template::NewMode.builtin().to_owned(),
            fix_feel: Template::FixFeel.builtin().to_owned(),
            rules: Template::Rules.builtin().to_owned(),
        }
    }
}

/// The request to paste into an AI assistant to get a mode made for `game`,
/// answered in `language`; `profile` describes the game's profile (advanced requests).
pub fn new_mode_prompt(t: &Templates, game: &str, language: &str, depth: Depth, profile: Option<&str>, scenes: &[String]) -> String {
    sections(&t.new_mode, true)
        .replace("{{RULES}}", t.rules.trim_end())
        .replace("{{PROFILE}}", &profile_text(depth, profile, scenes))
        .replace("{{SPEC}}", &spec(depth))
        .replace("{{LANGUAGE}}", language)
        .replace("{{GAME}}", game.trim())
}

/// `text` with the `FULL_START`..`FULL_END` blocks kept (without their markers)
/// or left out.
fn sections(text: &str, full: bool) -> String {
    let mut out = String::new();
    let mut in_full = false;
    for line in text.lines() {
        match line.trim() {
            FULL_START => in_full = true,
            FULL_END => in_full = false,
            _ if full || !in_full => {
                out.push_str(line);
                out.push('\n');
            }
            _ => {}
        }
    }
    // Blocks left out leave blank lines behind.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

/// The specification without the sections of `SPEC_LEFT_OUT` (and `SPEC_ADVANCED` for a quick request).
fn spec(depth: Depth) -> String {
    let advanced: &[&str] = if depth == Depth::Quick { &SPEC_ADVANCED } else { &[] };
    let mut out = String::new();
    // Heading level of the section being skipped.
    let mut skipping: Option<usize> = None;
    let mut in_code = false;
    for line in SPEC.lines() {
        if line.starts_with("```") {
            in_code = !in_code;
        }
        let level = (!in_code && line.starts_with('#')).then(|| line.chars().take_while(|c| *c == '#').count());
        if let Some(level) = level {
            if skipping.is_some_and(|skipped| level <= skipped) {
                skipping = None;
            }
            if skipping.is_none() && SPEC_LEFT_OUT.iter().chain(advanced).any(|s| line.starts_with(s)) {
                skipping = Some(level);
            }
        }
        if skipping.is_none() {
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end().to_owned()
}

/// A mode the player finds wrong, and what they say about it.
pub struct FeelReport<'a> {
    pub name: &'a str,
    pub game: &'a str,
    /// Problems ticked in the GUI, then the player's own words.
    pub problems: &'a [String],
    /// Earlier rounds of fixing this mode (`FeedbackHistory::lines`).
    pub history: &'a [String],
    pub params: &'a [ParamDef],
    pub values: &'a BTreeMap<String, ParamValue>,
    pub source: &'a str,
    /// What the mode did during a session (`report::Simulation::report`), if any.
    pub session: Option<&'a str>,
    /// With the context, rules and API, for a new conversation; without them,
    /// for the conversation that wrote the mode.
    pub full: bool,
    pub language: &'a str,
    /// The game's profile, described (`Profile::describe`).
    pub profile: Option<&'a str>,
}

impl FeelReport<'_> {
    /// A mode reading the raw sound or image, indicators or other programs gets the
    /// advanced specification; so does any mode while the game has a profile.
    fn depth(&self) -> Depth {
        const ADVANCED: [&str; 10] = [
            "input.audio", "input.screen", "input.indicators", "input.external", "on_audio_hit", "on_indicator", "on_event",
            // The names of the first version of the API.
            "input.zones", "input.custom", "on_zone",
        ];
        let profile = self.profile.is_some_and(|p| !p.trim().is_empty());
        if profile || ADVANCED.iter().any(|a| self.source.contains(a)) {
            Depth::Advanced
        } else {
            Depth::Quick
        }
    }
}

/// The request to paste into an AI assistant to fix a mode that does not feel right.
pub fn feel_prompt(t: &Templates, r: &FeelReport) -> String {
    let game = match r.game.trim() {
        "" => String::new(),
        game => format!(" in **{game}**"),
    };
    let problems = match r.problems {
        [] => "Nothing specific: it just does not feel right.".to_owned(),
        list => list.iter().map(|p| format!("- {}", p.trim())).collect::<Vec<_>>().join("\n"),
    };
    let history = match r.history {
        [] => "None: this is the first request about this mode.".to_owned(),
        list => list.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n"),
    };
    let params = if r.params.is_empty() {
        "(no settings)".to_owned()
    } else {
        let rows = r.params.iter().map(|p| {
            let value = r.values.get(&p.name).unwrap_or(&p.default);
            format!("| {} | `{}` | {} | {} |", p.label, p.name, value_text(value), value_text(&p.default))
        });
        ["| Setting | Name | Value | Default |", "|---|---|---|---|"]
            .into_iter()
            .map(str::to_owned)
            .chain(rows)
            .collect::<Vec<_>>()
            .join("\n")
    };
    let session = match r.session {
        Some(report) => format!(
            "The player recorded a session while playing. Below is what the game sent and what they \
             pressed, replayed into a fresh copy of the mode with the settings above (times in seconds \
             from the start of the session). Moments the player marked while playing are the ones that \
             felt wrong.\n\n{}",
            report.trim_end()
        ),
        None => "No session was recorded.".to_owned(),
    };
    let depth = r.depth();
    let profile = match depth {
        Depth::Advanced => profile_text(depth, r.profile, &[]),
        Depth::Quick => "None set up for this game.".to_owned(),
    };
    sections(&t.fix_feel, r.full)
        .replace("{{RULES}}", t.rules.trim_end())
        .replace("{{PROFILE}}", &profile)
        .replace("{{SPEC}}", &spec(depth))
        .replace("{{LANGUAGE}}", r.language)
        .replace("{{GAME}}", &game)
        .replace("{{PROBLEMS}}", &problems)
        .replace("{{HISTORY}}", &history)
        .replace("{{NAME}}", r.name)
        .replace("{{PARAMS}}", &params)
        .replace("{{SOURCE}}", r.source.trim_end())
        .replace("{{SESSION}}", &session)
}

fn value_text(value: &ParamValue) -> String {
    match value {
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Number(n) => format!("{}", (n * 1000.0).round() / 1000.0),
        ParamValue::Text(t) => format!("\"{t}\""),
    }
}

/// The answer holds a mode script (it may only suggest settings instead).
pub fn has_script(answer: &str) -> bool {
    let script = extract_script(answer);
    script.contains("mode {") || script.contains("mode{")
}

/// Follow-up request when the generated mode does not load.
pub fn fix_prompt(error: &str) -> String {
    format!(
        "GameViber cannot load the mode you wrote. The error is:\n\n```\n{}\n```\n\n\
         Fix it, following the mode API specification, and send the complete corrected \
         .luau file again in a single code block.",
        error.trim()
    )
}

/// The mode script inside an AI answer: the fenced code block that declares the
/// mode (or the longest one), or the whole text when it has no code block.
pub fn extract_script(answer: &str) -> String {
    let mut blocks = Vec::new();
    let mut current: Option<Vec<&str>> = None;
    for line in answer.lines() {
        if line.trim_start().starts_with("```") {
            match current.take() {
                Some(block) => blocks.push(block.join("\n")),
                None => current = Some(Vec::new()),
            }
        } else if let Some(block) = &mut current {
            block.push(line);
        }
    }
    // An unterminated block (answer cut short) still holds code.
    if let Some(block) = current {
        blocks.push(block.join("\n"));
    }
    let declares_mode = |b: &&String| b.contains("mode {") || b.contains("mode{");
    let script = match blocks.iter().find(declares_mode) {
        Some(block) => block.clone(),
        None => blocks.into_iter().max_by_key(String::len).unwrap_or_else(|| answer.to_owned()),
    };
    format!("{}\n", script.trim())
}

/// A file name stem for a mode made for `game`: "Prince of Persia: The Lost Crown"
/// gives "prince-of-persia-the-lost-crown".
pub fn file_stem(game: &str) -> String {
    let mut stem = String::new();
    for c in game.trim().chars() {
        if c.is_alphanumeric() {
            stem.extend(c.to_lowercase());
        } else if !stem.is_empty() && !stem.ends_with('-') {
            stem.push('-');
        }
    }
    stem.trim_end_matches('-').to_owned()
}
