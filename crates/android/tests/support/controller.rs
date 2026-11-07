//! A headless Desklink controller for exercising a receiver without capturing a real desktop.
//! Shared by the in-process integration test and the `controller` example used on emulators.
#![allow(dead_code)]
use anyhow::{Context, Result, bail};
use desklink_protocol::{
    ClipboardContent, ClipboardKind, ClipboardTransfer, Edge, Geometry, Input, Message, Stamp,
    VERSION,
};
use desklink_transport::{Identity, NetworkEvent, PeerHandle};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::{net::TcpStream, sync::mpsc, time::timeout};
use tokio_util::sync::CancellationToken;

pub struct Controller {
    pub key: [u8; 32],
    pub phone: [u8; 32],
    pub phone_geometry: Geometry,
    handle: PeerHandle,
    events: mpsc::Receiver<NetworkEvent>,
    cancel: CancellationToken,
    session: Option<Stamp>,
    counter: u64,
    clipboard_counter: u64,
    pub x: i32,
    pub y: i32,
}

impl Controller {
    pub async fn connect(address: &str, identity: Identity, phone: [u8; 32]) -> Result<Self> {
        let key = identity.public;
        let identity = Arc::new(identity);
        let trusted = Arc::new(HashSet::from([phone]));
        let (tx, mut events) = mpsc::channel(512);
        let cancel = CancellationToken::new();
        let hello = Message::Hello {
            version: VERSION,
            name: "Headless controller".into(),
            geometry: Geometry {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            clipboard: true,
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let socket = loop {
            match TcpStream::connect(address).await {
                Ok(socket) => break socket,
                Err(error) if tokio::time::Instant::now() < deadline => {
                    tracing::debug!(%error, "receiver not reachable yet");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(error) => return Err(error).context("receiver is not reachable"),
            }
        };
        let session_cancel = cancel.clone();
        tokio::spawn(async move {
            if let Err(error) = desklink_transport::session(
                socket,
                true,
                identity,
                trusted,
                hello,
                tx,
                session_cancel,
            )
            .await
            {
                eprintln!("controller session ended: {error:#}");
            }
        });
        let connected = timeout(Duration::from_secs(15), events.recv())
            .await
            .context("handshake did not complete")?
            .context("session closed during handshake")?;
        let NetworkEvent::Connected {
            key: remote,
            hello,
            handle,
        } = connected
        else {
            bail!("unexpected first network event");
        };
        anyhow::ensure!(remote == phone, "connected to an unexpected receiver");
        let Message::Hello { geometry, .. } = hello else {
            bail!("receiver did not say hello");
        };
        Ok(Self {
            key,
            phone,
            phone_geometry: geometry,
            handle,
            events,
            cancel,
            session: None,
            counter: 0,
            clipboard_counter: 0,
            x: 0,
            y: 0,
        })
    }

    /// Waits for the next control message, skipping keepalives.
    pub async fn next_message(&mut self, wait: Duration) -> Result<Message> {
        timeout(wait, async {
            loop {
                match self.events.recv().await {
                    Some(NetworkEvent::Message { message, .. }) => {
                        if !matches!(message, Message::Ping(_) | Message::Pong(_)) {
                            return Ok(message);
                        }
                    }
                    Some(NetworkEvent::Disconnected { .. }) | None => {
                        bail!("receiver disconnected")
                    }
                    Some(_) => {}
                }
            }
        })
        .await
        .context("timed out waiting for the receiver")?
    }

    pub async fn next_clipboard(&mut self, wait: Duration) -> Result<ClipboardContent> {
        timeout(wait, async {
            loop {
                match self.events.recv().await {
                    Some(NetworkEvent::Clipboard { transfer, .. }) => return Ok(transfer.content),
                    Some(NetworkEvent::Disconnected { .. }) | None => {
                        bail!("receiver disconnected")
                    }
                    Some(_) => {}
                }
            }
        })
        .await
        .context("timed out waiting for the receiver's clipboard")?
    }

    pub async fn enter(&mut self, edge: Edge, position: f64) -> Result<()> {
        self.counter += 1;
        let session = Stamp {
            counter: self.counter,
            origin: self.key,
        };
        self.handle.send(Message::Enter {
            session,
            edge,
            position,
        })?;
        loop {
            match self.next_message(Duration::from_secs(5)).await? {
                Message::Entered(ack) if ack == session => break,
                // A receiver that selected us as its controller asks for our screen layout.
                Message::LayoutRequest | Message::Geometry(_) => {}
                other => bail!("receiver refused control: {other:?}"),
            }
        }
        self.session = Some(session);
        (self.x, self.y) = self.phone_geometry.entry(edge, position);
        Ok(())
    }

    pub fn input(&mut self, event: Input) -> Result<()> {
        let session = self.session.context("enter the receiver first")?;
        if let Input::Motion { dx, dy } = event {
            (self.x, self.y) = self
                .phone_geometry
                .clamp(self.x.saturating_add(dx), self.y.saturating_add(dy));
        }
        self.handle.send(Message::Input { session, event })
    }

    /// Moves like a real mouse: many small relative steps.
    pub async fn move_by(&mut self, dx: i32, dy: i32) -> Result<()> {
        let steps = (dx.abs().max(dy.abs()) / 8).max(1);
        let (mut sent_x, mut sent_y) = (0, 0);
        for i in 1..=steps {
            let (tx, ty) = (dx * i / steps, dy * i / steps);
            self.input(Input::Motion {
                dx: tx - sent_x,
                dy: ty - sent_y,
            })?;
            (sent_x, sent_y) = (tx, ty);
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
        Ok(())
    }

    pub async fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.move_by(x - self.x, y - self.y).await
    }

    pub async fn click(&mut self, button: u8) -> Result<()> {
        self.input(Input::Button { button, down: true })?;
        tokio::time::sleep(Duration::from_millis(60)).await;
        self.input(Input::Button {
            button,
            down: false,
        })
    }

    pub async fn key(&mut self, code: u16) -> Result<()> {
        self.input(Input::Key { code, down: true })?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.input(Input::Key { code, down: false })?;
        tokio::time::sleep(Duration::from_millis(20)).await;
        Ok(())
    }

    pub async fn type_text(&mut self, text: &str) -> Result<()> {
        for c in text.chars() {
            let (code, shift) = evdev_for(c).with_context(|| format!("cannot type {c:?}"))?;
            if shift {
                self.input(Input::Key {
                    code: 42,
                    down: true,
                })?;
            }
            self.key(code).await?;
            if shift {
                self.input(Input::Key {
                    code: 42,
                    down: false,
                })?;
            }
        }
        Ok(())
    }

    pub fn copy_text(&mut self, text: &str) {
        self.clipboard_counter += 1;
        self.handle.clipboard(Arc::new(ClipboardTransfer {
            stamp: Stamp {
                counter: 1_000_000 + self.clipboard_counter,
                origin: self.key,
            },
            content: ClipboardContent {
                kind: ClipboardKind::Text,
                bytes: text.as_bytes().to_vec(),
            },
        }));
    }

    /// Moves into the edge control entered from, and waits for the receiver to hand it back.
    pub async fn leave_through(&mut self, edge: Edge) -> Result<f64> {
        let (dx, dy) = match edge {
            Edge::Left => (-1, 0),
            Edge::Right => (1, 0),
            Edge::Top => (0, -1),
            Edge::Bottom => (0, 1),
        };
        let travel = self.phone_geometry.width.max(self.phone_geometry.height) as i32 + 50;
        let step = 40;
        let mut moved = 0;
        while moved < travel {
            self.input(Input::Motion {
                dx: dx * step,
                dy: dy * step,
            })?;
            moved += step;
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
        loop {
            match self.next_message(Duration::from_secs(5)).await? {
                Message::Return { session, position } if Some(session) == self.session => {
                    self.session = None;
                    return Ok(position);
                }
                Message::Traverse { .. } => bail!("receiver asked to traverse instead of return"),
                _ => {}
            }
        }
    }

    pub fn close(&self) {
        self.cancel.cancel();
    }
}

/// US physical key positions for typing test text.
pub fn evdev_for(c: char) -> Option<(u16, bool)> {
    const ROW_Q: &str = "qwertyuiop";
    const ROW_A: &str = "asdfghjkl";
    const ROW_Z: &str = "zxcvbnm";
    let lower = c.to_ascii_lowercase();
    let shift = c.is_ascii_uppercase();
    if let Some(i) = ROW_Q.find(lower) {
        return Some((16 + i as u16, shift));
    }
    if let Some(i) = ROW_A.find(lower) {
        return Some((30 + i as u16, shift));
    }
    if let Some(i) = ROW_Z.find(lower) {
        return Some((44 + i as u16, shift));
    }
    Some(match c {
        '1'..='9' => (2 + (c as u16 - '1' as u16), false),
        '0' => (11, false),
        ' ' => (57, false),
        '-' => (12, false),
        '.' => (52, false),
        ',' => (51, false),
        '/' => (53, false),
        '!' => (2, true),
        '@' => (3, true),
        '?' => (53, true),
        '_' => (12, true),
        _ => return None,
    })
}
