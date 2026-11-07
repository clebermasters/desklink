//! Quartz session event taps, not root-only HID taps. All handles stay on the desktop thread.
use super::{Capabilities, Desktop, LocalEvent, keymap};
use anyhow::{Result, ensure};
use desklink_protocol::{Geometry, Input};
use std::{
    collections::{BTreeSet, VecDeque},
    ffi::{c_int, c_void},
    ptr,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

type Ref = *mut c_void;
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: Point,
    size: Size,
}
const TAG: i64 = 0x4445534b4c494e4b;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGRequestPostEventAccess() -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        callback: extern "C" fn(Ref, u32, Ref, *mut c_void) -> Ref,
        user: *mut c_void,
    ) -> Ref;
    fn CGEventTapEnable(tap: Ref, enable: bool);
    fn CGEventCreate(source: Ref) -> Ref;
    fn CGEventGetLocation(event: Ref) -> Point;
    fn CGEventGetFlags(event: Ref) -> u64;
    fn CGEventGetIntegerValueField(event: Ref, field: u32) -> i64;
    fn CGEventSetIntegerValueField(event: Ref, field: u32, value: i64);
    fn CGEventSetFlags(event: Ref, flags: u64);
    fn CGEventSetType(event: Ref, event_type: u32);
    fn CGEventCreateKeyboardEvent(source: Ref, key: u16, down: bool) -> Ref;
    fn CGEventCreateMouseEvent(source: Ref, event_type: u32, point: Point, button: u32) -> Ref;
    fn CGEventCreateScrollWheelEvent(source: Ref, units: u32, count: u32, ...) -> Ref;
    fn CGEventPost(tap: u32, event: Ref);
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> c_int;
    fn CGDisplayBounds(display: u32) -> Rect;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFRunLoopDefaultMode: Ref;
    fn CFMachPortCreateRunLoopSource(allocator: Ref, port: Ref, order: isize) -> Ref;
    fn CFRunLoopGetCurrent() -> Ref;
    fn CFRunLoopAddSource(loop_ref: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRemoveSource(loop_ref: Ref, source: Ref, mode: Ref);
    fn CFRunLoopRunInMode(mode: Ref, seconds: f64, return_after_source: bool) -> i32;
    fn CFRelease(value: Ref);
}

struct CallbackState {
    captured: AtomicBool,
    overflow: AtomicBool,
    events: Mutex<VecDeque<Input>>,
    modifiers: Mutex<BTreeSet<u16>>,
}

