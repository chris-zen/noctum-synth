use crate::f32x4;

pub use crate::blep::SawMethod;
use crate::blep::{blep_pulse, blep_saw};
use crate::rng::DspRng;
use crate::{DEFAULT_SAMPLE_RATE, LANES, wrap01};

pub(crate) const MIN_PHASE_INC: f32 = 0.0;
pub(crate) const MAX_PHASE_INC: f32 = 0.499;
pub(crate) const MIN_PULSE_WIDTH: f32 = 0.01;
pub(crate) const MAX_PULSE_WIDTH: f32 = 0.99;
const MAX_SLOP_CENTS: f32 = 14.0;
const MAX_POLYBLAMP2_PHASE_INC: f32 = 0.25;
const MIN_POLYBLAMP2_PHASE_INC: f32 = 1.0e-12;

/// Selectable oscillator waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Saw,
    SawTri,
    Triangle,
    Pulse,
}

/// Output of a single oscillator sample step, including phase-wrap metadata
/// used by oscillator sync.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OscillatorStep {
    /// Band-limited waveform output per SIMD lane.
    pub output: f32x4,
    /// Whether each lane wrapped past the end of its cycle this step.
    pub wrapped: [bool; LANES],
    /// Sub-sample position of the wrap within the step, in `[0, 1)`.
    pub wrap_phase_fraction: [f32; LANES],
}

/// A 4-lane (SIMD) virtual-analog oscillator.
///
/// Each lane is an independent voice sharing the same waveform/shape settings.
/// Pitch drift ("slop") is intrinsic and applied internally on top of the
/// intended frequency.
pub struct AnalogOscillator {
    waveform: Waveform,
    saw_method: SawMethod,
    shape: f32,
    fine_cents: f32,
    note_offset: f32,
    sample_rate: f32,
    phase: f32x4,
    phase_inc: f32x4,
    intended_frequency_hz: f32x4,
    effective_frequency_hz: f32x4,
    enabled_mask: f32x4,
    last_output: f32x4,
    triangle_integrator: f32x4,
    slop: OscSlopState,
}

impl Default for AnalogOscillator {
    fn default() -> Self {
        Self {
            waveform: Waveform::Saw,
            saw_method: SawMethod::Blep,
            shape: 0.0,
            fine_cents: 0.0,
            note_offset: 0.0,
            sample_rate: DEFAULT_SAMPLE_RATE,
            phase: f32x4::splat(0.0),
            phase_inc: f32x4::splat(0.0),
            intended_frequency_hz: f32x4::splat(0.0),
            effective_frequency_hz: f32x4::splat(0.0),
            enabled_mask: f32x4::splat(1.0),
            last_output: f32x4::splat(0.0),
            triangle_integrator: f32x4::splat(-1.0),
            slop: OscSlopState::new(),
        }
    }
}

