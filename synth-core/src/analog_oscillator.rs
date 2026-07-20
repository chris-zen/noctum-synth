use crate::f32x4;

pub use crate::blep::SawMethod;
use crate::blep::{
    PulseBlepState, blep_pulse, blep_pulse_prepared, blep_saw, table_points_per_side_lane,
};
#[cfg(feature = "profiling")]
use crate::profiling::NoopProfiler;
use crate::rng::DspRng;
use crate::{DEFAULT_SAMPLE_RATE, LANES, wrap01};
#[cfg(feature = "profiling")]
use crate::{RenderProfiler, RenderStage};

pub(crate) const MIN_PHASE_INC: f32 = 0.0;
pub(crate) const MAX_PHASE_INC: f32 = 0.499;
pub(crate) const MIN_PULSE_WIDTH: f32 = 0.01;
pub(crate) const MAX_PULSE_WIDTH: f32 = 0.99;
const MAX_SLOP_CENTS: f32 = 14.0;
const MAX_POLYBLAMP2_PHASE_INC: f32 = 0.25;
const MIN_POLYBLAMP2_PHASE_INC: f32 = 1.0e-12;
/// Correction-only crossfade length; requested pitch and phase change immediately.
const CORRECTION_TRANSITION_SAMPLES: u8 = 24;
/// Smaller within-tier changes track directly; table-support tier changes always crossfade.
const CORRECTION_STEP_RELATIVE_THRESHOLD: f32 = 0.01;

/// Selectable oscillator waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Saw,
    SawTri,
    Triangle,
    Pulse,
}

trait OscillatorKernel {
    fn saw_method(&self) -> SawMethod;

    #[inline(always)]
    fn supports_correction_transition(&self) -> bool {
        true
    }

    #[inline(always)]
    fn prepare_sample(&mut self, _phase_inc: f32x4) {}

    #[inline(always)]
    fn finish_sample(&mut self) {}

    #[inline(always)]
    fn saw(&self, phase: f32x4, phase_inc: f32x4) -> f32x4 {
        blep_saw(phase, phase_inc, self.saw_method())
    }

    #[inline(always)]
    fn pulse(&self, phase: f32x4, phase_inc: f32x4, state: &PulseBlepState) -> f32x4 {
        blep_pulse_prepared(phase, phase_inc, state, self.saw_method())
    }

    #[inline(always)]
    fn triangle(&self, phase: f32x4, phase_inc: f32x4, integrator: &mut f32x4) -> f32x4 {
        if self.saw_method() == SawMethod::PolyBlep {
            let square = blep_pulse(phase, phase_inc, f32x4::splat(0.5), SawMethod::PolyBlep);
            *integrator = (*integrator - square * phase_inc * f32x4::splat(4.0))
                .clamp(f32x4::splat(-1.2), f32x4::splat(1.2));
            *integrator
        } else {
            polyblamp2_triangle(phase, phase_inc)
        }
    }

    #[inline(always)]
    fn triangle_at(&self, phase: f32x4, phase_inc: f32x4) -> f32x4 {
        polyblamp2_triangle(phase, phase_inc)
    }

    #[inline(always)]
    fn needs_triangle_wrap_alignment(&self) -> bool {
        self.saw_method() == SawMethod::PolyBlep
    }
}

impl OscillatorKernel for crate::wavetable::WavetableOscillatorKernel {
    fn saw_method(&self) -> SawMethod {
        // The public SawMethod enum intentionally remains BLEP/PolyBLEP-only.
        SawMethod::Blep
    }

    fn supports_correction_transition(&self) -> bool {
        false
    }

    fn prepare_sample(&mut self, phase_inc: f32x4) {
        crate::wavetable::WavetableOscillatorKernel::prepare(self, phase_inc);
    }

    fn finish_sample(&mut self) {
        crate::wavetable::WavetableOscillatorKernel::finish(self);
    }

    fn saw(&self, phase: f32x4, _phase_inc: f32x4) -> f32x4 {
        crate::wavetable::WavetableOscillatorKernel::saw(self, phase)
    }

    fn pulse(&self, phase: f32x4, _phase_inc: f32x4, state: &PulseBlepState) -> f32x4 {
        let width = state.width();
        let shifted = wrap01(phase + width);
        crate::wavetable::WavetableOscillatorKernel::saw(self, phase)
            - crate::wavetable::WavetableOscillatorKernel::saw(self, shifted)
            + width * f32x4::splat(2.0)
            - f32x4::splat(1.0)
    }

