use crate::{
    agent, theme as t,
    tray::{self, Action},
};
use anyhow::{Context, Result, ensure};
use desklink_layout::{Geometry, Layout, Screen};
use desklink_settings::{
    Config,
    control::{Request, Status},
    identity_path,
    pairing::{PairingCode, advertised_address},
};
use desklink_transport::Identity;
use eframe::egui::{self, Align2, FontId, Id, Pos2, Rect, RichText, Sense, UiBuilder, Vec2};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Child,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct AddComputer {
    pub open: bool,
    pub code: String,
    pub name: String,
    pub address: String,
    pub key: String,
    pub use_controller: bool,
    pub error: Option<String>,
}
pub struct Drag {
    pub id: String,
    pub pointer: Pos2,
    pub rect: Geometry,
    pub resize: bool,
}

pub struct DesklinkApp {
    pub profile: PathBuf,
    pub config: Config,
    pub draft: Layout,
    pub own: String,
    pub selected: String,
    pub digest: String,
    pub dirty: bool,
    pub error: Option<String>,
    pub toast: Option<(String, Instant)>,
    pub add: AddComputer,
    pub zoom: f32,
    pub center: Vec2,
    pub fit: bool,
    pub drag: Option<Drag>,
    pub status: Status,
    pub client: agent::Client,
    pub child: Option<Child>,
    pub starting: bool,
    pub restarting: bool,
    pub restart_needed: bool,
    pub advertise: String,
    pub test_state: Option<PathBuf>,
    pub targets: BTreeMap<String, [f32; 4]>,
    last_export: Instant,
    last_disk: Instant,
    close_confirm: bool,
    details_open: bool,
    pub window_backend: String,
    pointer_debug: Option<[f32; 2]>,
    allow_close: bool,
    tray: Option<tray::Tray>,
    tray_initialized: bool,
    tray_hidden: bool,
    quit_requested: bool,
    quitting_since: Option<Instant>,
}

