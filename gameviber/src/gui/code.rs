//! The Creator's script editor: line numbers next to the highlighted source,
//! find and replace (Ctrl+F, Ctrl+H, F3), go to line (Ctrl+G), and what
//! writing a script needs: Enter keeping the indentation (one level in after
//! `then`, `do`, `{`..., `end` put back out), Tab and Shift+Tab on the
//! selected lines, Ctrl+/ commenting them.

use std::ops::Range;

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Align, Color32, Event, Key, Margin, Modifiers, RichText};

use super::luau;
use super::theme::*;

/// Background of the matches of a search, and of the one it is on.
const MATCH: Color32 = Color32::from_rgb(0x3b, 0x41, 0x55);
const CURRENT_MATCH: Color32 = Color32::from_rgb(0x82, 0x5a, 0x1e);

#[derive(Default)]
pub(super) struct Editor {
    /// Mode id the buffer was loaded from.
    pub(super) id: String,
    pub(super) text: String,
    pub(super) dirty: bool,
    pub(super) message: Option<String>,
    /// Line to put the cursor on, and scroll to, on the next frame.
    pub(super) goto: Option<usize>,
    find: Find,
    /// What the go to line field holds.
    line: String,
    /// Last highlighted text, error line and marks, with its layout.
    highlighted: Option<(String, Option<usize>, Vec<(Range<usize>, Color32)>, LayoutJob)>,
}

impl Editor {
    /// Emptied to load mode `id`'s script (none: the one played, as saved), the search kept.
    pub(super) fn reset(&mut self, id: String) {
        *self = Editor { id, find: std::mem::take(&mut self.find), ..Default::default() };
    }
}

#[derive(Default)]
struct Find {
    query: String,
    replacement: String,
    /// The replace field is shown.
    replace: bool,
    match_case: bool,
    whole_word: bool,
    /// Byte range of the match the search is on.
    current: Option<Range<usize>>,
}

impl Find {
    fn matches(&self, text: &str) -> Vec<Range<usize>> {
        find_all(text, &self.query, self.match_case, self.whole_word)
    }

    /// The matches' backgrounds.
    fn marks(&self, text: &str) -> Vec<(Range<usize>, Color32)> {
        self.matches(text).into_iter().map(|m| (m.clone(), if Some(&m) == self.current.as_ref() { CURRENT_MATCH } else { MATCH })).collect()
    }
}

/// Where a search goes.
enum Jump {
    Next,
    Previous,
    /// The first match from this byte on.
    From(usize),
}

/// What the editing keys do to the selected lines.
#[derive(Clone, Copy)]
enum Edit {
    Newline,
    Indent,
    Outdent,
    Comment,
}

