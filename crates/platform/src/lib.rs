//! Desktop-session input API. No privileged helpers or raw kernel input device access.
use anyhow::Result;
use desklink_protocol::{CaptureSpan, ClipboardContent, Edge, Geometry, Input};
use serde::Serialize;

pub mod keymap;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_policy;
#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
mod x11;

#[derive(Debug, Clone)]
pub enum LocalEvent {
    Pointer { x: i32, y: i32, dx: i32, dy: i32 },
    Input(Input),
    Clipboard(ClipboardContent),
    CaptureLost,
    Failure(String),
}

/// Establish modifiers before replaying held shortcuts or mouse buttons on a new screen.
#[cfg_attr(not(any(target_os = "linux", target_os = "macos")), allow(dead_code))]
fn order_held_inputs(held: &mut [Input]) {
    held.sort_by_key(|input| match input {
        Input::Key {
            code: 29 | 42 | 54 | 56 | 97 | 100 | 125 | 126,
            ..
        } => 0,
        Input::Key { .. } => 1,
        _ => 2,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_shortcuts_establish_modifiers_before_the_action_key() {
        let mut held = vec![
            Input::Key {
                code: 20,
                down: true,
            },
            Input::Button {
                button: 1,
                down: true,
            },
            Input::Key {
                code: 29,
                down: true,
            },
            Input::Key {
                code: 42,
                down: true,
            },
        ];
        order_held_inputs(&mut held);
        assert_eq!(
            held,
            vec![
                Input::Key {
                    code: 29,
                    down: true
                },
                Input::Key {
                    code: 42,
                    down: true
                },
                Input::Key {
                    code: 20,
                    down: true
                },
                Input::Button {
                    button: 1,
                    down: true
                }
            ]
        );
    }
}

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub backend: &'static str,
    pub capture: bool,
    pub inject: bool,
    pub geometry: Option<Geometry>,
    pub detail: String,
}

/// Owned by the main desktop thread; platform callbacks never perform network I/O.
pub trait Desktop {
    fn can_inject(&self) -> bool {
        true
    }
    fn can_update_capture_spans(&self) -> bool {
        true
    }
    fn set_capture_spans(&mut self, _spans: &[CaptureSpan]) -> Result<()> {
        Ok(())
    }
    fn geometry(&mut self) -> Result<Geometry>;
    fn poll(&mut self) -> Result<Vec<LocalEvent>>;
    fn capture(&mut self) -> Result<Vec<Input>>;
    fn release(&mut self) -> Result<()>;
    fn warp(&mut self, x: i32, y: i32) -> Result<()>;
    fn inject(&mut self, input: Input) -> Result<()>;
    fn owns_clipboard(&self) -> bool {
        false
    }
    fn clipboard_apply(&mut self, _content: ClipboardContent) -> Result<()> {
        anyhow::bail!("this backend uses the clipboard service")
    }
}

pub fn open(edges: &[Edge], clipboard: bool) -> Result<Box<dyn Desktop>> {
    let spans: Vec<_> = edges.iter().map(|edge| CaptureSpan::full(*edge)).collect();
    open_with_spans(&spans, clipboard)
}

pub fn open_with_spans(spans: &[CaptureSpan], clipboard: bool) -> Result<Box<dyn Desktop>> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland") {
            return Ok(Box::new(wayland::Wayland::open(spans.to_vec(), clipboard)?));
        }
        Ok(Box::new(x11::X11::open()?))
    }
    #[cfg(target_os = "macos")]
    {
        let _ = (spans, clipboard);
        Ok(Box::new(macos::MacDesktop::open()?))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (spans, clipboard);
        anyhow::bail!("only Linux and macOS are supported")
    }
}

pub fn capabilities() -> Capabilities {
    #[cfg(target_os = "macos")]
    {
        macos::capabilities()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Capabilities {
            backend: "embedded",
            capture: false,
            inject: false,
            geometry: None,
            detail: "this platform supplies its desktop through an embedding host".into(),
        }
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland") {
            return wayland::capabilities();
        }
        match open(&[], false).and_then(|mut d| d.geometry()) {
            Ok(geometry) => Capabilities {
                backend: "x11",
                capture: true,
                inject: true,
                geometry: Some(geometry),
                detail: "X11 session and XTEST are available; no root access required".into(),
            },
            Err(e) => Capabilities {
                backend: "unavailable",
                capture: false,
                inject: false,
                geometry: None,
                detail: format!("{e:#}"),
            },
        }
    }
}

pub fn request_permissions() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        macos::request_permissions();
    }
    Ok(())
}
