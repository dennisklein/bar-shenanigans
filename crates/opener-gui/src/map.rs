//! Map view: metal spots, build positions and builders at the current time.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use opener_format::{Action, Opener};

use crate::model::{kind_color, step_state, unit_kind, StepState};

/// Pan and zoom state, in screen pixels per elmo.
pub struct MapView {
    scale: f32,
    offset: Vec2,
    fitted: bool,
}

impl MapView {
    pub fn new() -> Self {
        Self { scale: 0.1, offset: Vec2::ZERO, fitted: false }
    }

    pub fn reset(&mut self) {
        self.fitted = false;
    }

    fn fit(&mut self, opener: &Opener, rect: Rect) {
        let (w, h) = (opener.map.size.x as f32, opener.map.size.z as f32);
        self.scale = (rect.width() / w).min(rect.height() / h) * 0.95;
        self.offset = (rect.size() - Vec2::new(w, h) * self.scale) / 2.0;
        self.fitted = true;
    }

    /// Draws the map. Returns the index of a step that the user clicked.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        opener: &Opener,
        t: f64,
        selected: Option<u32>,
    ) -> Option<u32> {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        if !self.fitted {
            self.fit(opener, rect);
        }
        if response.dragged() {
            self.offset += response.drag_delta();
        }
        if let (Some(hover), true) = (response.hover_pos(), response.hovered()) {
            let zoom = ui.input(|i| i.zoom_delta() * (i.smooth_scroll_delta.y / 200.0).exp());
            if zoom != 1.0 {
                let before = (hover - rect.min - self.offset) / self.scale;
                self.scale *= zoom;
                self.offset = hover - rect.min - before * self.scale;
            }
        }
        let to_screen = |x: f64, z: f64| rect.min + self.offset + Vec2::new(x as f32, z as f32) * self.scale;

        painter.rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        let map_rect = Rect::from_min_max(
            to_screen(0.0, 0.0),
            to_screen(opener.map.size.x, opener.map.size.z),
        );
        painter.rect_filled(map_rect, 0.0, ui.visuals().faint_bg_color);
        painter.rect_stroke(map_rect, 0.0, Stroke::new(1.0_f32, Color32::GRAY), StrokeKind::Inside);

        let spot_r = (24.0 * self.scale).max(2.0);
        for spot in &opener.map.metal_spots {
            painter.circle_stroke(
                to_screen(spot.x, spot.z),
                spot_r,
                Stroke::new(1.0_f32, Color32::from_rgb(110, 130, 170)),
            );
        }
        let start = opener.map.start_pos;
        painter.text(
            to_screen(start.x, start.z),
            Align2::CENTER_CENTER,
            "S",
            FontId::proportional(12.0),
            Color32::GRAY,
        );

        let mut clicked = None;
        let pointer = response.interact_pointer_pos().filter(|_| response.clicked());
        let mut best_hit: Option<(f32, u32)> = None;
        for step in opener.steps.iter().filter(|s| s.action == Action::Build) {
            let state = step_state(step, t);
            let base = kind_color(unit_kind(opener, &step.unit));
            let center = to_screen(step.position.x, step.position.z);
            let fp = opener.units.get(&step.unit).map_or((2, 2), |u| (u.footprint.x, u.footprint.z));
            let half = Vec2::new(fp.0 as f32, fp.1 as f32) * 4.0 * self.scale;
            let half = half.max(Vec2::splat(3.0));
            let r = Rect::from_center_size(center, half * 2.0);
            match state {
                StepState::Pending => {
                    painter.rect_stroke(r, 0.0, Stroke::new(1.0_f32, base.gamma_multiply(0.35)), StrokeKind::Inside);
                }
                StepState::Walking => {
                    painter.rect_stroke(r, 0.0, Stroke::new(1.5_f32, base), StrokeKind::Inside);
                }
                StepState::Building => {
                    let f = ((t - step.start_s) / (step.finish_s - step.start_s).max(1e-6)) as f32;
                    painter.rect_filled(r, 0.0, base.gamma_multiply(0.25));
                    let fill = Rect::from_min_max(Pos2::new(r.min.x, r.max.y - r.height() * f), r.max);
                    painter.rect_filled(fill, 0.0, base);
                }
                StepState::Done => {
                    painter.rect_filled(r, 0.0, base);
                }
            }
            if selected == Some(step.index) {
                painter.rect_stroke(r.expand(3.0), 0.0, Stroke::new(2.0_f32, Color32::WHITE), StrokeKind::Outside);
            }
            if let Some(p) = pointer {
                let d = p.distance(center);
                if d < half.max_elem() + 6.0 && best_hit.map_or(true, |(bd, _)| d < bd) {
                    best_hit = Some((d, step.index));
                }
            }
        }
        if let Some((_, index)) = best_hit {
            clicked = Some(index);
        }

        for b in opener.builders.iter().filter(|b| b.created_s <= t) {
            let pos = opener
                .builder_pos_at(b.id, t)
                .unwrap_or_else(|| builder_rest_pos(opener, b.id, t));
            let c = to_screen(pos.x, pos.z);
            painter.circle_filled(c, 5.0, Color32::WHITE);
            painter.circle_stroke(c, 5.0, Stroke::new(1.5_f32, kind_color(unit_kind(opener, &b.unit))));
            painter.text(
                c + Vec2::new(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                format!("#{}", b.id),
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        clicked
    }
}

/// Position of a builder without a track: the build position of its latest
/// own step, or the start position.
fn builder_rest_pos(opener: &Opener, id: u32, t: f64) -> opener_format::Pos {
    opener
        .steps
        .iter()
        .rfind(|s| s.builder_id == id && s.queued_s <= t)
        .map_or(opener.map.start_pos, |s| s.position)
}