    fn triangle(&self, phase: f32x4, _phase_inc: f32x4, _integrator: &mut f32x4) -> f32x4 {
        crate::wavetable::WavetableOscillatorKernel::triangle(self, phase)
    }

    fn triangle_at(&self, phase: f32x4, _phase_inc: f32x4) -> f32x4 {
        crate::wavetable::WavetableOscillatorKernel::triangle(self, phase)
    }

    fn needs_triangle_wrap_alignment(&self) -> bool {
        false
    }
}

/// Runtime-selectable kernel retained for oscillator analysis and the public
/// low-level oscillator API. The synth engine uses a fixed typed kernel.
#[doc(hidden)]
pub struct RuntimeOscillatorKernel {
    method: SawMethod,
}

impl Default for RuntimeOscillatorKernel {
    fn default() -> Self {
        Self {
            method: SawMethod::Blep,
        }
    }
}

impl OscillatorKernel for RuntimeOscillatorKernel {
    #[inline(always)]
    fn saw_method(&self) -> SawMethod {
        self.method
    }
}

#[cfg(any(test, not(feature = "oscillator-polyblep")))]
#[derive(Default)]
pub(crate) struct BlepOscillatorKernel;

#[cfg(any(test, not(feature = "oscillator-polyblep")))]
impl OscillatorKernel for BlepOscillatorKernel {
    #[inline(always)]
    fn saw_method(&self) -> SawMethod {
        SawMethod::Blep
    }
}

#[cfg(any(test, feature = "oscillator-polyblep"))]
#[derive(Default)]
pub(crate) struct PolyBlepOscillatorKernel;

#[cfg(any(test, feature = "oscillator-polyblep"))]
impl OscillatorKernel for PolyBlepOscillatorKernel {
    #[inline(always)]
    fn saw_method(&self) -> SawMethod {
        SawMethod::PolyBlep
    }
}

#[cfg(feature = "oscillator-polyblep")]
type EngineOscillatorKernel = PolyBlepOscillatorKernel;
#[cfg(not(feature = "oscillator-polyblep"))]
type EngineOscillatorKernel = BlepOscillatorKernel;
pub(crate) type EngineOscillator = AnalogOscillator<EngineOscillatorKernel>;

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
pub struct AnalogOscillator<K = RuntimeOscillatorKernel> {
    waveform: Waveform,
    kernel: K,
    shape: f32,
    sample_rate: f32,
    phase: f32x4,
    phase_inc: f32x4,
    correction_from_phase_inc: f32x4,
    correction_transition_remaining: [u8; LANES],
    correction_transition_mask: u8,
    pulse_blep: PulseBlepState,
    intended_frequency_hz: f32x4,
    effective_frequency_hz: f32x4,
    enabled_mask: f32x4,
    last_output: f32x4,
    triangle_integrator: f32x4,
    slop: OscSlopState,
}

impl Default for AnalogOscillator<RuntimeOscillatorKernel> {
    fn default() -> Self {
        Self {
            waveform: Waveform::Saw,
            kernel: RuntimeOscillatorKernel {
                method: SawMethod::Blep,
            },
            shape: 0.0,
            sample_rate: DEFAULT_SAMPLE_RATE,
            phase: f32x4::splat(0.0),
            phase_inc: f32x4::splat(0.0),
            correction_from_phase_inc: f32x4::splat(0.0),
            correction_transition_remaining: [0; LANES],
            correction_transition_mask: 0,
            pulse_blep: PulseBlepState::new(f32x4::splat(0.5)),
            intended_frequency_hz: f32x4::splat(0.0),
            effective_frequency_hz: f32x4::splat(0.0),
            enabled_mask: f32x4::splat(1.0),
            last_output: f32x4::splat(0.0),
            triangle_integrator: f32x4::splat(-1.0),
            slop: OscSlopState::new(),
        }
    }
}