/// The search and go to line bar, then the editor.
pub(super) fn code_view(ui: &mut egui::Ui, editor: &mut Editor, editable: bool, error_line: Option<usize>) {
    const MARGIN: egui::Margin = Margin { left: 6, right: 6, top: 4, bottom: 4 };
    let id = egui::Id::new(("script", &editor.id));
    let (find_id, replace_id, line_id) = (egui::Id::new("script find"), egui::Id::new("script replace"), egui::Id::new("script line"));
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let Editor { text, dirty, highlighted, goto, find, line, .. } = editor;
    let ctx = ui.ctx().clone();
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    let selection = state.cursor.char_range().map_or(0..0, |r| byte_range(text, r));
    let focused = ui.memory(|m| m.has_focus(id));
    // Where the cursor goes this frame, scrolled to (centered for a search or a line).
    let mut reveal: Option<(usize, Option<Align>)> = None;
    let mut focus_editor = false;

    let (open_find, open_replace, open_line, previous_key, next_key) = ui.input_mut(|i| {
        (
            i.consume_key(Modifiers::COMMAND, Key::F),
            i.consume_key(Modifiers::COMMAND, Key::H),
            i.consume_key(Modifiers::COMMAND, Key::G),
            i.consume_key(Modifiers::SHIFT, Key::F3),
            i.consume_key(Modifiers::NONE, Key::F3),
        )
    });
    let mut changed = false;
    let mut jump = if previous_key { Some(Jump::Previous) } else { next_key.then_some(Jump::Next) };
    if open_find || (open_replace && editable) {
        find.replace |= open_replace;
        // The selected word is searched.
        let selected = &text[selection.clone()];
        if !selected.is_empty() && !selected.contains('\n') && selected != find.query {
            find.query = selected.to_owned();
            changed = true;
        }
        select_all(&ctx, find_id, &find.query);
        ctx.memory_mut(|m| m.request_focus(find_id));
    }
    if open_line {
        select_all(&ctx, line_id, line);
        ctx.memory_mut(|m| m.request_focus(line_id));
    }

    // The bar: what to find, how, the matches; the line to go to on the right.
    let count = find.matches(text);
    let (mut replace_one, mut replace_all) = (false, false);
    let enter = |response: &egui::Response| response.lost_focus() && ctx.input(|i| i.key_pressed(Key::Enter));
    let escape = |response: &egui::Response| response.lost_focus() && ctx.input(|i| i.key_pressed(Key::Escape));
    let shift = ctx.input(|i| i.modifiers.shift);
    ui.horizontal(|ui| {
        let response = ui.add(egui::TextEdit::singleline(&mut find.query).id(find_id).hint_text("🔍 Find (Ctrl+F)").desired_width(200.0));
        changed |= response.changed();
        if enter(&response) {
            jump = Some(if shift { Jump::Previous } else { Jump::Next });
            response.request_focus();
        }
        focus_editor |= escape(&response);
        changed |= ui.toggle_value(&mut find.match_case, "Aa").on_hover_text("Match case").clicked();
        changed |= ui.toggle_value(&mut find.whole_word, RichText::new("ab").underline()).on_hover_text("Whole word").clicked();
        let any = !count.is_empty();
        if ui.add_enabled(any, egui::Button::new("⏶")).on_hover_text("Previous match (Shift+Enter, Shift+F3)").clicked() {
            jump = Some(Jump::Previous);
        }
        if ui.add_enabled(any, egui::Button::new("⏷")).on_hover_text("Next match (Enter, F3)").clicked() {
            jump = Some(Jump::Next);
        }
        if !find.query.is_empty() {
            let at = find.current.as_ref().and_then(|c| count.iter().position(|m| m == c));
            let label = match (at, count.len()) {
                (_, 0) => RichText::new("No match").color(DANGER_TEXT),
                (Some(at), n) => muted(format!("{} of {n}", at + 1)),
                (None, 1) => muted("1 match"),
                (None, n) => muted(format!("{n} matches")),
            };
            ui.label(label.size(12.0));
        }
        if editable {
            ui.toggle_value(&mut find.replace, "Replace").on_hover_text("Replace the matches (Ctrl+H)");
        }
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            let response = ui.add(egui::TextEdit::singleline(line).id(line_id).hint_text("Line (Ctrl+G)").desired_width(90.0));
            if enter(&response) {
                if let Ok(n) = line.trim().parse::<usize>() {
                    *goto = Some(n);
                }
            }
            focus_editor |= escape(&response);
        });
    });
    if find.replace && editable {
        ui.horizontal(|ui| {
            let response = ui.add(egui::TextEdit::singleline(&mut find.replacement).id(replace_id).hint_text("Replace with").desired_width(200.0));
            if enter(&response) {
                replace_one = true;
                response.request_focus();
            }
            focus_editor |= escape(&response);
            let any = !count.is_empty();
            replace_one |= ui.add_enabled(any, egui::Button::new("Replace")).on_hover_text("Replace this match and go to the next (Enter)").clicked();
            replace_all |= ui.add_enabled(any, egui::Button::new("Replace all")).clicked();
        });
    }

    // The match the search is on, unless the cursor was moved off it (or the text changed under it).
    let matches = find.matches(text);
    find.current = find.current.take().filter(|c| *c == selection && matches.contains(c));
    if replace_one {
        match find.current.clone() {
            Some(m) => {
                let mut new = text.clone();
                new.replace_range(m.clone(), &find.replacement);
                let after = m.start + find.replacement.len();
                commit(&mut state, text, new, after..after);
                *dirty = true;
                jump = Some(Jump::From(after));
            }
            None => jump = Some(Jump::Next),
        }
    }
    if replace_all && !matches.is_empty() {
        let mut new = String::with_capacity(text.len());
        let mut last = 0;
        for m in &matches {
            new.push_str(&text[last..m.start]);
            new.push_str(&find.replacement);
            last = m.end;
        }
        new.push_str(&text[last..]);
        let at = matches[0].start;
        commit(&mut state, text, new, at..at);
        *dirty = true;
        find.current = None;
        reveal = Some((at, Some(Align::Center)));
    }
    if changed && jump.is_none() {
        jump = Some(Jump::From(selection.start));
    }
    if let Some(jump) = jump {
        let matches = find.matches(text);
        let (start, end) = find.current.as_ref().map_or((selection.start, selection.start), |c| (c.start, c.end));
        let target = match jump {
            Jump::Next => matches.iter().find(|m| m.start >= end).or(matches.first()),
            Jump::Previous => matches.iter().rev().find(|m| m.end <= start).or(matches.last()),
            Jump::From(from) => matches.iter().find(|m| m.start >= from).or(matches.first()),
        };
        find.current = target.cloned();
        if let Some(m) = target {
            state.cursor.set_char_range(Some(char_range(text, m)));
            reveal = Some((m.start, Some(Align::Center)));
        }
    }

    if let Some(n) = goto.take() {
        let at = line_offset(text, n);
        state.cursor.set_char_range(Some(char_range(text, &(at..at))));
        reveal = Some((at, Some(Align::Center)));
        focus_editor = true;
    }

    // Enter, Tab, Shift+Tab and Ctrl+/ in the editor, unless typed along other
    // text in one frame: their order would be lost.
    let edits = if focused && editable {
        ui.input_mut(|i| {
            if i.events.iter().any(|e| matches!(e, Event::Text(_) | Event::Paste(_))) {
                return Vec::new();
            }
            let mut edits = vec![Edit::Comment; i.count_and_consume_key(Modifiers::COMMAND, Key::Slash)];
            edits.extend(vec![Edit::Outdent; i.count_and_consume_key(Modifiers::SHIFT, Key::Tab)]);
            edits.extend(vec![Edit::Indent; i.count_and_consume_key(Modifiers::NONE, Key::Tab)]);
            edits.extend(vec![Edit::Newline; i.count_and_consume_key(Modifiers::NONE, Key::Enter)]);
            edits
        })
    } else {
        Vec::new()
    };
    let mut selected = selection;
    for edit in edits {
        let unit = indent_unit(text);
        let (new, at) = match edit {
            Edit::Newline => newline(text, selected.clone(), &unit),
            Edit::Indent => indent(text, selected.clone(), &unit, false),
            Edit::Outdent => indent(text, selected.clone(), &unit, true),
            Edit::Comment => toggle_comment(text, selected.clone()),
        };
        commit(&mut state, text, new, at.clone());
        *dirty = true;
        reveal = Some((at.start, None));
        selected = at;
    }
    state.store(&ctx, id);
    if focus_editor && editable {
        ctx.memory_mut(|m| m.request_focus(id));
    }

    // Lines do not wrap: long ones scroll sideways.
    let width = ui.available_width();
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Line numbers, laid out with the same font as the code so rows match.
            let lines = text.split('\n').count();
            let mut numbers = LayoutJob::default();
            for line in 1..=lines {
                let color = if Some(line) == error_line { DANGER_TEXT } else { IDLE };
                let format = egui::TextFormat { font_id: font.clone(), color, ..Default::default() };
                numbers.append(&format!("{line:>4} {}", if line < lines { "\n" } else { "" }), 0.0, format);
            }
            let galley = ui.fonts_mut(|f| f.layout_job(numbers));
            let gutter_width = galley.size().x;
            ui.vertical(|ui| {
                ui.add_space(MARGIN.top as f32);
                ui.label(galley);
            });
            let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
                let source = buffer.as_str();
                let marks = find.marks(source);
                let stale = highlighted.as_ref().is_none_or(|(t, e, m, _)| t != source || *e != error_line || *m != marks);
                if stale {
                    let job = luau::highlight(source, font.clone(), error_line, &marks);
                    *highlighted = Some((source.to_owned(), error_line, marks, job));
                }
                let job = highlighted.as_ref().map(|(.., job)| job.clone()).unwrap_or_default();
                ui.fonts_mut(|f| f.layout_job(job))
            };
            let output = egui::TextEdit::multiline(text)
                .id(id)
                .code_editor()
                .interactive(editable)
                .margin(MARGIN)
                .desired_width(width - gutter_width - 2.0 * MARGIN.left as f32 - 12.0)
                .desired_rows(30)
                .layouter(&mut layouter)
                .show(ui);
            if output.response.changed() {
                *dirty = true;
            }
            if let Some((at, align)) = reveal {
                let cursor = CCursor::new(text[..at.min(text.len())].chars().count());
                let rect = output.galley.pos_from_cursor(cursor).translate(output.galley_pos.to_vec2());
                ui.scroll_to_rect(rect.expand(4.0), align);
            }
        });
    });
}

