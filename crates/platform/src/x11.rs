use super::{Desktop, LocalEvent};
use anyhow::{Context, Result, ensure};
use desklink_protocol::{Geometry, Input};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::{
    CURRENT_TIME, NONE,
    connection::Connection,
    protocol::{
        Event,
        xproto::{self, ConnectionExt as _, EventMask, GrabMode, GrabStatus, KeyButMask},
    },
    rust_connection::RustConnection,
};

pub struct X11 {
    connection: RustConnection,
    root: u32,
    cursor: u32,
    captured: bool,
    last: (i32, i32),
    geometry: Geometry,
}

impl X11 {
    pub fn open() -> Result<Self> {
        let (connection, screen) =
            x11rb::connect(None).context("cannot connect to the user’s X11 display")?;
        connection
            .xtest_get_version(2, 2)?
            .reply()
            .context("XTEST is not available on this display")?;
        let root = connection.setup().roots[screen].root;
        let g = connection.get_geometry(root)?.reply()?;
        let geometry = Geometry {
            x: 0,
            y: 0,
            width: g.width as u32,
            height: g.height as u32,
        };
        ensure!(geometry.valid(), "invalid desktop geometry");
        let pixmap = connection.generate_id()?;
        let gc = connection.generate_id()?;
        let cursor = connection.generate_id()?;
        connection.create_pixmap(1, pixmap, root, 1, 1)?.check()?;
        connection
            .create_gc(gc, pixmap, &xproto::CreateGCAux::new().foreground(0))?
            .check()?;
        connection
            .poly_fill_rectangle(
                pixmap,
                gc,
                &[xproto::Rectangle {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                }],
            )?
            .check()?;
        connection
            .create_cursor(cursor, pixmap, pixmap, 0, 0, 0, 0, 0, 0, 0, 0)?
            .check()?;
        connection.free_gc(gc)?;
        connection.free_pixmap(pixmap)?;
        let p = connection.query_pointer(root)?.reply()?;
        Ok(Self {
            connection,
            root,
            cursor,
            captured: false,
            last: (p.root_x as i32, p.root_y as i32),
            geometry,
        })
    }
    fn center(&self) -> (i32, i32) {
        (
            self.geometry.width as i32 / 2,
            self.geometry.height as i32 / 2,
        )
    }
}

