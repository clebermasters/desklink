//! The [`Desktop`] the sharing agent drives on Android.
use crate::{
    bridge::{Action, Bridge},
    keymap::{Modifiers, android_keycode},
};
use anyhow::{Context, Result, bail};
use desklink_platform::{Desktop, LocalEvent};
use desklink_protocol::{ClipboardContent, Geometry, Input};
use std::sync::Arc;

pub struct AndroidDesktop {
    bridge: Arc<Bridge>,
    modifiers: Modifiers,
}

impl AndroidDesktop {
    pub fn new(bridge: Arc<Bridge>) -> Self {
        Self {
            bridge,
            modifiers: Modifiers::default(),
        }
    }
}

impl Desktop for AndroidDesktop {
    fn geometry(&mut self) -> Result<Geometry> {
        self.bridge
            .geometry()
            .context("the Android app has not reported its screen size")
    }
    fn poll(&mut self) -> Result<Vec<LocalEvent>> {
        Ok(self
            .bridge
            .drain_clipboard()
            .into_iter()
            .map(LocalEvent::Clipboard)
            .collect())
    }
    fn capture(&mut self) -> Result<Vec<Input>> {
        bail!("an Android phone receives input but does not share its own")
    }
    /// Ends a receiving session: hide the pointer. The engine then releases held keys/buttons.
    fn release(&mut self) -> Result<()> {
        self.modifiers.release();
        self.bridge.push(Action::Hide)
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<()> {
        self.bridge.push(Action::Move { x, y })
    }
    fn inject(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Motion { .. } => Ok(()), // The engine converts motion into absolute warps.
            Input::Key { code, down } => {
                let meta = self.modifiers.update(code, down);
                match android_keycode(code) {
                    Some(keycode) => self.bridge.push(Action::Key {
                        keycode,
                        meta,
                        down,
                    }),
                    // Not an error: failing here would end the whole session over one key.
                    None => {
                        tracing::debug!("key without an Android equivalent ignored");
                        Ok(())
                    }
                }
            }
            Input::Button { button, down } => self.bridge.push(Action::Button { button, down }),
            Input::Scroll {
                horizontal,
                vertical,
            } => self.bridge.push(Action::Scroll {
                horizontal,
                vertical,
            }),
        }
    }
    fn owns_clipboard(&self) -> bool {
        true
    }
    fn clipboard_apply(&mut self, content: ClipboardContent) -> Result<()> {
        self.bridge.push(Action::Clipboard(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::Taken;
    use desklink_protocol::ClipboardKind;
    use std::time::Duration;

    fn desktop() -> (Arc<Bridge>, AndroidDesktop) {
        let bridge = Arc::new(Bridge::new());
        bridge.set_geometry(400, 800).unwrap();
        (bridge.clone(), AndroidDesktop::new(bridge))
    }
    fn drain(bridge: &Bridge) -> Vec<Action> {
        match bridge.take(Duration::ZERO) {
            Taken::Actions(a) => a,
            _ => vec![],
        }
    }

    #[test]
    fn engine_effects_become_app_actions() {
        let (bridge, mut d) = desktop();
        assert_eq!(d.geometry().unwrap().height, 800);
        d.warp(10, 20).unwrap();
        d.inject(Input::Key {
            code: 42,
            down: true,
        })
        .unwrap();
        d.inject(Input::Key {
            code: 30,
            down: true,
        })
        .unwrap();
        d.inject(Input::Key {
            code: 183,
            down: true,
        })
        .unwrap();
        d.inject(Input::Button {
            button: 1,
            down: true,
        })
        .unwrap();
        d.inject(Input::Scroll {
            horizontal: 0,
            vertical: -1,
        })
        .unwrap();
        d.release().unwrap();
        let actions = drain(&bridge);
        assert_eq!(actions[0], Action::Move { x: 10, y: 20 });
        assert!(
            matches!(actions[2], Action::Key { keycode: 29, meta, down: true } if meta & 1 == 1),
            "A is shifted: {:?}",
            actions[2]
        );
        assert_eq!(actions.len(), 6, "unmapped F13 was skipped: {actions:?}");
        assert_eq!(actions[5], Action::Hide);
    }

    #[test]
    fn phone_never_captures_and_owns_its_clipboard() {
        let (bridge, mut d) = desktop();
        assert!(d.capture().is_err());
        assert!(d.owns_clipboard());
        let content = ClipboardContent {
            kind: ClipboardKind::Text,
            bytes: b"copied".to_vec(),
        };
        bridge.local_clipboard(content.clone()).unwrap();
        assert!(matches!(&d.poll().unwrap()[..], [LocalEvent::Clipboard(c)] if *c == content));
        d.clipboard_apply(content.clone()).unwrap();
        assert_eq!(drain(&bridge), vec![Action::Clipboard(content)]);
    }

    #[test]
    fn geometry_must_be_reported_first() {
        let mut d = AndroidDesktop::new(Arc::new(Bridge::new()));
        assert!(d.geometry().is_err());
    }
}