/// The next time the field `id` shows, all its `text` is selected (typing replaces it).
fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()))));
    state.store(ctx, id);
}

/// Puts `new` in place of `text`, `selection` selected, as one step undo goes back over.
fn commit(state: &mut TextEditState, text: &mut String, new: String, selection: Range<usize>) {
    let mut undoer = state.undoer();
    undoer.add_undo(&(state.cursor.char_range().unwrap_or_default(), text.clone()));
    *text = new;
    let range = char_range(text, &selection);
    undoer.add_undo(&(range, text.clone()));
    state.set_undoer(undoer);
    state.cursor.set_char_range(Some(range));
}

fn byte_range(text: &str, range: CCursorRange) -> Range<usize> {
    let [start, end] = range.sorted_cursors();
    let byte = |c: CCursor| text.char_indices().nth(c.index.0).map_or(text.len(), |(b, _)| b);
    byte(start)..byte(end)
}

fn char_range(text: &str, range: &Range<usize>) -> CCursorRange {
    let char = |b: usize| CCursor::new(text[..b.min(text.len())].chars().count());
    CCursorRange::two(char(range.start), char(range.end))
}

/// Where `query` is in `text`, apart; without `match_case`, ASCII letters match either case.
fn find_all(text: &str, query: &str, match_case: bool, whole_word: bool) -> Vec<Range<usize>> {
    let (bytes, query) = (text.as_bytes(), query.as_bytes());
    let word = |b: &u8| b.is_ascii_alphanumeric() || *b == b'_';
    let mut found = Vec::new();
    let mut i = 0;
    while !query.is_empty() && i + query.len() <= bytes.len() {
        let candidate = &bytes[i..i + query.len()];
        let same = if match_case { candidate == query } else { candidate.eq_ignore_ascii_case(query) };
        let alone = !whole_word || ((i == 0 || !word(&bytes[i - 1])) && !bytes.get(i + query.len()).is_some_and(word));
        if same && alone && text.is_char_boundary(i) {
            found.push(i..i + query.len());
            i += query.len();
        } else {
            i += 1;
        }
    }
    found
}

