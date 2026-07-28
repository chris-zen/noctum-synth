use eframe::egui;

use crate::engine::AudioBlock;

const RMS_TAU_S: f32 = 0.300;
const PEAK_RELEASE_TAU_S: f32 = 0.080;
pub(crate) const DB_FLOOR: f32 = -60.0;
pub(crate) const DB_CEIL: f32 = 0.0;
const ZONE_YELLOW_DB: f32 = -18.0;
const ZONE_RED_DB: f32 = -6.0;
pub(crate) const VU_WIDTH: f32 = 132.0;
const DB_SCALE_W: f32 = 28.0;
const LABEL_GAP: f32 = 10.0;

const METER_GREEN: egui::Color32 = egui::Color32::from_rgb(40, 180, 70);
const METER_YELLOW: egui::Color32 = egui::Color32::from_rgb(220, 190, 40);
const METER_RED: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);

const INPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 150, 45);
const INPUT_RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 205, 80);
const OUTPUT_LEFT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 205, 255);
const OUTPUT_RIGHT_COLOR: egui::Color32 = egui::Color32::from_rgb(80, 125, 255);

pub struct VuMeterState {
    pub input_l: ChannelMeter,
    pub input_r: ChannelMeter,
    pub output_l: ChannelMeter,
    pub output_r: ChannelMeter,
}

impl Default for VuMeterState {
    fn default() -> Self {
        Self {
            input_l: ChannelMeter::default(),
            input_r: ChannelMeter::default(),
            output_l: ChannelMeter::default(),
            output_r: ChannelMeter::default(),
        }
    }
}

impl VuMeterState {
    pub fn feed(&mut self, block: &AudioBlock, sample_rate: f32) {
        let len = block.len as usize;
        if len == 0 || sample_rate <= 0.0 {
            return;
        }
        let dt = len as f32 / sample_rate;
        self.input_l.feed_samples(&block.input_left[..len], dt);
        self.input_r.feed_samples(&block.input_right[..len], dt);
        self.output_l.feed_samples(&block.output_left[..len], dt);
        self.output_r.feed_samples(&block.output_right[..len], dt);
    }

    pub fn reset_peaks(&mut self) {
        self.input_l.reset_peak_hold();
        self.input_r.reset_peak_hold();
        self.output_l.reset_peak_hold();
        self.output_r.reset_peak_hold();
    }
}

#[derive(Clone, Copy)]
pub struct ChannelMeter {
    pub rms_lin: f32,
    pub peak_lin: f32,
    pub max_peak_lin: f32,
    rms_energy: f32,
}

impl Default for ChannelMeter {
    fn default() -> Self {
        Self {
            rms_lin: 0.0,
            peak_lin: 0.0,
            max_peak_lin: 0.0,
            rms_energy: 0.0,
        }
    }
}

impl ChannelMeter {
    pub fn feed_samples(&mut self, samples: &[f32], dt: f32) {
        if samples.is_empty() || dt <= 0.0 {
            return;
        }

        let mut block_peak = 0.0_f32;
        let mut sum_sq = 0.0_f32;
        for &s in samples {
            let a = s.abs();
            if a > block_peak {
                block_peak = a;
            }
            sum_sq += s * s;
        }
        let block_energy = sum_sq / samples.len() as f32;

        let rms_alpha = 1.0 - (-dt / RMS_TAU_S).exp();
        self.rms_energy += (block_energy - self.rms_energy) * rms_alpha;
        self.rms_lin = self.rms_energy.sqrt();

        if block_peak >= self.peak_lin {
            self.peak_lin = block_peak;
        } else {
            let peak_alpha = 1.0 - (-dt / PEAK_RELEASE_TAU_S).exp();
            self.peak_lin += (block_peak - self.peak_lin) * peak_alpha;
        }

        if self.peak_lin > self.max_peak_lin {
            self.max_peak_lin = self.peak_lin;
        }
    }

    pub fn reset_peak_hold(&mut self) {
        self.max_peak_lin = self.peak_lin;
    }
}

pub(crate) fn linear_to_db(linear: f32) -> f32 {
    if linear <= 1e-10 {
        DB_FLOOR
    } else {
        (20.0 * linear.log10()).clamp(DB_FLOOR, DB_CEIL)
    }
}