impl AnalogOscillator<RuntimeOscillatorKernel> {
    /// Creates an oscillator running at `sample_rate` with default settings.
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            ..Default::default()
        }
    }

    /// Selects the band-limiting method used for saw/pulse edges.
    pub fn set_saw_method(&mut self, saw_method: SawMethod) {
        self.kernel.method = saw_method;
        self.clear_correction_transition();
        if saw_method == SawMethod::Blep && self.waveform == Waveform::Pulse {
            self.pulse_blep.set_phase_inc(self.phase_inc);
        }
        if saw_method == SawMethod::PolyBlep
            && matches!(self.waveform, Waveform::Triangle | Waveform::SawTri)
        {
            self.triangle_integrator = naive_triangle(self.phase);
        }
    }
}

impl EngineOscillator {
    pub(crate) fn new_engine(sample_rate: f32) -> Self {
        AnalogOscillator::new_with_kernel(sample_rate, EngineOscillatorKernel::default())
    }
}

pub type WavetableOscillator = AnalogOscillator<crate::wavetable::WavetableOscillatorKernel>;

impl WavetableOscillator {
    /// Creates a wavetable oscillator backed by the supplied immutable bank.
    pub fn new_wavetable(sample_rate: f32, bank: crate::wavetable::WavetableBank) -> Self {
        AnalogOscillator::new_with_kernel(
            sample_rate,
            crate::wavetable::WavetableOscillatorKernel::new(bank),
        )
    }
}

#[allow(private_bounds)]
impl<K: OscillatorKernel> AnalogOscillator<K> {
    fn new_with_kernel(sample_rate: f32, kernel: K) -> Self {
        Self {
            waveform: Waveform::Saw,
            kernel,
            shape: 0.0,
            sample_rate,
            phase: f32x4::splat(0.0),
            phase_inc: f32x4::splat(0.0),
            correction_from_phase_inc: f32x4::splat(0.0),
            correction_transition_remaining: [0; LANES],
            correction_transition_mask: 0,
            pulse_blep: PulseBlepState::new(f32x4::splat(0.5)),
            intended_frequency_hz: f32x4::splat(0.0),
            effective_frequency_hz: f32x4::splat(0.0),
            enabled_mask: f32x4::splat(1.0),
            last_output: f32x4::splat(0.0),
            triangle_integrator: f32x4::splat(-1.0),
            slop: OscSlopState::new(),
        }
    }

