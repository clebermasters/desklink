//! Versioned, bounded messages shared by every backend. Contains no OS or socket code.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const VERSION: u16 = 1;
pub const DEFAULT_PORT: u16 = 24880;
pub const MAX_FRAME: usize = 32 * 1024;
pub const CLIPBOARD_CHUNK: usize = 24 * 1024;
pub const MAX_CLIPBOARD: usize = 16 * 1024 * 1024;
pub const MAX_TEXT: usize = 1024 * 1024;
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;

/// Physical Linux evdev key identifier; X11 uses evdev + 8. macOS translates explicitly.
pub type KeyCode = u16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }
}

impl std::str::FromStr for Edge {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "left" => Ok(Self::Left),
            "right" => Ok(Self::Right),
            "top" => Ok(Self::Top),
            "bottom" => Ok(Self::Bottom),
            _ => Err("edge must be left, right, top or bottom".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Geometry {
    pub fn valid(self) -> bool {
        self.width > 2
            && self.height > 2
            && self.width <= 65535
            && self.height <= 65535
            && self.x.checked_add(self.width as i32 - 1).is_some()
            && self.y.checked_add(self.height as i32 - 1).is_some()
    }
    pub fn clamp(self, x: i32, y: i32) -> (i32, i32) {
        (
            x.clamp(self.x, self.x.saturating_add(self.width as i32 - 1)),
            y.clamp(self.y, self.y.saturating_add(self.height as i32 - 1)),
        )
    }
    pub fn entry(self, edge: Edge, position: f64) -> (i32, i32) {
        let t = position.clamp(0.0, 1.0);
        let x = self.x + (t * (self.width - 1) as f64).round() as i32;
        let y = self.y + (t * (self.height - 1) as f64).round() as i32;
        match edge {
            Edge::Left => (self.x + 1, y),
            Edge::Right => (self.x + self.width as i32 - 2, y),
            Edge::Top => (x, self.y + 1),
            Edge::Bottom => (x, self.y + self.height as i32 - 2),
        }
    }
    pub fn edge_at(self, x: i32, y: i32, dx: i32, dy: i32) -> Option<(Edge, f64)> {
        if x <= self.x && dx < 0 {
            Some((Edge::Left, (y - self.y) as f64 / (self.height - 1) as f64))
        } else if x >= self.x + self.width as i32 - 1 && dx > 0 {
            Some((Edge::Right, (y - self.y) as f64 / (self.height - 1) as f64))
        } else if y <= self.y && dy < 0 {
            Some((Edge::Top, (x - self.x) as f64 / (self.width - 1) as f64))
        } else if y >= self.y + self.height as i32 - 1 && dy > 0 {
            Some((Edge::Bottom, (x - self.x) as f64 / (self.width - 1) as f64))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Input {
    Motion { dx: i32, dy: i32 },
    Key { code: KeyCode, down: bool },
    Button { button: u8, down: bool },
    Scroll { horizontal: i32, vertical: i32 },
}

impl Input {
    pub fn valid(self) -> bool {
        match self {
            Self::Motion { dx, dy } => dx.unsigned_abs() <= 100_000 && dy.unsigned_abs() <= 100_000,
            Self::Key { code, .. } => code > 0 && code <= 767,
            Self::Button { button, .. } => (1..=8).contains(&button),
            Self::Scroll {
                horizontal,
                vertical,
            } => horizontal.unsigned_abs() <= 120 && vertical.unsigned_abs() <= 120,
        }
    }
}

/// Total ordering resolves concurrent clipboard changes without bouncing them back and forth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub counter: u64,
    pub origin: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardKind {
    Text,
    Png,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardContent {
    pub kind: ClipboardKind,
    pub bytes: Vec<u8>,
}

impl ClipboardContent {
    pub fn hash(&self) -> [u8; 32] {
        let mut hash = Sha256::new();
        hash.update([match self.kind {
            ClipboardKind::Text => 0,
            ClipboardKind::Png => 1,
        }]);
        hash.update(&self.bytes);
        hash.finalize().into()
    }
    pub fn valid(&self) -> bool {
        match self.kind {
            ClipboardKind::Text => {
                self.bytes.len() <= MAX_TEXT && std::str::from_utf8(&self.bytes).is_ok()
            }
            ClipboardKind::Png => {
                if self.bytes.len() < 33
                    || self.bytes.len() > MAX_CLIPBOARD
                    || !self.bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                    || &self.bytes[12..16] != b"IHDR"
                {
                    return false;
                }
                let w = u32::from_be_bytes(self.bytes[16..20].try_into().unwrap());
                let h = u32::from_be_bytes(self.bytes[20..24].try_into().unwrap());
                w > 0 && h > 0 && (w as u64) * (h as u64) <= MAX_PIXELS
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClipboardTransfer {
    pub stamp: Stamp,
    pub content: ClipboardContent,
}

impl ClipboardTransfer {
    pub fn messages(&self) -> impl Iterator<Item = Message> + '_ {
        std::iter::once(Message::ClipboardStart {
            stamp: self.stamp,
            kind: self.content.kind,
            size: self.content.bytes.len() as u32,
            hash: self.content.hash(),
        })
        .chain(
            self.content
                .bytes
                .chunks(CLIPBOARD_CHUNK)
                .map(|data| Message::ClipboardChunk {
                    data: data.to_vec(),
                }),
        )
        .chain(std::iter::once(Message::ClipboardEnd))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Hello {
        version: u16,
        name: String,
        geometry: Geometry,
        clipboard: bool,
    },
    Enter {
        session: Stamp,
        edge: Edge,
        position: f64,
    },
    Entered(Stamp),
    Return {
        session: Stamp,
        position: f64,
    },
    Release(Stamp),
    Input {
        session: Stamp,
        event: Input,
    },
    Geometry(Geometry),
    Ping(u64),
    Pong(u64),
    ClipboardStart {
        stamp: Stamp,
        kind: ClipboardKind,
        size: u32,
        hash: [u8; 32],
    },
    ClipboardChunk {
        data: Vec<u8>,
    },
    ClipboardEnd,
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("message exceeds the frame limit")]
    Oversized,
    #[error("invalid wire message: {0}")]
    Decode(#[from] postcard::Error),
    #[error("invalid clipboard transfer")]
    Clipboard,
}

pub fn encode(message: &Message) -> Result<Vec<u8>, ProtocolError> {
    let bytes = postcard::to_allocvec(message)?;
    if bytes.len() > MAX_FRAME {
        return Err(ProtocolError::Oversized);
    }
    Ok(bytes)
}

pub fn decode(bytes: &[u8]) -> Result<Message, ProtocolError> {
    if bytes.len() > MAX_FRAME {
        return Err(ProtocolError::Oversized);
    }
    let (message, remaining) = postcard::take_from_bytes(bytes)?;
    if !remaining.is_empty() {
        return Err(ProtocolError::Decode(
            postcard::Error::DeserializeBadEncoding,
        ));
    }
    Ok(message)
}

#[derive(Default)]
pub struct ClipboardReceiver {
    pending: Option<Pending>,
}

struct Pending {
    stamp: Stamp,
    content: ClipboardContent,
    size: usize,
    hash: [u8; 32],
}

impl ClipboardReceiver {
    pub fn accept(&mut self, message: Message) -> Result<Option<ClipboardTransfer>, ProtocolError> {
        match message {
            Message::ClipboardStart {
                stamp,
                kind,
                size,
                hash,
            } => {
                let limit = if kind == ClipboardKind::Text {
                    MAX_TEXT
                } else {
                    MAX_CLIPBOARD
                };
                if size as usize > limit || self.pending.is_some() {
                    return Err(ProtocolError::Clipboard);
                }
                self.pending = Some(Pending {
                    stamp,
                    content: ClipboardContent {
                        kind,
                        bytes: Vec::new(),
                    },
                    size: size as usize,
                    hash,
                });
                Ok(None)
            }
            Message::ClipboardChunk { data } => {
                let p = self.pending.as_mut().ok_or(ProtocolError::Clipboard)?;
                if data.len() > CLIPBOARD_CHUNK
                    || data.is_empty()
                    || p.content.bytes.len() + data.len() > p.size
                {
                    return Err(ProtocolError::Clipboard);
                }
                p.content.bytes.extend(data);
                Ok(None)
            }
            Message::ClipboardEnd => {
                let p = self.pending.take().ok_or(ProtocolError::Clipboard)?;
                if p.content.bytes.len() != p.size
                    || p.content.hash() != p.hash
                    || !p.content.valid()
                {
                    return Err(ProtocolError::Clipboard);
                }
                Ok(Some(ClipboardTransfer {
                    stamp: p.stamp,
                    content: p.content,
                }))
            }
            _ => Err(ProtocolError::Clipboard),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_round_trip_is_chunked_and_verified() {
        let t = ClipboardTransfer {
            stamp: Stamp {
                counter: 3,
                origin: [7; 32],
            },
            content: ClipboardContent {
                kind: ClipboardKind::Text,
                bytes: vec![b'x'; 100_000],
            },
        };
        let mut receiver = ClipboardReceiver::default();
        let mut result = None;
        for message in t.messages() {
            result = receiver
                .accept(decode(&encode(&message).unwrap()).unwrap())
                .unwrap()
                .or(result);
        }
        assert_eq!(result.unwrap().content, t.content);
    }
    #[test]
    fn rejects_corruption_and_unbounded_transfers() {
        let mut r = ClipboardReceiver::default();
        assert!(
            r.accept(Message::ClipboardStart {
                stamp: Stamp {
                    counter: 0,
                    origin: [0; 32]
                },
                kind: ClipboardKind::Text,
                size: MAX_TEXT as u32 + 1,
                hash: [0; 32]
            })
            .is_err()
        );
        assert!(r.accept(Message::ClipboardChunk { data: vec![1] }).is_err());
        r.accept(Message::ClipboardStart {
            stamp: Stamp {
                counter: 1,
                origin: [0; 32],
            },
            kind: ClipboardKind::Text,
            size: 1,
            hash: [0; 32],
        })
        .unwrap();
        r.accept(Message::ClipboardChunk { data: vec![1] }).unwrap();
        assert!(r.accept(Message::ClipboardEnd).is_err());
    }
    #[test]
    fn rejects_trailing_bytes() {
        let mut b = encode(&Message::Pong(0)).unwrap();
        b.push(1);
        assert!(decode(&b).is_err());
    }
    #[test]
    fn position_preserves_offset_and_rejects_outward_only() {
        let g = Geometry {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(g.entry(Edge::Right, 0.5), (-2, 540));
        assert!(g.edge_at(-1920, 540, 1, 0).is_none());
        assert_eq!(g.edge_at(-1920, 540, -1, 0).unwrap().0, Edge::Left);
    }
    #[test]
    fn rejects_overflowing_geometry_and_png_dimensions() {
        assert!(
            !Geometry {
                x: i32::MAX,
                y: 0,
                width: 800,
                height: 600
            }
            .valid()
        );
        let mut data = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(&u32::MAX.to_be_bytes());
        data.extend_from_slice(&[0; 9]);
        assert!(
            !ClipboardContent {
                kind: ClipboardKind::Png,
                bytes: data
            }
            .valid()
        );
    }
}