pub(crate) fn db_to_norm(db: f32) -> f32 {
    ((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0)
}

pub(crate) fn format_db(linear: f32) -> String {
    let db = linear_to_db(linear);
    if db <= DB_FLOOR + 0.05 {
        "-∞".to_string()
    } else {
        format!("{db:.0}")
    }
}

fn meter_color_for_db(db: f32) -> egui::Color32 {
    if db >= ZONE_RED_DB {
        METER_RED
    } else if db >= ZONE_YELLOW_DB {
        METER_YELLOW
    } else {
        METER_GREEN
    }
}

fn fill_meter_bar(
    painter: &egui::Painter,
    bar_rect: egui::Rect,
    level_db: f32,
    plot_h: f32,
    bottom: f32,
) {
    let level_t = db_to_norm(level_db);
    if level_t <= 0.0 {
        return;
    }

    let zones = [
        (DB_FLOOR, ZONE_YELLOW_DB, METER_GREEN),
        (ZONE_YELLOW_DB, ZONE_RED_DB, METER_YELLOW),
        (ZONE_RED_DB, DB_CEIL, METER_RED),
    ];
    for &(lo, hi, color) in &zones {
        let seg_lo = lo.max(DB_FLOOR);
        let seg_hi = hi.min(level_db);
        if seg_hi <= seg_lo {
            continue;
        }
        let y_lo = bottom - db_to_norm(seg_lo) * plot_h;
        let y_hi = bottom - db_to_norm(seg_hi) * plot_h;
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(bar_rect.left(), y_hi),
                egui::pos2(bar_rect.right(), y_lo),
            ),
            0.0,
            color,
        );
    }
}