impl Desktop for X11 {
    fn geometry(&mut self) -> Result<Geometry> {
        let g = self.connection.get_geometry(self.root)?.reply()?;
        self.geometry = Geometry {
            x: 0,
            y: 0,
            width: g.width as u32,
            height: g.height as u32,
        };
        Ok(self.geometry)
    }
    fn poll(&mut self) -> Result<Vec<LocalEvent>> {
        let mut events = Vec::new();
        for _ in 0..1024 {
            let Some(event) = self.connection.poll_for_event()? else {
                break;
            };
            if !self.captured {
                continue;
            }
            let input = match event {
                Event::KeyPress(e) => Some(Input::Key {
                    code: e.detail.saturating_sub(8) as u16,
                    down: true,
                }),
                Event::KeyRelease(e) => Some(Input::Key {
                    code: e.detail.saturating_sub(8) as u16,
                    down: false,
                }),
                Event::ButtonPress(e) if (4..=7).contains(&e.detail) => Some(match e.detail {
                    4 => Input::Scroll {
                        horizontal: 0,
                        vertical: 1,
                    },
                    5 => Input::Scroll {
                        horizontal: 0,
                        vertical: -1,
                    },
                    6 => Input::Scroll {
                        horizontal: -1,
                        vertical: 0,
                    },
                    _ => Input::Scroll {
                        horizontal: 1,
                        vertical: 0,
                    },
                }),
                Event::ButtonPress(e) => Some(Input::Button {
                    button: if e.detail >= 8 {
                        e.detail - 4
                    } else {
                        e.detail
                    },
                    down: true,
                }),
                Event::ButtonRelease(e) if !(4..=7).contains(&e.detail) => Some(Input::Button {
                    button: if e.detail >= 8 {
                        e.detail - 4
                    } else {
                        e.detail
                    },
                    down: false,
                }),
                _ => None,
            };
            if let Some(input) = input.filter(|v| v.valid()) {
                events.push(LocalEvent::Input(input));
            }
        }
        let p = self.connection.query_pointer(self.root)?.reply()?;
        let (x, y) = (p.root_x as i32, p.root_y as i32);
        if self.captured {
            let (cx, cy) = self.center();
            let (dx, dy) = (x - cx, y - cy);
            if dx != 0 || dy != 0 {
                events.push(LocalEvent::Input(Input::Motion { dx, dy }));
                self.warp(cx, cy)?;
            }
        } else {
            let (dx, dy) = (x - self.last.0, y - self.last.1);
            if dx != 0 || dy != 0 {
                events.push(LocalEvent::Pointer { x, y, dx, dy });
            }
            self.last = (x, y);
        }
        self.connection.flush()?;
        Ok(events)
    }
    fn capture(&mut self) -> Result<Vec<Input>> {
        if self.captured {
            return Ok(Vec::new());
        }
        let status = self
            .connection
            .grab_pointer(
                false,
                self.root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                self.root,
                self.cursor,
                CURRENT_TIME,
            )?
            .reply()?
            .status;
        ensure!(
            status == GrabStatus::SUCCESS,
            "another application has grabbed the pointer"
        );
        let keyboard = self
            .connection
            .grab_keyboard(
                false,
                self.root,
                CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )?
            .reply()?
            .status;
        if keyboard != GrabStatus::SUCCESS {
            self.connection.ungrab_pointer(CURRENT_TIME)?;
            anyhow::bail!("another application has grabbed the keyboard");
        }
        self.captured = true;
        let keymap = self.connection.query_keymap()?.reply()?.keys;
        let mut held = Vec::new();
        for code in 8..=255u16 {
            if keymap[(code / 8) as usize] & (1 << (code % 8)) != 0 {
                held.push(Input::Key {
                    code: code - 8,
                    down: true,
                });
            }
        }
        let p = self.connection.query_pointer(self.root)?.reply()?;
        for (mask, button) in [
            (KeyButMask::BUTTON1, 1),
            (KeyButMask::BUTTON2, 2),
            (KeyButMask::BUTTON3, 3),
        ] {
            if p.mask.contains(mask) {
                held.push(Input::Button { button, down: true });
            }
        }
        let (cx, cy) = self.center();
        self.warp(cx, cy)?;
        Ok(held)
    }
    fn release(&mut self) -> Result<()> {
        if self.captured {
            self.connection.ungrab_keyboard(CURRENT_TIME)?.check()?;
            self.connection.ungrab_pointer(CURRENT_TIME)?.check()?;
            self.captured = false;
            self.connection.flush()?;
        }
        Ok(())
    }
    fn warp(&mut self, x: i32, y: i32) -> Result<()> {
        let (x, y) = self.geometry.clamp(x, y);
        self.connection
            .warp_pointer(NONE, self.root, 0, 0, 0, 0, x as i16, y as i16)?
            .check()?;
        self.last = (x, y);
        self.connection.flush()?;
        Ok(())
    }
    fn inject(&mut self, input: Input) -> Result<()> {
        match input {
            Input::Motion { .. } => anyhow::bail!("motion must be routed to absolute coordinates"),
            Input::Key { code, down } => {
                ensure!(code <= 247, "key {code} cannot be represented by X11");
                self.connection
                    .xtest_fake_input(
                        if down {
                            xproto::KEY_PRESS_EVENT
                        } else {
                            xproto::KEY_RELEASE_EVENT
                        },
                        (code + 8) as u8,
                        CURRENT_TIME,
                        self.root,
                        0,
                        0,
                        0,
                    )?
                    .check()?;
            }
            Input::Button { button, down } => {
                let button = if button >= 4 { button + 4 } else { button };
                self.connection
                    .xtest_fake_input(
                        if down {
                            xproto::BUTTON_PRESS_EVENT
                        } else {
                            xproto::BUTTON_RELEASE_EVENT
                        },
                        button,
                        CURRENT_TIME,
                        self.root,
                        0,
                        0,
                        0,
                    )?
                    .check()?;
            }
            Input::Scroll {
                horizontal,
                vertical,
            } => {
                for (amount, positive, negative) in [(vertical, 4, 5), (horizontal, 7, 6)] {
                    for _ in 0..amount.unsigned_abs().min(120) {
                        let button = if amount > 0 { positive } else { negative };
                        self.connection
                            .xtest_fake_input(
                                xproto::BUTTON_PRESS_EVENT,
                                button,
                                CURRENT_TIME,
                                self.root,
                                0,
                                0,
                                0,
                            )?
                            .check()?;
                        self.connection
                            .xtest_fake_input(
                                xproto::BUTTON_RELEASE_EVENT,
                                button,
                                CURRENT_TIME,
                                self.root,
                                0,
                                0,
                                0,
                            )?
                            .check()?;
                    }
                }
            }
        }
        self.connection.flush()?;
        Ok(())
    }
}
impl Drop for X11 {
    fn drop(&mut self) {
        let _ = self.release();
        let _ = self.connection.free_cursor(self.cursor);
        let _ = self.connection.flush();
    }
}
