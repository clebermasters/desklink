//! Presents Desklink input as a standard keyboard and mouse.
//!
//! [`HidDesktop`] is a receiving [`Desktop`] that needs no software on the controlled computer:
//! it turns the engine's absolute pointer positions, keys, buttons and wheel into HID input
//! reports for a transport such as Bluetooth (an Android phone's HID device profile, or a
//! Linux adapter). The host sees an ordinary keyboard and mouse.
pub mod descriptor;
pub mod usage;

use anyhow::{Result, bail, ensure};
use descriptor::{ABS_MAX, KEYBOARD, KEYBOARD_LEN, POINTER_ABSOLUTE, POINTER_RELATIVE};
use desklink_platform::{Desktop, LocalEvent};
use desklink_protocol::{ClipboardContent, Geometry, Input};

/// One HID input report, without the report-ID byte (transports add it as they need).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub id: u8,
    pub data: Vec<u8>,
    /// A pointer move with no wheel motion and unchanged buttons: a newer one supersedes it.
    pub position_only: bool,
}

/// Where reports go. Errors end the receiving session, so sinks should buffer briefly.
pub trait ReportSink: Send {
    fn send(&mut self, report: Report) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerMode {
    /// The host pointer lands exactly where the shared layout puts it. macOS reads every
    /// absolute report as "all buttons up", so while a button is held (a drag) the pointer moves
    /// relatively, and the next move after the release lands exactly again.
    Absolute,
    /// Movement deltas, for hosts that ignore absolute pointers. Host acceleration applies.
    Relative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// The controlled screen, in the units advertised to the controller (e.g. macOS points).
    pub width: u32,
    pub height: u32,
    pub pointer: PointerMode,
    /// Sends Control as Command and Command as Control, so Linux shortcuts work on a Mac.
    pub swap_control_and_command: bool,
}

const MAX_KEYS: usize = 6;

pub struct HidDesktop<S> {
    sink: S,
    options: Options,
    modifiers: u8,
    keys: Vec<u8>,
    buttons: u8,
    position: Option<(i32, i32)>,
}

impl<S: ReportSink> HidDesktop<S> {
    pub fn new(sink: S, options: Options) -> Result<Self> {
        ensure!(
            geometry(options).valid(),
            "invalid screen size {}x{}",
            options.width,
            options.height
        );
        Ok(Self {
            sink,
            options,
            modifiers: 0,
            keys: Vec::with_capacity(MAX_KEYS),
            buttons: 0,
            position: None,
        })
    }

    fn keyboard_report(&mut self) -> Result<()> {
        let mut data = vec![0; KEYBOARD_LEN];
        data[0] = self.modifiers;
        data[2..2 + self.keys.len()].copy_from_slice(&self.keys);
        self.sink.send(Report {
            id: KEYBOARD,
            data,
            position_only: false,
        })
    }

    fn absolute_report(&mut self, wheel: i8, pan: i8, position_only: bool) -> Result<()> {
        let (x, y) = self.position.unwrap_or((0, 0));
        let scale = |v: i32, extent: u32| -> u16 {
            let span = extent.saturating_sub(1).max(1) as i64;
            (v.clamp(0, span as i32) as i64 * ABS_MAX as i64 / span) as u16
        };
        let (x, y) = (scale(x, self.options.width), scale(y, self.options.height));
        let mut data = Vec::with_capacity(descriptor::ABSOLUTE_LEN);
        data.extend_from_slice(&x.to_le_bytes());
        data.extend_from_slice(&y.to_le_bytes());
        data.extend_from_slice(&[wheel as u8, pan as u8]);
        self.sink.send(Report {
            id: POINTER_ABSOLUTE,
            data,
            position_only,
        })
    }

    fn relative_report(&mut self, dx: i16, dy: i16, wheel: i8, pan: i8) -> Result<()> {
        let mut data = vec![self.buttons];
        data.extend_from_slice(&dx.to_le_bytes());
        data.extend_from_slice(&dy.to_le_bytes());
        data.extend_from_slice(&[wheel as u8, pan as u8]);
        // Deltas accumulate, so a relative report can never be dropped in favour of a later one.
        self.sink.send(Report {
            id: POINTER_RELATIVE,
            data,
            position_only: false,
        })
    }