    /// Sets the active waveform.
    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.waveform = waveform;
        self.clear_correction_transition();
        if waveform == Waveform::Pulse && self.kernel.saw_method() == SawMethod::Blep {
            self.pulse_blep.set_phase_inc(self.phase_inc);
        }
        if matches!(waveform, Waveform::Triangle | Waveform::SawTri) {
            self.triangle_integrator = naive_triangle(self.phase);
        }
    }

    /// Sets the waveform shape/morph amount, clamped to `[0, 1]`.
    pub fn set_shape(&mut self, shape: f32) {
        self.shape = shape.clamp(0.0, 1.0);
        self.pulse_blep
            .set_width(f32x4::splat(pulse_width_from_shape(self.shape)));
    }

    /// Enables or mutes all lanes uniformly.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled_mask = f32x4::splat(if enabled { 1.0 } else { 0.0 });
    }

    /// Sets a per-lane enable gain mask, clamped to `[0, 1]`.
    pub fn set_enabled_mask(&mut self, enabled_mask: f32x4) {
        self.enabled_mask = enabled_mask.clamp(f32x4::splat(0.0), f32x4::splat(1.0));
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

    /// Advances one sample and returns just the waveform output.
    pub fn next(&mut self) -> f32x4 {
        self.next_step().output
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn next_profiled(&mut self, profiler: &mut impl RenderProfiler) -> f32x4 {
        self.next_step_inner(profiler).output
    }

    /// Advances one sample, returning the output plus phase-wrap metadata.
    pub(crate) fn next_step(&mut self) -> OscillatorStep {
        #[cfg(feature = "profiling")]
        {
            return self.next_step_inner(&mut NoopProfiler);
        }
        #[cfg(not(feature = "profiling"))]
        self.next_step_inner()
    }

    #[cfg(feature = "profiling")]
    pub(crate) fn next_step_profiled(
        &mut self,
        profiler: &mut impl RenderProfiler,
    ) -> OscillatorStep {
        self.next_step_inner(profiler)
    }

    fn next_step_inner(
        &mut self,
        #[cfg(feature = "profiling")] profiler: &mut impl RenderProfiler,
    ) -> OscillatorStep {
        if self.slop.is_enabled() {
            self.slop.advance(self.sample_rate);
            self.refresh_effective_frequency();
        }

        let phi = self.phase;
        let next_phase = self.phase + self.phase_inc;
        let wrapped = phase_wrapped(phi, self.phase_inc, next_phase);
        let wrap_phase_fraction = wrap_phase_fraction(phi, self.phase_inc, wrapped);
        self.phase = wrap01(next_phase);

        #[cfg(feature = "profiling")]
        profiler.begin(RenderStage::OscillatorWaveform);
        self.kernel.prepare_sample(self.phase_inc);
        let raw = self.sample_waveform(phi);
        let current_output = self.apply_shape_morph(phi, raw);
        let output = if let Some((previous_phase_inc, blend)) = self.correction_transition_step() {
            self.previous_correction_output(phi, previous_phase_inc)
                .map(|previous| previous + (current_output - previous) * blend)
                .unwrap_or(current_output)
        } else {
            current_output
        };
        self.last_output = output;
        self.align_triangle_integrator_after_wrap(wrapped);
        self.kernel.finish_sample();
        #[cfg(feature = "profiling")]
        profiler.end(RenderStage::OscillatorWaveform);
        OscillatorStep {
            output: output * self.enabled_mask,
            wrapped,
            wrap_phase_fraction,
        }
    }

    /// Evaluates the band-limited base waveform at phase `phi`.
    fn sample_waveform(&mut self, phi: f32x4) -> f32x4 {
        match self.waveform {
            Waveform::Saw => self.kernel.saw(phi, self.phase_inc),
            Waveform::SawTri => {
                let saw = self.kernel.saw(phi, self.phase_inc);
                let tri = self.triangle(phi);
                let mix = f32x4::splat(self.shape.abs());
                saw + (tri - saw) * mix
            }
            Waveform::Triangle => self.triangle(phi),
            Waveform::Pulse => self.kernel.pulse(phi, self.phase_inc, &self.pulse_blep),
        }
    }

    fn triangle(&mut self, phi: f32x4) -> f32x4 {
        self.kernel
            .triangle(phi, self.phase_inc, &mut self.triangle_integrator)
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
            Waveform::Saw => self.kernel.saw(phi, self.phase_inc),
            Waveform::Triangle => self.kernel.triangle_at(phi, self.phase_inc),
            _ => self.last_output,
        }
    }

    fn previous_correction_output(&self, phi: f32x4, phase_inc: f32x4) -> Option<f32x4> {
        if !self.kernel.supports_correction_transition() {
            return None;
        }

        match self.waveform {
            Waveform::Saw => {
                let raw = self.kernel.saw(phi, phase_inc);
                let shape = self.shape.abs();
                if shape == 0.0 {
                    return Some(raw);
                }
                let shifted_phi = wrap01(phi + f32x4::splat(self.shape * 0.5));
                let shifted = self.kernel.saw(shifted_phi, phase_inc);
                Some(raw + (shifted - raw) * f32x4::splat(shape))
            }
            Waveform::Pulse => Some(blep_pulse(
                phi,
                phase_inc,
                self.pulse_blep.width(),
                self.kernel.saw_method(),
            )),
            Waveform::SawTri | Waveform::Triangle => None,
        }
    }

    fn correction_transition_step(&mut self) -> Option<(f32x4, f32x4)> {
        if self.correction_transition_mask == 0 {
            return None;
        }

        let previous_phase_inc = self.correction_from_phase_inc;
        let mut blend = [1.0; LANES];
        let mut correction_from = self.correction_from_phase_inc.to_array();
        let current_phase_inc = self.phase_inc.to_array();
        let denominator = f32::from(CORRECTION_TRANSITION_SAMPLES - 1);

        for lane in 0..LANES {
            let remaining = self.correction_transition_remaining[lane];
            if remaining == 0 {
                continue;
            }
            blend[lane] = f32::from(CORRECTION_TRANSITION_SAMPLES - remaining) / denominator;
            self.correction_transition_remaining[lane] -= 1;
            if self.correction_transition_remaining[lane] == 0 {
                correction_from[lane] = current_phase_inc[lane];
                self.correction_transition_mask &= !(1 << lane);
            }
        }
        self.correction_from_phase_inc = f32x4::new(correction_from);

        Some((previous_phase_inc, f32x4::new(blend)))
    }

    fn begin_correction_transition(&mut self, previous: f32x4, current: f32x4) {
        if !self.kernel.supports_correction_transition()
            || !matches!(self.waveform, Waveform::Saw | Waveform::Pulse)
        {
            if self.correction_transition_mask != 0 {
                self.clear_correction_transition();
            }
            return;
        }

        let previous = previous.to_array();
        let current = current.to_array();
        let mut correction_from = self.correction_from_phase_inc.to_array();
        for lane in 0..LANES {
            if correction_step_needs_transition(
                previous[lane],
                current[lane],
                self.kernel.saw_method(),
            ) {
                correction_from[lane] = previous[lane];
                self.correction_transition_remaining[lane] = CORRECTION_TRANSITION_SAMPLES;
                self.correction_transition_mask |= 1 << lane;
            } else if self.correction_transition_remaining[lane] == 0 {
                correction_from[lane] = current[lane];
            }
        }
        self.correction_from_phase_inc = f32x4::new(correction_from);
    }

    fn clear_correction_transition(&mut self) {
        self.correction_from_phase_inc = self.phase_inc;
        self.correction_transition_remaining = [0; LANES];
        self.correction_transition_mask = 0;
    }

    fn align_triangle_integrator_after_wrap(&mut self, wrapped: [bool; LANES]) {
        if !matches!(self.waveform, Waveform::Triangle | Waveform::SawTri)
            || !self.kernel.needs_triangle_wrap_alignment()
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
        let previous_phase_inc = self.phase_inc;
        let freq = self.intended_frequency_hz * self.slop.frequency_ratio();
        let freq = clamp_frequency(freq, self.sample_rate);
        self.effective_frequency_hz = freq;
        self.phase_inc = freq * f32x4::splat(1.0 / self.sample_rate);
        self.begin_correction_transition(previous_phase_inc, self.phase_inc);
        if self.waveform == Waveform::Pulse && self.kernel.saw_method() == SawMethod::Blep {
            self.pulse_blep.set_phase_inc(self.phase_inc);
        }
    }
}

