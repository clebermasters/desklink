use anyhow::{Context, Result, ensure};
use ashpd::desktop::input_capture::{Barrier, BarrierID, BarrierPosition, Region};
use desklink_protocol::{CaptureSpan, Edge, Geometry};
use std::collections::HashMap;

pub fn build(
    zones: &[Region],
    geometry: Geometry,
    spans: &[CaptureSpan],
) -> Result<(Vec<Barrier>, HashMap<BarrierID, Edge>)> {
    let mut barriers = vec![];
    let mut edges = HashMap::new();
    for region in zones {
        for span in spans {
            ensure!(
                span.start.is_finite()
                    && span.end.is_finite()
                    && span.start >= 0.0
                    && span.end <= 1.0
                    && span.start < span.end,
                "Invalid capture span."
            );
            let (x, y, w, h) = (
                region.x_offset(),
                region.y_offset(),
                region.width() as i32,
                region.height() as i32,
            );
            let horizontal = matches!(span.edge, Edge::Left | Edge::Right);
            let (origin, size) = if horizontal {
                (geometry.y, geometry.height)
            } else {
                (geometry.x, geometry.width)
            };
            let (start, end) = pixel_span(origin, size, span.start, span.end);
            let (start, end) = if horizontal {
                (start.max(y), end.min(y + h - 1))
            } else {
                (start.max(x), end.min(x + w - 1))
            };
            if start > end {
                continue;
            }
            let line = match span.edge {
                Edge::Left if x == geometry.x => Some((x, start, x, end)),
                Edge::Right if x + w == geometry.x + geometry.width as i32 => {
                    Some((x + w, start, x + w, end))
                }
                Edge::Top if y == geometry.y => Some((start, y, end, y)),
                Edge::Bottom if y + h == geometry.y + geometry.height as i32 => {
                    Some((start, y + h, end, y + h))
                }
                _ => None,
            };
            if let Some((x1, y1, x2, y2)) = line {
                let id =
                    BarrierID::new((barriers.len() + 1) as u32).context("Invalid barrier id")?;
                barriers.push(Barrier::new(id, BarrierPosition::new(x1, y1, x2, y2)));
                edges.insert(id, span.edge);
            }
        }
    }
    Ok((barriers, edges))
}

fn pixel_span(origin: i32, size: u32, start: f64, end: f64) -> (i32, i32) {
    let low = origin + (start * (size - 1) as f64).ceil() as i32;
    let high = if end == 1.0 {
        origin + size as i32 - 1
    } else {
        origin + (end * (size - 1) as f64).ceil() as i32 - 1
    };
    (low, high)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_seams_cover_pixels_without_overlap() {
        assert_eq!(pixel_span(0, 600, 0.0, 0.5), (0, 299));
        assert_eq!(pixel_span(0, 600, 0.5, 1.0), (300, 599));
        assert_eq!(pixel_span(-100, 600, 0.1, 0.9), (-40, 439));
    }
}
