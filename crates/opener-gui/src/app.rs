use std::path::PathBuf;

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot, PlotPoints, VLine};
use opener_format::{Action, Opener, Sample};

use crate::map::MapView;
use crate::model::{duration, format_time, kind_color, step_state, unit_kind, unit_name, StepState};

/// One economy plot: a title and the sample fields to draw.
struct PlotSpec {
    title: &'static str,
    stored: fn(&Sample) -> f64,
    income: fn(&Sample) -> f64,
    usage: fn(&Sample) -> f64,
    color: Color32,
}

pub struct App {
    path_text: String,
    loaded: Option<Loaded>,
    error: Option<String>,
}

struct Loaded {
    opener: Opener,
    t: f64,
    end: f64,
    playing: bool,
    speed: f64,
    selected: Option<u32>,
    scroll_to_selected: bool,
    map: MapView,
}

impl App {
    pub fn new(path: Option<PathBuf>) -> Self {
        let mut app = Self { path_text: String::new(), loaded: None, error: None };
        if let Some(path) = path {
            app.path_text = path.display().to_string();
            app.load();
        }
        app
    }

    fn load(&mut self) {
        match Opener::read(self.path_text.trim()) {
            Ok(opener) => {
                let end = duration(&opener);
                self.loaded = Some(Loaded {
                    opener,
                    t: 0.0,
                    end,
                    playing: false,
                    speed: 4.0,
                    selected: None,
                    scroll_to_selected: false,
                    map: MapView::new(),
                });
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut reload = false;
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("File:");
                let edit = ui.add(egui::TextEdit::singleline(&mut self.path_text).desired_width(420.0));
                if ui.button("Load").clicked()
                    || (edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    reload = true;
                }
                if let Some(e) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, e);
                }
            });
            if let Some(l) = &mut self.loaded {
                l.controls(ui, ctx);
            }
        });
        if reload {
            self.load();
        }
        match &mut self.loaded {
            Some(l) => l.panels(ctx),
            None => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("Enter the path of an opener.json file and press Load.");
                });
            }
        }
    }
}