extern "C" fn callback(_proxy: Ref, kind: u32, event: Ref, user: *mut c_void) -> Ref {
    // SAFETY: the boxed state outlives the enabled tap; the callback runs on its CF run loop.
    let state = unsafe { &*(user as *const CallbackState) };
    if kind >= u32::MAX - 1 {
        state.overflow.store(true, Ordering::Relaxed);
        return event;
    }
    if event.is_null() {
        return event;
    }
    // SAFETY: Quartz supplies a live event for the lifetime of this callback.
    let (tag, code, flags) = unsafe {
        (
            CGEventGetIntegerValueField(event, 42),
            CGEventGetIntegerValueField(event, 9) as u16,
            CGEventGetFlags(event),
        )
    };
    if tag == TAG {
        return event;
    }
    let mut input = None;
    if kind == 12 {
        if let Some(code) = keymap::from_mac(code)
            && let Ok(mut held) = state.modifiers.lock()
        {
            let mask = modifier_mask(code);
            let down = flags & mask != 0 && !held.contains(&code);
            if down {
                held.insert(code);
            } else {
                held.remove(&code);
            }
            input = Some(Input::Key { code, down });
        }
    } else if kind == 10 || kind == 11 {
        input = keymap::from_mac(code).map(|code| Input::Key {
            code,
            down: kind == 10,
        });
    }
    if !state.captured.load(Ordering::Relaxed) {
        return event;
    }
    if kind == 12 && keymap::from_mac(code) == Some(58) {
        if let Ok(mut events) = state.events.lock() {
            if events.len() < 4094 {
                events.push_back(Input::Key {
                    code: 58,
                    down: true,
                });
                events.push_back(Input::Key {
                    code: 58,
                    down: false,
                });
            } else {
                state.overflow.store(true, Ordering::Relaxed);
            }
        }
        return ptr::null_mut();
    }
    // SAFETY: all fields are queried on the live callback event.
    if input.is_none() {
        input = unsafe {
            match kind {
                5 | 6 | 7 | 27 => Some(Input::Motion {
                    dx: CGEventGetIntegerValueField(event, 4) as i32,
                    dy: CGEventGetIntegerValueField(event, 5) as i32,
                }),
                1 | 2 => Some(Input::Button {
                    button: 1,
                    down: kind == 1,
                }),
                3 | 4 => Some(Input::Button {
                    button: 3,
                    down: kind == 3,
                }),
                25 | 26 => Some(Input::Button {
                    button: match CGEventGetIntegerValueField(event, 3) as u8 {
                        2 => 2,
                        other => other.saturating_add(1),
                    },
                    down: kind == 25,
                }),
                22 => Some(Input::Scroll {
                    horizontal: CGEventGetIntegerValueField(event, 12).clamp(-120, 120) as i32,
                    vertical: CGEventGetIntegerValueField(event, 11).clamp(-120, 120) as i32,
                }),
                _ => None,
            }
        }
    };
    if let Some(input) = input {
        if let Ok(mut events) = state.events.lock() {
            if events.len() < 4096 {
                events.push_back(input);
            } else {
                state.overflow.store(true, Ordering::Relaxed);
            }
        } else {
            state.overflow.store(true, Ordering::Relaxed);
        }
    }
    ptr::null_mut() // suppress local delivery only while sharing to a remote screen
}

fn modifier_mask(code: u16) -> u64 {
    match code {
        42 | 54 => 1 << 17,
        29 | 97 => 1 << 18,
        56 | 100 => 1 << 19,
        125 | 126 => 1 << 20,
        58 => 1 << 16,
        _ => 0,
    }
}

