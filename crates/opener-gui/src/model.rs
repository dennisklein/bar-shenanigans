//! Derived views of an [`Opener`] at a point in time.

use eframe::egui::Color32;
use opener_format::{Opener, Step, UnitKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    /// The builder has not received the order yet.
    Pending,
    /// The builder has the order and moves to the build position.
    Walking,
    Building,
    Done,
}

pub fn step_state(step: &Step, t: f64) -> StepState {
    if t >= step.finish_s {
        StepState::Done
    } else if t >= step.start_s {
        StepState::Building
    } else if t >= step.queued_s {
        StepState::Walking
    } else {
        StepState::Pending
    }
}

pub fn kind_color(kind: UnitKind) -> Color32 {
    match kind {
        UnitKind::Commander => Color32::from_rgb(240, 200, 60),
        UnitKind::Constructor => Color32::from_rgb(120, 200, 120),
        UnitKind::Factory => Color32::from_rgb(200, 140, 90),
        UnitKind::MetalExtractor => Color32::from_rgb(150, 170, 210),
        UnitKind::EnergyProducer => Color32::from_rgb(240, 220, 80),
        UnitKind::MetalMaker => Color32::from_rgb(200, 120, 200),
        UnitKind::Storage => Color32::from_rgb(160, 160, 160),
        UnitKind::Other => Color32::from_rgb(190, 100, 100),
    }
}

pub fn unit_kind(opener: &Opener, unit: &str) -> UnitKind {
    opener.units.get(unit).map_or(UnitKind::Other, |u| u.kind)
}

pub fn unit_name<'a>(opener: &'a Opener, unit: &'a str) -> &'a str {
    opener.units.get(unit).map_or(unit, |u| u.human_name.as_str())
}

/// End of the timeline: the last step or sample, whichever is later.
pub fn duration(opener: &Opener) -> f64 {
    let steps = opener.steps.iter().map(|s| s.finish_s).fold(0.0, f64::max);
    let samples = opener.samples.last().map_or(0.0, |s| s.t_s);
    steps.max(samples).max(1.0)
}

pub fn format_time(t: f64) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_time() {
        assert_eq!(format_time(75.5), "1:15.5");
        assert_eq!(format_time(-1.0), "0:00.0");
    }
}
