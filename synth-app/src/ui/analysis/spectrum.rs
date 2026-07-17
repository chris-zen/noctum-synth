use eframe::egui;

pub const FFT_LABEL_W: f32 = 38.0;
pub const FFT_BOTTOM_H: f32 = 18.0;
pub const FFT_TOP_H: f32 = 8.0;

pub struct SpectrumConfig {
    pub fft_size: usize,
    pub sample_rate: f32,
    pub db_floor: f32,
    pub db_top: f32,
    pub log_scale: bool,
    pub min_freq: f32,
}

#[derive(Clone, Copy)]
pub struct SpectrumTrace<'a> {
    pub db_values: &'a [f32],
    pub color: egui::Color32,
}

/// Map a frequency (Hz) to an x position within the plot rectangle.
pub fn freq_to_x(
    hz: f32,
    log_scale: bool,
    min_freq: f32,
    max_freq: f32,
    x_left: f32,
    x_right: f32,
) -> f32 {
    let width = x_right - x_left;
    if log_scale {
        let fraction = ((hz.max(min_freq).ln() - min_freq.ln()) / (max_freq.ln() - min_freq.ln()))
            .clamp(0.0, 1.0);
        x_left + width * fraction
    } else {
        let fraction = (hz / max_freq).clamp(0.0, 1.0);
        x_left + width * fraction
    }
}

/// Draw a complete spectrum analyser: background, dB grid, frequency grid, FFT bars.
///
/// `db_values` must have at least `fft_size / 2` entries (one per FFT bin).
/// `controls_h` reserves vertical space at the bottom for the caller to draw
/// controls after this function returns.
///
/// Returns the plot rectangle so the caller can overlay additional elements.
pub fn render_spectrum(
    ui: &mut egui::Ui,
    db_values: &[f32],
    config: &SpectrumConfig,
    controls_h: f32,
) -> egui::Rect {
    render_spectra(
        ui,
        &[SpectrumTrace {
            db_values,
            color: egui::Color32::from_rgb(100, 200, 255),
        }],
        config,
        controls_h,
    )
}