/// Byte the 1-based `line` starts at (the last line's past the end).
fn line_offset(text: &str, line: usize) -> usize {
    match line {
        0 | 1 => 0,
        n => text.match_indices('\n').nth(n - 2).map_or_else(|| line_start(text, text.len()), |(i, _)| i + 1),
    }
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

fn line_end(text: &str, at: usize) -> usize {
    text[at..].find('\n').map_or(text.len(), |i| at + i)
}

fn leading(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// One level of indentation: a tab, or the spaces the script uses (two by default).
fn indent_unit(text: &str) -> String {
    let indents: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).map(leading).filter(|l| !l.is_empty()).collect();
    if indents.iter().filter(|l| l.starts_with('\t')).count() * 2 > indents.len() {
        return "\t".to_owned();
    }
    let spaces = indents.iter().filter(|l| !l.contains('\t')).map(|l| l.len());
    let four = spaces.clone().count() > 0 && spaces.clone().all(|n| n % 4 == 0);
    " ".repeat(if four { 4 } else { 2 })
}

/// Starts of the lines `selection` is on (a line it only ends at the start of is not).
fn selected_lines(text: &str, selection: &Range<usize>) -> Vec<usize> {
    let first = line_start(text, selection.start);
    let last = if selection.end > selection.start && text[..selection.end].ends_with('\n') { selection.end - 1 } else { selection.end };
    std::iter::once(first).chain(text[first..last.max(first)].match_indices('\n').map(|(i, _)| first + i + 1)).collect()
}

/// `text` with `edits` (byte ranges replaced, in order and apart) made, and where `selection` ends up.
fn splice(text: &str, edits: &[(Range<usize>, String)], selection: Range<usize>) -> (String, Range<usize>) {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, with) in edits {
        out.push_str(&text[last..range.start]);
        out.push_str(with);
        last = range.end;
    }
    out.push_str(&text[last..]);
    let moved = |at: usize| {
        let mut new = at as isize;
        for (range, with) in edits {
            if at >= range.end {
                new += with.len() as isize - range.len() as isize;
            } else if at > range.start {
                new -= (at - range.start) as isize;
            }
        }
        new as usize
    };
    (out, moved(selection.start)..moved(selection.end))
}