impl Loaded {
    fn controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.playing {
            let dt = ctx.input(|i| i.stable_dt) as f64;
            self.t = (self.t + dt * self.speed).min(self.end);
            if self.t >= self.end {
                self.playing = false;
            }
            ctx.request_repaint();
        }
        ui.horizontal(|ui| {
            if ui.button(if self.playing { "⏸" } else { "▶" }).clicked() {
                if self.t >= self.end {
                    self.t = 0.0;
                }
                self.playing = !self.playing;
            }
            if ui.button("⏮").clicked() {
                self.t = 0.0;
            }
            if ui.button("◀ step").clicked() {
                self.jump(false);
            }
            if ui.button("step ▶").clicked() {
                self.jump(true);
            }
            if ui.button("Fit map").clicked() {
                self.map.reset();
            }
            ui.add(egui::DragValue::new(&mut self.speed).range(0.25..=60.0).suffix("×"));
            ui.spacing_mut().slider_width = (ui.available_width() - 120.0).max(100.0);
            ui.add(egui::Slider::new(&mut self.t, 0.0..=self.end).show_value(false));
            ui.monospace(format_time(self.t));
        });
        ui.horizontal_wrapped(|ui| {
            let o = &self.opener;
            ui.label(format!("{} · {}", o.map.name, o.faction));
            ui.label(format!(
                "Objective {}: {:.2} at {}",
                o.objective.kind,
                o.objective.score,
                format_time(o.objective.horizon_s)
            ));
            ui.weak(&o.objective.description);
        });
    }

    /// Jump to the next or previous event (start or finish of a step).
    fn jump(&mut self, forward: bool) {
        let times = self.opener.steps.iter().flat_map(|s| [s.start_s, s.finish_s]);
        let t = self.t;
        let target = if forward {
            times.filter(|&x| x > t + 1e-6).fold(f64::INFINITY, f64::min)
        } else {
            times.filter(|&x| x < t - 1e-6).fold(f64::NEG_INFINITY, f64::max)
        };
        self.t = if target.is_finite() { target } else if forward { self.end } else { 0.0 };
        self.playing = false;
    }

    fn panels(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("queue").default_width(420.0).show(ctx, |ui| self.queue(ui));
        egui::TopBottomPanel::bottom("curves").default_height(260.0).resizable(true).show(ctx, |ui| {
            self.curves(ui);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(index) = self.map.show(ui, &self.opener, self.t, self.selected) {
                self.select(index);
            }
        });
    }

    fn select(&mut self, index: u32) {
        self.selected = Some(index);
        self.scroll_to_selected = true;
        if let Some(s) = self.opener.steps.get(index as usize) {
            self.t = s.start_s;
        }
    }

    fn queue(&mut self, ui: &mut egui::Ui) {
        ui.heading("Build queue");
        let mut clicked = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("steps").striped(true).num_columns(6).show(ui, |ui| {
                for h in ["#", "Builder", "", "Unit", "Start", "End"] {
                    ui.strong(h);
                }
                ui.end_row();
                for step in &self.opener.steps {
                    let state = step_state(step, self.t);
                    let color = match state {
                        StepState::Pending => Color32::GRAY,
                        StepState::Walking => Color32::LIGHT_BLUE,
                        StepState::Building => Color32::YELLOW,
                        StepState::Done => ui.visuals().text_color(),
                        StepState::Unfinished => Color32::LIGHT_RED,
                    };
                    let kind = unit_kind(&self.opener, &step.unit);
                    let name = unit_name(&self.opener, &step.unit);
                    let label = match step.action {
                        Action::Build => name.to_owned(),
                        Action::Assist => format!("assist {name}"),
                    };
                    let selected = self.selected == Some(step.index);
                    let row = ui.selectable_label(selected, RichText::new(step.index.to_string()).color(color));
                    ui.label(RichText::new(format!("#{}", step.builder_id)).color(color));
                    ui.label(RichText::new("■ ").color(kind_color(kind)));
                    ui.label(RichText::new(label).color(color));
                    ui.monospace(format_time(step.start_s));
                    ui.monospace(format_time(step.finish_s));
                    ui.end_row();
                    if selected && self.scroll_to_selected {
                        row.scroll_to_me(Some(egui::Align::Center));
                    }
                    if row.clicked() {
                        clicked = Some(step.index);
                    }
                }
            });
        });
        self.scroll_to_selected = false;
        if let Some(i) = clicked {
            self.selected = Some(i);
            if let Some(s) = self.opener.steps.get(i as usize) {
                self.t = s.start_s;
            }
        }
    }

    fn curves(&mut self, ui: &mut egui::Ui) {
        let o = &self.opener;
        let t = self.t;
        let now = o.sample_at(t);
        ui.horizontal(|ui| {
            if let Some(s) = now {
                ui.label(format!(
                    "Metal {:.0}/{:.0} (+{:.1}/−{:.1})  Energy {:.0}/{:.0} (+{:.1}/−{:.1})  Build power {:.0}/{:.0}",
                    s.metal,
                    s.metal_storage,
                    s.metal_income,
                    s.metal_usage,
                    s.energy,
                    s.energy_storage,
                    s.energy_income,
                    s.energy_usage,
                    s.build_power_used,
                    s.build_power,
                ));
            }
        });
        let plots = [
            PlotSpec {
                title: "Metal",
                stored: |s| s.metal,
                income: |s| s.metal_income,
                usage: |s| s.metal_usage,
                color: Color32::from_rgb(120, 160, 230),
            },
            PlotSpec {
                title: "Energy",
                stored: |s| s.energy,
                income: |s| s.energy_income,
                usage: |s| s.energy_usage,
                color: Color32::from_rgb(235, 205, 70),
            },
        ];
        let mut seek = None;
        ui.columns(2, |cols| {
            for (col, PlotSpec { title, stored, income, usage, color }) in cols.iter_mut().zip(plots) {
                col.label(RichText::new(title).strong());
                let line = |f: fn(&Sample) -> f64| -> PlotPoints<'_> {
                    o.samples.iter().map(|s| [s.t_s, f(s)]).collect()
                };
                let r = Plot::new(title)
                    .legend(egui_plot::Legend::default())
                    .allow_drag(false)
                    .allow_scroll(false)
                    .allow_zoom(false)
                    .allow_boxed_zoom(false)
                    .include_x(0.0)
                    .include_x(self.end)
                    .x_axis_formatter(|m, _| format_time(m.value))
                    .show(col, |p| {
                        p.line(Line::new(line(stored)).name("stored").color(color));
                        p.line(Line::new(line(income)).name("income/s").color(Color32::LIGHT_GREEN));
                        p.line(Line::new(line(usage)).name("usage/s").color(Color32::LIGHT_RED));
                        p.vline(VLine::new(t).name("now").color(Color32::WHITE));
                    });
                if r.response.clicked() || r.response.dragged() {
                    if let Some(pos) = r.response.interact_pointer_pos() {
                        seek = Some(r.transform.value_from_position(pos).x);
                    }
                }
            }
        });
        if let Some(x) = seek {
            self.t = x.clamp(0.0, self.end);
            self.playing = false;
        }
    }
}
