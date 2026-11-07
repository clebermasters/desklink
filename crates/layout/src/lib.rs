//! Portable desk topology, independent of UI, networking and actual display pixels.
#![forbid(unsafe_code)]
mod geometry;
use anyhow::{Result, ensure};
pub use geometry::{Edge, Geometry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const TILE_WIDTH: u32 = 240;
pub const TILE_HEIGHT: u32 = 150;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Screen {
    pub id: String,
    pub name: String,
    pub rect: Geometry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub schema: u16,
    pub revision: u64,
    pub owner: String,
    pub screens: Vec<Screen>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Crossing {
    pub target: String,
    pub entry: Edge,
    pub position: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Seam {
    pub from: String,
    pub to: String,
    pub edge: Edge,
    pub start: i32,
    pub end: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CaptureSpan {
    pub edge: Edge,
    pub start: f64,
    pub end: f64,
}
impl CaptureSpan {
    pub fn full(edge: Edge) -> Self {
        Self {
            edge,
            start: 0.0,
            end: 1.0,
        }
    }
}

pub fn key(id: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(id)?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("computer identity must be a 32-byte public key"))
}
fn right(r: Geometry) -> i32 {
    r.x + r.width as i32
}
fn bottom(r: Geometry) -> i32 {
    r.y + r.height as i32
}
pub fn overlaps(a: Geometry, b: Geometry) -> bool {
    a.x < right(b) && right(a) > b.x && a.y < bottom(b) && bottom(a) > b.y
}

impl Layout {
    pub fn capture_spans(&self, id: &str) -> Vec<CaptureSpan> {
        let Some(screen) = self.screen(id) else {
            return vec![];
        };
        let mut spans: Vec<_> = self
            .seams()
            .into_iter()
            .filter(|s| s.from == id)
            .map(|s| {
                let (base, size) = if matches!(s.edge, Edge::Left | Edge::Right) {
                    (screen.rect.y, screen.rect.height)
                } else {
                    (screen.rect.x, screen.rect.width)
                };
                CaptureSpan {
                    edge: s.edge,
                    start: (s.start - base) as f64 / size as f64,
                    end: (s.end - base) as f64 / size as f64,
                }
            })
            .collect();
        let rank = |e: Edge| match e {
            Edge::Left => 0,
            Edge::Right => 1,
            Edge::Top => 2,
            Edge::Bottom => 3,
        };
        spans.sort_by(|a, b| {
            rank(a.edge)
                .cmp(&rank(b.edge))
                .then_with(|| a.start.total_cmp(&b.start))
        });
        let mut merged: Vec<CaptureSpan> = vec![];
        for span in spans {
            if let Some(last) = merged.last_mut()
                && last.edge == span.edge
                && span.start <= last.end + 1e-9
            {
                last.end = last.end.max(span.end);
            } else {
                merged.push(span);
            }
        }
        merged
    }
    pub fn single(id: String, name: String) -> Self {
        Self {
            schema: 1,
            revision: 1,
            owner: id.clone(),
            screens: vec![Screen {
                id,
                name,
                rect: Geometry {
                    x: 0,
                    y: 0,
                    width: TILE_WIDTH,
                    height: TILE_HEIGHT,
                },
            }],
        }
    }
    pub fn screen(&self, id: &str) -> Option<&Screen> {
        self.screens.iter().find(|s| s.id == id)
    }
    pub fn screen_mut(&mut self, id: &str) -> Option<&mut Screen> {
        self.screens.iter_mut().find(|s| s.id == id)
    }
    pub fn validate(&self) -> Result<()> {
        self.validate_rectangles()?;
        ensure!(
            self.screen(&self.owner).is_some(),
            "The controller must be in the arrangement."
        );
        let mut reachable = BTreeSet::from([self.owner.as_str()]);
        loop {
            let n = reachable.len();
            for seam in self.seams() {
                if reachable.contains(seam.from.as_str()) {
                    reachable.insert(self.screen(&seam.to).unwrap().id.as_str());
                }
            }
            if reachable.len() == n {
                break;
            }
        }
        ensure!(
            reachable.len() == self.screens.len(),
            "Snap every computer to another screen so they connect to the controller."
        );
        Ok(())
    }
    pub fn validate_rectangles(&self) -> Result<()> {
        ensure!(self.schema == 1, "Unsupported layout version.");
        ensure!(
            self.revision > 0 && self.revision < u64::MAX,
            "Invalid layout revision."
        );
        key(&self.owner)?;
        ensure!(
            !self.screens.is_empty() && self.screens.len() <= 17,
            "An arrangement supports 1–17 computers."
        );
        let mut ids = BTreeSet::new();
        for (i, screen) in self.screens.iter().enumerate() {
            key(&screen.id)?;
            ensure!(
                ids.insert(screen.id.to_lowercase()),
                "A computer appears more than once."
            );
            ensure!(
                !screen.name.trim().is_empty()
                    && screen.name.len() <= 64
                    && !screen.name.chars().any(char::is_control),
                "Computer names must have 1–64 visible characters."
            );
            let r = screen.rect;
            ensure!(
                r.valid()
                    && r.width >= 80
                    && r.height >= 80
                    && r.width <= 4000
                    && r.height <= 4000
                    && r.x.unsigned_abs() <= 100_000
                    && r.y.unsigned_abs() <= 100_000,
                "Screen placement is outside the supported canvas."
            );
            for other in &self.screens[..i] {
                ensure!(
                    !overlaps(r, other.rect),
                    "{} overlaps {}. Move them apart.",
                    screen.name,
                    other.name
                );
            }
        }
        Ok(())
    }
    pub fn seams(&self) -> Vec<Seam> {
        let mut seams = Vec::new();
        for a in &self.screens {
            for b in &self.screens {
                if a.id == b.id {
                    continue;
                }
                let ar = a.rect;
                let br = b.rect;
                let relation = if right(ar) == br.x {
                    Some((Edge::Right, ar.y.max(br.y), bottom(ar).min(bottom(br))))
                } else if ar.x == right(br) {
                    Some((Edge::Left, ar.y.max(br.y), bottom(ar).min(bottom(br))))
                } else if bottom(ar) == br.y {
                    Some((Edge::Bottom, ar.x.max(br.x), right(ar).min(right(br))))
                } else if ar.y == bottom(br) {
                    Some((Edge::Top, ar.x.max(br.x), right(ar).min(right(br))))
                } else {
                    None
                };
                if let Some((edge, start, end)) = relation
                    && start < end
                {
                    seams.push(Seam {
                        from: a.id.clone(),
                        to: b.id.clone(),
                        edge,
                        start,
                        end,
                    });
                }
            }
        }
        seams
    }
    pub fn crossing(&self, from: &str, edge: Edge, position: f64) -> Option<Crossing> {
        if !position.is_finite() || !(0.0..=1.0).contains(&position) {
            return None;
        }
        let source = self.screen(from)?.rect;
        let horizontal = matches!(edge, Edge::Left | Edge::Right);
        let coordinate = if horizontal {
            source.y as f64 + position * source.height as f64
        } else {
            source.x as f64 + position * source.width as f64
        };
        let seam = self.seams().into_iter().find(|s| {
            s.from == from
                && s.edge == edge
                && coordinate >= s.start as f64
                && (coordinate < s.end as f64 || (position == 1.0 && coordinate == s.end as f64))
        })?;
        let target = self.screen(&seam.to)?.rect;
        let mapped = if horizontal {
            (coordinate - target.y as f64) / target.height as f64
        } else {
            (coordinate - target.x as f64) / target.width as f64
        };
        Some(Crossing {
            target: seam.to,
            entry: edge.opposite(),
            position: mapped.clamp(0.0, 1.0),
        })
    }
    /// Snap a dragged rectangle to a neighboring edge; reject candidates intersecting other cards.
    pub fn snap(&self, id: &str, x: i32, y: i32, distance: i32) -> Geometry {
        let current = self.screen(id).expect("screen selected from layout").rect;
        let proposed = Geometry { x, y, ..current };
        let mut candidates = Vec::new();
        for other in self.screens.iter().filter(|s| s.id != id) {
            let r = other.rect;
            for sx in [right(r), r.x - current.width as i32] {
                if (sx - x).abs() <= distance {
                    let mut ys = vec![(y, false)];
                    for sy in [
                        r.y,
                        bottom(r) - current.height as i32,
                        r.y + (r.height as i32 - current.height as i32) / 2,
                    ] {
                        if (sy - y).abs() <= distance {
                            ys.push((sy, true));
                        }
                    }
                    for (sy, aligned) in ys {
                        let c = Geometry {
                            x: sx,
                            y: sy,
                            ..current
                        };
                        if c.y < bottom(r) && bottom(c) > r.y {
                            candidates.push((c, aligned));
                        }
                    }
                }
            }
            for sy in [bottom(r), r.y - current.height as i32] {
                if (sy - y).abs() <= distance {
                    let mut xs = vec![(x, false)];
                    for sx in [
                        r.x,
                        right(r) - current.width as i32,
                        r.x + (r.width as i32 - current.width as i32) / 2,
                    ] {
                        if (sx - x).abs() <= distance {
                            xs.push((sx, true));
                        }
                    }
                    for (sx, aligned) in xs {
                        let c = Geometry {
                            x: sx,
                            y: sy,
                            ..current
                        };
                        if c.x < right(r) && right(c) > r.x {
                            candidates.push((c, aligned));
                        }
                    }
                }
            }
        }
        candidates
            .into_iter()
            .filter(|(c, _)| {
                !self
                    .screens
                    .iter()
                    .any(|s| s.id != id && overlaps(*c, s.rect))
            })
            .min_by_key(|(c, aligned)| {
                let dx = c.x as i64 - x as i64;
                let dy = c.y as i64 - y as i64;
                (!*aligned, dx * dx + dy * dy)
            })
            .map(|(c, _)| c)
            .unwrap_or(Geometry {
                x: (x as f64 / 10.0).round() as i32 * 10,
                y: (y as f64 / 10.0).round() as i32 * 10,
                ..proposed
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(n: u8) -> String {
        hex::encode([n; 32])
    }
    fn screen(n: u8, x: i32, y: i32, w: u32, h: u32) -> Screen {
        Screen {
            id: id(n),
            name: format!("Computer {n}"),
            rect: Geometry {
                x,
                y,
                width: w,
                height: h,
            },
        }
    }
    #[test]
    fn split_edges_preserve_physical_offsets() {
        let l = Layout {
            schema: 1,
            revision: 1,
            owner: id(1),
            screens: vec![
                screen(1, 0, 0, 240, 300),
                screen(2, 240, 0, 240, 150),
                screen(3, 240, 150, 240, 150),
            ],
        };
        l.validate().unwrap();
        let c = l.crossing(&id(1), Edge::Right, 0.25).unwrap();
        assert_eq!(c.target, id(2));
        assert_eq!(c.position, 0.5);
        let c = l.crossing(&id(1), Edge::Right, 0.5).unwrap();
        assert_eq!(c.target, id(3));
        assert_eq!(c.position, 0.0);
        assert_eq!(l.crossing(&id(3), Edge::Left, 0.5).unwrap().position, 0.75);
    }
    #[test]
    fn rejects_overlaps_gaps_and_corner_only_contacts() {
        let mut l = Layout::single(id(1), "Main".into());
        l.screens.push(screen(2, 100, 0, 240, 150));
        assert!(l.validate().is_err());
        l.screens[1].rect.x = 250;
        assert!(l.validate().is_err());
        l.screens[1].rect.x = 240;
        l.screens[1].rect.y = 150;
        assert!(l.validate().is_err());
        l.screens[1].rect.y = 0;
        l.validate().unwrap();
    }
    #[test]
    fn snaps_to_edges_without_overlapping() {
        let mut l = Layout::single(id(1), "Main".into());
        l.screens.push(screen(2, 500, 0, 240, 150));
        let r = l.snap(&id(2), 251, 7, 24);
        assert_eq!((r.x, r.y), (240, 0));
        l.screens[1].rect = r;
        l.validate().unwrap();
    }
    #[test]
    fn chain_and_vertical_neighbors_have_reciprocal_seams() {
        let l = Layout {
            schema: 1,
            revision: 1,
            owner: id(1),
            screens: vec![
                screen(1, 0, 0, 240, 150),
                screen(2, 240, 0, 240, 150),
                screen(3, 480, 0, 240, 150),
                screen(4, 240, -150, 240, 150),
            ],
        };
        l.validate().unwrap();
        assert_eq!(l.crossing(&id(2), Edge::Right, 0.5).unwrap().target, id(3));
        assert_eq!(l.crossing(&id(2), Edge::Top, 0.5).unwrap().target, id(4));
    }
}