/// Enter: the line's indentation kept, one level more after a line opening a
/// block; a line starting by closing one (`end`, `else`, `}`) left at the
/// block's level is first put one level out.
fn newline(text: &str, selection: Range<usize>, unit: &str) -> (String, Range<usize>) {
    let start = line_start(text, selection.start);
    let mut line = text[start..selection.start].to_owned();
    if luau::starts_with_closer(&line) {
        let previous = text[..start].lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("");
        let expected = format!("{}{}", leading(previous), if luau::opens_block(previous) { unit } else { "" });
        if leading(&line) == expected && expected.ends_with(unit) {
            line.replace_range(..unit.len(), "");
        }
    }
    // The line keeps no trailing spaces, nor the new one any before the cursor's text.
    let base = leading(&line).to_owned();
    line.truncate(line.trim_end().len());
    let mut end = selection.end;
    while text[end..].starts_with([' ', '\t']) {
        end += 1;
    }
    let opens = luau::opens_block(&line);
    let mut out = format!("{}{line}\n{base}{}", &text[..start], if opens { unit } else { "" });
    let cursor = out.len();
    // Between a block's opening and its end (`{|}`): the end goes on its own line.
    if opens && luau::starts_with_closer(&text[end..line_end(text, end)]) {
        out.push('\n');
        out.push_str(&base);
    }
    out.push_str(&text[end..]);
    (out, cursor..cursor)
}

/// Tab: one level in for the selected lines, or the indentation typed at the
/// cursor; Shift+Tab: one level out.
fn indent(text: &str, selection: Range<usize>, unit: &str, outdent: bool) -> (String, Range<usize>) {
    if !outdent && !text[selection.clone()].contains('\n') {
        return splice(text, &[(selection.clone(), unit.to_owned())], selection.end..selection.end);
    }
    let edits: Vec<(Range<usize>, String)> = selected_lines(text, &selection)
        .into_iter()
        .filter_map(|start| {
            let line = &text[start..line_end(text, start)];
            if outdent {
                let removed = if line.starts_with(unit) { unit.len() } else { leading(line).len().min(unit.len()) };
                let removed = if line.starts_with('\t') { 1 } else { removed };
                (removed > 0).then(|| (start..start + removed, String::new()))
            } else {
                (!line.trim().is_empty()).then(|| (start..start, unit.to_owned()))
            }
        })
        .collect();
    splice(text, &edits, selection)
}