fn correction_step_needs_transition(previous: f32, current: f32, method: SawMethod) -> bool {
    if previous <= MIN_PHASE_INC || current <= MIN_PHASE_INC {
        return false;
    }

    let large_step =
        (current - previous).abs() >= previous.max(current) * CORRECTION_STEP_RELATIVE_THRESHOLD;
    large_step
        || (method == SawMethod::Blep
            && table_points_per_side_lane(previous) != table_points_per_side_lane(current))
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
    #[cfg(feature = "embedded-math")]
    {
        return polyblamp2_triangle_scalar_lanes(phi, dt);
    }
    #[cfg(not(feature = "embedded-math"))]
    polyblamp2_triangle_simd(phi, dt)
}

/// Cortex-M7 has a scalar FPU, so skip corner arithmetic independently for
/// every inactive lane instead of evaluating and blending four divisions.
#[cfg(feature = "embedded-math")]
fn polyblamp2_triangle_scalar_lanes(phi: f32x4, dt: f32x4) -> f32x4 {
    let phases = phi.to_array();
    let phase_increments = dt.to_array();
    let mut output = [0.0; LANES];

    for lane in 0..LANES {
        let phase = phases[lane];
        let phase_increment = phase_increments[lane];
        let naive = naive_triangle_lane(phase);
        if !(phase_increment > 0.0 && phase_increment < MAX_POLYBLAMP2_PHASE_INC) {
            output[lane] = naive;
            continue;
        }

        let midpoint_phase = if phase >= 0.5 {
            phase - 0.5
        } else {
            phase + 0.5
        };
        output[lane] = naive + 8.0 * polyblamp2_corner_lane(phase, phase_increment)
            - 8.0 * polyblamp2_corner_lane(midpoint_phase, phase_increment);
    }

    f32x4::new(output)
}