pub struct MacDesktop {
    tap: Ref,
    source: Ref,
    loop_ref: Ref,
    state: Box<CallbackState>,
    geometry: Geometry,
    last: (i32, i32),
    keys: BTreeSet<u16>,
    caps_lock: bool,
    buttons: BTreeSet<u8>,
}
impl MacDesktop {
    pub fn open() -> Result<Self> {
        // SAFETY: permission probes have no pointer parameters or ownership transfer.
        ensure!(
            unsafe { AXIsProcessTrusted() && CGPreflightPostEventAccess() },
            "grant Desklink Accessibility access in System Settings → Privacy & Security; run `desklink permissions --request` to request input access"
        );
        let mut state = Box::new(CallbackState {
            captured: AtomicBool::new(false),
            overflow: AtomicBool::new(false),
            events: Mutex::new(VecDeque::new()),
            modifiers: Mutex::new(BTreeSet::new()),
        });
        let mask = [1, 2, 3, 4, 5, 6, 7, 10, 11, 12, 22, 25, 26, 27]
            .iter()
            .fold(0u64, |v, n| v | (1u64 << n));
        // SAFETY: callback state is stable in its box, removed before destruction. 1 is a session tap.
        let tap = unsafe {
            CGEventTapCreate(
                1,
                0,
                0,
                mask,
                callback,
                (&mut *state as *mut CallbackState).cast(),
            )
        };
        ensure!(
            !tap.is_null(),
            "macOS refused the input event tap; check Accessibility and Input Monitoring permissions"
        );
        // SAFETY: tap is a valid owned CFMachPort; CoreFoundation retains the registered source.
        let source = unsafe { CFMachPortCreateRunLoopSource(ptr::null_mut(), tap, 0) };
        if source.is_null() {
            unsafe { CFRelease(tap) };
            anyhow::bail!("could not create event-tap run-loop source");
        }
        let loop_ref = unsafe { CFRunLoopGetCurrent() };
        unsafe {
            CFRunLoopAddSource(loop_ref, source, kCFRunLoopDefaultMode);
            CGEventTapEnable(tap, true);
        }
        let mut desktop = Self {
            tap,
            source,
            loop_ref,
            state,
            geometry: Geometry {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
            last: (0, 0),
            keys: BTreeSet::new(),
            caps_lock: false,
            buttons: BTreeSet::new(),
        };
        desktop.geometry = desktop.geometry()?;
        desktop.last = position()?;
        Ok(desktop)
    }
    fn post(&self, event: Ref) -> Result<()> {
        ensure!(!event.is_null(), "macOS could not create an input event");
        let flags = self
            .keys
            .iter()
            .filter(|k| **k != 58)
            .fold(if self.caps_lock { 1 << 16 } else { 0 }, |a, k| {
                a | modifier_mask(*k)
            });
        // SAFETY: event is owned by this call; it is posted synchronously then released exactly once.
        unsafe {
            CGEventSetIntegerValueField(event, 42, TAG);
            CGEventSetFlags(event, flags);
            CGEventPost(1, event);
            CFRelease(event);
        }
        Ok(())
    }
}

fn position() -> Result<(i32, i32)> {
    // SAFETY: create returns an owned event; it is released after reading its location.
    let event = unsafe { CGEventCreate(ptr::null_mut()) };
    ensure!(!event.is_null(), "could not query cursor");
    let p = unsafe { CGEventGetLocation(event) };
    unsafe { CFRelease(event) };
    Ok((p.x.round() as i32, p.y.round() as i32))
}

impl Desktop for MacDesktop {
    fn geometry(&mut self) -> Result<Geometry> {
        let mut count = 0;
        let mut displays = [0u32; 32];
        // SAFETY: output array has the advertised capacity; count is valid writable storage.
        ensure!(
            unsafe { CGGetActiveDisplayList(32, displays.as_mut_ptr(), &mut count) } == 0
                && count > 0,
            "could not enumerate displays"
        );
        let (mut left, mut top, mut right, mut bottom) = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for id in &displays[..count.min(32) as usize] {
            let r = unsafe { CGDisplayBounds(*id) };
            left = left.min(r.origin.x);
            top = top.min(r.origin.y);
            right = right.max(r.origin.x + r.size.width);
            bottom = bottom.max(r.origin.y + r.size.height);
        }
        self.geometry = Geometry {
            x: left as i32,
            y: top as i32,
            width: (right - left) as u32,
            height: (bottom - top) as u32,
        };
        ensure!(self.geometry.valid(), "invalid display geometry");
        Ok(self.geometry)
    }
    fn poll(&mut self) -> Result<Vec<LocalEvent>> {
        // SAFETY: service this thread's registered input source without blocking the networking loop.
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.0, false);
        }
        ensure!(
            !self.state.overflow.swap(false, Ordering::Relaxed),
            "macOS disabled the event tap or its queue overflowed; restoring local input"
        );
        let mut events: Vec<_> = self
            .state
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("event queue poisoned"))?
            .drain(..)
            .map(LocalEvent::Input)
            .collect();
        if !self.state.captured.load(Ordering::Relaxed) {
            let (x, y) = position()?;
            let (dx, dy) = (x - self.last.0, y - self.last.1);
            if dx != 0 || dy != 0 {
                events.push(LocalEvent::Pointer { x, y, dx, dy });
            }
            self.last = (x, y);
        }
        Ok(events)
    }
    fn capture(&mut self) -> Result<Vec<Input>> {
        self.state
            .events
            .lock()
            .map_err(|_| anyhow::anyhow!("event queue poisoned"))?
            .clear();
        self.state.captured.store(true, Ordering::Relaxed);
        Ok(self
            .state
            .modifiers
            .lock()
            .map_err(|_| anyhow::anyhow!("modifier state poisoned"))?
            .iter()
            .map(|code| Input::Key {
                code: *code,
                down: true,
            })
            .collect())
    }
    fn release(&mut self) -> Result<()> {
        self.state.captured.store(false, Ordering::Relaxed);
        self.last = position()?;
        Ok(())
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<()> {
        let (x, y) = self.geometry.clamp(x, y);
        let kind = if self.buttons.contains(&1) {
            6
        } else if self.buttons.contains(&3) {
            7
        } else if !self.buttons.is_empty() {
            27
        } else {
            5
        };
        self.post(unsafe {
            CGEventCreateMouseEvent(
                ptr::null_mut(),
                kind,
                Point {
                    x: x as f64,
                    y: y as f64,
                },
                0,
            )
        })?;
        self.last = (x, y);
        Ok(())
    }
    fn inject(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Key { code, down } => {
                let mac = keymap::to_mac(code)
                    .ok_or_else(|| anyhow::anyhow!("key {code} has no macOS mapping"))?;
                let repeat = down && self.keys.contains(&code);
                if code == 58 && down && !repeat {
                    self.caps_lock = !self.caps_lock;
                }
                if down {
                    self.keys.insert(code);
                } else {
                    self.keys.remove(&code);
                }
                let event = unsafe { CGEventCreateKeyboardEvent(ptr::null_mut(), mac, down) };
                if repeat && !event.is_null() {
                    unsafe {
                        CGEventSetIntegerValueField(event, 8, 1);
                    }
                }
                if modifier_mask(code) != 0 && !event.is_null() {
                    unsafe {
                        CGEventSetType(event, 12);
                    }
                }
                self.post(event)
            }
            Input::Button { button, down } => {
                if down {
                    self.buttons.insert(button);
                } else {
                    self.buttons.remove(&button);
                }
                let (x, y) = position()?;
                let (cg, kind) = match button {
                    1 => (0, if down { 1 } else { 2 }),
                    3 => (1, if down { 3 } else { 4 }),
                    2 => (2, if down { 25 } else { 26 }),
                    _ => (button as u32 - 1, if down { 25 } else { 26 }),
                };
                self.post(unsafe {
                    CGEventCreateMouseEvent(
                        ptr::null_mut(),
                        kind,
                        Point {
                            x: x as f64,
                            y: y as f64,
                        },
                        cg,
                    )
                })
            }
            Input::Scroll {
                horizontal,
                vertical,
            } => self.post(unsafe {
                CGEventCreateScrollWheelEvent(ptr::null_mut(), 1, 2, vertical, horizontal)
            }),
            Input::Motion { .. } => anyhow::bail!("motion must be routed to absolute coordinates"),
        }
    }
}
impl Drop for MacDesktop {
    fn drop(&mut self) {
        self.state.captured.store(false, Ordering::Relaxed);
        let keys: Vec<_> = self.keys.iter().copied().collect();
        for code in keys {
            let _ = self.inject(Input::Key { code, down: false });
        }
        let buttons: Vec<_> = self.buttons.iter().copied().collect();
        for button in buttons {
            let _ = self.inject(Input::Button {
                button,
                down: false,
            });
        }
        // SAFETY: source and tap are the owned handles created in open; deregistration precedes freeing state.
        unsafe {
            CGEventTapEnable(self.tap, false);
            CFRunLoopRemoveSource(self.loop_ref, self.source, kCFRunLoopDefaultMode);
            CFRelease(self.source);
            CFRelease(self.tap);
        }
    }
}

pub fn capabilities() -> Capabilities {
    let (accessibility, listen, post) = unsafe {
        (
            AXIsProcessTrusted(),
            CGPreflightListenEventAccess(),
            CGPreflightPostEventAccess(),
        )
    };
    Capabilities {
        backend: "quartz-session",
        capture: accessibility,
        inject: post,
        geometry: None,
        detail: format!(
            "Accessibility={accessibility}, Input Monitoring={listen}, event posting={post}; OS privacy approval is required"
        ),
    }
}
pub fn request_permissions() {
    unsafe {
        CGRequestListenEventAccess();
        CGRequestPostEventAccess();
    }
}
