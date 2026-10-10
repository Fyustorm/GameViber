//! Requests for an AI assistant (ChatGPT, Claude...) to write a mode for one
//! game — what the player wants to feel, the mode at once or after questions to
//! the player, or after an analysis of the game proposing its phases and
//! indicators — or to fix a mode that does not
//! feel right, and extraction of the script (or of the proposed setup) from its
//! answer. The request templates ship with GameViber; players can override them
//! with files in `~/.config/gameviber/prompts/`.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{ParamDef, ParamValue};
use crate::config;
use crate::package::IndicatorKind;

const SPEC: &str = include_str!("../../../docs/spec-modes.md");
/// Blocks of the templates, between `<!-- name -->` and `<!-- /name -->`, kept
/// only in some requests: `full` (the context, rules and API) is left out of a
/// request sent in the conversation that already has them; the others hold the
/// steps of one kind of request (`Step`, `Wishes::ask_first`).
const BLOCKS: [&str; 6] = ["full", "direct", "conversation", "after-analysis", "after-analysis-direct", "after-analysis-conversation"];
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
/// Left out of quick requests too: the raw sound and image, the inputs set
/// up for the mode and the advanced inputs (`Depth::Quick`).
const SPEC_ADVANCED: [&str; 3] = ["### 6.4 ", "### 6.5 ", "### 7.1 "];
/// The strokers' own outputs, left out when the player has none.
const SPEC_STROKERS: &str = "### 8.5 ";

/// How much of GameViber a request shows the assistant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Depth {
    /// The rumble, the buttons and what GameViber makes of the sound and the
    /// image (phases, impacts, intensity): a mode in a couple of minutes.
    #[default]
    Quick,
    /// Also the raw sound and image measures, the indicators of the screen, the
    /// example images and values other programs send (the inputs set up for the mode).
    Advanced,
}

/// What the inputs set up for the mode bring to a request: their description
/// (`Inputs::describe`), if any; a quick request only gets the phases.
fn inputs_text(depth: Depth, described: Option<&str>, phases: &[String]) -> String {
    match (depth, described.map(str::trim).filter(|p| !p.is_empty())) {
        (Depth::Quick, _) if !phases.is_empty() => {
            let names: Vec<String> = phases.iter().map(|s| format!("`{s}`")).collect();
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
        (Depth::Advanced, Some(described)) => format!(
            "The player set up inputs for this game in GameViber (§6.5). Use what helps:\n\n{described}"
        ),
        (Depth::Advanced, None) => "The player has not set up anything for this game yet. If an indicator of the screen \
             would help (an interface shown only in battles, a health bar), tell the player which indicators to draw in \
             GameViber's Creator (Captures & indicators tab): a name, its kind (visibility or gauge) and where to draw it. Read them defensively: \
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
    /// An analysis of a game proposing its phases and indicators, before its mode.
    Analysis,
    /// What makes a mode feel good and the script rules, shared by all.
    Rules,
}

impl Template {
    pub const ALL: [Template; 4] = [Template::NewMode, Template::Analysis, Template::FixFeel, Template::Rules];

    pub fn title(self) -> &'static str {
        match self {
            Template::NewMode => "A mode for a game",
            Template::Analysis => "Analyse a game",
            Template::FixFeel => "Fix a mode",
            Template::Rules => "Shared rules",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Template::NewMode => "new-mode.md",
            Template::Analysis => "analyse-game.md",
            Template::FixFeel => "fix-feel.md",
            Template::Rules => "rules.md",
        }
    }

    pub fn builtin(self) -> &'static str {
        match self {
            Template::NewMode => include_str!("../../prompts/new-mode.md"),
            Template::Analysis => include_str!("../../prompts/analyse-game.md"),
            Template::FixFeel => include_str!("../../prompts/fix-feel.md"),
            Template::Rules => include_str!("../../prompts/rules.md"),
        }
    }

    /// Placeholders the template should keep, so the request holds what it needs.
    pub fn placeholders(self) -> &'static [&'static str] {
        match self {
            Template::NewMode | Template::Analysis => &["{{GAME}}", "{{LANGUAGE}}", "{{INPUTS}}", "{{RULES}}", "{{SPEC}}"],
            Template::FixFeel => &[
                "{{GAME}}", "{{LANGUAGE}}", "{{PROBLEMS}}", "{{HISTORY}}", "{{NAME}}", "{{PARAMS}}",
                "{{SOURCE}}", "{{SESSION}}", "{{INPUTS}}", "{{RULES}}", "{{SPEC}}",
            ],
            Template::Rules => &[],
        }
    }

    /// The player's version of the template.
    pub fn path(self) -> PathBuf {
        config::config_dir().join("prompts").join(self.file_name())
    }

    /// The player's version, or the shipped one. `{{INPUTS}}` was `{{PROFILE}}`
    /// in versions the player saved before the terms changed.
    pub fn text(self) -> String {
        std::fs::read_to_string(self.path()).map(|t| t.replace("{{PROFILE}}", "{{INPUTS}}")).unwrap_or_else(|_| self.builtin().to_owned())
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
    pub analysis: String,
    pub fix_feel: String,
    pub rules: String,
}

