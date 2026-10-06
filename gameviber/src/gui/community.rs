//! Community page: modes other players made for their games, to try and to
//! publish one's own (not there yet: modes are shared as `.gameviber` files
//! meanwhile, `sharing.rs`).

use eframe::egui::{self, Margin, RichText};

use super::theme::*;
use super::App;

impl App {
    pub(super) fn community_ui(&mut self, ui: &mut egui::Ui) {
        let frame = egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(24, 18));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.sharing_ui(ui);
            heading(ui, "Community");
            ui.add_space(8.0);
            card(PANEL).inner_margin(Margin::symmetric(18, 14)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Coming soon").strong().size(16.0));
                ui.label(
                    "Modes other players made for your games, ranked by those who kept playing them, to install in a \
                     click; and a place to publish yours, or to share it with a few testers first.",
                );
                ui.add_space(6.0);
                ui.label(muted("Meanwhile, modes are shared as files: export one from its Sharing tab, import one here."));
                ui.add_space(6.0);
                let import = ui.add_enabled(!self.sharing.busy(), egui::Button::new("Import a file"));
                if import.on_hover_text("A .gameviber file someone shared: a mode with its inputs and its game").clicked() {
                    self.import_mode();
                }
            });
        });
    }
}
