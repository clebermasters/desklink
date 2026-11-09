//! Native Linux StatusNotifierItem and macOS menu bar, owned by the GUI thread.
use anyhow::Result;
use eframe::egui;
use std::sync::mpsc::{self, Receiver};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[cfg(target_os = "linux")]
pub fn require_host() -> Result<()> {
    let connection = zbus::blocking::Connection::session()?;
    let watcher = zbus::blocking::Proxy::new(
        &connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )?;
    anyhow::ensure!(
        watcher.get_property::<bool>("IsStatusNotifierHostRegistered")?,
        "No system tray host is available on this desktop"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    ToggleSharing,
    Quit,
}

pub struct Tray {
    _icon: TrayIcon,
    toggle: MenuItem,
    status: MenuItem,
    actions: Receiver<Action>,
    last_status: String,
    last_running: bool,
}
impl Tray {
    pub fn new(ctx: &egui::Context) -> Result<Self> {
        #[cfg(target_os = "linux")]
        require_host()?;
        let menu = Menu::new();
        let status = MenuItem::new("Desklink — sharing is off", false, None);
        let open = MenuItem::new("Open Desklink", true, None);
        let toggle = MenuItem::new("Start sharing", true, None);
        let quit = MenuItem::new("Quit Desklink", true, None);
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &open,
            &toggle,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let (tx, actions) = mpsc::channel();
        let (open_id, toggle_id, quit_id) =
            (open.id().clone(), toggle.id().clone(), quit.id().clone());
        let repaint = ctx.clone();
        let menu_tx = tx.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let action = if event.id == open_id {
                Some(Action::Open)
            } else if event.id == toggle_id {
                Some(Action::ToggleSharing)
            } else if event.id == quit_id {
                Some(Action::Quit)
            } else {
                None
            };
            if let Some(action) = action {
                let _ = menu_tx.send(action);
                repaint.request_repaint();
            }
        }));
        let repaint = ctx.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            if cfg!(target_os = "linux")
                && matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                )
            {
                let _ = tx.send(Action::Open);
                repaint.request_repaint();
            }
        }));
        let builder = TrayIconBuilder::new()
            .with_id("desklink")
            .with_tooltip("Desklink")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(cfg!(target_os = "macos"));
        let pixels = Icon::from_rgba(icon_rgba(), 32, 32)?;
        #[cfg(target_os = "macos")]
        let builder = builder.with_icon_templated(pixels);
        #[cfg(not(target_os = "macos"))]
        let builder = builder.with_icon(pixels);
        let icon = builder.build()?;
        Ok(Self {
            _icon: icon,
            toggle,
            status,
            actions,
            last_status: String::new(),
            last_running: false,
        })
    }
    pub fn actions(&self) -> Vec<Action> {
        self.actions.try_iter().collect()
    }
    pub fn update(&mut self, running: bool, starting: bool, active: &str) {
        let status = if starting {
            "Desklink — starting…".to_owned()
        } else if running {
            format!("Controlling {active}")
        } else {
            "Desklink — sharing is off".to_owned()
        };
        if status != self.last_status {
            self.status.set_text(&status);
            self.last_status = status;
        }
        self.toggle.set_enabled(!starting);
        if running != self.last_running {
            self.toggle.set_text(if running {
                "Stop sharing"
            } else {
                "Start sharing"
            });
            self.last_running = running;
        }
    }
}

// Two linked monitors. A monochrome template follows the macOS menu bar appearance.
fn icon_rgba() -> Vec<u8> {
    let mut pixels = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            let outline = |left: usize, top: usize| {
                (left..left + 18).contains(&x)
                    && (top..top + 13).contains(&y)
                    && (x < left + 2 || x >= left + 16 || y < top + 2 || y >= top + 11)
            };
            if outline(1, 3) || outline(13, 15) || ((14..18).contains(&x) && (13..19).contains(&y))
            {
                let offset = (y * 32 + x) * 4;
                let color = if cfg!(target_os = "macos") {
                    [0, 0, 0, 255]
                } else {
                    [75, 151, 255, 255]
                };
                pixels[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
    pixels
}