impl AnalogOscillator {
    /// Creates an oscillator running at `sample_rate` with default settings.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            ..Default::default()
        }
    }

    /// Sets the active waveform.
    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
        if matches!(waveform, Waveform::Triangle | Waveform::SawTri) {
            self.triangle_integrator = naive_triangle(self.phase);
        }
    }

    /// Selects the band-limiting method used for saw/pulse edges.
    pub fn set_saw_method(&mut self, saw_method: SawMethod) {
        self.saw_method = saw_method;
        if saw_method == SawMethod::PolyBlep
            && matches!(self.waveform, Waveform::Triangle | Waveform::SawTri)
        {
            self.triangle_integrator = naive_triangle(self.phase);
        }
    }

    /// Sets the waveform shape/morph amount, clamped to `[0, 1]`.
    pub fn set_shape(&mut self, shape: f32) {
        self.shape = shape.clamp(0.0, 1.0);
    }

    /// Enables or mutes all lanes uniformly.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled_mask = f32x4::splat(if enabled { 1.0 } else { 0.0 });
    }

    /// Sets a per-lane enable gain mask, clamped to `[0, 1]`.
    pub fn set_enabled_mask(&mut self, enabled_mask: f32x4) {
        self.enabled_mask = enabled_mask.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
    }

    /// Sets fine tuning in cents (`[-100, 100]`) and recomputes frequency.
    pub fn set_fine_cents(&mut self, cents: f32, note_frequency_hz: f32x4) {
        self.fine_cents = cents.clamp(-100.0, 100.0);
        self.update_frequency_from_note(note_frequency_hz);
    }

    /// Sets the coarse pitch offset in semitones and recomputes frequency.
    pub fn set_note_offset(&mut self, offset: f32, note_frequency_hz: f32x4) {
        self.note_offset = offset;
        self.update_frequency_from_note(note_frequency_hz);
    }

    /// Sets the phase of all lanes, wrapped into `[0, 1)`.
    pub fn set_phase(&mut self, phase: f32x4) {
        self.phase = wrap01(phase);
        self.triangle_integrator = naive_triangle(self.phase);
    }

    /// Resets all lanes to mid-cycle phase (`0.5`).
    pub fn reset_phase(&mut self) {
        self.phase = f32x4::splat(0.5);
        self.triangle_integrator = naive_triangle(self.phase);
    }

    /// Resets one lane to mid-cycle phase (`0.5`).
    pub fn reset_phase_lane(&mut self, lane: usize) {
        let mut phase = self.phase.to_array();
        phase[lane] = 0.5;
        self.phase = f32x4::new(phase);
        self.triangle_integrator = set_lane(self.triangle_integrator, lane, 1.0);
    }

    /// Resets one lane to the start of its cycle (`0.0`).
    pub fn reset_cycle_lane(&mut self, lane: usize) {
        let mut phase = self.phase.to_array();
        phase[lane] = 0.0;
        self.phase = f32x4::new(phase);
        self.triangle_integrator = set_lane(self.triangle_integrator, lane, -1.0);
    }

    /// Resets the phase of each flagged lane to the start of its cycle, as
    /// driven by an oscillator-sync master.
    pub fn sync_reset_lanes(&mut self, reset: [bool; LANES]) {
        let mut phase = self.phase.to_array();
        for lane in 0..LANES {
            if reset[lane] {
                phase[lane] = 0.0;
                self.triangle_integrator = set_lane(self.triangle_integrator, lane, -1.0);
            }
        }
        self.phase = f32x4::new(phase);
    }

    /// Applies oscillator-sync resets at sub-sample positions from the master.
    ///
    /// The slave phase is set to where it would be at the end of the current
    /// sample after resetting at `wrap_phase_fraction` and advancing for the
    /// remaining part of the sample.
    pub(crate) fn sync_reset_lanes_at(
        &mut self,
        reset: [bool; LANES],
        wrap_phase_fraction: [f32; LANES],
    ) {
        let mut phase = self.phase.to_array();
        let phase_inc = self.phase_inc.to_array();
        let mut triangle_integrator = self.triangle_integrator.to_array();

        for lane in 0..LANES {
            if reset[lane] {
                let remaining = (1.0 - wrap_phase_fraction[lane]).clamp(0.0, 1.0);
                phase[lane] = phase_inc[lane] * remaining;
                triangle_integrator[lane] = naive_triangle_lane(phase[lane]);
            }
        }

        self.phase = f32x4::new(phase);
        self.triangle_integrator = f32x4::new(triangle_integrator);
    }

    /// Sets the intended (pre-slop) frequency per lane and refreshes the
    /// effective drifted frequency.
    pub fn set_frequency(&mut self, freq: f32x4) {
        let freq = sanitize_frequency(freq, self.sample_rate);
        self.intended_frequency_hz = freq;
        self.refresh_effective_frequency();
    }

    /// Returns the current effective (slop-drifted) frequency per lane.
    pub fn frequency_hz(&self) -> f32x4 {
        self.effective_frequency_hz
    }

    /// Sets the normalized pitch-drift ("slop") amount in `[0, 1]`.
    pub(crate) fn set_slop_amount(&mut self, amount: f32) {
        self.slop.set_amount(amount);
        self.refresh_effective_frequency();
    }

    /// Triggers oscillator-local note-on behavior for one SIMD lane.
    ///
    /// This reseeds that lane's slop state and optionally resets its phase.
    /// It does not set note pitch, velocity, gate state, or voice-level note state.
    pub(crate) fn trigger_lane(&mut self, lane: usize, reset_phase: bool) {
        self.slop.trigger_lane(lane);
        if reset_phase {
            self.reset_phase_lane(lane);
        }
        self.refresh_effective_frequency();
    }

    /// Recomputes frequency from a per-lane note pitch, applying note offset
    /// and fine tuning.
    pub fn update_frequency_from_note(&mut self, note_frequency_hz: f32x4) {
        let semitones = self.note_offset + self.fine_cents / 100.0;
        let ratio = f32x4::splat(crate::math::powf(2.0, semitones / 12.0));
        self.set_frequency(note_frequency_hz * ratio);
    }
    /// Advances one sample and returns just the waveform output.
    pub fn next(&mut self) -> f32x4 {
        self.next_step().output
    }

    /// Advances one sample, returning the output plus phase-wrap metadata.
    pub(crate) fn next_step(&mut self) -> OscillatorStep {
        if self.slop.is_enabled() {
            self.slop.advance(self.sample_rate);
            self.refresh_effective_frequency();
        }

        let phi = self.phase;
        let next_phase = self.phase + self.phase_inc;
        let wrapped = phase_wrapped(phi, self.phase_inc, next_phase);
        let wrap_phase_fraction = wrap_phase_fraction(phi, self.phase_inc, wrapped);
        self.phase = wrap01(next_phase);

        let raw = self.sample_waveform(phi);
        let output = self.apply_shape_morph(phi, raw);
        self.last_output = output;
        self.align_triangle_integrator_after_wrap(wrapped);
        OscillatorStep {
            output: output * self.enabled_mask,
            wrapped,
            wrap_phase_fraction,
        }
    }

    /// Evaluates the band-limited base waveform at phase `phi`.
    fn sample_waveform(&mut self, phi: f32x4) -> f32x4 {
        match self.waveform {
            Waveform::Saw => blep_saw(phi, self.phase_inc, self.saw_method),
            Waveform::SawTri => {
                let saw = blep_saw(phi, self.phase_inc, self.saw_method);
                let tri = self.triangle(phi);
                let mix = f32x4::splat(self.shape.abs());
                saw + (tri - saw) * mix
            }
            Waveform::Triangle => self.triangle(phi),
            Waveform::Pulse => blep_pulse(
                phi,
                self.phase_inc,
                f32x4::splat(pulse_width_from_shape(self.shape)),
                self.saw_method,
            ),
        }
    }

    fn triangle(&mut self, phi: f32x4) -> f32x4 {
        match self.saw_method {
            SawMethod::PolyBlep => self.polyblep_integrated_triangle(phi),
            SawMethod::Blep => polyblamp2_triangle(phi, self.phase_inc),
        }
    }

    fn polyblep_integrated_triangle(&mut self, phi: f32x4) -> f32x4 {
        let square = blep_pulse(phi, self.phase_inc, f32x4::splat(0.5), SawMethod::PolyBlep);
        self.triangle_integrator = (self.triangle_integrator
            - square * self.phase_inc * f32x4::splat(4.0))
        .clamp(f32x4::splat(-1.2), f32x4::splat(1.2));
        self.triangle_integrator
    }

    /// Morphs saw/triangle timbre by crossfading `raw` with a phase-shifted
    /// copy of the same waveform; other waveforms pass through unchanged.
    fn apply_shape_morph(&self, phi: f32x4, raw: f32x4) -> f32x4 {
        if !matches!(self.waveform, Waveform::Saw | Waveform::Triangle) {
            return raw;
        }
        let shape = self.shape.abs();
        if shape == 0.0 {
            return raw;
        }
        let shifted_phi = wrap01(phi + f32x4::splat(self.shape * 0.5));
        let shifted = self.sample_waveform_at(shifted_phi);
        let amount = f32x4::splat(shape);
        raw + (shifted - raw) * amount
    }

    fn sample_waveform_at(&self, phi: f32x4) -> f32x4 {
        match self.waveform {
            Waveform::Saw => blep_saw(phi, self.phase_inc, self.saw_method),
            Waveform::Triangle => match self.saw_method {
                SawMethod::PolyBlep => polyblamp2_triangle(phi, self.phase_inc),
                SawMethod::Blep => polyblamp2_triangle(phi, self.phase_inc),
            },
            _ => self.last_output,
        }
    }

    fn align_triangle_integrator_after_wrap(&mut self, wrapped: [bool; LANES]) {
        if !matches!(self.waveform, Waveform::Triangle | Waveform::SawTri)
            || self.saw_method != SawMethod::PolyBlep
        {
            return;
        }

        let phase = self.phase.to_array();
        let mut integrator = self.triangle_integrator.to_array();
        for lane in 0..LANES {
            if wrapped[lane] {
                integrator[lane] = naive_triangle_lane(phase[lane]);
            }
        }
        self.triangle_integrator = f32x4::new(integrator);
    }

    /// Recomputes the effective frequency and phase increment from the
    /// intended frequency plus current slop offset.
    fn refresh_effective_frequency(&mut self) {
        let freq = self.intended_frequency_hz * self.slop.frequency_ratio();
        let freq = clamp_frequency(freq, self.sample_rate);
        self.effective_frequency_hz = freq;
        self.phase_inc = freq * f32x4::splat(1.0 / self.sample_rate);
    }
}

