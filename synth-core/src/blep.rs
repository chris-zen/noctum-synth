use crate::f32x4;

use crate::LANES;
use crate::analog_oscillator::{MAX_PULSE_WIDTH, MIN_PULSE_WIDTH};
#[cfg(test)]
use crate::wrap01;

// Pre-computed 4096-point Blackman-Harris BLEP table.
include!("blep_table.rs");

const TABLE_SIZE: usize = 4096;
const TABLE_CENTER: usize = TABLE_SIZE / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SawMethod {
    /// 4-point, 4th-order B-spline polyBLEP. Best-quality polynomial method
    /// (no lookup table); aliasing-free across the piano range.
    PolyBlep,
    /// Table-based BLEP with adaptive 4/2/1 point windows.
    Blep,
}

#[derive(Clone, Copy)]
pub(crate) struct PulseBlepState {
    width: f32x4,
    edge: f32x4,
    table_window: f32x4,
}

impl PulseBlepState {
    pub(crate) fn new(width: f32x4) -> Self {
        let width = clamp_pulse_width(width);
        Self {
            width,
            edge: f32x4::splat(1.0) - width,
            table_window: f32x4::ZERO,
        }
    }

    pub(crate) fn set_width(&mut self, width: f32x4) {
        self.width = clamp_pulse_width(width);
        self.edge = f32x4::splat(1.0) - self.width;
    }

    pub(crate) fn set_phase_inc(&mut self, phase_inc: f32x4) {
        self.table_window = table_blep_windows(phase_inc);
    }
}

pub(crate) fn blep_pulse(phi: f32x4, dt: f32x4, width: f32x4, method: SawMethod) -> f32x4 {
    let mut state = PulseBlepState::new(width);
    if method == SawMethod::Blep {
        state.set_phase_inc(dt);
    }
    blep_pulse_prepared(phi, dt, &state, method)
}

pub(crate) fn blep_pulse_prepared(
    phi: f32x4,
    dt: f32x4,
    state: &PulseBlepState,
    method: SawMethod,
) -> f32x4 {
    match method {
        SawMethod::PolyBlep => polyblep_pulse(phi, dt, state),
        SawMethod::Blep => table_blep_pulse(phi, state),
    }
}

#[inline]
fn clamp_pulse_width(width: f32x4) -> f32x4 {
    width.clamp(f32x4::splat(MIN_PULSE_WIDTH), f32x4::splat(MAX_PULSE_WIDTH))
}

/// Generates a pulse directly from its two discontinuities.
///
/// This is algebraically equivalent to subtracting two band-limited saws, but
/// avoids calculating two ramps that cancel. Callers keep `phi` in `[0, 1)`,
/// so the width-shifted phase needs at most one subtraction to wrap.
#[cfg(feature = "embedded-math")]
fn polyblep_pulse(phi: f32x4, dt: f32x4, state: &PulseBlepState) -> f32x4 {
    let phase = phi.to_array();
    let phase_inc = dt.to_array();
    let pulse_width = state.width.to_array();
    let pulse_edge = state.edge.to_array();
    let mut output = [0.0; LANES];

    for lane in 0..LANES {
        let (naive, shifted_phase) =
            pulse_phases_lane(phase[lane], pulse_width[lane], pulse_edge[lane]);
        output[lane] = naive + polyblep_falling_correction_lane(phase[lane], phase_inc[lane])
            - polyblep_falling_correction_lane(shifted_phase, phase_inc[lane]);
    }

    f32x4::new(output)
}

#[cfg(not(feature = "embedded-math"))]
fn polyblep_pulse(phi: f32x4, dt: f32x4, state: &PulseBlepState) -> f32x4 {
    let before_edge = phi.simd_lt(state.edge);
    let shifted_phase = before_edge.blend(phi + state.width, phi - state.edge);
    let phase = phi.to_array();
    let shifted = shifted_phase.to_array();
    let phase_inc = dt.to_array();
    let mut correction = [0.0; LANES];

    for lane in 0..LANES {
        correction[lane] = polyblep_falling_correction_lane(phase[lane], phase_inc[lane])
            - polyblep_falling_correction_lane(shifted[lane], phase_inc[lane]);
    }

    let one = f32x4::splat(1.0);
    let naive = before_edge.blend(-one, one);
    naive + f32x4::new(correction)
}