#[cfg(feature = "embedded-math")]
#[inline]
fn polyblamp2_corner_lane(phase_from_corner: f32, dt: f32) -> f32 {
    let distance = if phase_from_corner < dt {
        phase_from_corner
    } else if phase_from_corner > 1.0 - dt {
        1.0 - phase_from_corner
    } else {
        return 0.0;
    };
    let safe_dt = dt.max(MIN_POLYBLAMP2_PHASE_INC);
    let t = 1.0 - distance / safe_dt;
    t * t * t * safe_dt * (1.0 / 3.0)
}

/// SIMD-capable hosts retain branchless evaluation, but share one reciprocal
/// across both corners instead of issuing four vector divisions.
#[cfg(not(feature = "embedded-math"))]
fn polyblamp2_triangle_simd(phi: f32x4, dt: f32x4) -> f32x4 {
    let zero = f32x4::splat(0.0);
    let one = f32x4::splat(1.0);
    let slope_jump = f32x4::splat(8.0);
    let naive = naive_triangle(phi);
    let active = dt.simd_gt(zero) & dt.simd_lt(f32x4::splat(MAX_POLYBLAMP2_PHASE_INC));
    let safe_dt = dt.max(f32x4::splat(MIN_POLYBLAMP2_PHASE_INC));
    let inverse_dt = one / safe_dt;

    active.blend(
        naive + slope_jump * polyblamp2_corner_simd(phi, dt, safe_dt, inverse_dt)
            - slope_jump
                * polyblamp2_corner_simd(wrap01(phi - f32x4::splat(0.5)), dt, safe_dt, inverse_dt),
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

#[cfg(not(feature = "embedded-math"))]
fn polyblamp2_corner_simd(
    phase_from_corner: f32x4,
    dt: f32x4,
    safe_dt: f32x4,
    inverse_dt: f32x4,
) -> f32x4 {
    let zero = f32x4::splat(0.0);
    let one = f32x4::splat(1.0);
    let third = f32x4::splat(1.0 / 3.0);
    let right_t = one - phase_from_corner * inverse_dt;
    let right = right_t * right_t * right_t * safe_dt * third;
    let left_t = one - (one - phase_from_corner) * inverse_dt;
    let left = left_t * left_t * left_t * safe_dt * third;

    phase_from_corner
        .simd_lt(dt)
        .blend(right, phase_from_corner.simd_gt(one - dt).blend(left, zero))
}

#[cfg(test)]
fn polyblamp2_triangle_reference(phi: f32x4, dt: f32x4) -> f32x4 {
    let zero = f32x4::splat(0.0);
    let slope_jump = f32x4::splat(8.0);
    let naive = naive_triangle(phi);
    let active = dt.simd_gt(zero) & dt.simd_lt(f32x4::splat(MAX_POLYBLAMP2_PHASE_INC));

    active.blend(
        naive + slope_jump * polyblamp2_corner_reference(phi, dt)
            - slope_jump * polyblamp2_corner_reference(wrap01(phi - f32x4::splat(0.5)), dt),
        naive,
    )
}

#[cfg(test)]
fn polyblamp2_corner_reference(phase_from_corner: f32x4, dt: f32x4) -> f32x4 {
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
    crate::math::exp2(cents / 1200.0)
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
    fn optimized_polyblamp_matches_branchless_reference() {
        let phase_increments = [
            [0.0, 1.0e-15, 110.0 / 48_000.0, 440.0 / 48_000.0],
            [880.0 / 48_000.0, 3_520.0 / 48_000.0, 0.249, 0.25],
        ];
        let mut maximum_error = 0.0f32;

        for increments in phase_increments {
            let dt = f32x4::new(increments);
            for sample in 0..4_096 {
                let base = sample as f32 / 4_096.0;
                let phi = f32x4::new([
                    base,
                    (base + 0.127).fract(),
                    (base + 0.499).fract(),
                    (base + 0.773).fract(),
                ]);
                let optimized = polyblamp2_triangle(phi, dt).to_array();
                let reference = polyblamp2_triangle_reference(phi, dt).to_array();
                for lane in 0..LANES {
                    maximum_error = maximum_error.max((optimized[lane] - reference[lane]).abs());
                }
            }
        }

        assert!(
            maximum_error <= 2.0e-6,
            "optimized PolyBLAMP diverged from reference by {maximum_error}"
        );
    }

    #[test]
    fn typed_kernels_match_runtime_method_selection() {
        for method in [SawMethod::Blep, SawMethod::PolyBlep] {
            for waveform in [
                Waveform::Saw,
                Waveform::SawTri,
                Waveform::Triangle,
                Waveform::Pulse,
            ] {
                let mut runtime = AnalogOscillator::new(48_000.0);
                runtime.set_saw_method(method);
                runtime.set_waveform(waveform);
                runtime.set_shape(0.37);
                runtime.set_frequency(f32x4::new([110.0, 220.0, 440.0, 880.0]));

                let mut typed = match method {
                    SawMethod::Blep => TypedKernel::Blep(AnalogOscillator::new_with_kernel(
                        48_000.0,
                        BlepOscillatorKernel,
                    )),
                    SawMethod::PolyBlep => TypedKernel::PolyBlep(
                        AnalogOscillator::new_with_kernel(48_000.0, PolyBlepOscillatorKernel),
                    ),
                };
                typed.set_waveform(waveform);
                typed.set_shape(0.37);
                typed.set_frequency(f32x4::new([110.0, 220.0, 440.0, 880.0]));

                for sample in 0..4096 {
                    assert_eq!(
                        runtime.next().to_array().map(f32::to_bits),
                        typed.next().to_array().map(f32::to_bits),
                        "typed {method:?} {waveform:?} diverged at sample {sample}"
                    );
                }
            }
        }
    }

    fn sample_at(method: SawMethod, waveform: Waveform, frequency: f32, phase: f32) -> f32 {
        let mut oscillator = AnalogOscillator::new(48_000.0);
        oscillator.set_saw_method(method);
        oscillator.set_waveform(waveform);
        oscillator.set_shape(0.383_838_4);
        oscillator.set_frequency(f32x4::splat(frequency));
        oscillator.set_phase(f32x4::splat(phase));
        oscillator.next().to_array()[0]
    }

    #[test]
    fn abrupt_frequency_step_crossfades_blep_correction_without_slewing_phase() {
        const OLD_FREQUENCY: f32 = 7_649.9;
        const NEW_FREQUENCY: f32 = 5_697.1;

        for method in [SawMethod::Blep, SawMethod::PolyBlep] {
            for waveform in [Waveform::Saw, Waveform::Pulse] {
                let (phase, unsmoothed_jump) = (0..4_096)
                    .map(|index| index as f32 / 4_096.0)
                    .map(|phase| {
                        let old = sample_at(method, waveform, OLD_FREQUENCY, phase);
                        let new = sample_at(method, waveform, NEW_FREQUENCY, phase);
                        (phase, (new - old).abs())
                    })
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .unwrap();
                assert!(
                    unsmoothed_jump > 0.05,
                    "test step should expose a {method:?} {waveform:?} correction jump, got {unsmoothed_jump}"
                );

                let expected_old = sample_at(method, waveform, OLD_FREQUENCY, phase);
                let mut transitioned = AnalogOscillator::new(48_000.0);
                transitioned.set_saw_method(method);
                transitioned.set_waveform(waveform);
                transitioned.set_shape(0.383_838_4);
                transitioned.set_frequency(f32x4::splat(OLD_FREQUENCY));
                transitioned.set_phase(f32x4::splat(phase));
                transitioned.set_frequency(f32x4::splat(NEW_FREQUENCY));

                let first = transitioned.next().to_array()[0];
                assert!(
                    (first - expected_old).abs() <= 2.0e-6,
                    "first {method:?} {waveform:?} transition sample changed correction: expected {expected_old}, got {first}"
                );
                let expected_phase =
                    wrap01(f32x4::splat(phase + NEW_FREQUENCY / 48_000.0)).to_array()[0];
                assert!(
                    (transitioned.phase.to_array()[0] - expected_phase).abs() <= f32::EPSILON,
                    "pitch phase should advance immediately at the new frequency"
                );
                assert_eq!(
                    transitioned.correction_transition_remaining,
                    [CORRECTION_TRANSITION_SAMPLES - 1; LANES]
                );

                for _ in 1..CORRECTION_TRANSITION_SAMPLES {
                    transitioned.next();
                }
                assert_eq!(transitioned.correction_transition_remaining, [0; LANES]);
            }
        }
    }

    #[test]
    fn table_blep_support_boundary_always_transitions() {
        let mut oscillator = AnalogOscillator::new(48_000.0);
        oscillator.set_waveform(Waveform::Pulse);
        oscillator.set_frequency(f32x4::splat(0.124_9 * 48_000.0));
        oscillator.set_frequency(f32x4::splat(0.125_1 * 48_000.0));

        assert_eq!(
            oscillator.correction_transition_remaining,
            [CORRECTION_TRANSITION_SAMPLES; LANES]
        );
    }

    #[test]
    fn small_frequency_update_within_one_blep_tier_stays_on_fast_path() {
        let mut oscillator = AnalogOscillator::new(48_000.0);
        oscillator.set_waveform(Waveform::Pulse);
        oscillator.set_frequency(f32x4::splat(0.10 * 48_000.0));
        oscillator.set_frequency(f32x4::splat(0.100_5 * 48_000.0));

        assert_eq!(oscillator.correction_transition_remaining, [0; LANES]);
    }

    enum TypedKernel {
        Blep(AnalogOscillator<BlepOscillatorKernel>),
        PolyBlep(AnalogOscillator<PolyBlepOscillatorKernel>),
    }

    impl TypedKernel {
        fn set_waveform(&mut self, waveform: Waveform) {
            match self {
                Self::Blep(oscillator) => oscillator.set_waveform(waveform),
                Self::PolyBlep(oscillator) => oscillator.set_waveform(waveform),
            }
        }

        fn set_shape(&mut self, shape: f32) {
            match self {
                Self::Blep(oscillator) => oscillator.set_shape(shape),
                Self::PolyBlep(oscillator) => oscillator.set_shape(shape),
            }
        }

        fn set_frequency(&mut self, frequency: f32x4) {
            match self {
                Self::Blep(oscillator) => oscillator.set_frequency(frequency),
                Self::PolyBlep(oscillator) => oscillator.set_frequency(frequency),
            }
        }

        fn next(&mut self) -> f32x4 {
            match self {
                Self::Blep(oscillator) => oscillator.next(),
                Self::PolyBlep(oscillator) => oscillator.next(),
            }
        }
    }

    #[cfg(feature = "profiling")]
    struct BoundaryCounter {
        waveform_begins: u32,
        waveform_ends: u32,
    }

    #[cfg(feature = "profiling")]
    impl RenderProfiler for BoundaryCounter {
        fn begin(&mut self, stage: RenderStage) {
            self.waveform_begins += u32::from(stage == RenderStage::OscillatorWaveform);
        }

        fn end(&mut self, stage: RenderStage) {
            self.waveform_ends += u32::from(stage == RenderStage::OscillatorWaveform);
        }
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn profiled_triangle_is_bit_exact_and_balances_waveform_boundaries() {
        let mut normal = AnalogOscillator::new(48_000.0);
        let mut profiled = AnalogOscillator::new(48_000.0);
        for oscillator in [&mut normal, &mut profiled] {
            oscillator.set_waveform(Waveform::Triangle);
            oscillator.set_frequency(f32x4::new([110.0, 220.0, 440.0, 880.0]));
            oscillator.set_shape(0.35);
        }
        let mut profiler = BoundaryCounter {
            waveform_begins: 0,
            waveform_ends: 0,
        };

        for _ in 0..1_024 {
            assert_eq!(
                normal.next().to_array(),
                profiled.next_profiled(&mut profiler).to_array()
            );
        }
        assert_eq!(profiler.waveform_begins, 1_024);
        assert_eq!(profiler.waveform_ends, 1_024);
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn profiled_pulse_is_bit_exact_and_balances_waveform_boundaries() {
        let mut normal = AnalogOscillator::new(48_000.0);
        let mut profiled = AnalogOscillator::new(48_000.0);
        for oscillator in [&mut normal, &mut profiled] {
            oscillator.set_waveform(Waveform::Pulse);
            oscillator.set_frequency(f32x4::new([110.0, 220.0, 440.0, 880.0]));
            oscillator.set_shape(0.5);
        }
        let mut profiler = BoundaryCounter {
            waveform_begins: 0,
            waveform_ends: 0,
        };

        for _ in 0..1_024 {
            assert_eq!(
                normal.next().to_array(),
                profiled.next_profiled(&mut profiler).to_array()
            );
        }
        assert_eq!(profiler.waveform_begins, 1_024);
        assert_eq!(profiler.waveform_ends, 1_024);
    }

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
