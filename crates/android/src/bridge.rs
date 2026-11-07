//! Bounded hand-off between the sharing agent thread and the Android app's input pump.
use anyhow::{Result, ensure};
use desklink_hid::{Report, ReportSink};
use desklink_protocol::{ClipboardContent, ClipboardKind, Geometry};
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::Duration,
};

/// A queue that the app cannot drain within this many actions is treated as stalled. The
/// agent then returns control to the controller rather than piling up stale input.
pub const MAX_ACTIONS: usize = 2048;
const MAX_LOCAL: usize = 4;

/// Work for the app to perform. Coordinates are in the logical units advertised in `Hello`.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Move {
        x: i32,
        y: i32,
    },
    Hide,
    Button {
        button: u8,
        down: bool,
    },
    Key {
        keycode: i32,
        meta: i32,
        down: bool,
    },
    Scroll {
        horizontal: i32,
        vertical: i32,
    },
    Clipboard(ClipboardContent),
    /// A Bluetooth keyboard or mouse report for the computer the phone presents itself to.
    Report(Report),
}

#[derive(Debug, PartialEq)]
pub enum Taken {
    Actions(Vec<Action>),
    Timeout,
    Closed,
}

#[derive(Default)]
struct State {
    actions: VecDeque<Action>,
    clipboard: VecDeque<ClipboardContent>,
    geometry: Option<Geometry>,
    closed: bool,
}

#[derive(Default)]
pub struct Bridge {
    state: Mutex<State>,
    wake: Condvar,
}

impl Bridge {
    pub fn new() -> Self {
        Self::default()
    }
    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic while holding the lock cannot leave the queues logically inconsistent.
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// Clears queued work from an earlier agent run. Geometry is kept.
    pub fn reopen(&self) {
        let mut state = self.lock();
        state.actions.clear();
        state.clipboard.clear();
        state.closed = false;
    }
    /// Wakes the app's pump so it can stop.
    pub fn close(&self) {
        self.lock().closed = true;
        self.wake.notify_all();
    }
    pub fn push(&self, action: Action) -> Result<()> {
        let mut state = self.lock();
        // Only the newest pointer position matters, unless a button event is in between.
        if let Some(last) = state.actions.back_mut()
            && supersedes(&action, last)
        {
            *last = action;
        } else {
            ensure!(
                state.actions.len() < MAX_ACTIONS,
                "the Android input service is not responding"
            );
            state.actions.push_back(action);
        }
        drop(state);
        self.wake.notify_one();
        Ok(())
    }
    pub fn take(&self, timeout: Duration) -> Taken {
        let state = self.lock();
        let (mut state, _) = self
            .wake
            .wait_timeout_while(state, timeout, |s| s.actions.is_empty() && !s.closed)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !state.actions.is_empty() {
            Taken::Actions(state.actions.drain(..).collect())
        } else if state.closed {
            Taken::Closed
        } else {
            Taken::Timeout
        }
    }
    pub fn set_geometry(&self, width: u32, height: u32) -> Result<()> {
        let geometry = Geometry {
            x: 0,
            y: 0,
            width,
            height,
        };
        ensure!(geometry.valid(), "invalid screen size {width}x{height}");
        self.lock().geometry = Some(geometry);
        Ok(())
    }
    pub fn geometry(&self) -> Option<Geometry> {
        self.lock().geometry
    }
    /// A copy made on the phone. Older unsent copies are superseded, never queued without bound.
    pub fn local_clipboard(&self, content: ClipboardContent) -> Result<()> {
        ensure!(
            !content.bytes.is_empty() && content.valid(),
            "clipboard content is empty, too large or malformed"
        );
        let mut state = self.lock();
        if state.clipboard.len() >= MAX_LOCAL {
            state.clipboard.pop_front();
        }
        state.clipboard.push_back(content);
        Ok(())
    }
    pub fn drain_clipboard(&self) -> Vec<ClipboardContent> {
        self.lock().clipboard.drain(..).collect()
    }
}

const MOVE: u8 = 1;
const HIDE: u8 = 2;
const BUTTON: u8 = 3;
const KEY: u8 = 4;
const SCROLL: u8 = 5;
const CLIPBOARD: u8 = 6;
const REPORT: u8 = 7;