#[cfg(feature = "embedded-math")]
fn table_blep_pulse(phi: f32x4, state: &PulseBlepState) -> f32x4 {
    let phase = phi.to_array();
    let pulse_width = state.width.to_array();
    let pulse_edge = state.edge.to_array();
    let table_window = state.table_window.to_array();
    let mut output = [0.0; LANES];

    for lane in 0..LANES {
        let (naive, shifted_phase) =
            pulse_phases_lane(phase[lane], pulse_width[lane], pulse_edge[lane]);
        let window = table_window[lane];
        output[lane] = naive + table_blep_falling_correction_lane(phase[lane], window)
            - table_blep_falling_correction_lane(shifted_phase, window);
    }

    f32x4::new(output)
}

#[cfg(not(feature = "embedded-math"))]
fn table_blep_pulse(phi: f32x4, state: &PulseBlepState) -> f32x4 {
    let before_edge = phi.simd_lt(state.edge);
    let shifted_phase = before_edge.blend(phi + state.width, phi - state.edge);
    let phase = phi.to_array();
    let shifted = shifted_phase.to_array();
    let table_window = state.table_window.to_array();
    let mut correction = [0.0; LANES];

    for lane in 0..LANES {
        let window = table_window[lane];
        correction[lane] = table_blep_falling_correction_lane(phase[lane], window)
            - table_blep_falling_correction_lane(shifted[lane], window);
    }

    let one = f32x4::splat(1.0);
    let naive = before_edge.blend(-one, one);
    naive + f32x4::new(correction)
}

#[cfg(feature = "embedded-math")]
#[inline]
fn pulse_phases_lane(phase: f32, width: f32, edge: f32) -> (f32, f32) {
    if phase >= edge {
        (1.0, phase - edge)
    } else {
        (-1.0, phase + width)
    }
}

pub(crate) fn blep_saw(phi: f32x4, dt: f32x4, method: SawMethod) -> f32x4 {
    let one = f32x4::splat(1.0);
    let two = f32x4::splat(2.0);
    let mut out = phi * two - one;

    if method == SawMethod::PolyBlep {
        return out + polyblep_falling_correction(phi, dt);
    }

    let points = table_points_per_side(dt);
    out = out + table_blep_falling_correction(phi, dt, points);

    out
}

/// Double-sided 4-point polyBLEP correction for the positive-going ramp's
/// downward step: added on the right of the discontinuity (phase 0) and
/// subtracted on the left (phase 1).
fn polyblep_falling_correction(phi: f32x4, dt: f32x4) -> f32x4 {
    let phase = phi.to_array();
    let phase_inc = dt.to_array();
    let mut correction = [0.0; LANES];

    for lane in 0..LANES {
        correction[lane] = polyblep_falling_correction_lane(phase[lane], phase_inc[lane]);
    }

    f32x4::new(correction)
}

#[inline]
fn polyblep_falling_correction_lane(phase: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }

    polyblep_residual_lane(phase, dt) - polyblep_residual_lane(1.0 - phase, dt)
}

/// Single-sided 4-point, 4th-order B-spline polyBLEP residual for `t ∈ [0, 2·dt)`.
#[inline]
fn polyblep_residual_lane(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 || t >= 2.0 * dt {
        return 0.0;
    }

    let x = t / dt;
    let right = 2.0 - x;
    let mut y = right * right * right * right;

    if t < dt {
        let left = 1.0 - x;
        y -= 4.0 * left * left * left * left;
    }

    y / 12.0
}

#[inline]
fn table_points_per_side(dt: f32x4) -> [u32; LANES] {
    let dt = dt.to_array();
    let mut points = [1; LANES];

    for (idx, phase_inc) in dt.iter().enumerate() {
        points[idx] = table_points_per_side_lane(*phase_inc);
    }

    points
}

fn table_blep_windows(dt: f32x4) -> f32x4 {
    let phase_inc = dt.to_array();
    let mut windows = [0.0; LANES];

    for lane in 0..LANES {
        windows[lane] = phase_inc[lane] * table_points_per_side_lane(phase_inc[lane]) as f32;
    }

    f32x4::new(windows)
}

