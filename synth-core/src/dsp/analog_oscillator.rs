use crate::{
    DEFAULT_SAMPLE_RATE,
    dsp::{
        blep::{
            PulseBlepState, SawMethod, blep_pulse, blep_pulse_prepared, blep_saw,
            table_points_per_side_lane,
        },
        rng::DspRng,
    },
    math::{F32, WideF32},
    profiling::{RenderContext, RenderStage},
    wrap01,
};

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

impl Waveform {
    pub fn from_index(index: usize) -> Self {
        match index {
            1 => Self::SawTri,
            2 => Self::Triangle,
            3 => Self::Pulse,
            _ => Self::Saw,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Self::Saw => 0,
            Self::SawTri => 1,
            Self::Triangle => 2,
            Self::Pulse => 3,
        }
    }
}

trait OscillatorKernel {
    fn saw_method(&self) -> SawMethod;

    #[inline(always)]
    fn supports_correction_transition(&self) -> bool {
        true
    }

    #[inline(always)]
    fn prepare_sample(&mut self, _phase_inc: WideF32) {}

    #[inline(always)]
    fn finish_sample(&mut self) {}

    #[inline(always)]
    fn saw(&self, phase: WideF32, phase_inc: WideF32) -> WideF32 {
        blep_saw(phase, phase_inc, self.saw_method())
    }

    #[inline(always)]
    fn pulse(&self, phase: WideF32, phase_inc: WideF32, state: &PulseBlepState) -> WideF32 {
        blep_pulse_prepared(phase, phase_inc, state, self.saw_method())
    }

    #[inline(always)]
    fn triangle(&self, phase: WideF32, phase_inc: WideF32, integrator: &mut WideF32) -> WideF32 {
        if self.saw_method() == SawMethod::PolyBlep {
            let square = blep_pulse(phase, phase_inc, WideF32::splat(0.5), SawMethod::PolyBlep);
            *integrator = (*integrator - square * phase_inc * WideF32::splat(4.0))
                .clamp(WideF32::splat(-1.2), WideF32::splat(1.2));
            *integrator
        } else {
            polyblamp2_triangle(phase, phase_inc)
        }
    }

    #[inline(always)]
    fn triangle_at(&self, phase: WideF32, phase_inc: WideF32) -> WideF32 {
        polyblamp2_triangle(phase, phase_inc)
    }

    #[inline(always)]
    fn needs_triangle_wrap_alignment(&self) -> bool {
        self.saw_method() == SawMethod::PolyBlep
    }
}

impl OscillatorKernel for crate::dsp::wavetable::WavetableOscillatorKernel {
    fn saw_method(&self) -> SawMethod {
        // The public SawMethod enum intentionally remains BLEP/PolyBLEP-only.
        SawMethod::Blep
    }

    fn supports_correction_transition(&self) -> bool {
        false
    }

    fn prepare_sample(&mut self, phase_inc: WideF32) {
        crate::dsp::wavetable::WavetableOscillatorKernel::prepare(self, phase_inc);
    }

    fn finish_sample(&mut self) {
        crate::dsp::wavetable::WavetableOscillatorKernel::finish(self);
    }

    fn saw(&self, phase: WideF32, _phase_inc: WideF32) -> WideF32 {
        crate::dsp::wavetable::WavetableOscillatorKernel::saw(self, phase)
    }

    fn pulse(&self, phase: WideF32, _phase_inc: WideF32, state: &PulseBlepState) -> WideF32 {
        let width = state.width();
        let shifted = wrap01(phase + width);
        crate::dsp::wavetable::WavetableOscillatorKernel::saw(self, phase)
            - crate::dsp::wavetable::WavetableOscillatorKernel::saw(self, shifted)
            + width * WideF32::splat(2.0)
            - WideF32::splat(1.0)
    }

    fn triangle(&self, phase: WideF32, _phase_inc: WideF32, _integrator: &mut WideF32) -> WideF32 {
        crate::dsp::wavetable::WavetableOscillatorKernel::triangle(self, phase)
    }