/// Per-lane analog pitch instability ("slop").
///
/// Combines a fixed per-note pitch ratio with a slow random-walk pitch ratio,
/// both scaled by `amount`.
struct OscSlopState {
    amount: f32,
    static_ratio: f32x4,
    drift_ratio: f32x4,
    drift_target_ratio: f32x4,
    drift_ratio_step: f32x4,
    samples_until_target: [u32; LANES],
    rng: [DspRng; LANES],
}

impl OscSlopState {
    /// Creates a cleared slop state with zero amount.
    fn new() -> Self {
        Self {
            amount: 0.0,
            static_ratio: f32x4::splat(1.0),
            drift_ratio: f32x4::splat(1.0),
            drift_target_ratio: f32x4::splat(1.0),
            drift_ratio_step: f32x4::splat(1.0),
            samples_until_target: [0; LANES],
            rng: [
                DspRng::new(0x0a50_0001, 0x51ab_0001),
                DspRng::new(0x0a50_0002, 0x51ab_0002),
                DspRng::new(0x0a50_0003, 0x51ab_0003),
                DspRng::new(0x0a50_0004, 0x51ab_0004),
            ],
        }
    }

    /// Sets the normalized amount in `[0, 1]`, clearing state when zero.
    fn set_amount(&mut self, amount: f32) {
        self.amount = amount.clamp(0.0, 1.0);
        if self.amount == 0.0 {
            self.clear();
        }
    }