/// Draw one or more alpha-blended FFT bar series on a shared grid.
pub fn render_spectra(
    ui: &mut egui::Ui,
    traces: &[SpectrumTrace<'_>],
    config: &SpectrumConfig,
    controls_h: f32,
) -> egui::Rect {
    let available = ui.available_size();
    let total_h = (available.y - controls_h).max(80.0);

    let (rect, _resp) =
        ui.allocate_exact_size(egui::vec2(available.x, total_h), egui::Sense::hover());
    let plot_left = rect.left() + FFT_LABEL_W;
    let plot_rect = egui::Rect::from_min_max(
        egui::pos2(plot_left, rect.top() + FFT_TOP_H),
        egui::pos2(rect.right(), rect.bottom() - FFT_BOTTOM_H),
    );
    let painter = ui.painter_at(rect);

    painter.rect_filled(plot_rect, 0.0, egui::Color32::from_rgb(20, 20, 24));
    let grid_color = egui::Color32::from_rgb(50, 50, 58);

    let fft_size = config.fft_size;
    let sr = config.sample_rate;
    let floor = config.db_floor;
    let top = config.db_top;
    let db_range = top - floor;
    let max_freq = sr / 2.0;
    let bin_hz = sr / fft_size as f32;

    // ── dB grid ──────────────────────────────────────────────────────────
    let db_step = if db_range <= 72.0 {
        6.0
    } else if db_range <= 120.0 {
        12.0
    } else {
        24.0
    };
    let mut db = (top / db_step).floor() * db_step;
    while db >= floor {
        let line_y = plot_rect.bottom() - plot_rect.height() * ((db - floor) / db_range);
        if line_y >= plot_rect.top() {
            painter.line_segment(
                [
                    egui::pos2(plot_rect.left(), line_y),
                    egui::pos2(plot_rect.right(), line_y),
                ],
                egui::Stroke::new(1.0_f32, grid_color),
            );
            painter.text(
                egui::pos2(plot_rect.left() - 4.0, line_y),
                egui::Align2::RIGHT_CENTER,
                format!("{:.0}", db),
                egui::FontId::monospace(9.0),
                egui::Color32::from_rgb(120, 120, 130),
            );
        }
        db -= db_step;
    }

    // ── Frequency ticks ──────────────────────────────────────────────────
    let f_ticks: Vec<f32> = if config.log_scale {
        log_freq_ticks(config.min_freq, max_freq)
    } else {
        lin_freq_ticks(config.min_freq, max_freq)
    };

    for (idx, &hz) in f_ticks.iter().enumerate() {
        let tick_x = freq_to_x(
            hz,
            config.log_scale,
            config.min_freq,
            max_freq,
            plot_rect.left(),
            plot_rect.right(),
        );
        if tick_x > plot_rect.right() {
            continue;
        }
        painter.line_segment(
            [
                egui::pos2(tick_x, plot_rect.top()),
                egui::pos2(tick_x, plot_rect.bottom()),
            ],
            egui::Stroke::new(1.0_f32, grid_color),
        );
        let label = if config.log_scale {
            // label every tick in log mode (sparse ticks)
            true
        } else {
            idx % 4 == 0 || hz == config.min_freq || hz >= 10000.0
        };
        if label {
            let text = if hz >= 1000.0 {
                format!("{:.1}k", hz / 1000.0)
            } else {
                format!("{:.0}Hz", hz)
            };
            painter.text(
                egui::pos2(tick_x, plot_rect.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                text,
                egui::FontId::monospace(9.0),
                egui::Color32::from_rgb(120, 120, 130),
            );
        }
    }

    // ── FFT bars ─────────────────────────────────────────────────────────
    let num_bins = fft_size / 2;
    if traces.iter().any(|trace| trace.db_values.len() < num_bins) {
        return plot_rect;
    }

    if config.log_scale {
        for trace in traces {
            for bin in 0..num_bins {
                let db = trace.db_values[bin].clamp(floor, top);
                if db <= floor {
                    continue;
                }
                let freq = bin as f32 * bin_hz;
                if freq < config.min_freq {
                    continue;
                }
                let x_from = freq_to_x(
                    freq,
                    true,
                    config.min_freq,
                    max_freq,
                    plot_rect.left(),
                    plot_rect.right(),
                );
                let next_freq = (bin + 1) as f32 * bin_hz;
                let x_to = freq_to_x(
                    next_freq,
                    true,
                    config.min_freq,
                    max_freq,
                    plot_rect.left(),
                    plot_rect.right(),
                );
                draw_bar(
                    &painter,
                    egui::pos2(x_from, plot_rect.bottom()),
                    (x_to - x_from).max(1.0),
                    db,
                    floor,
                    db_range,
                    plot_rect.height(),
                    trace.color,
                );
            }
        }
    } else {
        let bar_width = plot_rect.width() / num_bins as f32;
        for trace in traces {
            for bin in 0..num_bins {
                let db = trace.db_values[bin].clamp(floor, top);
                if db <= floor {
                    continue;
                }
                let bar_x = plot_rect.left() + bin as f32 * bar_width;
                draw_bar(
                    &painter,
                    egui::pos2(bar_x, plot_rect.bottom()),
                    bar_width.max(1.0),
                    db,
                    floor,
                    db_range,
                    plot_rect.height(),
                    trace.color,
                );
            }
        }
    }

    plot_rect
}

#[allow(clippy::too_many_arguments)]
fn draw_bar(
    painter: &egui::Painter,
    bottom_left: egui::Pos2,
    width: f32,
    db: f32,
    floor: f32,
    db_range: f32,
    plot_height: f32,
    color: egui::Color32,
) {
    let intensity = ((db - floor) / db_range).clamp(0.0, 1.0);
    let bar_height = plot_height * intensity;
    let alpha = (intensity * 180.0) as u8;
    painter.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(bottom_left.x, bottom_left.y - bar_height),
            egui::vec2(width, bar_height),
        ),
        0.0,
        egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha),
    );
}

// ── Tick generators ──────────────────────────────────────────────────────

/// Logarithmically-spaced frequency ticks: 20, 50, 100, 200, 500, 1k, 2k…
fn log_freq_ticks(min_freq: f32, max_freq: f32) -> Vec<f32> {
    let mut ticks = Vec::new();
    let mut decade = 10.0f32;
    while decade * 0.5 < min_freq {
        decade *= 10.0;
    }
    while decade * 0.1 <= max_freq {
        for &multiplier in &[1.0, 2.0, 5.0] {
            let hz = decade * multiplier / 10.0;
            if hz >= min_freq && hz <= max_freq {
                ticks.push(hz);
            }
        }
        decade *= 10.0;
    }
    ticks
}

/// Linear-spaced (pseudo-log increment) frequency ticks for linear axis.
fn lin_freq_ticks(min_freq: f32, max_freq: f32) -> Vec<f32> {
    let mut ticks = Vec::new();
    let mut hz = min_freq;
    while hz <= max_freq {
        ticks.push(hz);
        if hz < 100.0 {
            hz += 10.0;
        } else if hz < 200.0 {
            hz += 20.0;
        } else if hz < 500.0 {
            hz += 50.0;
        } else if hz < 1000.0 {
            hz += 100.0;
        } else if hz < 2000.0 {
            hz += 200.0;
        } else if hz < 5000.0 {
            hz += 500.0;
        } else if hz < 10000.0 {
            hz += 1000.0;
        } else {
            hz += 2000.0;
        }
    }
    ticks
}
