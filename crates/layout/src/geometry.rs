use serde::{Deserialize, Serialize};
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