    /// Absolute reports would release held buttons on macOS; see [`PointerMode::Absolute`].
    fn absolute_now(&self) -> bool {
        self.options.pointer == PointerMode::Absolute && self.buttons == 0
    }

    /// A wheel change at the current position. macOS scrolls only with the absolute pointer's
    /// wheel, other hosts with either.
    fn pointer_report(&mut self, wheel: i8, pan: i8) -> Result<()> {
        if self.absolute_now() {
            self.absolute_report(wheel, pan, false)
        } else {
            self.relative_report(0, 0, wheel, pan)
        }
    }

    fn relative_motion(&mut self, dx: i32, dy: i32) -> Result<()> {
        let (mut dx, mut dy) = (dx, dy);
        while dx != 0 || dy != 0 {
            let (sx, sy) = (dx.clamp(-32767, 32767), dy.clamp(-32767, 32767));
            self.relative_report(sx as i16, sy as i16, 0, 0)?;
            dx -= sx;
            dy -= sy;
        }
        Ok(())
    }

    fn key(&mut self, code: u16, down: bool) -> Result<()> {
        let Some(mut usage) = usage::usage(code) else {
            // Not an error: failing here would end the whole session over one key.
            tracing::debug!("key without a keyboard usage ignored");
            return Ok(());
        };
        if self.options.swap_control_and_command {
            usage = usage::swap_control_and_command(usage);
        }
        let before = (self.modifiers, self.keys.clone());
        if usage::is_modifier(usage) {
            let bit = 1 << (usage - 0xe0);
            if down {
                self.modifiers |= bit;
            } else {
                self.modifiers &= !bit;
            }
        } else if down {
            if !self.keys.contains(&usage) {
                if self.keys.len() == MAX_KEYS {
                    tracing::debug!("more than six keys held; extra key ignored");
                    return Ok(());
                }
                self.keys.push(usage);
            }
        } else {
            self.keys.retain(|k| *k != usage);
        }
        // Auto-repeat is the host's job; repeated key-downs need no report.
        if (self.modifiers, &self.keys) == (before.0, &before.1) {
            return Ok(());
        }
        self.keyboard_report()
    }

    fn button(&mut self, button: u8, down: bool) -> Result<()> {
        // Desklink numbers buttons in X11 order; HID puts right before middle.
        let bit = match button {
            1 => 0,
            3 => 1,
            2 => 2,
            4 => 3,
            5 => 4,
            _ => return Ok(()),
        };
        let before = self.buttons;
        if down {
            self.buttons |= 1 << bit;
        } else {
            self.buttons &= !(1 << bit);
        }
        if self.buttons == before {
            return Ok(());
        }
        // Buttons always use the relative pointer (see the descriptor), with no movement.
        self.relative_report(0, 0, 0, 0)
    }

    fn scroll(&mut self, horizontal: i32, vertical: i32) -> Result<()> {
        let (mut horizontal, mut vertical) = (horizontal, vertical);
        while horizontal != 0 || vertical != 0 {
            let h = horizontal.clamp(-127, 127);
            let v = vertical.clamp(-127, 127);
            self.pointer_report(v as i8, h as i8)?;
            horizontal -= h;
            vertical -= v;
        }
        Ok(())
    }
}

fn geometry(options: Options) -> Geometry {
    Geometry {
        x: 0,
        y: 0,
        width: options.width,
        height: options.height,
    }
}

impl<S: ReportSink> Desktop for HidDesktop<S> {
    fn geometry(&mut self) -> Result<Geometry> {
        Ok(geometry(self.options))
    }
    fn poll(&mut self) -> Result<Vec<LocalEvent>> {
        Ok(vec![])
    }
    fn capture(&mut self) -> Result<Vec<Input>> {
        bail!("a keyboard and mouse connection only receives input")
    }
    /// Lets go of every key and button; the pointer stays where it is.
    fn release(&mut self) -> Result<()> {
        self.modifiers = 0;
        self.keys.clear();
        self.buttons = 0;
        self.keyboard_report()?;
        self.relative_report(0, 0, 0, 0)
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<()> {
        let previous = self.position.replace((x, y));
        if self.absolute_now() {
            return self.absolute_report(0, 0, true);
        }
        // The host's position is unknown until the first move in relative mode.
        let Some((px, py)) = previous else {
            return Ok(());
        };
        self.relative_motion(x - px, y - py)
    }
    fn inject(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Motion { .. } => Ok(()), // The engine converts motion into absolute warps.
            Input::Key { code, down } => self.key(code, down),
            Input::Button { button, down } => self.button(button, down),
            Input::Scroll {
                horizontal,
                vertical,
            } => self.scroll(horizontal, vertical),
        }
    }
    /// The controlled computer runs no Desklink software, so there is no clipboard to share.
    fn owns_clipboard(&self) -> bool {
        true
    }
    fn clipboard_apply(&mut self, _content: ClipboardContent) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<Report>>>);
    impl ReportSink for Sink {
        fn send(&mut self, report: Report) -> Result<()> {
            self.0.lock().unwrap().push(report);
            Ok(())
        }
    }
    impl Sink {
        fn take(&self) -> Vec<Report> {
            std::mem::take(&mut self.0.lock().unwrap())
        }
    }