impl Templates {
    /// The player's templates (the shipped ones where they changed nothing).
    pub fn load() -> Self {
        Self {
            new_mode: Template::NewMode.text(),
            analysis: Template::Analysis.text(),
            fix_feel: Template::FixFeel.text(),
            rules: Template::Rules.text(),
        }
    }

    #[cfg(test)]
    pub fn builtin() -> Self {
        Self {
            new_mode: Template::NewMode.builtin().to_owned(),
            analysis: Template::Analysis.builtin().to_owned(),
            fix_feel: Template::FixFeel.builtin().to_owned(),
            rules: Template::Rules.builtin().to_owned(),
        }
    }
}

/// Which request of the way to a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Step {
    /// The mode, in a new conversation.
    #[default]
    Script,
    /// The game's phases and indicators proposed, to set up in GameViber (`Setup`)...
    Analysis,
    /// ...then the mode, asked in the same conversation.
    AfterAnalysis,
}

/// What a moment the player may want to feel is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeelKind {
    /// Something happening: a pulse, a jolt.
    Moment,
    /// A background running while it lasts.
    Background,
    /// Only some genres have it: left out by the assistant where the game has none.
    Genre,
}

/// Something the player may want to feel, ticked when asking for a mode.
pub struct Feel {
    pub key: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    /// What the request asks for.
    pub ask: &'static str,
    pub kind: FeelKind,
    /// Ticked at first: most games have it.
    pub common: bool,
    /// Read from the screen, through an indicator.
    pub screen: bool,
}

const fn feel(key: &'static str, label: &'static str, help: &'static str, ask: &'static str, kind: FeelKind, common: bool, screen: bool) -> Feel {
    Feel { key, label, help, ask, kind, common, screen }
}

/// What a player may tick, the most common first in each kind.
pub const FEELS: [Feel; 14] = [
    feel("damage_taken", "Damage you take", "a jolt, stronger for big hits", "a jolt when the player takes damage, stronger for big hits", FeelKind::Moment, true, false),
    feel("hits_landed", "Hits you land", "short pulses on impacts", "short pulses on the hits the player lands", FeelKind::Moment, true, false),
    feel("buttons", "Button presses", "attack, shoot, dodge...", "a short response to the main action buttons (attack, shoot, dodge)", FeelKind::Moment, true, false),
    feel("big_moments", "Big moments", "boss down, level up, death", "a strong burst on big moments: a boss down, a level up, a death", FeelKind::Moment, true, false),
    feel("fight_wave", "A wave in fights", "rises with the action", "a wave running through fights, rising with the action", FeelKind::Background, true, false),
    feel("explore_wave", "A calm wave exploring", "low, never quite 0", "a calm, low wave while exploring", FeelKind::Background, true, false),
    feel("low_health", "Heartbeat on low health", "reads the health bar on screen", "a heartbeat while health is low, faster as it drops (read from a health gauge on screen)", FeelKind::Background, true, true),
    feel("quiet_menus", "Silence in menus and cutscenes", "nothing while not playing", "nothing at all in menus, pauses and cutscenes", FeelKind::Background, true, false),
    feel("parry", "Parries and dodges", "a release, a short burst", "a short burst on successful parries and dodges, felt as a release rather than a hit", FeelKind::Genre, true, false),
    feel("charge", "Charged attacks", "builds while charging", "a rise while an attack charges, released on the hit", FeelKind::Genre, true, false),
    feel("special", "Special moves", "ultimates, finishers, magic", "a long, strong surge on special moves (ultimates, finishers, magic)", FeelKind::Genre, false, false),
    feel("combo", "Combos and streaks", "rises with the combo", "a level rising with combos and kill streaks, falling when they break", FeelKind::Genre, false, false),
    feel("recoil", "Weapon recoil", "each shot, by weapon", "a kick on each shot, heavier weapons kicking harder", FeelKind::Genre, false, false),
    feel("speed", "Speed", "vehicles, sprinting", "a level following speed (vehicles, sprinting)", FeelKind::Genre, false, false),
];