    fn is_enabled(&self) -> bool {
        self.amount > 0.0
    }

    /// Zeroes all accumulated detune and drift state.
    fn clear(&mut self) {
        self.static_ratio = f32x4::splat(1.0);
        self.drift_ratio = f32x4::splat(1.0);
        self.drift_target_ratio = f32x4::splat(1.0);
        self.drift_ratio_step = f32x4::splat(1.0);
        self.samples_until_target = [0; LANES];
    }

    /// Reseeds one lane's fixed per-note detune with a fresh random offset.
    fn trigger_lane(&mut self, lane: usize) {
        let mut static_ratio = self.static_ratio.to_array();
        static_ratio[lane] =
            cents_to_ratio(bipolar_random(&mut self.rng[lane]) * self.depth_cents() * 0.5);
        self.static_ratio = f32x4::new(static_ratio);
    }

    /// Advances the per-lane drift random walk by one sample.
    fn advance(&mut self, sample_rate: f32) {
        let depth_cents = self.depth_cents();
        if depth_cents <= 0.0 {
            self.clear();
            return;
        }

        let mut drift_ratio = self.drift_ratio.to_array();
        let mut drift_target_ratio = self.drift_target_ratio.to_array();
        let mut drift_ratio_step = self.drift_ratio_step.to_array();
        for lane in 0..LANES {
            if self.samples_until_target[lane] == 0 {
                let sample_rate = sample_rate.max(1.0);
                let min_samples = ((0.5 * sample_rate) as u32).max(1);
                let max_samples = ((4.0 * sample_rate) as u32).max(min_samples);
                let samples = self.rng[lane].u32_inclusive(min_samples, max_samples);
                drift_target_ratio[lane] =
                    cents_to_ratio(bipolar_random(&mut self.rng[lane]) * depth_cents * 0.5);
                drift_ratio_step[lane] = crate::math::powf(
                    drift_target_ratio[lane] / drift_ratio[lane],
                    1.0 / samples as f32,
                );
                self.samples_until_target[lane] = samples;
            }

            if self.samples_until_target[lane] > 0 {
                drift_ratio[lane] *= drift_ratio_step[lane];
                self.samples_until_target[lane] = self.samples_until_target[lane].saturating_sub(1);
                if self.samples_until_target[lane] == 0 {
                    drift_ratio[lane] = drift_target_ratio[lane];
                }
            }
        }

        self.drift_ratio = f32x4::new(drift_ratio);
        self.drift_target_ratio = f32x4::new(drift_target_ratio);
        self.drift_ratio_step = f32x4::new(drift_ratio_step);
    }

