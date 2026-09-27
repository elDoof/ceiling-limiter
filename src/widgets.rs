//! Custom painted controls: vertical dB fader and toggle button.

use nih_plug::prelude::{BoolParam, FloatParam, Param, ParamSetter};
use nih_plug_egui::egui::{
    pos2, vec2, Align2, Color32, FontId, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui,
};

use crate::{FADER_MAX_DB, FADER_MIN_DB};

pub const TEXT: Color32 = Color32::from_rgb(0xd8, 0xdc, 0xe2);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8a, 0x91, 0x9c);
pub const PANEL: Color32 = Color32::from_rgb(0x23, 0x27, 0x2e);
pub const TRACK: Color32 = Color32::from_rgb(0x12, 0x14, 0x18);
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x8a, 0x3d);
const HANDLE: Color32 = Color32::from_rgb(0xc9, 0xce, 0xd6);
const HANDLE_ACTIVE: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);

/// Drag speed multiplier while Shift is held.
const FINE_DRAG_FACTOR: f32 = 0.1;

/// A parameter that can follow a fader (Fader Link). `is_enabled` is sampled once when a drag
/// starts, so begin/end gestures on the follower always pair up.
pub struct Follower<'a> {
    pub param: &'a FloatParam,
    pub is_enabled: bool,
}

#[derive(Clone, Copy)]
struct DragState {
    value: f32,
    start: f32,
    /// Follower start value, if the follower was linked when the drag began.
    follower_start: Option<f32>,
}

pub fn db_to_y(db: f32, track: Rect) -> f32 {
    let t = (FADER_MAX_DB - db) / (FADER_MAX_DB - FADER_MIN_DB);
    track.top() + t * track.height()
}

fn clamp_db(db: f32) -> f32 {
    db.clamp(FADER_MIN_DB, FADER_MAX_DB)
}

/// Vertical fader. Drag to change (Shift for fine), double-click to reset to the default.
pub fn fader(
    ui: &mut Ui,
    id: Id,
    track: Rect,
    scale: f32,
    param: &FloatParam,
    follower: Option<Follower>,
    setter: &ParamSetter,
) {
    let handle_size = vec2(34.0, 18.0) * scale;
    let hit_rect = track.expand2(handle_size / 2.0);
    let response = ui.interact(hit_rect, id, Sense::click_and_drag());

    let linked = follower.as_ref().filter(|f| f.is_enabled).map(|f| f.param);

    if response.double_clicked() {
        let default = param.default_plain_value();
        let offset = default - param.unmodulated_plain_value();
        setter.begin_set_parameter(param);
        setter.set_parameter(param, default);
        setter.end_set_parameter(param);
        if let Some(f) = linked {
            setter.begin_set_parameter(f);
            setter.set_parameter(f, clamp_db(f.unmodulated_plain_value() + offset));
            setter.end_set_parameter(f);
        }
    }

    if response.drag_started() {
        let current = param.unmodulated_plain_value();
        let state = DragState {
            value: current,
            start: current,
            follower_start: linked.map(|f| f.unmodulated_plain_value()),
        };
        ui.data_mut(|d| d.insert_temp(id, state));
        setter.begin_set_parameter(param);
        if let Some(f) = linked {
            setter.begin_set_parameter(f);
        }
    }

    if response.dragged() {
        if let Some(state) = ui.data(|d| d.get_temp::<DragState>(id)) {
            let is_fine = ui.input(|i| i.modifiers.shift);
            let speed = if is_fine { FINE_DRAG_FACTOR } else { 1.0 };
            let range = FADER_MAX_DB - FADER_MIN_DB;
            let delta_db = -response.drag_delta().y / track.height() * range * speed;
            let value = clamp_db(state.value + delta_db);
            setter.set_parameter(param, value);
            if let (Some(f), Some(start)) = (&follower, state.follower_start) {
                setter.set_parameter(f.param, clamp_db(start + (value - state.start)));
            }
            ui.data_mut(|d| d.insert_temp(id, DragState { value, ..state }));
        }
    }

    if response.drag_stopped() {
        setter.end_set_parameter(param);
        let state = ui.data(|d| d.get_temp::<DragState>(id));
        if let (Some(f), Some(DragState { follower_start: Some(_), .. })) = (&follower, state) {
            setter.end_set_parameter(f.param);
        }
        ui.data_mut(|d| d.remove::<DragState>(id));
    }

    let painter = ui.painter();
    let slot = Rect::from_center_size(track.center(), vec2(6.0 * scale, track.height()));
    painter.rect_filled(slot, 3.0 * scale, TRACK);

    let y = db_to_y(param.modulated_plain_value(), track);
    let fill = Rect::from_min_max(pos2(slot.left(), y), slot.max);
    painter.rect_filled(fill, 3.0 * scale, ACCENT.gamma_multiply(0.35));

    let handle = Rect::from_center_size(pos2(track.center().x, y), handle_size);
    let is_active = response.dragged() || response.hovered();
    painter.rect_filled(handle, 3.0 * scale, if is_active { HANDLE_ACTIVE } else { HANDLE });
    painter.line_segment(
        [
            pos2(handle.left() + 5.0 * scale, y),
            pos2(handle.right() - 5.0 * scale, y),
        ],
        Stroke::new(1.5 * scale, TRACK),
    );
}

/// dB readout box shown above a fader.
pub fn readout(ui: &Ui, rect: Rect, scale: f32, value_db: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0 * scale, TRACK);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        format!("{value_db:.2} dB"),
        FontId::monospace(12.0 * scale),
        TEXT,
    );
}

/// Toggle button bound to a boolean parameter.
pub fn toggle(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    scale: f32,
    label: &str,
    param: &BoolParam,
    setter: &ParamSetter,
) {
    let response = ui.interact(rect, id, Sense::click());
    let is_on = param.value();
    if response.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !is_on);
        setter.end_set_parameter(param);
    }

    let painter = ui.painter();
    let border = if response.hovered() { TEXT_DIM } else { TRACK };
    painter.rect_filled(rect, 4.0 * scale, PANEL);
    painter.rect_stroke(
        rect,
        4.0 * scale,
        Stroke::new(1.0 * scale, border),
        StrokeKind::Inside,
    );

    let led_center = pos2(rect.left() + 14.0 * scale, rect.center().y);
    painter.circle_filled(led_center, 4.5 * scale, if is_on { ACCENT } else { TRACK });
    painter.text(
        pos2(rect.left() + 26.0 * scale, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(11.5 * scale),
        if is_on { TEXT } else { TEXT_DIM },
    );
}

pub fn draw_text(ui: &Ui, pos: Pos2, align: Align2, text: &str, size: f32, color: Color32) {
    ui.painter()
        .text(pos, align, text, FontId::proportional(size), color);
}
