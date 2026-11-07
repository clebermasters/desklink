use crate::{
    app::{DesklinkApp, Drag, short},
    theme as t,
};
use desklink_layout::{Edge, Geometry};
use eframe::egui::{
    self, Align2, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2,
};

impl DesklinkApp {
    pub fn canvas(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let painter = ui.painter().with_clip_rect(rect);
        painter.rect_filled(rect, 12.0, t::CANVAS);
        painter.rect_stroke(rect, 12.0, (1.0, t::BORDER), StrokeKind::Inside);
        self.target("canvas", rect);
        if self.fit {
            let minx = self
                .draft
                .screens
                .iter()
                .map(|s| s.rect.x)
                .min()
                .unwrap_or(0) as f32;
            let maxx = self
                .draft
                .screens
                .iter()
                .map(|s| s.rect.x + s.rect.width as i32)
                .max()
                .unwrap_or(240) as f32;
            let miny = self
                .draft
                .screens
                .iter()
                .map(|s| s.rect.y)
                .min()
                .unwrap_or(0) as f32;
            let maxy = self
                .draft
                .screens
                .iter()
                .map(|s| s.rect.y + s.rect.height as i32)
                .max()
                .unwrap_or(150) as f32;
            self.center = egui::vec2((minx + maxx) / 2.0, (miny + maxy) / 2.0);
            self.zoom = ((rect.width() - 80.0) / (maxx - minx).max(240.0))
                .min((rect.height() - 80.0) / (maxy - miny).max(150.0))
                .clamp(0.35, 1.15);
            self.fit = false;
        }
        let center = self.center;
        let zoom = self.zoom;
        let transform =
            |x: f32, y: f32| -> Pos2 { rect.center() + (egui::vec2(x, y) - center) * zoom };
        let origin = transform(0.0, 0.0);
        let spacing = (20.0 * self.zoom).max(8.0);
        let startx = rect.left() + (origin.x - rect.left()).rem_euclid(spacing);
        let starty = rect.top() + (origin.y - rect.top()).rem_euclid(spacing);
        for x in 0..((rect.width() / spacing) as usize + 1) {
            for y in 0..((rect.height() / spacing) as usize + 1) {
                painter.circle_filled(
                    egui::pos2(startx + x as f32 * spacing, starty + y as f32 * spacing),
                    0.85,
                    Color32::from_rgb(209, 218, 233),
                );
            }
        }
        let background = ui.interact(rect, Id::unique("desk-canvas"), Sense::click_and_drag());
        let pressed_card = ui.input(|i| i.pointer.press_origin()).is_some_and(|p| {
            self.draft.screens.iter().any(|s| {
                Rect::from_min_size(
                    transform(s.rect.x as f32, s.rect.y as f32),
                    egui::vec2(s.rect.width as f32, s.rect.height as f32) * zoom,
                )
                .contains(p)
            })
        });
        if background.dragged() && self.drag.is_none() && !pressed_card {
            self.center -= ui.input(|i| i.pointer.delta()) / self.zoom;
        }
        if self.draft.validate_rectangles().is_ok() {
            for seam in self.draft.seams() {
                if seam.from > seam.to {
                    continue;
                }
                let r = self.draft.screen(&seam.from).unwrap().rect;
                let (a, b) = match seam.edge {
                    Edge::Right => (
                        transform((r.x + r.width as i32) as f32, seam.start as f32),
                        transform((r.x + r.width as i32) as f32, seam.end as f32),
                    ),
                    Edge::Left => (
                        transform(r.x as f32, seam.start as f32),
                        transform(r.x as f32, seam.end as f32),
                    ),
                    Edge::Top => (
                        transform(seam.start as f32, r.y as f32),
                        transform(seam.end as f32, r.y as f32),
                    ),
                    Edge::Bottom => (
                        transform(seam.start as f32, (r.y + r.height as i32) as f32),
                        transform(seam.end as f32, (r.y + r.height as i32) as f32),
                    ),
                };
                painter.line_segment([a, b], Stroke::new(5.0, Color32::from_rgb(171, 190, 247)));
            }
        }
        let screens = self.draft.screens.clone();
        let editable = self.editable();
        for screen in screens {
            let r = screen.rect;
            let card = Rect::from_min_size(
                transform(r.x as f32, r.y as f32),
                egui::vec2(r.width as f32, r.height as f32) * self.zoom,
            );
            let selected = self.selected == screen.id;
            let overlap = self
                .draft
                .screens
                .iter()
                .any(|s| s.id != screen.id && desklink_layout::overlaps(r, s.rect));
            painter.rect_filled(
                card.translate(egui::vec2(0.0, 4.0)).expand(2.0),
                10.0,
                Color32::from_black_alpha(7),
            );
            painter.rect_filled(card, 9.0, t::WHITE);
            painter.rect_stroke(
                card,
                9.0,
                Stroke::new(
                    if selected { 2.0 } else { 1.0 },
                    if overlap {
                        t::RED
                    } else if selected {
                        t::BLUE
                    } else {
                        t::BORDER
                    },
                ),
                StrokeKind::Inside,
            );
            let response = ui.interact(
                card,
                Id::unique(("screen", &screen.id)),
                if editable {
                    Sense::click_and_drag()
                } else {
                    Sense::click()
                },
            );
            self.target(format!("screen:{}", screen.id), card);
            if response.clicked() {
                self.selected = screen.id.clone();
                response.request_focus();
            }
            if response.drag_started() {
                self.selected = screen.id.clone();
                if let Some(pointer) = ui.input(|i| i.pointer.press_origin()) {
                    self.drag = Some(Drag {
                        id: screen.id.clone(),
                        pointer,
                        rect: r,
                        resize: false,
                    });
                }
            }
            if response.dragged()
                && let Some(drag) = &self.drag
                && drag.id == screen.id
                && !drag.resize
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let delta = (pointer - drag.pointer) / self.zoom;
                let next = Geometry {
                    x: (drag.rect.x + (delta.x.round() as i32)).clamp(-5000, 5000),
                    y: (drag.rect.y + (delta.y.round() as i32)).clamp(-5000, 5000),
                    ..r
                };
                if let Some(node) = self.draft.screen_mut(&screen.id) {
                    node.rect = next;
                }
                self.dirty = true;
            }
            if response.drag_stopped()
                && self
                    .drag
                    .as_ref()
                    .is_some_and(|d| d.id == screen.id && !d.resize)
            {
                let moved = self.draft.screen(&screen.id).unwrap().rect;
                let snapped =
                    self.draft
                        .snap(&screen.id, moved.x, moved.y, (20.0 / self.zoom) as i32);
                self.draft.screen_mut(&screen.id).unwrap().rect = snapped;
                self.drag = None;
                self.mark_dirty();
            }
            if response.has_focus() && editable {
                let delta = ui.input(|i| {
                    egui::vec2(
                        if i.key_pressed(egui::Key::ArrowRight) {
                            10.0
                        } else if i.key_pressed(egui::Key::ArrowLeft) {
                            -10.0
                        } else {
                            0.0
                        },
                        if i.key_pressed(egui::Key::ArrowDown) {
                            10.0
                        } else if i.key_pressed(egui::Key::ArrowUp) {
                            -10.0
                        } else {
                            0.0
                        },
                    )
                });
                if delta != Vec2::ZERO {
                    if let Some(node) = self.draft.screen_mut(&screen.id) {
                        node.rect.x += delta.x as i32;
                        node.rect.y += delta.y as i32;
                    }
                    self.mark_dirty();
                }
            }
            let scale = self.zoom.min(1.0);
            t::monitor(
                &painter,
                egui::pos2(card.center().x, card.top() + 35.0 * scale),
                28.0 * scale,
                if selected { t::BLUE } else { t::MUTED },
            );
            painter.text(
                egui::pos2(card.center().x, card.top() + 72.0 * scale),
                Align2::CENTER_CENTER,
                short(&screen.name, 22),
                FontId::proportional(15.0 * scale.max(0.8)),
                t::INK,
            );
            let badge = if screen.id == self.own {
                "THIS COMPUTER"
            } else if self.connected(&screen.id) {
                "CONNECTED"
            } else if self.status.running {
                "NOT CONNECTED"
            } else {
                "PAIRED"
            };
            let pill = Rect::from_center_size(
                egui::pos2(card.center().x, card.bottom() - 24.0 * scale),
                egui::vec2(122.0, 22.0) * scale,
            );
            painter.rect_filled(
                pill,
                5.0,
                if self.connected(&screen.id) {
                    Color32::from_rgb(231, 247, 241)
                } else {
                    t::BG
                },
            );
            painter.text(
                pill.center(),
                Align2::CENTER_CENTER,
                badge,
                FontId::proportional(9.0 * scale.max(0.9)),
                if self.connected(&screen.id) {
                    t::GREEN
                } else {
                    t::MUTED
                },
            );
            if screen.id == self.draft.owner {
                painter.circle_filled(card.left_top() + egui::vec2(14.0, 14.0), 3.0, t::BLUE);
            }
            if editable && selected {
                let handle = Rect::from_center_size(
                    card.right_bottom() - egui::vec2(7.0, 7.0),
                    egui::vec2(16.0, 16.0),
                );
                let resize = ui.interact(handle, Id::unique(("resize", &screen.id)), Sense::drag());
                self.target(format!("resize:{}", screen.id), handle);
                painter.line_segment(
                    [
                        card.right_bottom() - egui::vec2(14.0, 5.0),
                        card.right_bottom() - egui::vec2(5.0, 14.0),
                    ],
                    Stroke::new(1.5, t::MUTED),
                );
                if resize.drag_started()
                    && let Some(pointer) = ui.input(|i| i.pointer.press_origin())
                {
                    self.drag = Some(Drag {
                        id: screen.id.clone(),
                        pointer,
                        rect: r,
                        resize: true,
                    });
                }
                if resize.dragged()
                    && let Some(drag) = &self.drag
                    && drag.id == screen.id
                    && drag.resize
                    && let Some(pointer) = resize.interact_pointer_pos()
                {
                    let delta = (pointer - drag.pointer) / self.zoom;
                    if let Some(node) = self.draft.screen_mut(&screen.id) {
                        node.rect.width = (drag.rect.width as f32 + delta.x)
                            .round()
                            .clamp(160.0, 1200.0) as u32;
                        node.rect.height = (drag.rect.height as f32 + delta.y)
                            .round()
                            .clamp(100.0, 900.0) as u32;
                    }
                    self.dirty = true;
                }
                if resize.drag_stopped()
                    && self
                        .drag
                        .as_ref()
                        .is_some_and(|d| d.id == screen.id && d.resize)
                {
                    self.drag = None;
                    self.mark_dirty();
                }
            }
        }
        painter.text(
            rect.left_bottom() + egui::vec2(16.0, -16.0),
            Align2::LEFT_BOTTOM,
            if editable {
                "Touching edges connect  ·  Drag empty space to pan"
            } else {
                "Read-only layout from controller"
            },
            FontId::proportional(10.0),
            t::MUTED,
        );
    }
}