impl DesklinkApp {
    pub fn load(profile: PathBuf, test_state: Option<PathBuf>) -> Result<Self> {
        let idpath = identity_path(&profile);
        let identity = if idpath.exists() {
            Identity::load(&idpath)?
        } else {
            Identity::create(&idpath)?
        };
        if !profile.exists() {
            Config::new(
                std::env::var("HOSTNAME")
                    .ok()
                    .unwrap_or("This computer".into()),
                "0.0.0.0:24880".parse()?,
            )
            .save(&profile)?;
        }
        let config = Config::load(&profile)?;
        let draft = config.editable_layout(identity.public);
        let own = hex::encode(identity.public);
        let digest = Config::digest(&profile)?;
        let advertise = advertised_address(config.listen);
        let client = agent::Client::new(profile.clone());
        Ok(Self {
            profile,
            config,
            draft,
            own: own.clone(),
            selected: own,
            digest,
            dirty: false,
            error: None,
            toast: None,
            add: Default::default(),
            zoom: 1.0,
            center: Vec2::ZERO,
            fit: true,
            drag: None,
            status: Status::default(),
            client,
            child: None,
            starting: false,
            restarting: false,
            restart_needed: false,
            advertise,
            test_state,
            targets: BTreeMap::new(),
            last_export: Instant::now() - Duration::from_secs(1),
            last_disk: Instant::now(),
            close_confirm: false,
            details_open: false,
            window_backend: "unknown".into(),
            pointer_debug: None,
            allow_close: false,
            tray: None,
            tray_initialized: false,
            tray_hidden: false,
            quit_requested: false,
            quitting_since: None,
        })
    }
    pub fn editable(&self) -> bool {
        self.config.layout_owner.is_none()
    }
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.error = None;
    }
    pub fn note(&mut self, text: impl Into<String>) {
        self.toast = Some((text.into(), Instant::now()));
    }
    pub fn target(&mut self, name: impl Into<String>, rect: Rect) {
        self.targets.insert(
            name.into(),
            [rect.min.x, rect.min.y, rect.width(), rect.height()],
        );
    }
    pub fn save(&mut self) -> Result<()> {
        let mut next = self.config.clone();
        if self.editable() {
            self.draft.validate()?;
            self.draft.revision = self.config.layout.as_ref().map_or(1, |l| l.revision + 1);
            next.layout = Some(self.draft.clone());
        } else if next
            .layout
            .as_ref()
            .is_some_and(|l| Some(&l.owner) != next.layout_owner.as_ref())
        {
            next.layout = None;
        }
        next.validate_for(desklink_layout::key(&self.own)?)?;
        self.digest = next.save_checked(&self.profile, &self.digest)?;
        self.config = next;
        self.dirty = false;
        if self.status.running {
            self.client.send(Request::Apply);
        }
        self.note("Layout saved");
        Ok(())
    }
    pub fn reload(&mut self) -> Result<()> {
        self.config = Config::load(&self.profile)?;
        self.draft = self
            .config
            .editable_layout(desklink_layout::key(&self.own)?);
        self.digest = Config::digest(&self.profile)?;
        self.dirty = false;
        self.error = None;
        self.fit = true;
        Ok(())
    }
    fn start(&mut self) -> Result<()> {
        if self.dirty || self.config.layout.is_none() && self.editable() {
            self.save()?;
        }
        ensure!(!self.starting, "Sharing is already starting.");
        let lock = desklink_settings::lock_profile(&self.profile)
            .context("The previous sharing agent is still stopping.")?;
        drop(lock);
        self.child = Some(agent::spawn(&self.profile)?);
        self.starting = true;
        self.restart_needed = false;
        self.error = None;
        Ok(())
    }
    fn tick(&mut self) {
        while let Ok(reply) = self.client.replies.try_recv() {
            match reply.result {
                Ok(status) => {
                    if status.restart_required {
                        self.restart_needed = true;
                        if !self.restarting {
                            self.restarting = true;
                            self.client.send(Request::Stop);
                        }
                        self.note("Applying computer settings…");
                    } else if let Some(error) = &status.error {
                        self.error = Some(error.clone());
                    }
                    if status.running {
                        self.starting = false;
                    }
                    self.status = status;
                }
                Err(error) => {
                    if matches!(reply.request, Request::Status) {
                        self.status.running = false;
                    } else if !matches!(reply.request, Request::Stop) {
                        self.error = Some(error);
                    }
                }
            }
        }
        if let Some(child) = &mut self.child
            && let Ok(Some(exit)) = child.try_wait()
        {
            self.child = None;
            self.starting = false;
            if !exit.success() {
                self.error = Some(agent::last_error(&self.profile));
            }
        }
        if self.restarting
            && !self.status.running
            && !self.starting
            && desklink_settings::lock_profile(&self.profile).is_ok()
        {
            self.restarting = false;
            if let Err(e) = self.start() {
                self.error = Some(e.to_string());
            }
        }
        if self.last_disk.elapsed() > Duration::from_millis(600) {
            self.last_disk = Instant::now();
            if !self.dirty && Config::digest(&self.profile).is_ok_and(|d| d != self.digest) {
                let _ = self.reload();
            }
        }
    }
    fn show_window(&mut self, ctx: &egui::Context) {
        self.tray_hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }
    fn hide_window(&mut self, ctx: &egui::Context) {
        if self.tray.is_some() {
            self.tray_hidden = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }
    fn request_quit(&mut self, ctx: &egui::Context) {
        self.quit_requested = true;
        self.show_window(ctx);
        if self.dirty {
            self.close_confirm = true;
        } else {
            self.begin_quit();
        }
    }
    fn begin_quit(&mut self) {
        self.restarting = false;
        // A startup process may still be waiting for desktop consent, without a control socket.
        if self.starting
            && let Some(child) = &mut self.child
        {
            let _ = child.kill();
            let _ = child.wait();
            self.starting = false;
        }
        self.quitting_since = Some(Instant::now());
        self.client.send(Request::Stop);
    }
    fn background(&mut self, ctx: &egui::Context) {
        self.tick();
        if !self.tray_initialized {
            self.tray_initialized = true;
            let result = if self.window_backend == "wayland" {
                Err(anyhow::anyhow!(
                    "This Wayland window cannot be hidden and restored; Xwayland is required for tray minimization"
                ))
            } else {
                tray::Tray::new(ctx)
            };
            match result {
                Ok(tray) => self.tray = Some(tray),
                Err(error) => self.note(format!(
                    "Tray unavailable: {error}. Closing keeps sharing running."
                )),
            }
        }
        let actions = self.tray.as_ref().map(|t| t.actions()).unwrap_or_default();
        for action in actions {
            match action {
                Action::Open => self.show_window(ctx),
                Action::ToggleSharing if self.quitting_since.is_none() => {
                    if self.status.running {
                        self.client.send(Request::Stop);
                    } else if !self.starting
                        && let Err(error) = self.start()
                    {
                        self.error = Some(error.to_string());
                        self.show_window(ctx);
                    }
                }
                Action::ToggleSharing => {}
                Action::Quit => self.request_quit(ctx),
            }
        }
        if let Some(since) = self.quitting_since {
            // Wait for the stop request to complete before removing the tray icon.
            if !self.status.running && since.elapsed() > Duration::from_millis(750) {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if since.elapsed() > Duration::from_secs(4) {
                self.quitting_since = None;
                self.quit_requested = false;
                self.error =
                    Some("Sharing did not stop. Stop sharing and try quitting again.".into());
                self.show_window(ctx);
            }
        }
        if let Some(tray) = &mut self.tray {
            let active = self
                .status
                .active_computer
                .as_deref()
                .and_then(|key| self.config.peers.iter().find(|p| p.public_key == key))
                .map(|p| p.name.as_str())
                .unwrap_or(&self.config.name);
            tray.update(self.status.running, self.starting, active);
        }
        if !self.allow_close && self.quit_requested && ctx.input(|i| i.viewport().close_requested())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if !self.allow_close && self.tray.is_some() && !self.quit_requested {
            let (close, minimized) = ctx.input(|i| {
                (
                    i.viewport().close_requested(),
                    i.viewport().minimized == Some(true),
                )
            });
            if close || minimized && !self.tray_hidden {
                self.hide_window(ctx);
            }
        }
        ctx.request_repaint_after(Duration::from_millis(250));
    }
    fn header(&mut self, ui: &mut egui::Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, t::WHITE);
        ui.painter()
            .hline(rect.x_range(), rect.bottom(), (1.0, t::BORDER));
        t::monitor(
            ui.painter(),
            egui::pos2(rect.left() + 35.0, rect.center().y - 3.0),
            26.0,
            t::BLUE,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 60.0, rect.center().y - 8.0),
            Align2::LEFT_CENTER,
            "Desklink",
            FontId::proportional(22.0),
            t::INK,
        );
        ui.painter().text(
            egui::pos2(rect.left() + 61.0, rect.center().y + 13.0),
            Align2::LEFT_CENTER,
            "YOUR COMPUTERS, ONE DESK",
            FontId::proportional(9.0),
            t::MUTED,
        );
        let text = if self.restarting {
            "Applying settings…"
        } else if self.starting {
            "Approve desktop access"
        } else if self.status.running {
            "Sharing is active"
        } else {
            "Sharing is off"
        };
        let color = if self.status.running {
            t::GREEN
        } else {
            t::MUTED
        };
        ui.painter().circle_filled(
            egui::pos2(rect.right() - 352.0, rect.center().y),
            4.0,
            color,
        );
        ui.painter().text(
            egui::pos2(rect.right() - 339.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(13.0),
            color,
        );
        let button = Rect::from_min_size(
            egui::pos2(rect.right() - 184.0, rect.top() + 18.0),
            egui::vec2(160.0, 38.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(button), |ui| {
            let label = if self.restarting {
                "Applying settings…"
            } else if self.starting {
                "Cancel startup"
            } else if self.restart_needed && self.status.running {
                "Restart sharing"
            } else if self.status.running {
                "Stop sharing"
            } else {
                "Start sharing"
            };
            let enabled = !self.restarting
                && (self.status.running || self.starting || !self.config.peers.is_empty());
            let response = ui.add_enabled(
                enabled,
                egui::Button::new(RichText::new(label).color(t::WHITE))
                    .fill(t::BLUE)
                    .min_size(button.size()),
            );
            self.target("sharing", response.rect);
            if response.clicked() {
                if self.starting {
                    if let Some(child) = &mut self.child {
                        let _ = child.kill();
                    }
                    self.starting = false;
                } else if self.status.running {
                    self.restarting = self.restart_needed;
                    self.client.send(Request::Stop);
                } else if let Err(error) = self.start() {
                    self.error = Some(error.to_string());
                }
            }
        });
    }
    fn sidebar(&mut self, ui: &mut egui::Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, t::WHITE);
        ui.painter()
            .vline(rect.right(), rect.y_range(), (1.0, t::BORDER));
        ui.scope_builder(
            UiBuilder::new().max_rect(rect.shrink2(egui::vec2(20.0, 24.0))),
            |ui| {
                t::small_title(ui, "WORKSPACE");
                ui.label(RichText::new("Screen layout").strong().size(16.0));
                ui.add_space(24.0);
                let mut screens = self.draft.screens.clone();
                for peer in &self.config.peers {
                    if !screens.iter().any(|s| s.id == peer.public_key) {
                        screens.push(Screen {
                            id: peer.public_key.clone(),
                            name: peer.name.clone(),
                            rect: Geometry {
                                x: 0,
                                y: 0,
                                width: 240,
                                height: 150,
                            },
                        });
                    }
                }
                ui.horizontal(|ui| {
                    t::small_title(ui, "COMPUTERS");
                    ui.label(
                        RichText::new(screens.len().to_string())
                            .size(11.0)
                            .color(t::MUTED),
                    );
                });
                egui::ScrollArea::vertical()
                    .id_salt("computers-scroll")
                    .max_height((rect.height() - 300.0).max(100.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for screen in screens {
                            let (r, response) = ui.allocate_exact_size(
                                egui::vec2(rect.width() - 40.0, 58.0),
                                Sense::click(),
                            );
                            if self.selected == screen.id {
                                ui.painter().rect_filled(r, 7.0, t::BLUE_SOFT);
                            }
                            t::monitor(
                                ui.painter(),
                                egui::pos2(r.left() + 20.0, r.center().y - 4.0),
                                21.0,
                                if self.selected == screen.id {
                                    t::BLUE
                                } else {
                                    t::MUTED
                                },
                            );
                            ui.painter().text(
                                egui::pos2(r.left() + 41.0, r.top() + 18.0),
                                Align2::LEFT_CENTER,
                                short(&screen.name, 19),
                                FontId::proportional(13.0),
                                t::INK,
                            );
                            let state = if screen.id == self.own {
                                "This computer"
                            } else if self.connected(&screen.id) {
                                "Connected"
                            } else if self.status.running {
                                "Not connected"
                            } else {
                                "Paired"
                            };
                            ui.painter().text(
                                egui::pos2(r.left() + 41.0, r.top() + 38.0),
                                Align2::LEFT_CENTER,
                                state,
                                FontId::proportional(11.0),
                                if self.connected(&screen.id) {
                                    t::GREEN
                                } else {
                                    t::MUTED
                                },
                            );
                            self.target(format!("computer:{}", screen.id), r);
                            if response.clicked() {
                                self.selected = screen.id;
                            }
                        }
                    });
                ui.add_space(12.0);
                let add = ui.add_sized(
                    [rect.width() - 40.0, 38.0],
                    egui::Button::new("+  Add computer"),
                );
                self.target("add", add.rect);
                if add.clicked() {
                    self.add.open = true;
                }
                ui.add_space(22.0);
                ui.separator();
                ui.add_space(12.0);
                t::small_title(ui, "CLIPBOARD");
                let check = ui.checkbox(&mut self.config.clipboard, "Share text & images");
                self.target("clipboard", check.rect);
                if check.changed() {
                    self.mark_dirty();
                }
                t::hint(ui, "Copy on one computer. Paste on another.");
            },
        );
    }
    pub fn connected(&self, id: &str) -> bool {
        self.status.running
            && (id == self.own
                || self
                    .status
                    .peers
                    .iter()
                    .any(|p| p.public_key == id && p.connected))
    }

    fn add_dialog(&mut self, ctx: &egui::Context) {
        if !self.add.open {
            return;
        }
        egui::Modal::new(Id::unique("add-computer")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Add a computer");
            t::hint(
                ui,
                "Paste its pairing code from Desklink's computer details.",
            );
            ui.add_space(12.0);
            let edit = ui.add(
                egui::TextEdit::multiline(&mut self.add.code)
                    .desired_width(440.0)
                    .desired_rows(3)
                    .hint_text("desklink://pair/…"),
            );
            self.target("pair-code", edit.rect);
            if edit.changed()
                && let Ok(code) = PairingCode::decode(&self.add.code)
            {
                self.add.name = code.name;
                self.add.address = code.address;
                self.add.key = code.public_key;
                self.add.error = None;
            }
            ui.collapsing("Connection details", |ui| {
                ui.label("Computer name");
                ui.text_edit_singleline(&mut self.add.name);
                ui.label("Address");
                ui.text_edit_singleline(&mut self.add.address);
                ui.label("Public pairing key");
                ui.text_edit_singleline(&mut self.add.key);
            });
            if !self.add.name.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new(&self.add.name).strong());
                t::hint(ui, &self.add.address);
            }
            ui.checkbox(
                &mut self.add.use_controller,
                "Use its keyboard and screen layout",
            );
            if let Some(error) = &self.add.error {
                ui.colored_label(t::RED, error);
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    self.add.open = false;
                }
                let add = ui.add(
                    egui::Button::new(RichText::new("Add computer").color(t::WHITE)).fill(t::BLUE),
                );
                self.target("confirm-add", add.rect);
                if add.clicked() {
                    match self.add_peer() {
                        Ok(()) => self.add = Default::default(),
                        Err(e) => self.add.error = Some(e.to_string()),
                    }
                }
            });
        });
    }
    fn add_peer(&mut self) -> Result<()> {
        let code = PairingCode {
            version: 1,
            name: self.add.name.clone(),
            public_key: self.add.key.clone(),
            address: self.add.address.clone(),
        };
        code.validate()?;
        ensure!(
            code.public_key != self.own,
            "This is the current computer. Paste the other computer's code."
        );
        ensure!(
            !self
                .config
                .peers
                .iter()
                .any(|p| p.public_key == code.public_key),
            "This computer is already paired."
        );
        let mut next = self.config.clone();
        next.peers.push(code.peer());
        next.validate()?;
        if self.add.use_controller {
            next.layout_owner = Some(code.public_key.clone());
            next.layout = None;
            self.draft = Layout::single(self.own.clone(), self.config.name.clone());
        } else if self.editable() {
            self.place_screen(code.public_key.clone(), code.name.clone())?;
        }
        self.config = next;
        self.mark_dirty();
        self.fit = true;
        Ok(())
    }
    pub(crate) fn place_screen(&mut self, id: String, name: String) -> Result<()> {
        let anchor = self
            .draft
            .screens
            .iter()
            .max_by_key(|s| s.rect.x + s.rect.width as i32)
            .context("The layout has no local computer.")?;
        let mut next = self.draft.clone();
        next.screens.push(Screen {
            id: id.clone(),
            name,
            rect: Geometry {
                x: anchor.rect.x + anchor.rect.width as i32,
                y: anchor.rect.y,
                width: 240,
                height: 150,
            },
        });
        next.validate()?;
        self.draft = next;
        self.selected = id;
        self.mark_dirty();
        self.fit = true;
        Ok(())
    }
    fn footer(&mut self, ui: &mut egui::Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 0.0, t::WHITE);
        ui.painter()
            .hline(rect.x_range(), rect.top(), (1.0, t::BORDER));
        let mut text = "Ctrl + Alt + Shift + Escape returns control to this computer.".to_owned();
        if self.status.running {
            text = if self.tray.is_some() {
                "Close or minimize to keep Desklink in the tray. Use the tray menu to reopen or quit.".into()
            } else {
                "Sharing stays on when this window closes. Use Stop sharing to disconnect.".into()
            };
        }
        if let Some((toast, at)) = &self.toast
            && at.elapsed() < Duration::from_secs(5)
        {
            text = toast.clone();
        }
        ui.painter().text(
            egui::pos2(rect.left() + 24.0, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            FontId::proportional(11.0),
            t::MUTED,
        );
        ui.painter().text(
            egui::pos2(rect.right() - 24.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("Desklink {}", env!("CARGO_PKG_VERSION")),
            FontId::proportional(11.0),
            t::MUTED,
        );
    }
    fn export(&mut self, size: Vec2) {
        if self.last_export.elapsed() < Duration::from_millis(150) {
            return;
        }
        self.last_export = Instant::now();
        if let Some(path) = &self.test_state {
            let state = serde_json::json!({"size":[size.x,size.y],"window_backend":self.window_backend,"tray_available":self.tray.is_some(),"tray_hidden":self.tray_hidden,"pointer":self.pointer_debug,"targets":self.targets,"layout":self.draft,"dirty":self.dirty,"error":self.error,"layout_error":self.draft.validate().err().map(|e|e.to_string()),"running":self.status.running,"status":self.status,"starting":self.starting,"agent_pid":self.child.as_ref().map(Child::id),"selected":self.selected});
            if let Ok(bytes) = serde_json::to_vec(&state) {
                let temporary = path.with_extension("tmp");
                if std::fs::write(&temporary, bytes).is_ok() {
                    let _ = std::fs::rename(temporary, path);
                }
            }
        }
    }
}

