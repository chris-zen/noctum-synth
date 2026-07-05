use wide::f32x4;

use crate::analog_oscillator::{MAX_PULSE_WIDTH, MIN_PULSE_WIDTH};
use crate::{LANES, wrap01};

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

pub(crate) fn blep_pulse(phi: f32x4, dt: f32x4, width: f32x4, method: SawMethod) -> f32x4 {
    let one = f32x4::splat(1.0);
    let half = f32x4::splat(0.5);
    let width = width.clamp(f32x4::splat(MIN_PULSE_WIDTH), f32x4::splat(MAX_PULSE_WIDTH));
    let saw1 = blep_saw(phi, dt, method);
    let phi2 = wrap01(phi + width);
    let saw2 = blep_saw(phi2, dt, method);
    let out = half * saw1 - half * saw2;
    out * f32x4::splat(2.0) + width * f32x4::splat(2.0) - one
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
        if phase_inc[lane] > 0.0 {
            correction[lane] = polyblep_residual_lane(phase[lane], phase_inc[lane])
                - polyblep_residual_lane(1.0 - phase[lane], phase_inc[lane]);
        }
    }

    f32x4::new(correction)
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
        points[idx] = if *phase_inc <= 0.125 {
            4
        } else if *phase_inc <= 0.25 {
            2
        } else {
            1
        };
    }

    points
}

fn table_blep_falling_correction(phi: f32x4, dt: f32x4, points: [u32; LANES]) -> f32x4 {
    let phase = phi.to_array();
    let phase_inc = dt.to_array();
    let mut correction = [0.0; LANES];

    for lane in 0..LANES {
        let window = phase_inc[lane] * points[lane] as f32;
        if window <= 0.0 {
            continue;
        }

        if phase[lane] > 1.0 - window {
            let t = (1.0 - phase[lane]) / window;
            correction[lane] = -blep_table_lookup(t, true);
        } else if phase[lane] < window {
            let t = phase[lane] / window;
            correction[lane] = -blep_table_lookup(t, false);
        }
    }

    f32x4::new(correction)
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