/// Ctrl+/: the selected lines commented out (`-- ` at their indentation), or back in when all already are.
fn toggle_comment(text: &str, selection: Range<usize>) -> (String, Range<usize>) {
    let lines: Vec<(usize, &str)> = selected_lines(text, &selection)
        .into_iter()
        .map(|start| (start, &text[start..line_end(text, start)]))
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    let commented = !lines.is_empty() && lines.iter().all(|(_, line)| line.trim_start().starts_with("--"));
    let column = lines.iter().map(|(_, line)| leading(line).len()).min().unwrap_or(0);
    let edits: Vec<(Range<usize>, String)> = lines
        .iter()
        .map(|(start, line)| {
            if commented {
                let at = start + leading(line).len();
                let len = if text[at..].starts_with("-- ") { 3 } else { 2 };
                (at..at + len, String::new())
            } else {
                (start + column..start + column, "-- ".to_owned())
            }
        })
        .collect();
    splice(text, &edits, selection)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `|` marking the cursor, or `[` and `]` a selection.
    fn parse(marked: &str) -> (String, Range<usize>) {
        let text = marked.replace(['|', '[', ']'], "");
        let at = |c: char| marked.find(c);
        match (at('|'), at('['), at(']')) {
            (Some(i), ..) => (text, i..i),
            (None, Some(s), Some(e)) => (text, s..e - 1),
            _ => panic!("no cursor in {marked:?}"),
        }
    }

    fn mark((text, range): (String, Range<usize>)) -> String {
        if range.is_empty() {
            format!("{}|{}", &text[..range.start], &text[range.start..])
        } else {
            format!("{}[{}]{}", &text[..range.start], &text[range.clone()], &text[range.end..])
        }
    }

    fn enter(marked: &str) -> String {
        let (text, selection) = parse(marked);
        mark(newline(&text, selection, "  "))
    }

    #[test]
    fn search_finds_words_and_cases() {
        let text = "set(x) Set(y) reset(z) set";
        assert_eq!(find_all(text, "set", false, false), [0..3, 7..10, 16..19, 23..26]);
        assert_eq!(find_all(text, "set", true, false), [0..3, 16..19, 23..26]);
        assert_eq!(find_all(text, "set", false, true), [0..3, 7..10, 23..26]);
        assert_eq!(find_all("aaaa", "aa", false, false), [0..2, 2..4]);
        assert_eq!(find_all("é = É", "é", false, false), [0..2]);
        assert!(find_all(text, "", false, false).is_empty());
    }

    #[test]
    fn lines_are_found() {
        let text = "a\nbb\nc";
        assert_eq!([1, 2, 3, 9].map(|n| line_offset(text, n)), [0, 2, 5, 5]);
    }

    #[test]
    fn enter_keeps_and_opens_indentation() {
        assert_eq!(enter("  x = 1|"), "  x = 1\n  |");
        assert_eq!(enter("  if x then|"), "  if x then\n    |");
        assert_eq!(enter("function tick(dt)|\nend"), "function tick(dt)\n  |\nend");
        assert_eq!(enter("t = {|}"), "t = {\n  |\n}");
        assert_eq!(enter("x = f(a, |b)"), "x = f(a,\n  |b)");
        // A closer typed at the block's level goes one level out.
        assert_eq!(enter("if x then\n  y()\n  end|"), "if x then\n  y()\nend\n|");
        assert_eq!(enter("if x then\n  y()\n  else|"), "if x then\n  y()\nelse\n  |");
        assert_eq!(enter("if x then\n  y()\nend|"), "if x then\n  y()\nend\n|");
        // A blank line loses its spaces.
        assert_eq!(enter("  x()\n  |"), "  x()\n\n  |");
    }

    #[test]
    fn tab_indents_lines_and_shift_tab_outdents() {
        let (text, selection) = parse("[a\n\n  b\n]c");
        assert_eq!(mark(indent(&text, selection.clone(), "  ", false)), "  [a\n\n    b\n]c");
        assert_eq!(mark(indent(&text, selection, "  ", true)), "[a\n\nb\n]c");
        let (text, selection) = parse("  x|y");
        assert_eq!(mark(indent(&text, selection.clone(), "  ", false)), "  x  |y");
        assert_eq!(mark(indent(&text, selection, "  ", true)), "x|y");
    }

    #[test]
    fn comments_toggle_at_the_indentation() {
        let (text, selection) = parse("[  a\n    b]");
        let (commented, selection) = toggle_comment(&text, selection);
        assert_eq!(commented, "  -- a\n  --   b");
        assert_eq!(toggle_comment(&commented, selection).0, text);
    }

    #[test]
    fn indent_unit_follows_the_script() {
        assert_eq!(indent_unit("a\n  b\n    c"), "  ");
        assert_eq!(indent_unit("a\n    b\n        c"), "    ");
        assert_eq!(indent_unit("a\n\tb"), "\t");
        assert_eq!(indent_unit("a"), "  ");
    }
}