    /// Returns the combined static plus drift pitch ratio per lane.
    fn frequency_ratio(&self) -> f32x4 {
        self.static_ratio * self.drift_ratio
    }

    /// Returns the peak detune depth in cents for the current amount.
    fn depth_cents(&self) -> f32 {
        MAX_SLOP_CENTS * self.amount * self.amount
    }
}

fn set_lane(value: f32x4, lane: usize, lane_value: f32) -> f32x4 {
    let mut values = value.to_array();
    values[lane] = lane_value;
    f32x4::new(values)
}

/// Clamps each lane's frequency to a finite, non-negative value below Nyquist.
fn sanitize_frequency(freq: f32x4, sample_rate: f32) -> f32x4 {
    let max_freq = max_frequency(sample_rate);
    let clamped = freq.clamp(f32x4::splat(0.0), f32x4::splat(max_freq));
    freq.is_finite().blend(clamped, f32x4::splat(0.0))
}

/// Clamps each lane's frequency to `[0, max_frequency]` for the sample rate.
fn clamp_frequency(freq: f32x4, sample_rate: f32) -> f32x4 {
    freq.clamp(f32x4::splat(0.0), f32x4::splat(max_frequency(sample_rate)))
}

/// Returns the highest allowed frequency (below Nyquist) for `sample_rate`,
/// or `0.0` if the sample rate is invalid.
fn max_frequency(sample_rate: f32) -> f32 {
    if sample_rate.is_finite() && sample_rate > 0.0 {
        sample_rate * MAX_PHASE_INC
    } else {
        0.0
    }
}

/// Flags lanes whose phase crosses the `1.0` cycle boundary this step.
fn phase_wrapped(phi: f32x4, phase_inc: f32x4, next_phase: f32x4) -> [bool; LANES] {
    let old = phi.to_array();
    let inc = phase_inc.to_array();
    let next = next_phase.to_array();
    let mut wrapped = [false; LANES];
    for lane in 0..LANES {
        wrapped[lane] = inc[lane] > 0.0 && next[lane] >= 1.0 && old[lane] < 1.0;
    }
    wrapped
}

/// Computes the sub-sample position of each lane's wrap within the step, in
/// `[0, 1)`, for hard-sync alignment.
fn wrap_phase_fraction(phi: f32x4, phase_inc: f32x4, wrapped: [bool; LANES]) -> [f32; LANES] {
    let old = phi.to_array();
    let inc = phase_inc.to_array();
    let mut fraction = [0.0; LANES];
    for lane in 0..LANES {
        if wrapped[lane] && inc[lane] > 0.0 {
            fraction[lane] = ((1.0 - old[lane]) / inc[lane]).clamp(0.0, 1.0);
        }
    }
    fraction
}

/// Evaluates a band-limited triangle per SIMD lane, correcting the two
/// slope discontinuities with second-order polyBLAMP residuals.
fn polyblamp2_triangle(phi: f32x4, dt: f32x4) -> f32x4 {
    let zero = f32x4::splat(0.0);
    let slope_jump = f32x4::splat(8.0);
    let naive = naive_triangle(phi);
    let active = dt.simd_gt(zero) & dt.simd_lt(f32x4::splat(MAX_POLYBLAMP2_PHASE_INC));

    active.blend(
        naive + slope_jump * polyblamp2_corner(phi, dt)
            - slope_jump * polyblamp2_corner(wrap01(phi - f32x4::splat(0.5)), dt),
        naive,
    )
}