    fn triangle_at(&self, phase: WideF32, _phase_inc: WideF32) -> WideF32 {
        crate::dsp::wavetable::WavetableOscillatorKernel::triangle(self, phase)
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

/// Per-lane comparison mask for conditional SIMD updates (`blend` true branch).
type LaneMask = WideF32;

/// Output of a single oscillator sample step, including phase-wrap metadata
/// used by oscillator sync.
#[derive(Debug, Clone, Copy)]
pub struct OscillatorStep {
    /// Band-limited waveform output per SIMD lane.
    pub output: WideF32,
    /// Lanes that wrapped past the end of their cycle this step.
    pub(crate) wrapped: LaneMask,
    /// Sub-sample position of the wrap within the step, in `[0, 1)`.
    pub(crate) subsample_offset: WideF32,
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
    phase: WideF32,
    phase_inc: WideF32,
    correction_from_phase_inc: WideF32,
    correction_transition_remaining: [u8; WideF32::LANES],
    correction_transition_mask: u8,
    pulse_blep: PulseBlepState,
    intended_frequency_hz: WideF32,
    effective_frequency_hz: WideF32,
    enabled_mask: WideF32,
    last_output: WideF32,
    triangle_integrator: WideF32,
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
            phase: WideF32::ZERO,
            phase_inc: WideF32::ZERO,
            correction_from_phase_inc: WideF32::ZERO,
            correction_transition_remaining: [0; WideF32::LANES],
            correction_transition_mask: 0,
            pulse_blep: PulseBlepState::new(WideF32::splat(0.5)),
            intended_frequency_hz: WideF32::ZERO,
            effective_frequency_hz: WideF32::ZERO,
            enabled_mask: WideF32::splat(1.0),
            last_output: WideF32::ZERO,
            triangle_integrator: WideF32::splat(-1.0),
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

pub type WavetableOscillator = AnalogOscillator<crate::dsp::wavetable::WavetableOscillatorKernel>;

impl WavetableOscillator {
    /// Creates a wavetable oscillator backed by the supplied immutable bank.
    pub fn new_wavetable(sample_rate: f32, bank: crate::dsp::wavetable::WavetableBank) -> Self {
        AnalogOscillator::new_with_kernel(
            sample_rate,
            crate::dsp::wavetable::WavetableOscillatorKernel::new(bank),
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
            phase: WideF32::ZERO,
            phase_inc: WideF32::ZERO,
            correction_from_phase_inc: WideF32::ZERO,
            correction_transition_remaining: [0; WideF32::LANES],
            correction_transition_mask: 0,
            pulse_blep: PulseBlepState::new(WideF32::splat(0.5)),
            intended_frequency_hz: WideF32::ZERO,
            effective_frequency_hz: WideF32::ZERO,
            enabled_mask: WideF32::splat(1.0),
            last_output: WideF32::ZERO,
            triangle_integrator: WideF32::splat(-1.0),
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
            .set_width(WideF32::splat(pulse_width_from_shape(self.shape)));
    }

    /// Enables or mutes all lanes uniformly.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled_mask = WideF32::splat(if enabled { 1.0 } else { 0.0 });
    }

    /// Sets a per-lane enable gain mask, clamped to `[0, 1]`.
    pub fn set_enabled_mask(&mut self, enabled_mask: WideF32) {
        self.enabled_mask = enabled_mask.clamp(WideF32::ZERO, WideF32::splat(1.0));
    }

    /// Sets the phase of all lanes, wrapped into `[0, 1)`.
    pub fn set_phase(&mut self, phase: WideF32) {
        self.phase = wrap01(phase);
        self.triangle_integrator = naive_triangle(self.phase);
    }

    /// Resets one lane to its waveform-appropriate starting phase.
    pub fn start_phase_lane(&mut self, lane: usize) {
        let start_phase = 0.0;
        self.phase = self.phase.replace_lane(lane, start_phase);
        let tri = naive_triangle_scalar(start_phase);
        self.triangle_integrator = self.triangle_integrator.replace_lane(lane, tri);
    }

    /// Applies oscillator-sync resets at sub-sample positions from the master.
    ///
    /// The slave phase is set to where it would be at the end of the current
    /// sample after resetting at `subsample_offset` and advancing for the
    /// remaining part of the sample.
    pub(crate) fn hard_sync_reset(&mut self, reset: LaneMask, subsample_offset: WideF32) {
        let one = WideF32::splat(1.0);
        let remaining = (one - subsample_offset).clamp(WideF32::ZERO, one);
        let synced_phase = self.phase_inc * remaining;

        self.phase = reset.blend(synced_phase, self.phase);
        self.triangle_integrator =
            reset.blend(naive_triangle(synced_phase), self.triangle_integrator);
    }

    /// Sets the intended (pre-slop) frequency per lane and refreshes the
    /// effective drifted frequency.
    pub fn set_frequency(&mut self, freq: WideF32) {
        let freq = sanitize_frequency(freq, self.sample_rate);
        self.intended_frequency_hz = freq;
        self.refresh_effective_frequency();
    }

    /// Returns the current effective (slop-drifted) frequency per lane.
    pub fn frequency_hz(&self) -> WideF32 {
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
            self.start_phase_lane(lane);
        }
        self.refresh_effective_frequency();
    }

    /// Advances one sample, returning the output plus phase-wrap metadata.
    pub fn next(&mut self, ctx: &mut RenderContext<'_>) -> OscillatorStep {
        if self.slop.is_enabled() {
            self.slop.advance(self.sample_rate);
            self.refresh_effective_frequency();
        }

        let phi = self.phase;
        let next_phase = self.phase + self.phase_inc;
        let wrapped = phase_wrapped_mask(phi, self.phase_inc, next_phase);
        let subsample_offset = wrap_subsample_offset(phi, self.phase_inc, wrapped);
        self.phase = wrap01(next_phase);

        crate::profiler_begin!(ctx, RenderStage::OscillatorWaveform);
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
        crate::profiler_end!(ctx, RenderStage::OscillatorWaveform);
        OscillatorStep {
            output: output * self.enabled_mask,
            wrapped,
            subsample_offset,
        }
    }

    /// Evaluates the band-limited base waveform at phase `phi`.
    fn sample_waveform(&mut self, phi: WideF32) -> WideF32 {
        match self.waveform {
            Waveform::Saw => self.kernel.saw(phi, self.phase_inc),
            Waveform::SawTri => {
                let saw = self.kernel.saw(phi, self.phase_inc);
                let tri = self.triangle(phi);
                let mix = WideF32::splat(self.shape.abs());
                saw + (tri - saw) * mix
            }
            Waveform::Triangle => self.triangle(phi),
            Waveform::Pulse => self.kernel.pulse(phi, self.phase_inc, &self.pulse_blep),
        }
    }

    fn triangle(&mut self, phi: WideF32) -> WideF32 {
        self.kernel
            .triangle(phi, self.phase_inc, &mut self.triangle_integrator)
    }

    /// Morphs saw/triangle timbre by crossfading `raw` with a phase-shifted
    /// copy of the same waveform; other waveforms pass through unchanged.
    fn apply_shape_morph(&self, phi: WideF32, raw: WideF32) -> WideF32 {
        if !matches!(self.waveform, Waveform::Saw | Waveform::Triangle) {
            return raw;
        }
        let shape = self.shape.abs();
        if shape == 0.0 {
            return raw;
        }
        let shifted_phi = wrap01(phi + WideF32::splat(self.shape * 0.5));
        let shifted = self.sample_waveform_at(shifted_phi);
        let amount = WideF32::splat(shape);
        raw + (shifted - raw) * amount
    }

    fn sample_waveform_at(&self, phi: WideF32) -> WideF32 {
        match self.waveform {
            Waveform::Saw => self.kernel.saw(phi, self.phase_inc),
            Waveform::Triangle => self.kernel.triangle_at(phi, self.phase_inc),
            _ => self.last_output,
        }
    }

    fn previous_correction_output(&self, phi: WideF32, phase_inc: WideF32) -> Option<WideF32> {
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
                let shifted_phi = wrap01(phi + WideF32::splat(self.shape * 0.5));
                let shifted = self.kernel.saw(shifted_phi, phase_inc);
                Some(raw + (shifted - raw) * WideF32::splat(shape))
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

    fn correction_transition_step(&mut self) -> Option<(WideF32, WideF32)> {
        if self.correction_transition_mask == 0 {
            return None;
        }

        let previous_phase_inc = self.correction_from_phase_inc;
        let mut blend = [1.0; WideF32::LANES];
        let mut correction_from = self.correction_from_phase_inc.to_array();
        let current_phase_inc = self.phase_inc.to_array();
        let denominator = f32::from(CORRECTION_TRANSITION_SAMPLES - 1);

        for lane in 0..WideF32::LANES {
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
        self.correction_from_phase_inc = WideF32::new(correction_from);

        Some((previous_phase_inc, WideF32::new(blend)))
    }

    fn begin_correction_transition(&mut self, previous: WideF32, current: WideF32) {
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
        for lane in 0..WideF32::LANES {
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
        self.correction_from_phase_inc = WideF32::new(correction_from);
    }

    fn clear_correction_transition(&mut self) {
        self.correction_from_phase_inc = self.phase_inc;
        self.correction_transition_remaining = [0; WideF32::LANES];
        self.correction_transition_mask = 0;
    }

    fn align_triangle_integrator_after_wrap(&mut self, wrapped: LaneMask) {
        if !matches!(self.waveform, Waveform::Triangle | Waveform::SawTri)
            || !self.kernel.needs_triangle_wrap_alignment()
        {
            return;
        }

        self.triangle_integrator =
            wrapped.blend(naive_triangle(self.phase), self.triangle_integrator);
    }

    /// Recomputes the effective frequency and phase increment from the
    /// intended frequency plus current slop offset.
    fn refresh_effective_frequency(&mut self) {
        let previous_phase_inc = self.phase_inc;
        let freq = self.intended_frequency_hz * self.slop.frequency_ratio();
        let freq = clamp_frequency(freq, self.sample_rate);
        self.effective_frequency_hz = freq;
        self.phase_inc = freq * WideF32::splat(1.0 / self.sample_rate);
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
    static_ratio: WideF32,
    drift_ratio: WideF32,
    drift_target_ratio: WideF32,
    drift_ratio_step: WideF32,
    samples_until_target: [u32; WideF32::LANES],
    rng: [DspRng; WideF32::LANES],
}

impl OscSlopState {
    /// Creates a cleared slop state with zero amount.
    fn new() -> Self {
        Self {
            amount: 0.0,
            static_ratio: WideF32::splat(1.0),
            drift_ratio: WideF32::splat(1.0),
            drift_target_ratio: WideF32::splat(1.0),
            drift_ratio_step: WideF32::splat(1.0),
            samples_until_target: [0; WideF32::LANES],
            rng: core::array::from_fn(|i| {
                let seeds = [
                    (0x0a50_0001, 0x51ab_0001),
                    (0x0a50_0002, 0x51ab_0002),
                    (0x0a50_0003, 0x51ab_0003),
                    (0x0a50_0004, 0x51ab_0004),
                    (0x0a50_0005, 0x51ab_0005),
                    (0x0a50_0006, 0x51ab_0006),
                    (0x0a50_0007, 0x51ab_0007),
                    (0x0a50_0008, 0x51ab_0008),
                ];
                DspRng::new(seeds[i].0, seeds[i].1)
            }),
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
        self.static_ratio = WideF32::splat(1.0);
        self.drift_ratio = WideF32::splat(1.0);
        self.drift_target_ratio = WideF32::splat(1.0);
        self.drift_ratio_step = WideF32::splat(1.0);
        self.samples_until_target = [0; WideF32::LANES];
    }

    /// Reseeds one lane's fixed per-note detune with a fresh random offset.
    fn trigger_lane(&mut self, lane: usize) {
        let mut static_ratio = self.static_ratio.to_array();
        static_ratio[lane] =
            cents_to_ratio(bipolar_random(&mut self.rng[lane]) * self.depth_cents() * 0.5);
        self.static_ratio = WideF32::new(static_ratio);
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
        for lane in 0..WideF32::LANES {
            if self.samples_until_target[lane] == 0 {
                let sample_rate = sample_rate.max(1.0);
                let min_samples = ((0.5 * sample_rate) as u32).max(1);
                let max_samples = ((4.0 * sample_rate) as u32).max(min_samples);
                let samples = self.rng[lane].u32_inclusive(min_samples, max_samples);
                drift_target_ratio[lane] =
                    cents_to_ratio(bipolar_random(&mut self.rng[lane]) * depth_cents * 0.5);
                drift_ratio_step[lane] = F32(drift_target_ratio[lane] / drift_ratio[lane])
                    .powf(F32(1.0 / samples as f32))
                    .as_f32();
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

        self.drift_ratio = WideF32::new(drift_ratio);
        self.drift_target_ratio = WideF32::new(drift_target_ratio);
        self.drift_ratio_step = WideF32::new(drift_ratio_step);
    }

    /// Returns the combined static plus drift pitch ratio per lane.
    fn frequency_ratio(&self) -> WideF32 {
        self.static_ratio * self.drift_ratio
    }

    /// Returns the peak detune depth in cents for the current amount.
    fn depth_cents(&self) -> f32 {
        MAX_SLOP_CENTS * self.amount * self.amount
    }
}

/// Clamps each lane's frequency to a finite, non-negative value below Nyquist.
fn sanitize_frequency(freq: WideF32, sample_rate: f32) -> WideF32 {
    let max_freq = max_frequency(sample_rate);
    let clamped = freq.clamp(WideF32::ZERO, WideF32::splat(max_freq));
    freq.is_finite().blend(clamped, WideF32::ZERO)
}

/// Clamps each lane's frequency to `[0, max_frequency]` for the sample rate.
fn clamp_frequency(freq: WideF32, sample_rate: f32) -> WideF32 {
    freq.clamp(WideF32::ZERO, WideF32::splat(max_frequency(sample_rate)))
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

/// Returns a per-lane mask for lanes whose phase crosses the `1.0` boundary.
fn phase_wrapped_mask(phi: WideF32, phase_inc: WideF32, next_phase: WideF32) -> LaneMask {
    let zero = WideF32::ZERO;
    let one = WideF32::splat(1.0);
    phase_inc.simd_gt(zero) & next_phase.simd_ge(one) & phi.simd_lt(one)
}

/// Computes the sub-sample position of each lane's wrap within the step, in
/// `[0, 1)`, for hard-sync alignment.
fn wrap_subsample_offset(phi: WideF32, phase_inc: WideF32, wrapped: LaneMask) -> WideF32 {
    let zero = WideF32::ZERO;
    let one = WideF32::splat(1.0);
    let offset = ((one - phi) / phase_inc).clamp(zero, one);
    wrapped.blend(offset, zero)
}

/// Evaluates a band-limited triangle per SIMD lane, correcting the two
/// slope discontinuities with second-order polyBLAMP residuals.
fn polyblamp2_triangle(phi: WideF32, dt: WideF32) -> WideF32 {
    #[cfg(feature = "fast-math")]
    {
        return polyblamp2_triangle_scalar_lanes(phi, dt);
    }
    #[cfg(not(feature = "fast-math"))]
    polyblamp2_triangle_simd(phi, dt)
}

/// Cortex-M7 has a scalar FPU, so skip corner arithmetic independently for
/// every inactive lane instead of evaluating and blending four divisions.
#[cfg(feature = "fast-math")]
fn polyblamp2_triangle_scalar_lanes(phi: WideF32, dt: WideF32) -> WideF32 {
    let phases = phi.to_array();
    let phase_increments = dt.to_array();
    let mut output = [0.0; WideF32::LANES];

    for lane in 0..WideF32::LANES {
        let phase = phases[lane];
        let phase_increment = phase_increments[lane];
        let naive = naive_triangle_scalar(phase);
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

    WideF32::new(output)
}

#[cfg(feature = "fast-math")]
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
#[cfg(not(feature = "fast-math"))]
fn polyblamp2_triangle_simd(phi: WideF32, dt: WideF32) -> WideF32 {
    let zero = WideF32::ZERO;
    let one = WideF32::splat(1.0);
    let slope_jump = WideF32::splat(8.0);
    let naive = naive_triangle(phi);
    let active = dt.simd_gt(zero) & dt.simd_lt(WideF32::splat(MAX_POLYBLAMP2_PHASE_INC));
    let safe_dt = dt.max(WideF32::splat(MIN_POLYBLAMP2_PHASE_INC));
    let inverse_dt = one / safe_dt;

    active.blend(
        naive + slope_jump * polyblamp2_corner_simd(phi, dt, safe_dt, inverse_dt)
            - slope_jump
                * polyblamp2_corner_simd(
                    wrap01(phi - WideF32::splat(0.5)),
                    dt,
                    safe_dt,
                    inverse_dt,
                ),
        naive,
    )
}

/// Evaluates an aliased triangle per SIMD lane from phase in `[0, 1)`.
///
/// Peaks at `1.0` when `phi = 0.5` and reaches `0.0` at the cycle boundaries.
/// Used as the base waveform for [`polyblamp2_triangle`] and to seed the
/// triangle integrator after phase changes.
fn naive_triangle(phi: WideF32) -> WideF32 {
    WideF32::splat(1.0) - (phi - WideF32::splat(0.5)).abs() * WideF32::splat(4.0)
}

/// Scalar [`naive_triangle`] for a single lane.
fn naive_triangle_scalar(phase: f32) -> f32 {
    1.0 - (phase - 0.5).abs() * 4.0
}

#[cfg(not(feature = "fast-math"))]
fn polyblamp2_corner_simd(
    phase_from_corner: WideF32,
    dt: WideF32,
    safe_dt: WideF32,
    inverse_dt: WideF32,
) -> WideF32 {
    let zero = WideF32::ZERO;
    let one = WideF32::splat(1.0);
    let third = WideF32::splat(1.0 / 3.0);
    let right_t = one - phase_from_corner * inverse_dt;
    let right = right_t * right_t * right_t * safe_dt * third;
    let left_t = one - (one - phase_from_corner) * inverse_dt;
    let left = left_t * left_t * left_t * safe_dt * third;

    phase_from_corner
        .simd_lt(dt)
        .blend(right, phase_from_corner.simd_gt(one - dt).blend(left, zero))
}

#[cfg(test)]
fn polyblamp2_triangle_reference(phi: WideF32, dt: WideF32) -> WideF32 {
    let zero = WideF32::ZERO;
    let slope_jump = WideF32::splat(8.0);
    let naive = naive_triangle(phi);
    let active = dt.simd_gt(zero) & dt.simd_lt(WideF32::splat(MAX_POLYBLAMP2_PHASE_INC));

    active.blend(
        naive + slope_jump * polyblamp2_corner_reference(phi, dt)
            - slope_jump * polyblamp2_corner_reference(wrap01(phi - WideF32::splat(0.5)), dt),
        naive,
    )
}

#[cfg(test)]
fn polyblamp2_corner_reference(phase_from_corner: WideF32, dt: WideF32) -> WideF32 {
    let zero = WideF32::ZERO;
    let one = WideF32::splat(1.0);
    let third = WideF32::splat(1.0 / 3.0);
    let safe_dt = dt.max(WideF32::splat(MIN_POLYBLAMP2_PHASE_INC));
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
    F32(cents / 1200.0).exp2().as_f32()
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
    use crate::math::testing::{lane0, mask_lane, mask_lane_active, splat};

    fn next_output<K: OscillatorKernel>(osc: &mut AnalogOscillator<K>) -> WideF32 {
        let mut ctx = crate::create_render_context!();
        osc.next(&mut ctx).output
    }

    #[test]
    fn optimized_polyblamp_matches_branchless_reference() {
        let phase_increments = [
            0.0,
            1.0e-15,
            110.0 / 48_000.0,
            440.0 / 48_000.0,
            880.0 / 48_000.0,
            3_520.0 / 48_000.0,
            0.249,
            0.25,
        ];
        let phase_offsets = [0.0, 0.127, 0.499, 0.773];
        let mut maximum_error = 0.0f32;

        for dt_value in phase_increments {
            let dt = splat(dt_value);
            for sample in 0..4_096 {
                let base = sample as f32 / 4_096.0;
                for offset in phase_offsets {
                    let phi = splat((base + offset).fract());
                    let optimized = lane0(polyblamp2_triangle(phi, dt));
                    let reference = lane0(polyblamp2_triangle_reference(phi, dt));
                    maximum_error = maximum_error.max((optimized - reference).abs());
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
                for frequency in [110.0, 220.0, 440.0, 880.0] {
                    let mut runtime = AnalogOscillator::new(48_000.0);
                    runtime.set_saw_method(method);
                    runtime.set_waveform(waveform);
                    runtime.set_shape(0.37);
                    runtime.set_frequency(splat(frequency));

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
                    typed.set_frequency(splat(frequency));

                    for sample in 0..4096 {
                        assert_eq!(
                            lane0(next_output(&mut runtime)).to_bits(),
                            lane0(typed.next()).to_bits(),
                            "typed {method:?} {waveform:?} at {frequency}Hz diverged at sample {sample}"
                        );
                    }
                }
            }
        }
    }

    fn sample_at(method: SawMethod, waveform: Waveform, frequency: f32, phase: f32) -> f32 {
        let mut oscillator = AnalogOscillator::new(48_000.0);
        oscillator.set_saw_method(method);
        oscillator.set_waveform(waveform);
        oscillator.set_shape(0.383_838_4);
        oscillator.set_frequency(WideF32::splat(frequency));
        oscillator.set_phase(WideF32::splat(phase));
        next_output(&mut oscillator).to_array()[0]
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
                transitioned.set_frequency(WideF32::splat(OLD_FREQUENCY));
                transitioned.set_phase(WideF32::splat(phase));
                transitioned.set_frequency(WideF32::splat(NEW_FREQUENCY));

                let first = next_output(&mut transitioned).to_array()[0];
                assert!(
                    (first - expected_old).abs() <= 2.0e-6,
                    "first {method:?} {waveform:?} transition sample changed correction: expected {expected_old}, got {first}"
                );
                let expected_phase =
                    wrap01(WideF32::splat(phase + NEW_FREQUENCY / 48_000.0)).to_array()[0];
                assert!(
                    (transitioned.phase.to_array()[0] - expected_phase).abs() <= f32::EPSILON,
                    "pitch phase should advance immediately at the new frequency"
                );
                assert_eq!(
                    transitioned.correction_transition_remaining,
                    [CORRECTION_TRANSITION_SAMPLES - 1; WideF32::LANES]
                );

                for _ in 1..CORRECTION_TRANSITION_SAMPLES {
                    next_output(&mut transitioned);
                }
                assert_eq!(
                    transitioned.correction_transition_remaining,
                    [0; WideF32::LANES]
                );
            }
        }
    }

    #[test]
    fn table_blep_support_boundary_always_transitions() {
        let mut oscillator = AnalogOscillator::new(48_000.0);
        oscillator.set_waveform(Waveform::Pulse);
        oscillator.set_frequency(WideF32::splat(0.124_9 * 48_000.0));
        oscillator.set_frequency(WideF32::splat(0.125_1 * 48_000.0));

        assert_eq!(
            oscillator.correction_transition_remaining,
            [CORRECTION_TRANSITION_SAMPLES; WideF32::LANES]
        );
    }

    #[test]
    fn small_frequency_update_within_one_blep_tier_stays_on_fast_path() {
        let mut oscillator = AnalogOscillator::new(48_000.0);
        oscillator.set_waveform(Waveform::Pulse);
        oscillator.set_frequency(WideF32::splat(0.10 * 48_000.0));
        oscillator.set_frequency(WideF32::splat(0.100_5 * 48_000.0));

        assert_eq!(
            oscillator.correction_transition_remaining,
            [0; WideF32::LANES]
        );
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

        fn set_frequency(&mut self, frequency: WideF32) {
            match self {
                Self::Blep(oscillator) => oscillator.set_frequency(frequency),
                Self::PolyBlep(oscillator) => oscillator.set_frequency(frequency),
            }
        }

        fn next(&mut self) -> WideF32 {
            match self {
                Self::Blep(oscillator) => next_output(oscillator),
                Self::PolyBlep(oscillator) => next_output(oscillator),
            }
        }
    }

    #[cfg(feature = "profiling")]
    use crate::profiling::RenderProfiler;

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
            oscillator.set_frequency(splat(440.0));
            oscillator.set_shape(0.35);
        }
        let mut profiler = BoundaryCounter {
            waveform_begins: 0,
            waveform_ends: 0,
        };

        for _ in 0..1_024 {
            let mut ctx = RenderContext::new(&mut profiler);
            assert_eq!(
                next_output(&mut normal).to_array(),
                profiled.next(&mut ctx).output.to_array()
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
            oscillator.set_frequency(splat(440.0));
            oscillator.set_shape(0.5);
        }
        let mut profiler = BoundaryCounter {
            waveform_begins: 0,
            waveform_ends: 0,
        };

        for _ in 0..1_024 {
            let mut ctx = RenderContext::new(&mut profiler);
            assert_eq!(
                next_output(&mut normal).to_array(),
                profiled.next(&mut ctx).output.to_array()
            );
        }
        assert_eq!(profiler.waveform_begins, 1_024);
        assert_eq!(profiler.waveform_ends, 1_024);
    }

    #[test]
    fn next_reports_phase_wraps_per_lane() {
        for (frequency, phase, expect_wrap, expect_offset) in [
            (40.0, 0.7, true, 0.75),
            (10.0, 0.95, true, 0.5),
            (0.0, 0.99, false, 0.0),
            (25.0, 0.2, false, 0.0),
        ] {
            let mut osc = AnalogOscillator::new(100.0);
            osc.set_waveform(Waveform::Saw);
            osc.set_frequency(splat(frequency));
            osc.set_phase(splat(phase));

            let mut ctx = crate::create_render_context!();
            let step = osc.next(&mut ctx);

            assert_eq!(mask_lane_active(step.wrapped, 0), expect_wrap);
            if expect_offset > 0.0 {
                assert!((lane0(step.subsample_offset) - expect_offset).abs() < 0.001);
            }
        }
    }

    #[test]
    fn sync_reset_lanes_at_preserves_subsample_offset() {
        for (reset_offset, expected_phase) in [(0.25, 0.075), (0.75, 0.025)] {
            let mut osc = AnalogOscillator::new(100.0);
            osc.set_waveform(Waveform::Saw);
            osc.set_frequency(splat(10.0));
            osc.set_phase(splat(0.4));
            next_output(&mut osc);

            osc.hard_sync_reset(mask_lane(0), splat(reset_offset));
            let phase = lane0(osc.phase);

            assert!(
                (phase - expected_phase).abs() < 0.001,
                "lane reset at {reset_offset} should end at phase {expected_phase}, got {phase}"
            );
        }
    }

    #[test]
    fn sync_reset_lanes_at_resets_only_selected_lanes() {
        let mut osc = AnalogOscillator::new(100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(splat(1.0));
        let mut phase = splat(0.25);
        if WideF32::LANES > 1 {
            phase = phase.replace_lane(1, 0.75);
        }
        osc.set_phase(phase);

        osc.hard_sync_reset(mask_lane(0), splat(1.0));
        let out = next_output(&mut osc).to_array();

        assert!(
            out[0].abs() < 0.1,
            "reset lane should render from cycle start, got {}",
            out[0]
        );
        if WideF32::LANES > 1 {
            assert!(
                (out[1] - out[0]).abs() > 0.1,
                "non-reset lane should keep its previous phase"
            );
        }
    }

    #[test]
    fn saw_phase_zero_is_corrected() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(440.0));
        let out = next_output(&mut osc);
        let arr = out.to_array();
        assert!(
            arr[0].abs() < 0.1,
            "saw at phi=0 should be corrected to ~0, got {}",
            arr[0]
        );
    }

    #[test]
    fn saw_mid_cycle_near_zero() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(1.0));
        for _ in 0..22050 {
            next_output(&mut osc);
        }
        let out = next_output(&mut osc);
        let arr = out.to_array();
        assert!(
            (arr[0] - 0.0).abs() < 0.02,
            "mid-cycle saw should be near 0, got {}",
            arr[0]
        );
    }

    #[test]
    fn triangle_peak() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Triangle);
        osc.set_frequency(WideF32::splat(1.0));
        for _ in 0..22050 {
            next_output(&mut osc);
        }
        let out = next_output(&mut osc);
        let arr = out.to_array();
        assert!(
            arr[0] > 0.95,
            "triangle should peak near 1.0 at φ=0.5, got {}",
            arr[0]
        );
    }

    #[test]
    fn triangle_zero_crossing() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Triangle);
        osc.set_frequency(WideF32::splat(1.0));
        for _ in 0..11025 {
            next_output(&mut osc);
        }
        let out = next_output(&mut osc);
        let arr = out.to_array();
        assert!(
            arr[0].abs() < 0.02,
            "triangle should cross 0 at φ=0.25, got {}",
            arr[0]
        );
    }

    #[test]
    fn polyblamp_triangle_smooths_corners_below_overlap_limit() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Triangle);
        osc.set_frequency(WideF32::splat(4410.0));

        osc.set_phase(WideF32::ZERO);
        let valley = next_output(&mut osc).to_array()[0];
        assert!(
            valley > -0.95,
            "PolyBLAMP should raise the sharp triangle valley, got {valley}"
        );

        osc.set_phase(WideF32::splat(0.5));
        let peak = next_output(&mut osc).to_array()[0];
        assert!(
            peak < 0.95,
            "PolyBLAMP should lower the sharp triangle peak, got {peak}"
        );
    }