#[inline]
fn table_points_per_side_lane(phase_inc: f32) -> u32 {
    if phase_inc <= 0.125 {
        4
    } else if phase_inc <= 0.25 {
        2
    } else {
        1
    }
}

fn table_blep_falling_correction(phi: f32x4, dt: f32x4, points: [u32; LANES]) -> f32x4 {
    let phase = phi.to_array();
    let phase_inc = dt.to_array();
    let mut correction = [0.0; LANES];

    for lane in 0..LANES {
        let window = phase_inc[lane] * points[lane] as f32;
        correction[lane] = table_blep_falling_correction_lane(phase[lane], window);
    }

    f32x4::new(correction)
}

#[inline]
fn table_blep_falling_correction_lane(phase: f32, window: f32) -> f32 {
    if window <= 0.0 {
        return 0.0;
    }

    if phase > 1.0 - window {
        let t = (1.0 - phase) / window;
        -blep_table_lookup(t, true)
    } else if phase < window {
        let t = phase / window;
        -blep_table_lookup(t, false)
    } else {
        0.0
    }
}

/// Look up the BLEP residual at normalised t ∈ [0, 1). `left_side`: true = left of edge.
fn blep_table_lookup(t: f32, left_side: bool) -> f32 {
    if left_side {
        let idx = ((1.0 - t) * (TABLE_CENTER - 1) as f32) as usize;
        BLEP_TABLE_PIRKLE[idx.min(TABLE_CENTER - 1)]
    } else {
        let idx = (t * (TABLE_CENTER - 1) as f32) as usize + TABLE_CENTER;
        BLEP_TABLE_PIRKLE[idx.min(TABLE_SIZE - 1)]
    }
}

#[cfg(test)]
fn blep_pulse_reference(phi: f32x4, dt: f32x4, width: f32x4, method: SawMethod) -> f32x4 {
    let one = f32x4::splat(1.0);
    let half = f32x4::splat(0.5);
    let width = width.clamp(f32x4::splat(MIN_PULSE_WIDTH), f32x4::splat(MAX_PULSE_WIDTH));
    let saw1 = blep_saw(phi, dt, method);
    let phi2 = wrap01(phi + width);
    let saw2 = blep_saw(phi2, dt, method);
    let out = half * saw1 - half * saw2;
    out * f32x4::splat(2.0) + width * f32x4::splat(2.0) - one
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_pulse_matches_two_saw_reference() {
        const WIDTHS: [f32; 4] = [0.01, 0.5, 0.745, 0.99];
        const PHASE_INCREMENTS: [f32; 12] = [
            0.0,
            1.0e-6,
            55.0 / 48_000.0,
            440.0 / 48_000.0,
            4_000.0 / 48_000.0,
            0.124_999,
            0.125,
            0.125_001,
            0.249_999,
            0.25,
            0.250_001,
            0.499,
        ];
        const MAX_ERROR: f32 = 2.0e-6;

        for method in [SawMethod::PolyBlep, SawMethod::Blep] {
            for width in WIDTHS {
                for phase_inc in PHASE_INCREMENTS {
                    let dt = f32x4::splat(phase_inc);
                    let width = f32x4::splat(width);
                    for index in (0..4096).step_by(4) {
                        let phases = f32x4::new([
                            index as f32 / 4096.0,
                            (index + 1) as f32 / 4096.0,
                            (index + 2) as f32 / 4096.0,
                            (index + 3) as f32 / 4096.0,
                        ]);
                        let expected = blep_pulse_reference(phases, dt, width, method).to_array();
                        let actual = blep_pulse(phases, dt, width, method).to_array();

                        for lane in 0..LANES {
                            let error = (actual[lane] - expected[lane]).abs();
                            assert!(
                                error <= MAX_ERROR,
                                "method={method:?} width={} dt={phase_inc} phase={} expected={} actual={} error={error}",
                                width.to_array()[lane],
                                phases.to_array()[lane],
                                expected[lane],
                                actual[lane],
                            );
                        }
                    }
                }
            }
        }
    }
}
