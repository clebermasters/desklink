//! Native cross-platform layout editor, independent of the input-capture process.
mod agent;
mod app;
mod canvas;
mod inspector;
mod theme;
mod tray;
use anyhow::{Context, Result};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use std::path::PathBuf;

pub fn run(profile: PathBuf, test_state: Option<PathBuf>) -> Result<()> {
    let fullscreen =
        test_state.is_some() && std::env::var("DESKLINK_UI_TEST_FULLSCREEN").as_deref() == Ok("1");
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Desklink — Screen layout")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1000.0, 680.0])
            .with_fullscreen(fullscreen)
            .with_decorations(!fullscreen),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    // Winit's native Wayland set_visible/unminimize are unsupported. Use Xwayland
    // only for the editor when a tray host exists; the input worker stays on Wayland.
    #[cfg(target_os = "linux")]
    if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
        && std::env::var_os("DISPLAY").is_some()
        && tray::require_host().is_ok()
    {
        options.event_loop_builder = Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        }));
    }
    let mut application = app::DesklinkApp::load(profile, test_state)?;
    eframe::run_native(
        "org.desklink.Desklink",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            application.window_backend = cc
                .winit_window()
                .and_then(|window| {
                    window.display_handle().ok().map(|h| match h.as_raw() {
                        RawDisplayHandle::Wayland(_) => "wayland",
                        RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => "x11",
                        RawDisplayHandle::AppKit(_) => "macos",
                        _ => "other",
                    })
                })
                .unwrap_or("unknown")
                .into();
            Ok(Box::new(application))
        }),
    )
    .map_err(|e| anyhow::anyhow!(e.to_string()))
    .context("Could not open the Desklink window")
}