/// What the player asks of a mode, kept with it for its next requests.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Wishes {
    pub vibrators: bool,
    pub strokers: bool,
    /// The `FEELS` ticked, by key.
    pub feel: Vec<String>,
    /// The game's phases and indicators proposed first (`Step::Analysis`).
    pub phases_first: bool,
    /// Questions and designs to choose from before the script.
    pub ask_first: bool,
}

impl Default for Wishes {
    fn default() -> Self {
        Self {
            vibrators: true,
            strokers: true,
            feel: FEELS.iter().filter(|f| f.common).map(|f| f.key.to_owned()).collect(),
            phases_first: true,
            ask_first: false,
        }
    }
}

impl Wishes {
    pub fn wants(&self, key: &str) -> bool {
        self.feel.iter().any(|k| k == key)
    }

    pub fn set(&mut self, key: &str, on: bool) {
        self.feel.retain(|k| k != key);
        if on {
            self.feel.push(key.to_owned());
        }
    }

    fn feels(&self) -> impl Iterator<Item = &'static Feel> + '_ {
        FEELS.iter().filter(|f| self.wants(f.key))
    }

    /// Something ticked is read from the screen.
    pub fn needs_screen(&self) -> bool {
        self.feels().any(|f| f.screen)
    }

    /// What the request asks for, before the player's own instructions.
    fn text(&self, step: Step) -> String {
        let toys = match (self.vibrators, self.strokers) {
            (true, false) | (false, false) => "The player's toys vibrate.",
            (false, true) => "The player's toy is a stroker: shape its strokes (§8.5) where the game gives a rhythm.",
            (true, true) => "The player uses vibrators and strokers: the channels drive both; shape the strokes (§8.5) where the \
                             game gives a rhythm.",
        };
        let mut out = format!("# What the player wants\n\n{toys}");
        let feels: Vec<&Feel> = self.feels().collect();
        if !feels.is_empty() {
            out.push_str(match step {
                Step::Analysis => " They want to feel the moments below: propose the phases and indicators these need \
                                   (a health gauge for a heartbeat on low health).\n\n",
                _ => " Build the mode around these moments:\n\n",
            });
            for f in &feels {
                out.push_str(&format!("- {}\n", f.ask));
            }
            if feels.iter().any(|f| f.kind == FeelKind::Genre) {
                out.push_str("\nSome are for some genres only: leave out those the game has nothing for.\n");
            }
            if self.ask_first && step != Step::Analysis {
                out.push_str("\nYour questions are about the rest: do not ask again what is listed here.\n");
            }
        }
        out
    }
}

/// A request for a mode made for one game.
pub struct NewMode<'a> {
    pub game: &'a str,
    /// The language the assistant answers in.
    pub language: &'a str,
    pub step: Step,
    pub wishes: &'a Wishes,
    pub depth: Depth,
    /// The inputs set up for the mode, described (advanced requests).
    pub described: Option<&'a str>,
    pub phases: &'a [String],
    /// The player's own instructions, written last.
    pub instructions: &'a str,
}

/// The request to paste into an AI assistant to get a mode made for a game (or its analysis).
pub fn new_mode_prompt(t: &Templates, r: &NewMode) -> String {
    let ask = r.wishes.ask_first;
    let (template, keep): (&str, &[&str]) = match r.step {
        Step::Script if ask => (&t.new_mode, &["full", "conversation"]),
        Step::Script => (&t.new_mode, &["full", "direct"]),
        // The conversation of an analysis already has the context, rules and API.
        Step::AfterAnalysis if ask => (&t.new_mode, &["after-analysis", "after-analysis-conversation"]),
        Step::AfterAnalysis => (&t.new_mode, &["after-analysis", "after-analysis-direct"]),
        Step::Analysis => (&t.analysis, &["full"]),
    };
    // An analysis proposes indicators, and a heartbeat on low health reads one: they get their API.
    let depth = if r.step == Step::Analysis || r.wishes.needs_screen() { Depth::Advanced } else { r.depth };
    let text = sections(template, keep)
        .replace("{{RULES}}", t.rules.trim_end())
        .replace("{{INPUTS}}", &inputs_text(depth, r.described, r.phases))
        .replace("{{SPEC}}", &spec(depth, r.wishes.strokers))
        .replace("{{LANGUAGE}}", r.language)
        .replace("{{GAME}}", r.game.trim());
    format!("{}\n\n{}\n{}", text.trim_end(), r.wishes.text(r.step), instructions_text(r.instructions))
}

