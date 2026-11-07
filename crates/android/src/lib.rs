//! Android receiver for Desklink.
//!
//! The Rust side runs the unmodified sharing agent (`desklink::runtime`) with an Android
//! [`Desktop`](desklink_platform::Desktop) whose actions are queued on a [`bridge::Bridge`].
//! The Kotlin app drains that queue and performs each action through the input backend the
//! user enabled. Android never captures local input: a phone is a receiver.
pub mod agent;
pub mod bridge;
pub mod desktop;
pub mod keymap;
pub mod profile;

#[cfg(target_os = "android")]
mod jni_api;
#[cfg(target_os = "android")]
mod logcat;