/// Evaluates an aliased triangle per SIMD lane from phase in `[0, 1)`.
///
/// Peaks at `1.0` when `phi = 0.5` and reaches `0.0` at the cycle boundaries.
/// Used as the base waveform for [`polyblamp2_triangle`] and to seed the
/// triangle integrator after phase changes.
fn naive_triangle(phi: f32x4) -> f32x4 {
    f32x4::splat(1.0) - (phi - f32x4::splat(0.5)).abs() * f32x4::splat(4.0)
}

/// Scalar [`naive_triangle`] for a single lane.
fn naive_triangle_lane(phase: f32) -> f32 {
    1.0 - (phase - 0.5).abs() * 4.0
}

/// Computes the second-order polyBLAMP correction near a slope corner, given
/// the phase distance from that corner and the per-sample phase increment `dt`.
fn polyblamp2_corner(phase_from_corner: f32x4, dt: f32x4) -> f32x4 {
    let zero = f32x4::splat(0.0);
    let one = f32x4::splat(1.0);
    let third = f32x4::splat(1.0 / 3.0);
    let safe_dt = dt.max(f32x4::splat(MIN_POLYBLAMP2_PHASE_INC));
    let right_t = one - phase_from_corner / safe_dt;
    let right = right_t * right_t * right_t * safe_dt * third;
    let left_t = one - (one - phase_from_corner) / safe_dt;
    let left = left_t * left_t * left_t * safe_dt * third;

    phase_from_corner
        .simd_lt(dt)
        .blend(right, phase_from_corner.simd_gt(one - dt).blend(left, zero))
}

/// Converts a pitch offset in cents to a multiplicative frequency ratio.
fn cents_to_ratio(cents: f32) -> f32 {
    crate::math::powf(2.0, cents / 1200.0)
}

/// Returns a uniform random value in `[-1, 1)`.
fn bipolar_random(rng: &mut DspRng) -> f32 {
    rng.f32() * 2.0 - 1.0
}

/// Maps a shape value in `[0, 1]` to a pulse duty cycle in `[0.5, 0.99]`.
pub fn pulse_width_from_shape(shape_mod: f32) -> f32 {
    0.5 + shape_mod.clamp(0.0, 1.0) * 0.49
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_step_reports_phase_wraps_per_lane() {
        let mut osc = AnalogOscillator::new(100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(f32x4::new([40.0, 10.0, 0.0, 25.0]));
        osc.set_phase(f32x4::new([0.7, 0.95, 0.99, 0.2]));

        let step = osc.next_step();

        assert_eq!(step.wrapped, [true, true, false, false]);
        assert!((step.wrap_phase_fraction[0] - 0.75).abs() < 0.001);
        assert!((step.wrap_phase_fraction[1] - 0.5).abs() < 0.001);
    }

    #[test]
    fn sync_reset_lanes_at_preserves_subsample_offset() {
        let mut osc = AnalogOscillator::new(100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(f32x4::splat(10.0));
        osc.set_phase(f32x4::new([0.4, 0.4, 0.4, 0.4]));
        osc.next();

        osc.sync_reset_lanes_at([true, false, true, false], [0.25, 0.0, 0.75, 0.0]);
        let phase = osc.phase.to_array();

        assert!(
            (phase[0] - 0.075).abs() < 0.001,
            "lane reset at 25% should end at phase 0.075, got {}",
            phase[0]
        );
        assert!(
            (phase[2] - 0.025).abs() < 0.001,
            "lane reset at 75% should end at phase 0.025, got {}",
            phase[2]
        );
        assert!(
            (phase[1] - 0.5).abs() < 0.001,
            "non-reset lane should keep its advanced phase"
        );
        assert!(
            (phase[3] - 0.5).abs() < 0.001,
            "non-reset lane should keep its advanced phase"
        );
    }
}