    #[test]
    fn polyblamp_triangle_zero_frequency_lanes_stay_finite() {
        for (frequency, phase, expected) in [
            (0.0, 0.0, Some(-1.0)),
            (0.0, 0.5, Some(1.0)),
            (4410.0, 0.0, None),
            (4410.0, 0.5, None),
        ] {
            let mut osc = AnalogOscillator::new(44100.0);
            osc.set_waveform(Waveform::Triangle);
            osc.set_frequency(splat(frequency));
            osc.set_phase(splat(phase));

            let out = lane0(next_output(&mut osc));
            assert!(out.is_finite(), "triangle produced non-finite sample");
            if let Some(expected) = expected {
                assert!(
                    (out - expected).abs() < 1e-6,
                    "frequency={frequency} phase={phase} expected {expected}, got {out}"
                );
            }
        }
    }

    #[test]
    fn polyblamp_triangle_disables_correction_above_overlap_limit() {
        let mut osc = AnalogOscillator::new(100.0);
        osc.set_waveform(Waveform::Triangle);
        osc.set_frequency(WideF32::splat(30.0));

        osc.set_phase(WideF32::ZERO);
        let valley = next_output(&mut osc).to_array()[0];
        assert!(
            (valley + 1.0).abs() < 1e-6,
            "above the overlap limit the valley should stay naive, got {valley}"
        );

        osc.set_phase(WideF32::splat(0.5));
        let peak = next_output(&mut osc).to_array()[0];
        assert!(
            (peak - 1.0).abs() < 1e-6,
            "above the overlap limit the peak should stay naive, got {peak}"
        );
    }

