//! Native cross-platform layout editor, independent of the input-capture process.
mod agent;
mod app;
mod canvas;
mod inspector;
mod theme;
use anyhow::{Context, Result};
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use std::path::PathBuf;

pub fn run(profile: PathBuf, test_state: Option<PathBuf>) -> Result<()> {
    let fullscreen =
        test_state.is_some() && std::env::var("DESKLINK_UI_TEST_FULLSCREEN").as_deref() == Ok("1");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Desklink — Screen layout")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([1000.0, 680.0])
            .with_fullscreen(fullscreen)
            .with_decorations(!fullscreen),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
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
