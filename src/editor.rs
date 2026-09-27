//! Plugin GUI. Everything is laid out on a fixed design canvas and scaled to the window, so
//! dragging the lower-right corner enlarges the whole interface.

use atomic_float::AtomicF32;
use nih_plug::prelude::{Editor, Param, ParamSetter};
use nih_plug_egui::egui::{vec2, Align2, Color32, Id, Pos2, Rect, Ui};
use nih_plug_egui::resizable_window::ResizableWindow;
use nih_plug_egui::{create_egui_editor, EguiState};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::widgets::{self, Follower, ACCENT, PANEL, TEXT, TEXT_DIM, TRACK};
use crate::CeilingParams;

const DESIGN_WIDTH: f32 = 300.0;
const DESIGN_HEIGHT: f32 = 420.0;
const MIN_SCALE: f32 = 0.75;
const BACKGROUND: Color32 = Color32::from_rgb(0x1a, 0x1d, 0x22);

const METER_RANGE_DB: f32 = 24.0;
const METER_TICKS_DB: [f32; 5] = [0.0, 6.0, 12.0, 18.0, 24.0];
const METER_DECAY_DB_PER_SECOND: f32 = 18.0;
const MAX_FRAME_SECONDS: f32 = 0.1;
const FADER_TICKS_DB: [f32; 6] = [0.0, -6.0, -12.0, -18.0, -24.0, -30.0];

pub fn default_state() -> Arc<EguiState> {
    EguiState::from_size(DESIGN_WIDTH as u32, DESIGN_HEIGHT as u32)
}

#[derive(Default)]
struct GuiState {
    meter_db: f32,
}

pub fn create(
    params: Arc<CeilingParams>,
    gain_reduction: Arc<AtomicF32>,
) -> Option<Box<dyn Editor>> {
    let egui_state = params.editor_state.clone();
    create_egui_editor(
        params.editor_state.clone(),
        GuiState::default(),
        |_, _| {},
        move |ctx, setter, state| {
            let new_peak = gain_reduction.swap(0.0, Ordering::Relaxed);
            let dt = ctx.input(|i| i.stable_dt).min(MAX_FRAME_SECONDS);
            let decayed = state.meter_db - METER_DECAY_DB_PER_SECOND * dt;
            state.meter_db = new_peak.max(decayed).max(0.0);

            ResizableWindow::new("ceiling-window")
                .min_size(vec2(DESIGN_WIDTH, DESIGN_HEIGHT) * MIN_SCALE)
                .show(ctx, egui_state.as_ref(), |ui| {
                    draw(ui, &params, setter, state.meter_db);
                });
        },
    )
}

/// Maps design-canvas coordinates to screen coordinates.
struct Canvas {
    origin: Pos2,
    scale: f32,
}

impl Canvas {
    fn fit(area: Rect) -> Self {
        let scale = (area.width() / DESIGN_WIDTH).min(area.height() / DESIGN_HEIGHT);
        let size = vec2(DESIGN_WIDTH, DESIGN_HEIGHT) * scale;
        Self {
            origin: area.center() - size / 2.0,
            scale,
        }
    }

    fn pos(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.scale
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.pos(x, y), vec2(w, h) * self.scale)
    }

    fn size(&self, s: f32) -> f32 {
        s * self.scale
    }
}

fn draw(ui: &mut Ui, params: &CeilingParams, setter: &ParamSetter, meter_db: f32) {
    let area = ui.max_rect();
    ui.painter().rect_filled(area, 0.0, BACKGROUND);
    let c = Canvas::fit(area);

    let title_pos = c.pos(150.0, 22.0);
    widgets::draw_text(ui, title_pos, Align2::CENTER_CENTER, "CEILING LIMITER", c.size(15.0), TEXT);
    draw_meter(ui, &c, meter_db);
    draw_faders(ui, &c, params, setter);

    let link_rect = c.rect(20.0, 378.0, 125.0, 26.0);
    let isp_rect = c.rect(155.0, 378.0, 125.0, 26.0);
    widgets::toggle(ui, Id::new("link"), link_rect, c.scale, "FADER LINK", &params.fader_link, setter);
    widgets::toggle(ui, Id::new("isp"), isp_rect, c.scale, "ISP DETECTION", &params.isp_detection, setter);
}

fn draw_meter(ui: &Ui, c: &Canvas, meter_db: f32) {
    let small = c.size(10.5);
    widgets::draw_text(ui, c.pos(20.0, 50.0), Align2::LEFT_CENTER, "GAIN REDUCTION", small, TEXT_DIM);
    let reading = format!("-{meter_db:.1} dB");
    widgets::draw_text(ui, c.pos(280.0, 50.0), Align2::RIGHT_CENTER, &reading, c.size(11.0), TEXT);

    let bar = c.rect(20.0, 60.0, 260.0, 16.0);
    let painter = ui.painter();
    painter.rect_filled(bar, c.size(3.0), TRACK);
    let fraction = (meter_db / METER_RANGE_DB).clamp(0.0, 1.0);
    if fraction > 0.0 {
        let fill = Rect::from_min_size(bar.min, vec2(bar.width() * fraction, bar.height()));
        painter.rect_filled(fill, c.size(3.0), ACCENT);
    }

    for db in METER_TICKS_DB {
        let x = 20.0 + 260.0 * db / METER_RANGE_DB;
        let align = if db <= 0.0 {
            Align2::LEFT_CENTER
        } else if db >= METER_RANGE_DB {
            Align2::RIGHT_CENTER
        } else {
            Align2::CENTER_CENTER
        };
        let label = format!("{db:.0}");
        widgets::draw_text(ui, c.pos(x, 88.0), align, &label, c.size(9.5), TEXT_DIM);
    }
}

fn draw_faders(ui: &mut Ui, c: &Canvas, params: &CeilingParams, setter: &ParamSetter) {
    ui.painter()
        .rect_filled(c.rect(20.0, 104.0, 260.0, 262.0), c.size(6.0), PANEL);

    let threshold_track = c.rect(70.0, 150.0, 60.0, 180.0);
    let output_track = c.rect(170.0, 150.0, 60.0, 180.0);

    let scale_x = c.pos(150.0, 0.0).x;
    for db in FADER_TICKS_DB {
        let pos = Pos2::new(scale_x, widgets::db_to_y(db, threshold_track));
        let label = format!("{db:.0}");
        widgets::draw_text(ui, pos, Align2::CENTER_CENTER, &label, c.size(9.5), TEXT_DIM);
    }

    let follower = Some(Follower {
        param: &params.output,
        is_enabled: params.fader_link.value(),
    });
    widgets::fader(ui, Id::new("threshold"), threshold_track, c.scale, &params.threshold, follower, setter);
    widgets::fader(ui, Id::new("output"), output_track, c.scale, &params.output, None, setter);

    let threshold_db = params.threshold.modulated_plain_value();
    let output_db = params.output.modulated_plain_value();
    widgets::readout(ui, c.rect(52.0, 116.0, 96.0, 22.0), c.scale, threshold_db);
    widgets::readout(ui, c.rect(152.0, 116.0, 96.0, 22.0), c.scale, output_db);

    let label_size = c.size(11.0);
    widgets::draw_text(ui, c.pos(100.0, 350.0), Align2::CENTER_CENTER, "THRESHOLD", label_size, TEXT);
    widgets::draw_text(ui, c.pos(200.0, 350.0), Align2::CENTER_CENTER, "OUTPUT", label_size, TEXT);
}