    #[test]
    fn saw_method_selects_triangle_bandlimiting_path() {
        let mut polyblep = AnalogOscillator::new(44100.0);
        polyblep.set_waveform(Waveform::Triangle);
        polyblep.set_saw_method(SawMethod::PolyBlep);
        polyblep.set_frequency(WideF32::splat(4410.0));
        polyblep.set_phase(WideF32::ZERO);

        let mut polyblamp = AnalogOscillator::new(44100.0);
        polyblamp.set_waveform(Waveform::Triangle);
        polyblamp.set_saw_method(SawMethod::Blep);
        polyblamp.set_frequency(WideF32::splat(4410.0));
        polyblamp.set_phase(WideF32::ZERO);

        let polyblep_sample = next_output(&mut polyblep).to_array()[0];
        let polyblamp_sample = next_output(&mut polyblamp).to_array()[0];

        assert!(
            (polyblep_sample - polyblamp_sample).abs() > 0.001,
            "SawMethod should select distinct triangle paths, got {polyblep_sample} and {polyblamp_sample}"
        );
    }

    #[test]
    fn polyblep_integrated_triangle_stays_finite_and_bounded() {
        for frequency in [110.0, 440.0, 1760.0, 7040.0] {
            let mut osc = AnalogOscillator::new(44100.0);
            osc.set_waveform(Waveform::Triangle);
            osc.set_saw_method(SawMethod::PolyBlep);
            osc.set_frequency(splat(frequency));

            let mut max_abs = 0.0f32;
            for _ in 0..4096 {
                let sample = lane0(next_output(&mut osc));
                assert!(sample.is_finite(), "triangle produced non-finite sample");
                max_abs = max_abs.max(sample.abs());
            }

            assert!(
                max_abs <= 1.25,
                "triangle output at {frequency}Hz exceeded bounds: {max_abs}"
            );
            assert!(
                max_abs > 0.1,
                "triangle output at {frequency}Hz unexpectedly collapsed: {max_abs}"
            );
        }
    }