    fn desktop(pointer: PointerMode, swap: bool) -> (Sink, HidDesktop<Sink>) {
        let sink = Sink::default();
        let options = Options {
            width: 1512,
            height: 982,
            pointer,
            swap_control_and_command: swap,
        };
        (sink.clone(), HidDesktop::new(sink, options).unwrap())
    }
    fn key(d: &mut HidDesktop<Sink>, code: u16, down: bool) {
        d.inject(Input::Key { code, down }).unwrap();
    }

    #[test]
    fn shifted_letter_then_release() {
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        key(&mut d, 42, true); // Left Shift
        key(&mut d, 30, true); // A
        key(&mut d, 30, true); // auto-repeat: no report
        key(&mut d, 30, false);
        key(&mut d, 42, false);
        let reports: Vec<_> = sink.take().into_iter().map(|r| r.data).collect();
        assert_eq!(
            reports,
            vec![
                vec![0x02, 0, 0, 0, 0, 0, 0, 0],
                vec![0x02, 0, 0x04, 0, 0, 0, 0, 0],
                vec![0x02, 0, 0, 0, 0, 0, 0, 0],
                vec![0, 0, 0, 0, 0, 0, 0, 0],
            ]
        );
    }

    #[test]
    fn control_becomes_command_for_a_mac() {
        let (sink, mut d) = desktop(PointerMode::Absolute, true);
        key(&mut d, 29, true); // Left Ctrl
        key(&mut d, 46, true); // C
        let reports = sink.take();
        assert_eq!(reports[1].data[..3], [0x08, 0, 0x06], "Command+C");
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        key(&mut d, 29, true);
        assert_eq!(sink.take()[0].data[0], 0x01, "unswapped Control");
    }

    #[test]
    fn at_most_six_keys_and_unknown_keys_are_ignored() {
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        for code in [30, 31, 32, 33, 34, 35, 36] {
            key(&mut d, code, true);
        }
        key(&mut d, 240, true);
        let reports = sink.take();
        assert_eq!(reports.len(), 6);
        assert_eq!(reports[5].data[2..], [0x04, 0x16, 0x07, 0x09, 0x0a, 0x0b]);
    }

    #[test]
    fn absolute_positions_scale_to_the_full_range() {
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        d.warp(0, 0).unwrap();
        d.warp(1511, 981).unwrap();
        d.warp(756, 491).unwrap();
        let reports = sink.take();
        assert!(
            reports
                .iter()
                .all(|r| r.id == POINTER_ABSOLUTE && r.position_only)
        );
        assert_eq!(reports[0].data, [0, 0, 0, 0, 0, 0]);
        assert_eq!(reports[1].data[..4], [0xff, 0x7f, 0xff, 0x7f]);
        let x = u16::from_le_bytes([reports[2].data[0], reports[2].data[1]]);
        assert!((16380..16400).contains(&x), "{x}");
    }