impl eframe::App for DesklinkApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.background(ctx);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.pointer_debug = ui.input(|i| i.pointer.hover_pos().map(|p| [p.x, p.y]));
        self.targets.clear();
        ui.ctx().request_repaint_after(Duration::from_millis(180));
        let rect = ui.max_rect();
        let compact = rect.width() < 1120.0;
        if !compact {
            self.details_open = false;
        }
        ui.painter().rect_filled(rect, 0.0, t::BG);
        let header = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + 76.0));
        let footer = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 36.0), rect.max);
        let left = Rect::from_min_max(
            egui::pos2(rect.left(), header.bottom()),
            egui::pos2(
                rect.left() + if compact { 200.0 } else { 236.0 },
                footer.top(),
            ),
        );
        let right = Rect::from_min_max(
            egui::pos2(
                rect.right() - if compact { 0.0 } else { 270.0 },
                header.bottom(),
            ),
            egui::pos2(rect.right(), footer.top()),
        );
        self.header(ui, header);
        self.sidebar(ui, left);
        if !compact {
            self.inspector(ui, right);
        }
        let main = Rect::from_min_max(
            egui::pos2(left.right() + 26.0, header.bottom() + 25.0),
            egui::pos2(right.left() - 26.0, footer.top() - 20.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(main),|ui|{
            ui.heading("Arrange your desk");t::hint(ui,if self.editable(){"Drag computers to match their positions. Touching edges let your pointer cross."}else{"This arrangement is managed by your selected controller."});ui.add_space(16.0);
            ui.horizontal(|ui|{
                if self.tray.is_some() {
                    let hide=ui.button("Minimize to tray");self.target("hide-to-tray",hide.rect);
                    if hide.clicked(){self.hide_window(ui.ctx());}
                }
                let fit=ui.button("Fit view");self.target("fit",fit.rect);if fit.clicked(){self.fit=true;}
                if compact {let details=ui.button("Details");self.target("details",details.rect);if details.clicked(){self.details_open=true;}}
                ui.add(egui::Slider::new(&mut self.zoom,0.35..=1.8).show_value(false).text("Zoom"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.label(RichText::new(if self.dirty{"Unsaved changes"}else{"Layout saved"}).size(12.0).color(if self.dirty{t::AMBER}else{t::MUTED}));});
            });
            ui.add_space(12.0);
            let available=ui.available_rect_before_wrap();let canvas=Rect::from_min_max(available.min,egui::pos2(available.right(),available.bottom()-98.0));
            self.canvas(ui,canvas);
            ui.advance_cursor_after_rect(canvas);ui.add_space(12.0);
            let validation=if self.editable(){self.draft.validate().err().map(|e|e.to_string())}else{None};
            if let Some(error)=&self.error {ui.colored_label(t::RED,error);}else if let Some(error)=&validation{ui.colored_label(t::AMBER,error);}else{t::hint(ui,if self.editable(){"Tip: resize the corner of a card to match your screen proportions."}else{"Start sharing to receive layout updates from the controller."});}
            ui.horizontal(|ui|{
                let reload=ui.button("Reload");self.target("reload",reload.rect);if reload.clicked()&&let Err(e)=self.reload(){self.error=Some(e.to_string());}
                let enabled=self.dirty&&validation.is_none();let save=ui.add_enabled(enabled,egui::Button::new(RichText::new("Save layout").color(t::WHITE)).fill(t::BLUE));self.target("save",save.rect);
                if save.clicked()&&let Err(e)=self.save(){self.error=Some(e.to_string());}
            });
        });
        self.footer(ui, footer);
        if compact && self.details_open {
            egui::Modal::new(Id::unique("computer-details")).show(ui.ctx(), |ui| {
                ui.set_width(340.0);
                ui.horizontal(|ui| {
                    ui.heading("Computer details");
                    if ui.button("Close").clicked() {
                        self.details_open = false;
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("details-scroll")
                    .max_height((rect.height() - 150.0).max(200.0))
                    .show(ui, |ui| self.inspector_contents(ui, 384.0));
            });
        }
        self.add_dialog(ui.ctx());
        if self.fit {
            ui.ctx().request_repaint();
        }
        if ui.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && self.dirty
            && self.tray.is_none()
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_confirm = true;
        }
        if self.close_confirm {
            egui::Modal::new(Id::unique("unsaved")).show(ui.ctx(), |ui| {
                ui.heading("Save your layout?");
                ui.label("Your arrangement has unsaved changes.");
                ui.horizontal(|ui| {
                    if ui.button("Keep editing").clicked() {
                        self.close_confirm = false;
                        self.quit_requested = false;
                    }
                    if ui
                        .button(if self.quit_requested {
                            "Discard & quit"
                        } else {
                            "Discard & close"
                        })
                        .clicked()
                    {
                        self.close_confirm = false;
                        if self.quit_requested {
                            self.dirty = false;
                            self.begin_quit();
                        } else {
                            self.allow_close = true;
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                    if ui
                        .button(if self.quit_requested {
                            "Save & quit"
                        } else {
                            "Save & close"
                        })
                        .clicked()
                    {
                        match self.save() {
                            Ok(()) => {
                                self.close_confirm = false;
                                if self.quit_requested {
                                    self.begin_quit();
                                } else {
                                    self.allow_close = true;
                                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                                }
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                });
            });
        }
        self.export(rect.size());
    }
}

pub fn short(text: &str, limit: usize) -> String {
    let mut result: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        result.push('…');
    }
    result
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    #[test]
    fn quitting_with_unsaved_edits_preserves_them_until_the_user_decides() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = DesklinkApp::load(temp.path().join("config.toml"), None).unwrap();
        app.mark_dirty();
        app.status.running = true;
        app.request_quit(&egui::Context::default());
        assert!(app.close_confirm);
        assert!(app.dirty);
        assert!(app.status.running);
        assert!(app.quitting_since.is_none());
        assert!(!app.allow_close);
    }

    #[test]
    fn quitting_waits_for_sharing_to_stop_before_closing() {
        let temp = tempfile::tempdir().unwrap();
        let mut app = DesklinkApp::load(temp.path().join("config.toml"), None).unwrap();
        app.tray_initialized = true;
        app.status.running = true;
        app.request_quit(&egui::Context::default());
        assert!(app.quitting_since.is_some());
        assert!(!app.allow_close);
        app.quitting_since = Some(Instant::now() - Duration::from_secs(1));
        app.background(&egui::Context::default());
        assert!(!app.allow_close);
        app.status.running = false;
        app.background(&egui::Context::default());
        assert!(app.allow_close);
    }
}
