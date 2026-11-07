use crate::{app::DesklinkApp, theme as t};
use desklink_layout::{Geometry, Layout, Screen};
use desklink_settings::pairing::PairingCode;
use eframe::egui::{self, Rect, RichText, UiBuilder};
impl DesklinkApp {
    pub(crate) fn inspector(&mut self, ui: &mut egui::Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, t::WHITE);
        ui.painter()
            .vline(rect.left(), rect.y_range(), (1.0, t::BORDER));
        ui.scope_builder(
            UiBuilder::new().max_rect(rect.shrink2(egui::vec2(22.0, 24.0))),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("inspector-scroll")
                    .max_height(rect.height() - 48.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.inspector_contents(ui, rect.width()));
            },
        );
    }
    pub(crate) fn inspector_contents(&mut self, ui: &mut egui::Ui, width: f32) {
        t::small_title(ui, "LAYOUT CONTROLLER");
        let mut owner = self.config.layout_owner.clone();
        let label = owner
            .as_ref()
            .and_then(|id| self.config.peers.iter().find(|p| p.public_key == *id))
            .map(|p| p.name.as_str())
            .unwrap_or("This computer");
        let combo = egui::ComboBox::from_id_salt("controller")
            .selected_text(label)
            .width(width - 56.0)
            .show_ui(ui, |ui| {
                let response = ui.selectable_value(&mut owner, None, "This computer");
                self.target("controller:self", response.rect);
                for peer in self.config.peers.clone() {
                    let response =
                        ui.selectable_value(&mut owner, Some(peer.public_key.clone()), &peer.name);
                    self.target(format!("controller:{}", peer.public_key), response.rect);
                }
            });
        self.target("controller", combo.response.rect);
        if owner != self.config.layout_owner {
            self.config.layout_owner = owner;
            self.config.layout = None;
            self.draft = Layout::single(self.own.clone(), self.config.name.clone());
            self.mark_dirty();
            self.fit = true;
        }
        t::hint(
            ui,
            if self.editable() {
                "Arrange screens here. Your layout is shared with paired receivers."
            } else {
                "The controller manages this arrangement."
            },
        );
        ui.add_space(24.0);
        ui.separator();
        ui.add_space(18.0);
        t::small_title(ui, "SELECTED COMPUTER");
        let screen = self.draft.screen(&self.selected).cloned().or_else(|| {
            self.config
                .peers
                .iter()
                .find(|p| p.public_key == self.selected)
                .map(|p| Screen {
                    id: p.public_key.clone(),
                    name: p.name.clone(),
                    rect: Geometry {
                        x: 0,
                        y: 0,
                        width: 240,
                        height: 150,
                    },
                })
        });
        if let Some(screen) = screen {
            t::monitor(
                ui.painter(),
                ui.cursor().min + egui::vec2(28.0, 25.0),
                42.0,
                t::BLUE,
            );
            ui.add_space(64.0);
            ui.label(RichText::new(&screen.name).size(18.0).strong());
            ui.add_space(10.0);
            let editable = self.editable();
            let mut name = screen.name.clone();
            ui.label("Display name");
            let edit = ui.add_enabled(
                editable && self.draft.screen(&screen.id).is_some(),
                egui::TextEdit::singleline(&mut name).desired_width(width - 52.0),
            );
            self.target("name", edit.rect);
            if edit.changed() {
                if let Some(screen) = self.draft.screen_mut(&self.selected) {
                    screen.name = name;
                }
                self.mark_dirty();
            }
            ui.add_space(12.0);
            let state = if screen.id == self.own {
                "This computer"
            } else if self.connected(&screen.id) {
                "Connected securely"
            } else {
                "Waiting for connection"
            };
            ui.label(RichText::new(state).color(if self.connected(&screen.id) {
                t::GREEN
            } else {
                t::MUTED
            }));
            if let Some(address) = self
                .config
                .peers
                .iter()
                .find(|p| p.public_key == screen.id)
                .and_then(|p| p.address.as_deref())
            {
                t::hint(ui, address);
            }
            if screen.id == self.own {
                ui.add_space(20.0);
                t::small_title(ui, "PAIR ANOTHER COMPUTER");
                ui.label("Reachable address");
                ui.text_edit_singleline(&mut self.advertise);
                let copy =
                    ui.add_sized([width - 44.0, 36.0], egui::Button::new("Copy pairing code"));
                self.target("copy-code", copy.rect);
                if copy.clicked() {
                    let code = PairingCode {
                        version: 1,
                        name: self.config.name.clone(),
                        public_key: self.own.clone(),
                        address: self.advertise.clone(),
                    };
                    match code.encode() {
                        Ok(code) => {
                            ui.ctx().copy_text(code);
                            self.note("Pairing code copied");
                        }
                        Err(e) => self.error = Some(e.to_string()),
                    }
                }
                t::hint(
                    ui,
                    "Paste this code in Desklink on the other computer. Pair both computers before sharing.",
                );
            } else if editable {
                ui.add_space(20.0);
                if self.draft.screen(&screen.id).is_none() {
                    let place =
                        ui.add_sized([width - 44.0, 32.0], egui::Button::new("Place on desk"));
                    self.target("place", place.rect);
                    if place.clicked()
                        && let Err(error) =
                            self.place_screen(screen.id.clone(), screen.name.clone())
                    {
                        self.error = Some(error.to_string());
                    }
                    ui.add_space(8.0);
                }
                let forget = ui.add_sized(
                    [width - 44.0, 32.0],
                    egui::Button::new(RichText::new("Forget computer").color(t::RED)),
                );
                self.target("forget", forget.rect);
                if forget.clicked() {
                    self.config.peers.retain(|p| p.public_key != screen.id);
                    self.draft.screens.retain(|s| s.id != screen.id);
                    self.selected = self.own.clone();
                    self.mark_dirty();
                }
            }
        }
    }
}
