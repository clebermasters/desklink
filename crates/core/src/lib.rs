//! Deterministic sharing and clipboard state. No socket, platform or GUI dependencies.
#![forbid(unsafe_code)]
mod clipboard;
mod sharing;
pub use clipboard::ClipboardClock;
pub use sharing::{Effect, Engine};