/// The end of a request: the player's own instructions, and room for more.
fn instructions_text(instructions: &str) -> String {
    let heading = "# The player's own instructions\n\n\
                   Where they differ from the rest of this message, follow them.";
    match instructions.trim() {
        "" => format!("{heading} The player may write some below.\n"),
        text => format!("{heading}\n\n{text}\n"),
    }
}

/// `text` with the blocks named in `keep` kept (without their markers) and the
/// other blocks of `BLOCKS` left out.
fn sections(text: &str, keep: &[&str]) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        let marker = line.trim().strip_prefix("<!-- ").and_then(|m| m.strip_suffix(" -->"));
        match marker {
            Some(name) if BLOCKS.contains(&name) => skipping = !keep.contains(&name),
            Some(end) if end.strip_prefix('/').is_some_and(|name| BLOCKS.contains(&name)) => skipping = false,
            _ if !skipping => {
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

/// The specification without the sections of `SPEC_LEFT_OUT` (`SPEC_ADVANCED` for
/// a quick request, `SPEC_STROKERS` without strokers).
fn spec(depth: Depth, strokers: bool) -> String {
    let mut advanced: Vec<&str> = if depth == Depth::Quick { SPEC_ADVANCED.to_vec() } else { Vec::new() };
    if !strokers {
        advanced.push(SPEC_STROKERS);
    }
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
            if skipping.is_none() && SPEC_LEFT_OUT.iter().chain(&advanced).any(|s| line.starts_with(s)) {
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
    /// The inputs set up for the mode, described (`Inputs::describe`).
    pub inputs: Option<&'a str>,
}

impl FeelReport<'_> {
    /// A mode reading the raw sound or image, indicators or other programs gets the
    /// advanced specification; so does any mode with inputs set up.
    fn depth(&self) -> Depth {
        const ADVANCED: [&str; 10] = [
            "input.audio", "input.screen", "input.indicators", "input.external", "on_audio_hit", "on_indicator", "on_event",
            // The names of the first version of the API.
            "input.zones", "input.custom", "on_zone",
        ];
        let described = self.inputs.is_some_and(|p| !p.trim().is_empty());
        if described || ADVANCED.iter().any(|a| self.source.contains(a)) {
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
    let described = match depth {
        Depth::Advanced => inputs_text(depth, r.inputs, &[]),
        Depth::Quick => "None set up for this game.".to_owned(),
    };
    sections(&t.fix_feel, if r.full { &["full"] } else { &[] })
        .replace("{{RULES}}", t.rules.trim_end())
        .replace("{{INPUTS}}", &described)
        .replace("{{SPEC}}", &spec(depth, depth == Depth::Advanced || ["stroke(", "thrust("].iter().any(|a| r.source.contains(a))))
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

/// The setup an analysis proposes (`Step::Analysis`), from the `json` block of its answer.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Setup {
    pub phases: Vec<SetupPhase>,
    pub indicators: Vec<SetupIndicator>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SetupPhase {
    pub name: String,
    pub sound: Option<String>,
    pub indicators: Vec<String>,
    pub otherwise: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct SetupIndicator {
    pub name: String,
    pub kind: IndicatorKind,
    /// Where it is on the screen, and when to capture it.
    #[serde(rename = "where")]
    pub place: String,
}

/// The setup an analysis ends with, if the answer holds one.
pub fn extract_setup(answer: &str) -> Option<Setup> {
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
    // Pasted without its code fence.
    blocks.push(answer.to_owned());
    blocks.iter().rev().filter_map(|b| serde_json::from_str::<Setup>(b.trim()).ok()).find(|s| !s.phases.is_empty())
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