    #[test]
    fn buttons_wheel_and_release() {
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        d.warp(10, 10).unwrap();
        d.inject(Input::Button {
            button: 3,
            down: true,
        })
        .unwrap();
        d.inject(Input::Button {
            button: 3,
            down: false,
        })
        .unwrap();
        d.inject(Input::Scroll {
            horizontal: -1,
            vertical: 300,
        })
        .unwrap();
        key(&mut d, 30, true);
        d.release().unwrap();
        let reports = sink.take();
        assert_eq!(
            reports[1].id, POINTER_RELATIVE,
            "buttons use the relative pointer"
        );
        assert_eq!(
            reports[1].data,
            [0b10, 0, 0, 0, 0, 0, 0],
            "right is HID button 2"
        );
        assert!(!reports[1].position_only);
        assert_eq!(reports[2].data[0], 0, "right button released");
        assert!(reports[3..6].iter().all(|r| r.id == POINTER_ABSOLUTE));
        let wheel: Vec<(i8, i8)> = reports[3..6]
            .iter()
            .map(|r| (r.data[4] as i8, r.data[5] as i8))
            .collect();
        assert_eq!(wheel, vec![(127, -1), (127, 0), (46, 0)]);
        let released = &reports[reports.len() - 2..];
        assert_eq!(released[0].data, vec![0; 8], "all keys up");
        assert_eq!(
            (released[1].id, released[1].data[0]),
            (POINTER_RELATIVE, 0),
            "all buttons up"
        );
    }

    #[test]
    fn drags_move_relatively_and_land_exactly_after_release() {
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        d.warp(100, 100).unwrap();
        d.inject(Input::Button {
            button: 1,
            down: true,
        })
        .unwrap();
        d.warp(130, 90).unwrap(); // dragging
        d.inject(Input::Scroll {
            horizontal: 0,
            vertical: 1,
        })
        .unwrap(); // a wheel turn must not release the held button
        d.inject(Input::Button {
            button: 1,
            down: false,
        })
        .unwrap();
        d.warp(131, 90).unwrap();
        let r = sink.take();
        let kinds: Vec<_> = r.iter().map(|r| (r.id, r.data[0])).collect();
        assert_eq!(
            kinds,
            vec![
                (POINTER_ABSOLUTE, r[0].data[0]),
                (POINTER_RELATIVE, 1),            // press
                (POINTER_RELATIVE, 1),            // drag (+30, -10)
                (POINTER_RELATIVE, 1),            // wheel while held
                (POINTER_RELATIVE, 0),            // release
                (POINTER_ABSOLUTE, r[5].data[0]), // exact again
            ]
        );
        assert_eq!(
            (
                i16::from_le_bytes([r[2].data[1], r[2].data[2]]),
                i16::from_le_bytes([r[2].data[3], r[2].data[4]])
            ),
            (30, -10)
        );
        assert_eq!(r[3].data[5] as i8, 1, "wheel in the relative report");
    }

    #[test]
    fn relative_mode_sends_deltas() {
        let (sink, mut d) = desktop(PointerMode::Relative, false);
        d.warp(100, 100).unwrap(); // the host position is unknown until the next move
        d.warp(110, 95).unwrap();
        let reports = sink.take();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, POINTER_RELATIVE);
        assert!(!reports[0].position_only);
        assert_eq!(
            (
                i16::from_le_bytes([reports[0].data[1], reports[0].data[2]]),
                i16::from_le_bytes([reports[0].data[3], reports[0].data[4]])
            ),
            (10, -5)
        );
    }

    #[test]
    fn never_captures_and_ignores_clipboards() {
        let (sink, mut d) = desktop(PointerMode::Absolute, false);
        assert!(d.capture().is_err());
        assert!(d.owns_clipboard());
        d.clipboard_apply(ClipboardContent {
            kind: desklink_protocol::ClipboardKind::Text,
            bytes: b"x".to_vec(),
        })
        .unwrap();
        assert!(sink.take().is_empty());
        assert_eq!(d.geometry().unwrap().width, 1512);
        let invalid = Options {
            width: 0,
            height: 10,
            pointer: PointerMode::Absolute,
            swap_control_and_command: false,
        };
        assert!(HidDesktop::new(Sink::default(), invalid).is_err());
    }
}