pub(crate) fn draw_vu_meter(ui: &mut egui::Ui, state: &mut VuMeterState) {
    let available = ui.available_size();
    let (rect, response) = ui.allocate_exact_size(available, egui::Sense::click());
    let response = response.on_hover_text("Click to reset peak holds");
    if response.clicked() {
        state.reset_peaks();
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 20, 24));

    let title_h = 18.0;
    let label_h = 14.0;
    let readout_h = 14.0;
    let top = rect.top() + title_h;
    let bottom = rect.bottom() - LABEL_GAP - label_h - readout_h;
    if bottom <= top + 8.0 {
        return;
    }

    let pad_x = 4.0;
    let group_gap = 8.0;
    let bar_gap = 3.0;
    let bars_left = rect.left() + DB_SCALE_W;
    let inner_w = (rect.right() - pad_x - bars_left - group_gap).max(0.0);
    let bar_w = ((inner_w - bar_gap * 2.0) / 4.0).max(4.0);
    let bars_right = bars_left + bar_w * 4.0 + bar_gap * 2.0 + group_gap;
    let bars_center_x = 0.5 * (bars_left + bars_right);

    painter.text(
        egui::pos2(bars_center_x, rect.top() + 2.0),
        egui::Align2::CENTER_TOP,
        "VU",
        egui::FontId::proportional(11.0),
        egui::Color32::from_rgb(160, 160, 170),
    );

    let channels = [
        (&state.input_l, "IL", INPUT_LEFT_COLOR),
        (&state.input_r, "IR", INPUT_RIGHT_COLOR),
        (&state.output_l, "OL", OUTPUT_LEFT_COLOR),
        (&state.output_r, "OR", OUTPUT_RIGHT_COLOR),
    ];

    let plot_h = bottom - top;
    let grid_color = egui::Color32::from_rgb(50, 50, 58);
    let label_color = egui::Color32::from_rgb(120, 120, 130);
    let hold_color = egui::Color32::from_rgb(230, 230, 240);
    let db_marks = [-48, -36, -24, -18, -12, -6, 0];

    for &db in &db_marks {
        let t = db_to_norm(db as f32);
        let y = bottom - t * plot_h;
        painter.line_segment(
            [
                egui::pos2(bars_left - 2.0, y),
                egui::pos2(rect.right() - pad_x, y),
            ],
            egui::Stroke::new(1.0_f32, grid_color),
        );
        painter.text(
            egui::pos2(bars_left - 4.0, y),
            egui::Align2::RIGHT_CENTER,
            format!("{db}"),
            egui::FontId::monospace(8.0),
            label_color,
        );
    }

    let mut x = bars_left;
    for (i, (meter, label, accent)) in channels.iter().enumerate() {
        if i == 2 {
            x += group_gap;
        }

        let bar_rect = egui::Rect::from_min_max(egui::pos2(x, top), egui::pos2(x + bar_w, bottom));
        painter.rect_filled(bar_rect, 0.0, egui::Color32::from_rgb(28, 28, 34));

        let rms_db = linear_to_db(meter.rms_lin);
        fill_meter_bar(&painter, bar_rect, rms_db, plot_h, bottom);

        let peak_db = linear_to_db(meter.peak_lin);
        let peak_t = db_to_norm(peak_db);
        if peak_t > 0.0 {
            let peak_y = bottom - peak_t * plot_h;
            painter.line_segment(
                [
                    egui::pos2(bar_rect.left(), peak_y),
                    egui::pos2(bar_rect.right(), peak_y),
                ],
                egui::Stroke::new(1.5_f32, meter_color_for_db(peak_db)),
            );
        }

        let max_db = linear_to_db(meter.max_peak_lin);
        let max_t = db_to_norm(max_db);
        if max_t > 0.0 {
            let max_y = bottom - max_t * plot_h;
            painter.line_segment(
                [
                    egui::pos2(bar_rect.left() - 1.0, max_y),
                    egui::pos2(bar_rect.right() + 1.0, max_y),
                ],
                egui::Stroke::new(1.5_f32, hold_color),
            );
        }

        let label_y = bottom + LABEL_GAP;
        painter.text(
            egui::pos2(x + bar_w * 0.5, label_y),
            egui::Align2::CENTER_TOP,
            *label,
            egui::FontId::monospace(9.0),
            *accent,
        );

        painter.text(
            egui::pos2(x + bar_w * 0.5, label_y + label_h),
            egui::Align2::CENTER_TOP,
            format_db(meter.max_peak_lin),
            egui::FontId::monospace(8.0),
            hold_color,
        );

        x += bar_w + bar_gap;
    }

    painter.text(
        egui::pos2(
            bars_left + bar_w + bar_gap * 0.5,
            rect.top() + title_h - 2.0,
        ),
        egui::Align2::CENTER_BOTTOM,
        "I",
        egui::FontId::monospace(9.0),
        INPUT_LEFT_COLOR,
    );
    painter.text(
        egui::pos2(
            bars_left + (bar_w + bar_gap) * 2.0 + group_gap + bar_w + bar_gap * 0.5,
            rect.top() + title_h - 2.0,
        ),
        egui::Align2::CENTER_BOTTOM,
        "O",
        egui::FontId::monospace(9.0),
        OUTPUT_LEFT_COLOR,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::MAX_AUDIO_BUF;

    fn block_with_output(level: f32, len: usize) -> AudioBlock {
        let mut block = AudioBlock::default();
        let len = len.min(MAX_AUDIO_BUF);
        for i in 0..len {
            block.output_left[i] = level;
            block.output_right[i] = level;
        }
        block.len = len as u16;
        block
    }

    #[test]
    fn silent_block_stays_near_zero() {
        let mut vu = VuMeterState::default();
        vu.feed(&block_with_output(0.0, 256), 44100.0);
        assert!(vu.output_l.rms_lin < 1e-6);
        assert!(vu.output_l.peak_lin < 1e-6);
        assert!(vu.output_l.max_peak_lin < 1e-6);
    }

    #[test]
    fn full_scale_sets_peak_and_sticky_max() {
        let mut vu = VuMeterState::default();
        vu.feed(&block_with_output(1.0, 512), 44100.0);
        assert!((vu.output_l.peak_lin - 1.0).abs() < 1e-5);
        assert!((vu.output_l.max_peak_lin - 1.0).abs() < 1e-5);

        vu.feed(&block_with_output(0.1, 512), 44100.0);
        assert!(vu.output_l.max_peak_lin > 0.99);
        assert!(vu.output_l.peak_lin < vu.output_l.max_peak_lin);
    }

    #[test]
    fn reset_peaks_clears_hold_to_current_peak() {
        let mut vu = VuMeterState::default();
        vu.feed(&block_with_output(1.0, 256), 44100.0);
        vu.feed(&block_with_output(0.0, 256), 44100.0);
        assert!(vu.output_l.max_peak_lin > 0.5);
        let peak_before = vu.output_l.peak_lin;
        vu.reset_peaks();
        assert!((vu.output_l.max_peak_lin - peak_before).abs() < 1e-5);
    }

    #[test]
    fn rms_rises_with_sustained_signal() {
        let mut vu = VuMeterState::default();
        for _ in 0..40 {
            vu.feed(&block_with_output(0.5, 512), 44100.0);
        }
        assert!(vu.output_l.rms_lin > 0.3);
        assert!(vu.output_l.rms_lin < 0.55);
    }

    #[test]
    fn linear_to_db_floor_and_ceil() {
        assert!((linear_to_db(0.0) - DB_FLOOR).abs() < 1e-5);
        assert!((linear_to_db(1.0) - 0.0).abs() < 1e-5);
        assert!((db_to_norm(DB_FLOOR) - 0.0).abs() < 1e-5);
        assert!((db_to_norm(0.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn meter_zones_follow_ebu_style_thresholds() {
        assert_eq!(meter_color_for_db(-24.0), METER_GREEN);
        assert_eq!(meter_color_for_db(-18.0), METER_YELLOW);
        assert_eq!(meter_color_for_db(-12.0), METER_YELLOW);
        assert_eq!(meter_color_for_db(-6.0), METER_RED);
        assert_eq!(meter_color_for_db(0.0), METER_RED);
    }
}