/// Whether `next` makes the queued `last` redundant.
fn supersedes(next: &Action, last: &Action) -> bool {
    match (next, last) {
        (Action::Move { .. }, Action::Move { .. }) => true,
        // Consecutive absolute positions; wheel and button reports in between keep their order.
        (Action::Report(next), Action::Report(last)) => {
            next.position_only && last.position_only && next.id == last.id
        }
        _ => false,
    }
}

/// Big-endian records, read with `java.nio.ByteBuffer` on the app side.
pub fn encode(actions: &[Action]) -> Vec<u8> {
    let mut out = Vec::with_capacity(actions.len() * 13);
    for action in actions {
        match action {
            Action::Move { x, y } => {
                out.push(MOVE);
                out.extend(x.to_be_bytes());
                out.extend(y.to_be_bytes());
            }
            Action::Hide => out.push(HIDE),
            Action::Button { button, down } => out.extend([BUTTON, *button, u8::from(*down)]),
            Action::Key {
                keycode,
                meta,
                down,
            } => {
                out.push(KEY);
                out.extend(keycode.to_be_bytes());
                out.extend(meta.to_be_bytes());
                out.push(u8::from(*down));
            }
            Action::Scroll {
                horizontal,
                vertical,
            } => {
                out.push(SCROLL);
                out.extend(horizontal.to_be_bytes());
                out.extend(vertical.to_be_bytes());
            }
            Action::Clipboard(content) => {
                out.push(CLIPBOARD);
                out.push(kind_code(content.kind));
                out.extend((content.bytes.len() as u32).to_be_bytes());
                out.extend(&content.bytes);
            }
            Action::Report(report) => {
                out.extend([REPORT, report.id, report.data.len() as u8]);
                out.extend(&report.data);
            }
        }
    }
    out
}

/// Feeds a [`desklink_hid::HidDesktop`]'s reports to the app, which sends them over Bluetooth.
pub struct BridgeSink(pub Arc<Bridge>);

impl ReportSink for BridgeSink {
    fn send(&mut self, report: Report) -> Result<()> {
        ensure!(report.data.len() <= u8::MAX as usize, "report too large");
        self.0.push(Action::Report(report))
    }
}

pub fn kind_code(kind: ClipboardKind) -> u8 {
    match kind {
        ClipboardKind::Text => 0,
        ClipboardKind::Png => 1,
    }
}

