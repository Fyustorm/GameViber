//! Requests for an AI assistant (ChatGPT, Claude...) to write a mode for one
//! game or to fix a mode that does not feel right, and extraction of the
//! script from its answer.

use std::collections::BTreeMap;

use super::{ParamDef, ParamValue};

const TEMPLATE: &str = include_str!("../../prompts/new-mode.md");
const FEEL_TEMPLATE: &str = include_str!("../../prompts/fix-feel.md");
const EXAMPLE: &str = include_str!("../../modes/surge.luau");
const SPEC: &str = include_str!("../../../docs/spec-modes.md");

/// The request to paste into an AI assistant to get a mode made for `game`.
pub fn new_mode_prompt(game: &str) -> String {
    TEMPLATE
        .replace("{{GAME}}", game.trim())
        .replace("{{EXAMPLE}}", EXAMPLE.trim_end())
        .replace("{{SPEC}}", SPEC.trim_end())
}

/// A mode the player finds wrong, and what they say about it.
pub struct FeelReport<'a> {
    pub name: &'a str,
    pub game: &'a str,
    /// Problems ticked in the GUI, then the player's own words.
    pub problems: &'a [String],
    pub params: &'a [ParamDef],
    pub values: &'a BTreeMap<String, ParamValue>,
    pub source: &'a str,
    /// What the mode did during a session (`report::Simulation::report`), if any.
    pub session: Option<&'a str>,
}

/// The request to paste into an AI assistant to fix a mode that does not feel right.
pub fn feel_prompt(r: &FeelReport) -> String {
    let game = match r.game.trim() {
        "" => String::new(),
        game => format!(" in **{game}**"),
    };
    let problems = match r.problems {
        [] => "Nothing specific: it just does not feel right.".to_owned(),
        list => list.iter().map(|p| format!("- {}", p.trim())).collect::<Vec<_>>().join("\n"),
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
             from the start of the session).\n\n{}",
            report.trim_end()
        ),
        None => "No session was recorded.".to_owned(),
    };
    FEEL_TEMPLATE
        .replace("{{GAME}}", &game)
        .replace("{{PROBLEMS}}", &problems)
        .replace("{{NAME}}", r.name)
        .replace("{{PARAMS}}", &params)
        .replace("{{SOURCE}}", r.source.trim_end())
        .replace("{{SESSION}}", &session)
        .replace("{{SPEC}}", SPEC.trim_end())
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