    #[test]
    fn polyblamp_triangle_high_frequency_stays_finite_and_bounded() {
        for frequency in [8_000.0, 9_000.0, 10_000.0, 11_000.0] {
            let mut osc = AnalogOscillator::new(44100.0);
            osc.set_waveform(Waveform::Triangle);
            osc.set_frequency(splat(frequency));

            let mut max_abs = 0.0f32;
            for _ in 0..512 {
                let sample = lane0(next_output(&mut osc));
                assert!(sample.is_finite(), "triangle produced non-finite sample");
                max_abs = max_abs.max(sample.abs());
            }
            assert!(
                max_abs <= 1.25,
                "triangle output at {frequency}Hz exceeded bounds: {max_abs}"
            );
            assert!(
                max_abs > 0.1,
                "triangle output at {frequency}Hz unexpectedly collapsed: {max_abs}"
            );
        }
    }

    #[test]
    fn pulse_50_percent_is_square() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Pulse);
        osc.set_shape(0.0);
        osc.set_frequency(WideF32::splat(440.0));
        let out1 = next_output(&mut osc);
        let arr1 = out1.to_array();
        assert!(
            arr1[0].abs() < 0.1,
            "pulse at phi=0 should be ~0, got {}",
            arr1[0]
        );

        for _ in 0..(44100 / 440 / 8) as usize {
            next_output(&mut osc);
        }
        let out_low = next_output(&mut osc);
        let arr_low = out_low.to_array();
        assert!(
            arr_low[0] < -0.9,
            "pulse low should be near -1, got {}",
            arr_low[0]
        );
    }

    #[test]
    fn waveshape_modulation_on_saw() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_shape(0.0);
        osc.set_frequency(WideF32::splat(440.0));

        let unshaped = next_output(&mut osc).to_array()[0];

        osc.set_shape(0.5);
        osc.set_phase(WideF32::ZERO);
        osc.set_frequency(WideF32::splat(440.0));
        let shaped = next_output(&mut osc).to_array()[0];

        assert!(
            (unshaped - shaped).abs() > 0.001,
            "shape 0.5 should change saw output, got {unshaped} vs {shaped}"
        );
    }

    #[test]
    fn uniform_input_keeps_all_lanes_equal() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(splat(440.0));

        let out = next_output(&mut osc).to_array();
        for lane in 1..WideF32::LANES {
            assert!(
                (out[0] - out[lane]).abs() < 1e-6,
                "lane {lane} diverged from lane 0 with uniform input"
            );
        }
    }

    #[test]
    fn phase_wraps_correctly() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(440.0));

        for _ in 0..100000 {
            let out = next_output(&mut osc);
            let arr = out.to_array();
            for &v in &arr {
                assert!(v >= -1.02 && v <= 1.02, "output out of range: {v}");
            }
        }
    }

    #[test]
    fn phase_input_wraps_without_remainder_edge_cases() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(splat(440.0));

        for &(phase_a, phase_b) in &[(-0.25f32, 0.75), (0.25, 1.25)] {
            osc.set_phase(splat(phase_a));
            let out_a = lane0(next_output(&mut osc));
            osc.set_phase(splat(phase_b));
            let out_b = lane0(next_output(&mut osc));
            assert!(
                (out_a - out_b).abs() < 1e-6,
                "phases {phase_a} and {phase_b} should wrap to the same output"
            );
        }
    }

    #[test]
    fn invalid_frequency_does_not_poison_phase() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);

        for frequency in [440.0, f32::NAN, f32::INFINITY, -1.0] {
            osc.set_frequency(splat(frequency));
            for _ in 0..1024 {
                let sample = lane0(next_output(&mut osc));
                assert!(sample.is_finite(), "oscillator produced non-finite sample");
            }
        }
    }

    #[test]
    fn oscillator_enabled_gain_can_mute_without_stopping_phase() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(440.0));
        osc.set_enabled(false);

        let muted = next_output(&mut osc).to_array()[0];
        assert_eq!(muted, 0.0);

        osc.set_enabled(true);
        let audible = next_output(&mut osc).to_array()[0];
        assert!(
            audible.abs() > 0.001,
            "oscillator should keep advancing while muted and become audible when re-enabled"
        );
    }

    #[test]
    fn polyblep_saw_full_cycle() {
        let sr = 44100.0;
        let freq = 55.0;
        let period_samples = (sr as f64 / freq as f64).round() as usize;

        let mut osc = AnalogOscillator::new(sr);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(freq));

        let mut min_val = f32::MAX;
        let mut max_val = f32::MIN;

        for _ in 0..period_samples + 10 {
            let val = next_output(&mut osc).to_array()[0];
            min_val = min_val.min(val);
            max_val = max_val.max(val);
        }

        assert!(min_val < -0.95, "min too high: {min_val}");
        assert!(max_val > 0.95, "max too low: {max_val}");
    }

    #[test]
    fn polyblep_reduces_discontinuity() {
        let sr = 44100.0;
        let freq = 440.0;

        let mut osc = AnalogOscillator::new(sr);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(freq));

        let mut prev = 0.0;
        let mut max_jump = 0.0f32;
        for _ in 0..300 {
            let val = next_output(&mut osc).to_array()[0];
            max_jump = max_jump.max((val - prev).abs());
            prev = val;
        }

        assert!(max_jump < 1.75, "max jump {max_jump} should be < 1.75");
    }

    #[test]
    fn polyblep_saw_smooth_transition() {
        let sr = 44100.0;
        let freq = 440.0;
        let dt = freq / sr;

        let mut osc = AnalogOscillator::new(sr);
        osc.set_waveform(Waveform::Saw);
        osc.set_frequency(WideF32::splat(freq));

        let mut samples = [0.0f32; 300];
        for sample in &mut samples {
            *sample = next_output(&mut osc).to_array()[0];
        }

        let period = (1.0 / dt) as usize;
        let mut min_after_wrap = f32::MAX;
        let mut spike_count = 0;

        for i in period..samples.len() {
            let jump = samples[i] - samples[i - 1];
            if jump.abs() > 1.75 {
                spike_count += 1;
            }
            min_after_wrap = min_after_wrap.min(samples[i]);
        }

        assert_eq!(spike_count, 0, "found {spike_count} large jumps > 1.75");
        assert!(
            min_after_wrap > -1.1,
            "output went below -1.1: {min_after_wrap}"
        );
    }

    #[test]
    fn table_blep_left_edge_uses_falling_correction() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Saw);
        osc.set_saw_method(SawMethod::Blep);
        osc.set_frequency(WideF32::splat(440.0));

        osc.set_phase(WideF32::splat(0.999));
        let out = next_output(&mut osc).to_array()[0];
        assert!(
            out < 0.6,
            "left-edge BLEP should pull down before wrap, got {out}"
        );
    }

    #[test]
    fn polyblep_pulse_smooth_edges() {
        let sr = 44100.0;
        let freq = 55.0;

        let mut osc = AnalogOscillator::new(sr);
        osc.set_waveform(Waveform::Pulse);
        osc.set_shape(0.0);
        osc.set_frequency(WideF32::splat(freq));

        let mut prev = next_output(&mut osc).to_array()[0];
        let mut max_jump = 0.0f32;
        for _ in 0..3000 {
            let val = next_output(&mut osc).to_array()[0];
            max_jump = max_jump.max((val - prev).abs());
            prev = val;
        }

        assert!(
            max_jump < 1.95,
            "pulse max jump {max_jump} — should be below naive 2.0"
        );
    }

    #[test]
    fn polyblep_pulse_50_percent() {
        let sr = 44100.0;
        let freq = 440.0;
        let dt = freq / sr;

        let mut osc = AnalogOscillator::new(sr);
        osc.set_waveform(Waveform::Pulse);
        osc.set_shape(0.0);
        osc.set_frequency(WideF32::splat(freq));

        let period = (1.0 / dt) as usize;
        let mut samples = [0.0f32; 128];
        for sample in &mut samples[..period + 10] {
            *sample = next_output(&mut osc).to_array()[0];
        }

        let mut high_count = 0;
        for &s in &samples[..period] {
            if s > 0.0 {
                high_count += 1;
            }
        }

        let ratio = high_count as f32 / period as f32;
        assert!(
            (ratio - 0.5).abs() < 0.05,
            "duty cycle {ratio:.3} should be ~0.5"
        );
    }

    #[test]
    fn pulse_shape_mod_controls_pwm_duty_without_unbounded_levels() {
        fn measure(shape: f32) -> (f32, f32) {
            let sr = 44100.0f32;
            let freq = 55.0f32;
            let period = (sr / freq).round() as usize;
            let mut osc = AnalogOscillator::new(sr);
            osc.set_waveform(Waveform::Pulse);
            osc.set_shape(shape);
            osc.set_frequency(WideF32::splat(freq));

            let mut positive = 0usize;
            let mut peak = 0.0f32;
            for _ in 0..period {
                let sample = next_output(&mut osc).to_array()[0];
                if sample > 0.0 {
                    positive += 1;
                }
                peak = peak.max(sample.abs());
            }

            (positive as f32 / period as f32, peak)
        }

        let (square_ratio, square_peak) = measure(0.0);
        let (wide_ratio, wide_peak) = measure(0.51);

        assert!(
            (square_ratio - pulse_width_from_shape(0.0)).abs() < 0.05,
            "shape 0.0 should produce a ~50% square duty, got {square_ratio:.3}"
        );
        assert!(
            (wide_ratio - pulse_width_from_shape(0.51)).abs() < 0.05,
            "shape 0.51 should track its mapped pulse width, got {wide_ratio:.3}"
        );
        assert!(
            wide_ratio > square_ratio + 0.1,
            "higher shape should widen the positive duty, square={square_ratio:.3} wide={wide_ratio:.3}"
        );
        assert!(
            square_peak < 1.25 && wide_peak < 1.25,
            "PWM should stay bounded, peaks square={square_peak:.3} wide={wide_peak:.3}"
        );
    }

    #[test]
    fn pulse_shape_maps_to_width_without_phase_blend() {
        let mut osc = AnalogOscillator::new(44100.0);
        osc.set_waveform(Waveform::Pulse);
        osc.set_shape(1.0);
        osc.set_phase(WideF32::splat(0.123));
        osc.set_frequency(WideF32::splat(220.0));

        let sr = 44100.0f32;
        let freq = 220.0f32;
        let period = (sr / freq).round() as usize;

        let mut positive = 0usize;
        let mut peak = 0.0f32;
        for _ in 0..period {
            let sample = next_output(&mut osc).to_array()[0];
            if sample > 0.0 {
                positive += 1;
            }
            peak = peak.max(sample.abs());
        }

        let duty = positive as f32 / period as f32;
        assert!(
            (duty - pulse_width_from_shape(1.0)).abs() < 0.05,
            "Pulse shape should map straight to pulse width, expected ~{:.3} got {duty:.3}",
            pulse_width_from_shape(1.0)
        );
        assert!(
            peak < 1.25,
            "Pulse shape mapping should add no phase-blend overshoot, peak {peak:.3}"
        );
    }
}