pub fn kind_from_code(code: i32) -> Result<ClipboardKind> {
    match code {
        0 => Ok(ClipboardKind::Text),
        1 => Ok(ClipboardKind::Png),
        _ => anyhow::bail!("unknown clipboard format {code}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, thread, time::Instant};

    #[test]
    fn pointer_moves_coalesce_but_not_across_buttons() {
        let bridge = Bridge::new();
        bridge.push(Action::Move { x: 1, y: 1 }).unwrap();
        bridge.push(Action::Move { x: 2, y: 2 }).unwrap();
        bridge
            .push(Action::Button {
                button: 1,
                down: true,
            })
            .unwrap();
        bridge.push(Action::Move { x: 3, y: 3 }).unwrap();
        bridge.push(Action::Move { x: 4, y: 4 }).unwrap();
        assert_eq!(
            bridge.take(Duration::ZERO),
            Taken::Actions(vec![
                Action::Move { x: 2, y: 2 },
                Action::Button {
                    button: 1,
                    down: true
                },
                Action::Move { x: 4, y: 4 },
            ])
        );
        assert_eq!(bridge.take(Duration::ZERO), Taken::Timeout);
    }

    fn position(x: u8) -> Action {
        Action::Report(Report {
            id: 2,
            data: vec![x, 0, 0, 0, 0, 0],
            position_only: true,
        })
    }
    fn buttons(state: u8) -> Action {
        Action::Report(Report {
            id: 3,
            data: vec![state, 0, 0, 0, 0, 0, 0],
            position_only: false,
        })
    }

    #[test]
    fn pointer_positions_coalesce_but_never_across_buttons() {
        let bridge = Bridge::new();
        bridge.push(position(1)).unwrap();
        bridge.push(position(2)).unwrap();
        bridge.push(buttons(1)).unwrap(); // a press is never dropped or reordered
        bridge.push(position(3)).unwrap(); // dragging
        bridge.push(position(4)).unwrap();
        bridge.push(buttons(0)).unwrap();
        bridge.push(buttons(0)).unwrap(); // only positions coalesce
        assert_eq!(
            bridge.take(Duration::ZERO),
            Taken::Actions(vec![
                position(2),
                buttons(1),
                position(4),
                buttons(0),
                buttons(0)
            ])
        );
    }

    #[test]
    fn stalled_consumer_is_reported_instead_of_growing_forever() {
        let bridge = Bridge::new();
        for _ in 0..MAX_ACTIONS {
            bridge.push(Action::Hide).unwrap();
        }
        assert!(bridge.push(Action::Hide).is_err());
        // A pointer move cannot coalesce into a non-move tail either.
        assert!(bridge.push(Action::Move { x: 0, y: 0 }).is_err());
    }

    #[test]
    fn close_wakes_a_blocked_consumer() {
        let bridge = Arc::new(Bridge::new());
        let waiter = bridge.clone();
        let started = Instant::now();
        let handle = thread::spawn(move || waiter.take(Duration::from_secs(30)));
        thread::sleep(Duration::from_millis(50));
        bridge.close();
        assert_eq!(handle.join().unwrap(), Taken::Closed);
        assert!(started.elapsed() < Duration::from_secs(5));
        bridge.reopen();
        assert_eq!(bridge.take(Duration::ZERO), Taken::Timeout);
    }

    #[test]
    fn queued_actions_are_delivered_before_close() {
        let bridge = Bridge::new();
        bridge.push(Action::Hide).unwrap();
        bridge.close();
        assert_eq!(
            bridge.take(Duration::ZERO),
            Taken::Actions(vec![Action::Hide])
        );
        assert_eq!(bridge.take(Duration::ZERO), Taken::Closed);
    }

    #[test]
    fn geometry_is_validated() {
        let bridge = Bridge::new();
        assert!(bridge.set_geometry(2, 400).is_err());
        assert_eq!(bridge.geometry(), None);
        bridge.set_geometry(400, 800).unwrap();
        assert_eq!(bridge.geometry().unwrap().width, 400);
    }

    #[test]
    fn local_clipboard_is_validated_and_bounded() {
        let bridge = Bridge::new();
        assert!(
            bridge
                .local_clipboard(ClipboardContent {
                    kind: ClipboardKind::Text,
                    bytes: vec![]
                })
                .is_err()
        );
        for i in 0..10u8 {
            bridge
                .local_clipboard(ClipboardContent {
                    kind: ClipboardKind::Text,
                    bytes: vec![b'a' + i],
                })
                .unwrap();
        }
        let drained = bridge.drain_clipboard();
        assert_eq!(drained.len(), MAX_LOCAL);
        assert_eq!(drained.last().unwrap().bytes, b"j");
    }

    #[test]
    fn encoding_is_stable() {
        let bytes = encode(&[
            Action::Move { x: 1, y: -1 },
            Action::Hide,
            Action::Button {
                button: 3,
                down: true,
            },
            Action::Key {
                keycode: 29,
                meta: 0x1000,
                down: false,
            },
            Action::Scroll {
                horizontal: 0,
                vertical: -2,
            },
            Action::Clipboard(ClipboardContent {
                kind: ClipboardKind::Text,
                bytes: b"hi".to_vec(),
            }),
            Action::Report(Report {
                id: 1,
                data: vec![2, 0, 4],
                position_only: false,
            }),
        ]);
        assert_eq!(
            bytes,
            [
                vec![1, 0, 0, 0, 1, 255, 255, 255, 255],
                vec![2],
                vec![3, 3, 1],
                vec![4, 0, 0, 0, 29, 0, 0, 0x10, 0, 0],
                vec![5, 0, 0, 0, 0, 255, 255, 255, 254],
                vec![6, 0, 0, 0, 0, 2, b'h', b'i'],
                vec![7, 1, 3, 2, 0, 4],
            ]
            .concat()
        );
    }
}
