//! Settings page: updates (`updates.rs`), the language of the requests to AI
//! assistants, the request templates (editable, with the shipped version one
//! click away), the setup guide and the license notice.

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;
use crate::engine::{Command, Shared};
use crate::mode::prompt::Template;

/// Offered in the language picker; any other can be typed.
const LANGUAGES: [&str; 10] =
    ["English", "Français", "Español", "Deutsch", "Italiano", "Português", "Nederlands", "Polski", "Japanese", "Chinese"];

pub struct State {
    /// Language being typed (None: show the saved one).
    language: Option<String>,
    template: Template,
    /// Template text being edited (None: load it from disk).
    text: Option<String>,
    note: Option<Result<String, String>>,
}

impl Default for State {
    fn default() -> Self {
        Self { language: None, template: Template::NewMode, text: None, note: None }
    }
}

impl App {
    pub(super) fn settings_ui(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 20));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                heading(ui, "Settings");
                ui.add_space(8.0);
                card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.updates_card(ui, s);
                });
                ui.add_space(8.0);
                card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new("Language").strong().size(15.0));
                    ui.label(muted(
                        "AI assistants answer in this language, and write the texts you see in the modes they make \
                         (settings, help, questions, overlay messages) in it. GameViber itself is in English for now.",
                    ));
                    self.language_picker(ui, s);
                });
                ui.add_space(8.0);
                card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.templates_editor(ui);
                });
                ui.add_space(8.0);
                card(PANEL).inner_margin(Margin::same(16)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Setup guide").strong().size(15.0));
                        ui.label(muted("Intiface Central, toys, gamepad and a first mode, step by step."));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Run it again").clicked() {
                                self.onboarding = Some(0);
                            }
                        });
                    });
                });
                ui.add_space(8.0);
                card(PANEL).inner_margin(Margin::same(16)).show(ui, about);
            });
        });
    }

    fn language_picker(&mut self, ui: &mut egui::Ui, s: &Shared) {
        let saved = &s.settings.language;
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("language").selected_text(saved.as_str()).show_ui(ui, |ui| {
                for language in LANGUAGES {
                    if ui.selectable_label(saved == language, language).clicked() {
                        self.send(Command::SetLanguage(language.to_owned()));
                        self.settings.language = None;
                    }
                }
            });
            ui.label(muted("or"));
            let typed = self.settings.language.get_or_insert_with(String::new);
            ui.add(egui::TextEdit::singleline(typed).hint_text("another language").desired_width(160.0));
            let language = typed.trim().to_owned();
            if ui.add_enabled(!language.is_empty() && language != *saved, egui::Button::new("Use")).clicked() {
                self.send(Command::SetLanguage(language));
                self.settings.language = None;
            }
        });
    }

    fn templates_editor(&mut self, ui: &mut egui::Ui) {
        let st = &mut self.settings;
        ui.label(RichText::new("Requests to AI assistants").strong().size(15.0));
        ui.label(muted(
            "The templates GameViber fills in when it prepares a request. Changes apply to the next request. \
             Words in {{double braces}} are replaced by the game, the mode, the session... keep them.",
        ));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for template in Template::ALL {
                let label = if template.customized() { format!("{} ✏", template.title()) } else { template.title().to_owned() };
                if ui.selectable_label(st.template == template, label).clicked() && st.template != template {
                    st.template = template;
                    st.text = None;
                    st.note = None;
                }
            }
        });
        if st.template == Template::FixFeel {
            ui.label(
                muted("Lines between <!-- full --> and <!-- /full --> are left out of a short request, sent in the \
                       conversation that wrote the mode.")
                .size(12.0),
            );
        }
        let template = st.template;
        let text = st.text.get_or_insert_with(|| template.text());
        let missing: Vec<&str> = template.placeholders().iter().copied().filter(|p| !text.contains(p)).collect();
        let edited = *text != template.text();
        ui.horizontal(|ui| {
            if ui.add_enabled(edited, primary("Save")).clicked() {
                st.note = Some(template.save(text).map(|()| "✔ Saved: the next request uses it.".to_owned()).map_err(|e| e.to_string()));
            }
            if ui.add_enabled(edited, egui::Button::new("Discard changes")).clicked() {
                *text = template.text();
                st.note = None;
            }
            let shipped = *text == template.builtin();
            if ui.add_enabled(!shipped, egui::Button::new("Back to GameViber's version")).clicked() {
                *text = template.builtin().to_owned();
                st.note = Some(template.reset().map(|()| "✔ GameViber's version is used again.".to_owned()).map_err(|e| e.to_string()));
            }
            if !missing.is_empty() {
                ui.label(RichText::new(format!("⚠ Missing: {}", missing.join(" "))).color(WARN).size(12.5));
            }
        });
        match &st.note {
            Some(Ok(note)) => {
                ui.label(RichText::new(note).color(OK));
            }
            Some(Err(e)) => {
                ui.label(RichText::new(e).color(DANGER_TEXT));
            }
            None => {}
        }
        ui.add(
            egui::TextEdit::multiline(text)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .desired_rows(24),
        );
    }
}

/// The license notice the GNU GPL asks interactive programs to show.
fn about(ui: &mut egui::Ui) {
    ui.set_width(ui.available_width());
    ui.label(RichText::new(format!("GameViber {}", crate::update::current_version())).strong().size(15.0));
    ui.label(muted(
        "Copyright © 2026 The GameViber contributors. GameViber is free software: you can redistribute it and/or \
         modify it under the terms of the GNU General Public License, version 3 or any later version. It comes \
         with ABSOLUTELY NO WARRANTY.",
    ));
    ui.horizontal(|ui| {
        ui.hyperlink_to("Source code", format!("https://github.com/{}", crate::update::REPOSITORY));
        ui.hyperlink_to("License", "https://www.gnu.org/licenses/gpl-3.0.html");
    });
}
