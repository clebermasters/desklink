use anyhow::{Result, ensure};
use desklink_protocol::Input;
use reis::{
    ei,
    enumflags2::BitFlags,
    event::{Connection, Device, DeviceCapability, EiEvent},
};

pub fn requested_capabilities() -> BitFlags<DeviceCapability> {
    DeviceCapability::Pointer
        | DeviceCapability::PointerAbsolute
        | DeviceCapability::Keyboard
        | DeviceCapability::Button
        | DeviceCapability::Scroll
}

pub struct Injector {
    connection: Connection,
    devices: Vec<Device>,
    sequence: u32,
}
impl Injector {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            devices: Vec::new(),
            sequence: 0,
        }
    }
    pub fn event(&mut self, event: EiEvent) -> Result<()> {
        match event {
            EiEvent::SeatAdded(e) => e.seat.bind_capabilities(requested_capabilities()),
            EiEvent::DeviceResumed(e) => {
                self.devices.retain(|d| *d != e.device);
                self.sequence = self.sequence.wrapping_add(1);
                e.device.device().start_emulating(e.serial, self.sequence);
                self.devices.push(e.device);
            }
            EiEvent::DeviceRemoved(e) => self.devices.retain(|d| *d != e.device),
            EiEvent::DevicePaused(e) => {
                self.devices.retain(|d| *d != e.device);
                anyhow::bail!(
                    "input emulation device was paused; permissions may have been revoked"
                );
            }
            EiEvent::Disconnected(_) => anyhow::bail!("input emulation was disconnected"),
            _ => {}
        }
        self.connection.flush()?;
        Ok(())
    }
    fn device(&self, capability: DeviceCapability) -> Result<&Device> {
        self.devices
            .iter()
            .find(|d| d.has_capability(capability))
            .ok_or_else(|| anyhow::anyhow!("missing EIS {capability:?} device"))
    }
    pub fn ready(&self) -> bool {
        [
            DeviceCapability::PointerAbsolute,
            DeviceCapability::Keyboard,
            DeviceCapability::Button,
        ]
        .iter()
        .all(|c| self.device(*c).is_ok())
    }
    fn frame(&self, device: &Device) -> Result<()> {
        device
            .device()
            .frame(self.connection.serial(), monotonic_micros());
        self.connection.flush()?;
        Ok(())
    }
    pub fn warp(&self, x: i32, y: i32) -> Result<()> {
        let d = self.device(DeviceCapability::PointerAbsolute)?;
        d.interface::<ei::PointerAbsolute>()
            .unwrap()
            .motion_absolute(x as f32, y as f32);
        self.frame(d)
    }
    pub fn input(&self, input: Input) -> Result<()> {
        ensure!(input.valid(), "invalid input");
        let d = match input {
            Input::Key { code, down } => {
                let d = self.device(DeviceCapability::Keyboard)?;
                d.interface::<ei::Keyboard>().unwrap().key(
                    code as u32,
                    if down {
                        ei::keyboard::KeyState::Press
                    } else {
                        ei::keyboard::KeyState::Released
                    },
                );
                d
            }
            Input::Button { button, down } => {
                let d = self.device(DeviceCapability::Button)?;
                let code = match button {
                    1 => 272,
                    3 => 273,
                    2 => 274,
                    _ => button as u32 + 271,
                };
                d.interface::<ei::Button>().unwrap().button(
                    code,
                    if down {
                        ei::button::ButtonState::Press
                    } else {
                        ei::button::ButtonState::Released
                    },
                );
                d
            }
            Input::Scroll {
                horizontal,
                vertical,
            } => {
                let d = self.device(DeviceCapability::Scroll)?;
                d.interface::<ei::Scroll>()
                    .unwrap()
                    .scroll_discrete(horizontal * 120, -vertical * 120);
                d
            }
            Input::Motion { .. } => anyhow::bail!("motion requires absolute routing"),
        };
        self.frame(d)
    }
}

fn monotonic_micros() -> u64 {
    let time = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    time.tv_sec as u64 * 1_000_000 + time.tv_nsec as u64 / 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use reis::{
        PendingRequestResult, eis,
        handshake::EisHandshaker,
        request::{DeviceCapability as ServerCapability, EisRequest, EisRequestConverter},
    };
    use std::{
        os::unix::net::UnixStream,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    #[tokio::test]
    async fn actual_ei_wire_negotiates_devices_and_delivers_framed_input() {
        let (client, server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let server_thread = thread::spawn(move || {
            let context = eis::Context::new(server).unwrap();
            let mut handshake = EisHandshaker::new(&context, 1);
            let mut converter: Option<EisRequestConverter> = None;
            let mut devices = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(6);
            while !stopped.load(Ordering::Relaxed) && Instant::now() < deadline {
                match context.read() {
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => panic!("{e}"),
                }
                while let Some(request) = context.pending_request() {
                    let PendingRequestResult::Request(request) = request else {
                        panic!("invalid EI wire request")
                    };
                    if let Some(convert) = &mut converter {
                        convert.handle_request(request).unwrap();
                    } else if let Some(response) = handshake.handle_request(request).unwrap() {
                        let convert = EisRequestConverter::new(&context, response, 1);
                        let _seat = convert.handle().add_seat(
                            Some("test"),
                            ServerCapability::Keyboard
                                | ServerCapability::PointerAbsolute
                                | ServerCapability::Button
                                | ServerCapability::Scroll,
                        );
                        converter = Some(convert);
                    }
                    if let Some(convert) = &mut converter {
                        while let Some(request) = convert.next_request() {
                            match request {
                                EisRequest::Bind(binding) => {
                                    for caps in [
                                        ServerCapability::Keyboard.into(),
                                        ServerCapability::PointerAbsolute
                                            | ServerCapability::Button
                                            | ServerCapability::Scroll,
                                    ] {
                                        let d = binding.seat.add_device(
                                            Some("test device"),
                                            eis::device::DeviceType::Virtual,
                                            caps,
                                            |_| {},
                                        );
                                        d.resumed();
                                        devices.push(d);
                                    }
                                }
                                EisRequest::KeyboardKey(e) => {
                                    tx.send(format!("key:{}:{:?}", e.key, e.state)).unwrap();
                                }
                                EisRequest::PointerMotionAbsolute(e) => {
                                    tx.send(format!("pointer:{}:{}", e.dx_absolute, e.dy_absolute))
                                        .unwrap();
                                }
                                EisRequest::Button(e) => {
                                    tx.send(format!("button:{}:{:?}", e.button, e.state))
                                        .unwrap();
                                }
                                EisRequest::ScrollDiscrete(e) => {
                                    tx.send(format!("scroll:{}:{}", e.discrete_dx, e.discrete_dy))
                                        .unwrap();
                                }
                                EisRequest::Frame(e) => {
                                    assert!(
                                        e.time > 1_000_000,
                                        "frame timestamp must be CLOCK_MONOTONIC"
                                    );
                                }
                                _ => {}
                            }
                        }
                    }
                    context.flush().unwrap();
                }
                let _ = context.flush();
                thread::sleep(Duration::from_millis(1));
            }
        });
        let context = ei::Context::new(client).unwrap();
        context.flush().unwrap();
        let (connection, mut stream) = tokio::time::timeout(
            Duration::from_secs(3),
            context.handshake_tokio("test", ei::handshake::ContextType::Sender),
        )
        .await
        .unwrap()
        .unwrap();
        let mut injector = Injector::new(connection);
        while !injector.ready() {
            let event = tokio::time::timeout(Duration::from_secs(3), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            injector.event(event).unwrap();
        }
        injector.warp(140, 220).unwrap();
        injector
            .input(Input::Key {
                code: 30,
                down: true,
            })
            .unwrap();
        injector
            .input(Input::Key {
                code: 30,
                down: false,
            })
            .unwrap();
        injector
            .input(Input::Button {
                button: 2,
                down: true,
            })
            .unwrap();
        injector
            .input(Input::Button {
                button: 2,
                down: false,
            })
            .unwrap();
        injector
            .input(Input::Scroll {
                horizontal: 1,
                vertical: -2,
            })
            .unwrap();
        let observations: Vec<_> = (0..6)
            .map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap())
            .collect();
        assert_eq!(
            observations,
            [
                "pointer:140:220",
                "key:30:Press",
                "key:30:Released",
                "button:274:Press",
                "button:274:Released",
                "scroll:120:240"
            ]
        );
        let device = injector.devices[0].clone();
        let mut fractions = Fractions::default();
        for _ in 0..3 {
            assert!(
                fractions
                    .input(EiEvent::PointerMotion(reis::event::PointerMotion {
                        device: device.clone(),
                        time: 0,
                        dx: 0.25,
                        dy: -0.25
                    }))
                    .is_none()
            );
        }
        assert_eq!(
            fractions.input(EiEvent::PointerMotion(reis::event::PointerMotion {
                device: device.clone(),
                time: 0,
                dx: 0.25,
                dy: -0.25
            })),
            Some(Input::Motion { dx: 1, dy: -1 })
        );
        scroll_frames_become_clicks(&device);
        stop.store(true, Ordering::Relaxed);
        server_thread.join().unwrap();
    }

    /// Feeds one EI scroll frame (optional smooth delta, optional discrete value120).
    fn scroll(
        fractions: &mut Fractions,
        device: &Device,
        delta: Option<(f32, f32)>,
        discrete: Option<(i32, i32)>,
    ) -> Option<Input> {
        if let Some((dx, dy)) = delta {
            let event = reis::event::ScrollDelta {
                device: device.clone(),
                time: 0,
                dx,
                dy,
            };
            assert_eq!(fractions.input(EiEvent::ScrollDelta(event)), None);
        }
        if let Some((discrete_dx, discrete_dy)) = discrete {
            let event = reis::event::ScrollDiscrete {
                device: device.clone(),
                time: 0,
                discrete_dx,
                discrete_dy,
            };
            assert_eq!(fractions.input(EiEvent::ScrollDiscrete(event)), None);
        }
        fractions.input(EiEvent::Frame(reis::event::Frame {
            device: device.clone(),
            serial: 0,
            time: 0,
        }))
    }

    fn scroll_frames_become_clicks(device: &Device) {
        let down = Some(Input::Scroll {
            horizontal: 0,
            vertical: -1,
        });

        // Hyprland (0.56) sends the smooth delta as value120 too: 15 and 15 per wheel notch.
        let mut f = Fractions::default();
        for _ in 0..3 {
            assert_eq!(
                scroll(&mut f, device, Some((0.0, 15.0)), Some((0, 15))),
                down
            );
        }

        // Mutter and KWin: one notch of a standard wheel is a discrete 120.
        let mut f = Fractions::default();
        assert_eq!(scroll(&mut f, device, None, Some((0, 120))), down);
        assert_eq!(
            scroll(&mut f, device, None, Some((-240, 0))),
            Some(Input::Scroll {
                horizontal: -2,
                vertical: 0
            })
        );

        // A high-resolution wheel (KWin): eight 15ths of 120 make one click, never a zero scroll.
        let mut f = Fractions::default();
        for _ in 0..7 {
            assert_eq!(scroll(&mut f, device, None, Some((0, 15))), None);
        }
        assert_eq!(scroll(&mut f, device, None, Some((0, 15))), down);

        // Smooth-only scrolling (touchpads): 15 units per click, fractions carried over.
        let mut f = Fractions::default();
        assert_eq!(scroll(&mut f, device, Some((0.0, 7.5)), None), None);
        assert_eq!(scroll(&mut f, device, Some((0.0, 7.5)), None), down);
        assert_eq!(scroll(&mut f, device, Some((0.0, 0.0)), None), None);
    }
}

/// Smooth scroll units per wheel click (libinput's 15 degrees per notch).
const SMOOTH_PER_CLICK: f32 = 15.0;
/// EI discrete scroll is in 120ths of a wheel click.
const DISCRETE_PER_CLICK: i32 = 120;

/// Turns fractional EI pointer motion and scroll into whole units, carrying the remainders.
#[derive(Default)]
pub struct Fractions {
    x: f32,
    y: f32,
    /// Scroll seen since the last frame.
    delta: Option<(f32, f32)>,
    discrete: Option<(i32, i32)>,
    /// Scroll not yet sent as whole clicks.
    smooth_remainder: (f32, f32),
    discrete_remainder: (i32, i32),
}
impl Fractions {
    pub fn input(&mut self, event: EiEvent) -> Option<Input> {
        match event {
            EiEvent::PointerMotion(e) => {
                self.x += e.dx;
                self.y += e.dy;
                let (dx, dy) = (self.x.trunc() as i32, self.y.trunc() as i32);
                self.x -= dx as f32;
                self.y -= dy as f32;
                (dx != 0 || dy != 0).then_some(Input::Motion { dx, dy })
            }
            EiEvent::KeyboardKey(e) => Some(Input::Key {
                code: u16::try_from(e.key).ok()?,
                down: e.state == ei::keyboard::KeyState::Press,
            }),
            EiEvent::Button(e) => {
                let button = match e.button {
                    272 => 1,
                    273 => 3,
                    274 => 2,
                    275..=279 => (e.button - 271) as u8,
                    _ => return None,
                };
                Some(Input::Button {
                    button,
                    down: e.state == ei::button::ButtonState::Press,
                })
            }
            EiEvent::ScrollDiscrete(e) => {
                let (x, y) = self.discrete.unwrap_or_default();
                self.discrete = Some((
                    x.saturating_add(e.discrete_dx),
                    y.saturating_add(e.discrete_dy),
                ));
                None
            }
            EiEvent::ScrollDelta(e) => {
                let (x, y) = self.delta.unwrap_or_default();
                self.delta = Some((x + e.dx, y + e.dy));
                None
            }
            EiEvent::Frame(_) => self.scroll_frame(),
            _ => None,
        }
        .filter(|input| input.valid())
    }

    /// Mutter and KWin send either a smooth delta or a discrete value in a frame. Hyprland
    /// (0.56) sends both, with the smooth delta repeated as the discrete value, so a frame's
    /// delta wins whenever there is one.
    fn scroll_frame(&mut self) -> Option<Input> {
        let delta = self.delta.take();
        let discrete = self.discrete.take();
        let (h, v) = if let Some((x, y)) = delta {
            let (rx, ry) = &mut self.smooth_remainder;
            *rx += x / SMOOTH_PER_CLICK;
            *ry += y / SMOOTH_PER_CLICK;
            let clicks = (rx.trunc() as i32, ry.trunc() as i32);
            *rx -= clicks.0 as f32;
            *ry -= clicks.1 as f32;
            clicks
        } else if let Some((x, y)) = discrete {
            let (rx, ry) = &mut self.discrete_remainder;
            *rx = rx.saturating_add(x);
            *ry = ry.saturating_add(y);
            let clicks = (*rx / DISCRETE_PER_CLICK, *ry / DISCRETE_PER_CLICK);
            *rx -= clicks.0 * DISCRETE_PER_CLICK;
            *ry -= clicks.1 * DISCRETE_PER_CLICK;
            clicks
        } else {
            return None;
        };
        // EI scrolls down with positive y; Desklink scrolls up with positive vertical.
        (h != 0 || v != 0).then_some(Input::Scroll {
            horizontal: h.clamp(-120, 120),
            vertical: (-v).clamp(-120, 120),
        })
    }
}
