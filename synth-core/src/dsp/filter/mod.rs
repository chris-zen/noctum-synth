//! Heapless runtime-selectable low-pass filter models.

#[cfg(feature = "filter-cascaded-svf")]
mod cascaded_tpt_svf;
mod coefficient_math;
#[cfg(feature = "filter-distributed-newton")]
mod distributed_newton_tpt;
#[cfg(feature = "filter-gain-limited")]
mod gain_limited_tpt;
#[cfg(feature = "filter-huovilainen")]
mod huovilainen_ladder;
#[cfg(any(test, feature = "fast-math"))]
mod prewarp_table;
mod resonance_math;
#[cfg(feature = "filter-scalar-feedback")]
mod scalar_feedback_tpt;

use crate::math::WideF32;

#[cfg(feature = "filter-cascaded-svf")]
use cascaded_tpt_svf::CascadedTptSvf;
#[cfg(feature = "filter-distributed-newton")]
use distributed_newton_tpt::DistributedNewtonTpt;
#[cfg(feature = "filter-gain-limited")]
use gain_limited_tpt::GainLimitedTpt;
#[cfg(feature = "filter-huovilainen")]
use huovilainen_ladder::HuovilainenLadder;
#[cfg(feature = "filter-scalar-feedback")]
use scalar_feedback_tpt::ScalarFeedbackTpt;

/// Lowest cutoff used when applying filter coefficients after modulation.
pub(crate) const MIN_CUTOFF_HZ: f32 = 20.0;
/// Lowest base cutoff stored on the voice (Prophet program raw 0 ≈ 1 Hz).
/// Key-track / envelope modulation is applied to this base, then the result is
/// clamped to [`MIN_CUTOFF_HZ`] for coefficient calculation.
pub(crate) const MIN_BASE_CUTOFF_HZ: f32 = 1.0;
/// Highest cutoff accepted by every filter model.
pub(crate) const MAX_CUTOFF_HZ: f32 = 18_000.0;
/// Full filter-envelope modulation depth in semitones (Prophet Env Amount ±127 ticks).
const ENV_DEPTH_SEMITONES: f32 = 127.0;
/// Full audio-rate filter modulation depth in semitones (Appendix F: one octave).
const AUDIO_MOD_DEPTH_SEMITONES: f32 = 12.0;
/// MIDI note that produces zero semitones of filter keyboard tracking.
/// Prophet Key Amount 64 is 1:1; with cutoff raw 0, C4 tracks to C2 (−2 octaves).
const KEY_TRACK_REFERENCE_NOTE: f32 = -12.0;
/// Prophet Key Amount max (raw 127) relative to unity at raw 64.
const MAX_KEY_TRACK: f32 = 127.0 / 64.0;

/// Public resonance value where 4-pole nonlinear self-oscillation begins.
pub const SELF_OSC_RESONANCE_START: f32 = 0.71;
/// Baseline self-oscillation pitch trim, in cents.
pub const SELF_OSC_PITCH_TUNING_CENTS: f32 = 133.0;

/// Runtime-selectable filter model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum FilterType {
    /// Exact unity-gain path for raw oscillator audition.
    PassThrough,
    /// Existing Rev2-inspired distributed-Newton TPT model.
    DistributedNewtonTpt,
    /// Rev2-inspired scalar-feedback TPT candidate (introduced in Phase 1).
    ScalarFeedbackTpt,
    /// Analytic gain-limited TPT candidate (introduced in Phase 2).
    GainLimitedTpt,
    /// Huovilainen nonlinear ladder reference (introduced in Phase 3).
    HuovilainenLadder,
    /// Cascaded trapezoidal SVF reference (introduced in Phase 4).
    CascadedTptSvf,
}

impl Default for FilterType {
    fn default() -> Self {
        #[cfg(any(
            feature = "filter-all",
            all(not(feature = "filter-all"), feature = "filter-distributed-newton"),
        ))]
        {
            Self::DistributedNewtonTpt
        }
        #[cfg(all(not(feature = "filter-all"), feature = "filter-scalar-feedback"))]
        {
            Self::ScalarFeedbackTpt
        }
        #[cfg(all(not(feature = "filter-all"), feature = "filter-gain-limited"))]
        {
            Self::GainLimitedTpt
        }
        #[cfg(all(not(feature = "filter-all"), feature = "filter-huovilainen"))]
        {
            Self::HuovilainenLadder
        }
        #[cfg(all(not(feature = "filter-all"), feature = "filter-cascaded-svf"))]
        {
            Self::CascadedTptSvf
        }
    }
}

impl FilterType {
    pub const ALL: [Self; 6] = [
        Self::PassThrough,
        Self::DistributedNewtonTpt,
        Self::ScalarFeedbackTpt,
        Self::GainLimitedTpt,
        Self::HuovilainenLadder,
        Self::CascadedTptSvf,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::PassThrough => "Pass Through (Raw)",
            Self::DistributedNewtonTpt => "Distributed Newton TPT",
            Self::ScalarFeedbackTpt => "Scalar Feedback TPT",
            Self::GainLimitedTpt => "Gain-Limited TPT",
            Self::HuovilainenLadder => "Huovilainen Ladder",
            Self::CascadedTptSvf => "Cascaded TPT SVF",
        }
    }

    /// Whether this model has its own implementation in the current phase.
    pub const fn is_implemented(self) -> bool {
        if matches!(self, Self::PassThrough) {
            return cfg!(feature = "filter-pass-through");
        }
        matches!(
            self,
            Self::DistributedNewtonTpt
                | Self::ScalarFeedbackTpt
                | Self::GainLimitedTpt
                | Self::HuovilainenLadder
                | Self::CascadedTptSvf
        )
    }
}

/// Runtime quality setting for nonlinear filter oversampling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum FilterOversampling {
    Off,
    #[default]
    Auto,
    X2,
    X4,
}

impl FilterOversampling {
    pub fn factor(self, sample_rate: f32) -> usize {
        match self {
            Self::Off => 1,
            Self::Auto if sample_rate >= 176_400.0 => 1,
            Self::Auto if sample_rate >= 88_200.0 => 2,
            Self::Auto => 4,
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }
}

/// Common controls and modulation prepared for one model frame.
#[derive(Clone, Copy)]
pub(crate) struct FilterFrame {
    pub input: WideF32,
    pub cutoff_hz: f32,
    pub cutoff_mod_semitones: WideF32,
    pub cutoff_mod_uniform_semitones: Option<f32>,
    pub resonance_control: WideF32,
    pub shaped_resonance: WideF32,
    pub poles: u8,
    pub oversampling: FilterOversampling,
    pub sample_rate: f32,
    pub static_cutoff: bool,
    pub self_oscillation_color_enabled: bool,
}

pub(crate) trait FilterAlgorithm {
    fn reset(&mut self);
    fn reset_lane(&mut self, lane: usize);
    fn invalidate_coefficients(&mut self);
    fn clear_oversampling_state(&mut self);
    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32);
    fn self_osc_pitch_tuning_cents(&self) -> f32;
    fn process(&mut self, frame: FilterFrame) -> WideF32;
}

#[cfg(feature = "filter-all")]
enum FilterAlgorithmState {
    DistributedNewtonTpt(DistributedNewtonTpt),
    ScalarFeedbackTpt(ScalarFeedbackTpt),
    GainLimitedTpt(GainLimitedTpt),
    HuovilainenLadder(HuovilainenLadder),
    CascadedTptSvf(CascadedTptSvf),
}

#[cfg(feature = "filter-all")]
macro_rules! with_filter_algorithm_mut {
    ($state:expr, $algorithm:ident => $body:expr) => {
        match $state {
            FilterAlgorithmState::DistributedNewtonTpt($algorithm) => $body,
            FilterAlgorithmState::ScalarFeedbackTpt($algorithm) => $body,
            FilterAlgorithmState::GainLimitedTpt($algorithm) => $body,
            FilterAlgorithmState::HuovilainenLadder($algorithm) => $body,
            FilterAlgorithmState::CascadedTptSvf($algorithm) => $body,
        }
    };
}

#[cfg(feature = "filter-all")]
macro_rules! with_filter_algorithm {
    ($state:expr, $algorithm:ident => $body:expr) => {
        match $state {
            FilterAlgorithmState::DistributedNewtonTpt($algorithm) => $body,
            FilterAlgorithmState::ScalarFeedbackTpt($algorithm) => $body,
            FilterAlgorithmState::GainLimitedTpt($algorithm) => $body,
            FilterAlgorithmState::HuovilainenLadder($algorithm) => $body,
            FilterAlgorithmState::CascadedTptSvf($algorithm) => $body,
        }
    };
}

#[cfg(feature = "filter-all")]
impl FilterAlgorithmState {
    fn new(filter_type: FilterType) -> Self {
        match filter_type {
            FilterType::PassThrough => Self::DistributedNewtonTpt(DistributedNewtonTpt::default()),
            FilterType::ScalarFeedbackTpt => Self::ScalarFeedbackTpt(ScalarFeedbackTpt::default()),
            FilterType::GainLimitedTpt => Self::GainLimitedTpt(GainLimitedTpt::default()),
            FilterType::HuovilainenLadder => Self::HuovilainenLadder(HuovilainenLadder::default()),
            FilterType::CascadedTptSvf => Self::CascadedTptSvf(CascadedTptSvf::default()),
            FilterType::DistributedNewtonTpt => {
                Self::DistributedNewtonTpt(DistributedNewtonTpt::default())
            }
        }
    }

    fn reset(&mut self) {
        with_filter_algorithm_mut!(self, algorithm => algorithm.reset())
    }

    fn reset_lane(&mut self, lane: usize) {
        with_filter_algorithm_mut!(self, algorithm => algorithm.reset_lane(lane))
    }

    fn invalidate_coefficients(&mut self) {
        with_filter_algorithm_mut!(self, algorithm => algorithm.invalidate_coefficients())
    }

    fn clear_oversampling_state(&mut self) {
        with_filter_algorithm_mut!(self, algorithm => algorithm.clear_oversampling_state())
    }

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        with_filter_algorithm_mut!(self, algorithm => algorithm.set_self_osc_pitch_tuning_cents(cents))
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        with_filter_algorithm!(self, algorithm => algorithm.self_osc_pitch_tuning_cents())
    }

    #[inline(always)]
    fn process(&mut self, frame: FilterFrame) -> WideF32 {
        with_filter_algorithm_mut!(self, algorithm => algorithm.process(frame))
    }
}

#[cfg(not(any(
    feature = "filter-all",
    feature = "filter-distributed-newton",
    feature = "filter-scalar-feedback",
    feature = "filter-gain-limited",
    feature = "filter-huovilainen",
    feature = "filter-cascaded-svf",
)))]
compile_error!(
    "enable exactly one filter feature: filter-distributed-newton, \
     filter-scalar-feedback, filter-gain-limited, filter-huovilainen, \
     or filter-cascaded-svf (or use filter-all)"
);

#[cfg(all(not(feature = "filter-all"), feature = "filter-distributed-newton"))]
struct FilterAlgorithmState(DistributedNewtonTpt);
#[cfg(all(not(feature = "filter-all"), feature = "filter-scalar-feedback"))]
struct FilterAlgorithmState(ScalarFeedbackTpt);
#[cfg(all(not(feature = "filter-all"), feature = "filter-gain-limited"))]
struct FilterAlgorithmState(GainLimitedTpt);
#[cfg(all(not(feature = "filter-all"), feature = "filter-huovilainen"))]
struct FilterAlgorithmState(HuovilainenLadder);
#[cfg(all(not(feature = "filter-all"), feature = "filter-cascaded-svf"))]
struct FilterAlgorithmState(CascadedTptSvf);

#[cfg(not(feature = "filter-all"))]
impl FilterAlgorithmState {
    fn new(filter_type: FilterType) -> Self {
        match filter_type {
            #[cfg(feature = "filter-distributed-newton")]
            FilterType::DistributedNewtonTpt => Self(DistributedNewtonTpt::default()),
            #[cfg(feature = "filter-scalar-feedback")]
            FilterType::ScalarFeedbackTpt => Self(ScalarFeedbackTpt::default()),
            #[cfg(feature = "filter-gain-limited")]
            FilterType::GainLimitedTpt => Self(GainLimitedTpt::default()),
            #[cfg(feature = "filter-huovilainen")]
            FilterType::HuovilainenLadder => Self(HuovilainenLadder::default()),
            #[cfg(feature = "filter-cascaded-svf")]
            FilterType::CascadedTptSvf => Self(CascadedTptSvf::default()),
            _ => panic!("filter model not enabled in this build"),
        }
    }

    fn reset(&mut self) {
        self.0.reset();
    }

    fn reset_lane(&mut self, lane: usize) {
        self.0.reset_lane(lane);
    }

    fn invalidate_coefficients(&mut self) {
        self.0.invalidate_coefficients();
    }

    fn clear_oversampling_state(&mut self) {
        self.0.clear_oversampling_state();
    }

    fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.0.set_self_osc_pitch_tuning_cents(cents);
    }

    fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.0.self_osc_pitch_tuning_cents()
    }

    #[inline(always)]
    fn process(&mut self, frame: FilterFrame) -> WideF32 {
        self.0.process(frame)
    }
}

/// Four-lane heapless wrapper around the selected filter algorithm.
pub struct Filter {
    filter_type: FilterType,
    cutoff: f32,
    resonance: f32,
    poles: u8,
    key_track: f32,
    env_amount: f32,
    env_velocity_amount: f32,
    audio_mod: f32,
    shaped_resonance: f32,
    oversampling: FilterOversampling,
    self_oscillation_color_enabled: bool,
    algorithm: FilterAlgorithmState,
}

impl Default for Filter {
    fn default() -> Self {
        Self::new(FilterType::default())
    }
}

impl Filter {
    pub fn new(filter_type: FilterType) -> Self {
        let filter_type = available_filter_type(filter_type);
        Self {
            filter_type,
            cutoff: MAX_CUTOFF_HZ,
            resonance: 0.0,
            poles: 4,
            key_track: 0.0,
            env_amount: 0.0,
            env_velocity_amount: 0.0,
            audio_mod: 0.0,
            shaped_resonance: 0.0,
            oversampling: FilterOversampling::Auto,
            self_oscillation_color_enabled: true,
            algorithm: FilterAlgorithmState::new(filter_type),
        }
    }

    pub const fn filter_type(&self) -> FilterType {
        self.filter_type
    }

    /// Selects a fresh model state while retaining all common controls.
    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        let filter_type = available_filter_type(filter_type);
        if self.filter_type == filter_type {
            return;
        }
        self.filter_type = filter_type;
        self.algorithm = FilterAlgorithmState::new(filter_type);
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = cutoff.clamp(MIN_BASE_CUTOFF_HZ, MAX_CUTOFF_HZ);
        self.algorithm.invalidate_coefficients();
    }

    pub const fn cutoff(&self) -> f32 {
        self.cutoff
    }

    pub fn set_resonance(&mut self, resonance: f32) {
        self.resonance = resonance.clamp(0.0, 1.0);
        self.shaped_resonance = shape_resonance_scalar(self.resonance);
    }

    pub const fn resonance(&self) -> f32 {
        self.resonance
    }

    pub fn set_poles(&mut self, poles: u8) {
        self.poles = if poles <= 2 { 2 } else { 4 };
    }

    pub fn set_key_track(&mut self, key_track: f32) {
        self.key_track = key_track.clamp(0.0, MAX_KEY_TRACK);
    }

    pub fn set_env_amount(&mut self, env_amount: f32) {
        self.env_amount = env_amount.clamp(-1.0, 1.0);
    }

    /// Sets the Prophet filter-envelope velocity contribution.
    ///
    /// Velocity Amount adds an independent, positive envelope-shaped cutoff
    /// contribution; it does not multiply Env Amount. This follows the cutoff
    /// control sum described in the
    /// [Prophet Rev2 User's Guide](https://www.sequential.com/wp-content/uploads/2021/02/Prophet-Rev2-Users-Guide-1.2.4.pdf#page=31)
    /// and the measured
    /// [Rev2 cutoff formula](https://forum.sequential.com/index.php?topic=4444.0).
    pub fn set_env_velocity_amount(&mut self, env_velocity_amount: f32) {
        self.env_velocity_amount = env_velocity_amount.clamp(0.0, 1.0);
    }

    pub fn set_audio_mod(&mut self, audio_mod: f32) {
        self.audio_mod = audio_mod.clamp(0.0, 1.0);
    }

    pub fn set_oversampling(&mut self, oversampling: FilterOversampling) {
        if self.oversampling != oversampling {
            self.oversampling = oversampling;
            self.algorithm.clear_oversampling_state();
        }
    }

    pub const fn oversampling(&self) -> FilterOversampling {
        self.oversampling
    }

    /// Restricts post-filter harmonic color to autonomous self-oscillation.
    /// Fundamental resonance gain is independent of this policy flag.
    pub(crate) fn set_self_oscillation_color_enabled(&mut self, enabled: bool) {
        self.self_oscillation_color_enabled = enabled;
    }

    pub fn set_self_osc_pitch_tuning_cents(&mut self, cents: f32) {
        self.algorithm.set_self_osc_pitch_tuning_cents(cents);
    }

    pub fn self_osc_pitch_tuning_cents(&self) -> f32 {
        self.algorithm.self_osc_pitch_tuning_cents()
    }

    pub fn reset(&mut self) {
        self.algorithm.reset();
    }

    pub fn reset_lane(&mut self, lane: usize) {
        self.algorithm.reset_lane(lane);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        input: WideF32,
        note: WideF32,
        filter_env: WideF32,
        velocity: WideF32,
        osc1_audio: WideF32,
        cutoff_mod_semitones: WideF32,
        resonance_mod: WideF32,
        audio_mod: WideF32,
        sample_rate: f32,
    ) -> WideF32 {
        self.process_inner(
            input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            None,
            resonance_mod,
            audio_mod,
            sample_rate,
            self.env_amount,
            self.env_velocity_amount,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn process_prepared_env(
        &mut self,
        input: WideF32,
        note: WideF32,
        filter_env: WideF32,
        velocity: WideF32,
        osc1_audio: WideF32,
        cutoff_mod_semitones: WideF32,
        cutoff_mod_uniform_semitones: Option<f32>,
        resonance_mod: WideF32,
        audio_mod: WideF32,
        sample_rate: f32,
        env_amount: f32,
        env_velocity_amount: f32,
    ) -> WideF32 {
        self.process_inner(
            input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            cutoff_mod_semitones,
            cutoff_mod_uniform_semitones,
            resonance_mod,
            audio_mod,
            sample_rate,
            env_amount,
            env_velocity_amount,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn process_inner(
        &mut self,
        input: WideF32,
        note: WideF32,
        filter_env: WideF32,
        velocity: WideF32,
        osc1_audio: WideF32,
        cutoff_mod_semitones: WideF32,
        cutoff_mod_uniform_semitones: Option<f32>,
        resonance_mod: WideF32,
        audio_mod: WideF32,
        sample_rate: f32,
        env_amount: f32,
        env_velocity_amount: f32,
    ) -> WideF32 {
        #[cfg(feature = "filter-pass-through")]
        if self.filter_type == FilterType::PassThrough {
            return input;
        }

        let resonance_control = (WideF32::splat(self.resonance) + resonance_mod)
            .clamp(WideF32::ZERO, WideF32::splat(1.0));
        let shaped_resonance = if resonance_mod == WideF32::ZERO {
            WideF32::splat(self.shaped_resonance)
        } else {
            shape_resonance_control(resonance_control)
        };
        let key_semitones =
            (note - WideF32::splat(KEY_TRACK_REFERENCE_NOTE)) * WideF32::splat(self.key_track);
        let effective_env_amount = WideF32::splat(env_amount)
            + velocity.clamp(WideF32::ZERO, WideF32::splat(1.0))
                * WideF32::splat(env_velocity_amount);
        let env_semitones = filter_env.clamp(WideF32::ZERO, WideF32::splat(1.0))
            * effective_env_amount
            * WideF32::splat(ENV_DEPTH_SEMITONES);
        let audio_mod_amount =
            (WideF32::splat(self.audio_mod) + audio_mod).clamp(WideF32::ZERO, WideF32::splat(1.0));
        let audio_semitones = osc1_audio.clamp(WideF32::splat(-1.0), WideF32::splat(1.0))
            * audio_mod_amount
            * WideF32::splat(AUDIO_MOD_DEPTH_SEMITONES);
        let cutoff_mod_semitones =
            key_semitones + env_semitones + audio_semitones + cutoff_mod_semitones;
        let cutoff_mod_uniform_semitones = cutoff_mod_uniform_semitones.filter(|_| {
            self.key_track == 0.0
                && env_amount == 0.0
                && env_velocity_amount == 0.0
                && self.audio_mod == 0.0
                && all_lanes_near_zero(audio_mod)
        });
        let static_cutoff = self.key_track == 0.0
            && env_amount == 0.0
            && env_velocity_amount == 0.0
            && self.audio_mod == 0.0
            && all_lanes_near_zero(cutoff_mod_semitones)
            && all_lanes_near_zero(resonance_mod)
            && all_lanes_near_zero(audio_mod);

        self.algorithm.process(FilterFrame {
            input,
            cutoff_hz: self.cutoff,
            cutoff_mod_semitones,
            cutoff_mod_uniform_semitones,
            resonance_control,
            shaped_resonance,
            poles: self.poles,
            oversampling: self.oversampling,
            sample_rate,
            static_cutoff,
            self_oscillation_color_enabled: self.self_oscillation_color_enabled,
        })
    }
}

#[inline]
const fn available_filter_type(filter_type: FilterType) -> FilterType {
    if matches!(filter_type, FilterType::PassThrough) && !cfg!(feature = "filter-pass-through") {
        FilterType::DistributedNewtonTpt
    } else {
        filter_type
    }
}

/// Compatibility alias for existing callers.
pub type LadderFilter = Filter;

fn shape_resonance_control(value: WideF32) -> WideF32 {
    let mut values = value.to_array();
    for value in &mut values {
        *value = shape_resonance_scalar(*value);
    }
    WideF32::new(values)
}

fn shape_resonance_scalar(value: f32) -> f32 {
    resonance_math::shape(value.clamp(0.0, 1.0))
}

fn all_lanes_near_zero(value: WideF32) -> bool {
    value.abs().simd_lt(WideF32::splat(f32::EPSILON)).all()
}

#[cfg(all(test, feature = "filter-pass-through"))]
mod pass_through_tests {
    use super::{Filter, FilterOversampling, FilterType};
    use crate::math::WideF32;

    #[test]
    fn pass_through_is_bit_exact_under_ignored_controls_and_modulation() {
        let mut filter = Filter::new(FilterType::PassThrough);
        filter.set_cutoff(317.0);
        filter.set_resonance(1.0);
        filter.set_poles(2);
        filter.set_key_track(1.0);
        filter.set_env_amount(-1.0);
        filter.set_env_velocity_amount(1.0);
        filter.set_audio_mod(1.0);
        filter.set_oversampling(FilterOversampling::X4);
        filter.reset();
        filter.reset_lane(0);

        let mut random = 0x6d2b_79f5_u32;
        for frame in 0..1_024 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            let input = WideF32::new(core::array::from_fn(|lane| {
                if frame == 0 && lane == 0 {
                    -0.0
                } else if frame == 0 && lane == 1 {
                    f32::MAX
                } else {
                    let bits = random.wrapping_add((lane as u32).wrapping_mul(0x9e37_79b9));
                    (bits as i32 as f32) * (0.75 / i32::MAX as f32)
                }
            }));
            let output = filter.process(
                input,
                WideF32::new(core::array::from_fn(|lane| 12.0 + lane as f32 * 31.0)),
                WideF32::new(core::array::from_fn(|lane| {
                    lane as f32 / WideF32::LANES as f32
                })),
                WideF32::splat(0.73),
                WideF32::splat(-0.91),
                WideF32::splat(47.0),
                WideF32::splat(-0.31),
                WideF32::splat(0.88),
                48_000.0,
            );
            assert_eq!(
                output.to_array().map(f32::to_bits),
                input.to_array().map(f32::to_bits)
            );
        }
    }

    #[test]
    fn pass_through_impulse_has_zero_latency_and_no_tail() {
        let mut filter = Filter::new(FilterType::PassThrough);
        for frame in 0..64 {
            let input = WideF32::splat(if frame == 0 { 1.0 } else { 0.0 });
            let output = filter.process(
                input,
                WideF32::splat(60.0),
                WideF32::ZERO,
                WideF32::splat(1.0),
                WideF32::ZERO,
                WideF32::ZERO,
                WideF32::ZERO,
                WideF32::ZERO,
                48_000.0,
            );
            assert_eq!(output.to_array(), input.to_array());
        }
    }
}

#[cfg(all(test, not(feature = "filter-all")))]
mod embedded_tests {
    use super::{FilterType, LadderFilter};
    use crate::math::WideF32;

    #[test]
    fn gain_limited_filter_processes_silence() {
        let mut filter = LadderFilter::default();
        let out = filter.process(
            WideF32::ZERO,
            WideF32::splat(60.0),
            WideF32::ZERO,
            WideF32::splat(1.0),
            WideF32::ZERO,
            WideF32::ZERO,
            WideF32::ZERO,
            WideF32::ZERO,
            44_100.0,
        );
        assert!(out.to_array().iter().all(|sample| sample.is_finite()));
    }

    #[cfg(not(feature = "filter-pass-through"))]
    #[test]
    fn unavailable_pass_through_falls_back_to_the_normal_default() {
        let filter = LadderFilter::new(FilterType::PassThrough);
        assert_eq!(filter.filter_type(), FilterType::DistributedNewtonTpt);
    }
}

#[cfg(all(test, feature = "filter-all"))]
mod tests {
    use super::{
        ENV_DEPTH_SEMITONES, Filter, FilterOversampling, FilterType, LadderFilter,
        MIN_BASE_CUTOFF_HZ, SELF_OSC_PITCH_TUNING_CENTS, SELF_OSC_RESONANCE_START,
    };
    use crate::math::WideF32;
    use crate::tuning::midi_to_hz;

    extern crate std;
    use super::{
        CascadedTptSvf, DistributedNewtonTpt, GainLimitedTpt, HuovilainenLadder, ScalarFeedbackTpt,
    };
    use std::vec::Vec;

    fn process(
        filter: &mut LadderFilter,
        input: WideF32,
        note: WideF32,
        sample_rate: f32,
    ) -> WideF32 {
        process_modulated(
            filter,
            input,
            note,
            WideF32::ZERO,
            WideF32::splat(1.0),
            WideF32::ZERO,
            sample_rate,
        )
    }

    #[test]
    fn switching_filter_type_resets_state_and_retains_common_controls() {
        let mut switched = Filter::new(FilterType::DistributedNewtonTpt);
        switched.set_cutoff(930.0);
        switched.set_resonance(0.64);
        switched.set_poles(2);
        switched.set_key_track(0.55);
        switched.set_env_amount(-0.4);
        switched.set_env_velocity_amount(0.7);
        switched.set_audio_mod(0.45);
        switched.set_oversampling(FilterOversampling::Off);

        for _ in 0..64 {
            let _ = switched.process(
                WideF32::splat(0.2),
                WideF32::splat(64.0),
                WideF32::splat(0.3),
                WideF32::splat(0.8),
                WideF32::splat(-0.25),
                WideF32::splat(3.0),
                WideF32::splat(0.02),
                WideF32::splat(0.1),
                48_000.0,
            );
        }
        switched.set_filter_type(FilterType::ScalarFeedbackTpt);

        let mut fresh = Filter::new(FilterType::ScalarFeedbackTpt);
        fresh.set_cutoff(930.0);
        fresh.set_resonance(0.64);
        fresh.set_poles(2);
        fresh.set_key_track(0.55);
        fresh.set_env_amount(-0.4);
        fresh.set_env_velocity_amount(0.7);
        fresh.set_audio_mod(0.45);
        fresh.set_oversampling(FilterOversampling::Off);

        let args = (
            WideF32::splat(0.2),
            WideF32::splat(64.0),
            WideF32::splat(0.3),
            WideF32::splat(0.8),
            WideF32::splat(-0.25),
            WideF32::splat(3.0),
            WideF32::splat(0.02),
            WideF32::splat(0.1),
        );
        let switched_output = switched.process(
            args.0, args.1, args.2, args.3, args.4, args.5, args.6, args.7, 48_000.0,
        );
        let fresh_output = fresh.process(
            args.0, args.1, args.2, args.3, args.4, args.5, args.6, args.7, 48_000.0,
        );

        assert_eq!(switched.filter_type(), FilterType::ScalarFeedbackTpt);
        assert_eq!(switched_output, fresh_output);
    }

    fn process_modulated(
        filter: &mut LadderFilter,
        input: WideF32,
        note: WideF32,
        filter_env: WideF32,
        velocity: WideF32,
        osc1_audio: WideF32,
        sample_rate: f32,
    ) -> WideF32 {
        filter.process(
            input,
            note,
            filter_env,
            velocity,
            osc1_audio,
            WideF32::ZERO,
            WideF32::ZERO,
            WideF32::ZERO,
            sample_rate,
        )
    }

    #[cfg(not(feature = "fast-math"))]
    fn golden_stream(
        poles: u8,
        resonance: f32,
        oversampling: FilterOversampling,
        modulation_heavy: bool,
    ) -> [u32; 32] {
        let mut filter = LadderFilter::default();
        filter.set_cutoff(if modulation_heavy { 730.0 } else { 1_250.0 });
        filter.set_resonance(resonance);
        filter.set_poles(poles);
        filter.set_oversampling(oversampling);
        if modulation_heavy {
            filter.set_key_track(0.73);
            filter.set_env_amount(-0.61);
            filter.set_env_velocity_amount(0.84);
            filter.set_audio_mod(0.67);
        }

        let mut output = [0u32; 32];
        for frame in 0..520 {
            let phase = frame as f32;
            let input = WideF32::new(core::array::from_fn(|i| {
                [
                    (phase * 0.071).sin() * 0.21,
                    (phase * 0.047).cos() * 0.17,
                    if frame % 11 < 5 { 0.13 } else { -0.09 },
                    ((frame * 37 % 101) as f32 / 50.0 - 1.0) * 0.08,
                ][i % 4]
            }));
            let rendered = filter.process(
                input,
                WideF32::new(core::array::from_fn(|i| [36.0, 52.0, 67.0, 83.0][i % 4])),
                if modulation_heavy {
                    WideF32::new(core::array::from_fn(|i| [0.05, 0.35, 0.7, 0.95][i % 4]))
                } else {
                    WideF32::ZERO
                },
                WideF32::new(core::array::from_fn(|i| [0.2, 0.45, 0.75, 1.0][i % 4])),
                if modulation_heavy {
                    WideF32::new(core::array::from_fn(|i| {
                        [
                            (phase * 0.031).sin(),
                            (phase * 0.043).cos(),
                            (phase * 0.059).sin(),
                            (phase * 0.073).cos(),
                        ][i % 4]
                    }))
                } else {
                    WideF32::ZERO
                },
                if modulation_heavy {
                    WideF32::new(core::array::from_fn(|i| [-17.0, -3.5, 8.0, 23.0][i % 4]))
                } else {
                    WideF32::ZERO
                },
                if modulation_heavy {
                    WideF32::new(core::array::from_fn(|i| [-0.08, 0.02, 0.11, -0.03][i % 4]))
                } else {
                    WideF32::ZERO
                },
                if modulation_heavy {
                    WideF32::new(core::array::from_fn(|i| [0.2, -0.15, 0.25, -0.1][i % 4]))
                } else {
                    WideF32::ZERO
                },
                48_000.0,
            );
            if frame >= 512 {
                let values = rendered.to_array();
                let offset = (frame - 512) * WideF32::LANES;
                for lane in 0..WideF32::LANES {
                    output[offset + lane] = values[lane].to_bits();
                }
            }
        }
        output
    }

    #[cfg(not(feature = "fast-math"))]
    #[test]
    #[cfg(feature = "wide-4")]
    fn distributed_newton_tpt_golden_streams_are_bit_identical() {
        let actual = [
            golden_stream(2, 0.58, FilterOversampling::Off, false),
            golden_stream(4, 0.58, FilterOversampling::Off, false),
            golden_stream(4, 1.0, FilterOversampling::Off, false),
            golden_stream(4, 1.0, FilterOversampling::X2, true),
        ];
        let expected = [
            [
                0xbde6c3bb, 0x3c84e4ff, 0x3c82f663, 0xbb8abb93, 0xbdeaf662, 0x3ca9a1b8, 0x3c7bf08b,
                0xbb7d3b2f, 0xbdedfd64, 0x3ccdfb73, 0x3c49173a, 0xbb34cee2, 0xbdefd55a, 0x3cf1dd07,
                0x3bf59646, 0xbad9b7d0, 0xbdf07c37, 0x3d0a98d1, 0x3ade778a, 0xba90d6c6, 0xbdeff145,
                0x3d1bf26c, 0xbb5b96e2, 0xbabebe06, 0xbdee3528, 0x3d2cf158, 0xbba8c0e4, 0xbb041846,
                0xbdeb49db, 0x3d3d8bd8, 0xbb570d80, 0xbb1522e6,
            ],
            [
                0xbe7c8ef3, 0x3ac4a87c, 0x3c164cab, 0x39fa9d2a, 0xbe83d464, 0x3c3ae181, 0x3c1fa749,
                0xb910fcba, 0xbe88b5fb, 0x3cae7064, 0x3c2ff6a9, 0xba559dae, 0xbe8ce60a, 0x3cff1a10,
                0x3c423407, 0xbac17c6d, 0xbe905f50, 0x3d279f29, 0x3c50f1ac, 0xbb0731b0, 0xbe931d84,
                0x3d4f56e6, 0x3c57816c, 0xbb27ee5b, 0xbe951d58, 0x3d769caf, 0x3c5383e3, 0xbb44ed39,
                0xbe965c7a, 0x3d8eac90, 0x3c462761, 0xbb608f09,
            ],
            [
                0xbf32dde3, 0xbf219dce, 0x3f0036e7, 0xbf29f572, 0xbf457e36, 0xbf20e2ac, 0x3ed7bffa,
                0xbf2a5372, 0xbf5488a6, 0xbf1b072a, 0x3ea958b0, 0xbf25a612, 0xbf5f7228, 0xbf10362c,
                0x3e6d184c, 0xbf1c08c7, 0xbf65bcce, 0xbf00ce26, 0x3e016317, 0xbf0dcb36, 0xbf6704a0,
                0xbedab23e, 0x3c91df0a, 0xbef6dabb, 0xbf630d04, 0xbead0164, 0xbdbb42c5, 0xbecb2a24,
                0xbf59cc04, 0xbe74018c, 0xbe4b44ab, 0xbe99fe3a,
            ],
            [
                0xbf3ec35e, 0x3dda5c79, 0x3da5b4d8, 0x3e6e4471, 0xbf3ceae5, 0xbe3f1692, 0xbf0f8d76,
                0xbed3544d, 0xbf34b62b, 0xbedaec07, 0x3e9fbb71, 0x3eb67069, 0xbf26c3cf, 0xbf1076f0,
                0xbdef15a8, 0x3dac465d, 0xbf13f640, 0xbf0fdc45, 0xbed4d2f2, 0xbecc6c9b, 0xbefab7f6,
                0xbed68809, 0x3ec55dda, 0x3eb4b7b2, 0xbec82c67, 0xbe2f2219, 0xbe0b03a3, 0xbe4ef363,
                0xbe9285e8, 0x3e016d14, 0x3dde6e0a, 0xbe0787ad,
            ],
        ];
        assert_eq!(actual, expected);
    }

    #[test]
    fn static_filter_cache_tracks_control_and_sample_rate_changes_exactly() {
        let mut reused = LadderFilter::default();
        reused.set_cutoff(600.0);
        reused.set_resonance(0.2);
        for _ in 0..32 {
            process(
                &mut reused,
                WideF32::splat(0.1),
                WideF32::splat(60.0),
                44_100.0,
            );
        }

        reused.reset();
        reused.set_cutoff(2_400.0);
        reused.set_resonance(0.6);

        let mut fresh = LadderFilter::default();
        fresh.set_cutoff(2_400.0);
        fresh.set_resonance(0.6);

        for index in 0..128 {
            let input = WideF32::splat(if index % 2 == 0 { 0.2 } else { -0.15 });
            let reused_output = process(&mut reused, input, WideF32::splat(60.0), 48_000.0);
            let fresh_output = process(&mut fresh, input, WideF32::splat(60.0), 48_000.0);
            assert_eq!(reused_output, fresh_output);
        }
    }

    /// Measure steady-state RMS response at a given frequency using a small
    /// test signal to stay in the linear region.
    fn measure_response(
        filter: &mut LadderFilter,
        freq: f32,
        cutoff: f32,
        resonance: f32,
        poles: u8,
        sample_rate: f32,
        amplitude: f32,
    ) -> f32 {
        let dt = 1.0 / sample_rate;
        let omega = 2.0 * std::f32::consts::PI * freq;
        let settle = (sample_rate * 0.1) as usize;
        let measure = (sample_rate * 0.03) as usize;

        filter.set_cutoff(cutoff);
        filter.set_resonance(resonance);
        filter.set_poles(poles);

        let mut phase = 0.0f32;
        for _ in 0..settle {
            let input = WideF32::splat(phase.sin() * amplitude);
            let _ = process(filter, input, WideF32::splat(60.0), sample_rate);
            phase += omega * dt;
        }

        let mut sum_sq = 0.0f32;
        for _ in 0..measure {
            let input = WideF32::splat(phase.sin() * amplitude);
            let out = process(filter, input, WideF32::splat(60.0), sample_rate);
            sum_sq += out.to_array()[0].powi(2);
            phase += omega * dt;
        }
        (sum_sq / measure as f32).sqrt()
    }

    fn measure_projected_gain(
        filter: &mut LadderFilter,
        freq: f32,
        cutoff: f32,
        resonance: f32,
        poles: u8,
        sample_rate: f32,
        amplitude: f32,
    ) -> f32 {
        let dt = 1.0 / sample_rate;
        let omega = 2.0 * std::f32::consts::PI * freq;
        let settle = (sample_rate * 0.1) as usize;
        let measure = (sample_rate * 0.05) as usize;

        filter.reset();
        filter.set_cutoff(cutoff);
        filter.set_resonance(resonance);
        filter.set_poles(poles);

        let mut phase = 0.0f32;
        for _ in 0..settle {
            let input = WideF32::splat(phase.sin() * amplitude);
            let _ = process(filter, input, WideF32::splat(60.0), sample_rate);
            phase += omega * dt;
        }

        let mut sin_sum = 0.0f32;
        let mut cos_sum = 0.0f32;
        for _ in 0..measure {
            let sin = phase.sin();
            let cos = phase.cos();
            let input = WideF32::splat(sin * amplitude);
            let out = process(filter, input, WideF32::splat(60.0), sample_rate).to_array()[0];
            sin_sum += out * sin;
            cos_sum += out * cos;
            phase += omega * dt;
        }

        let output_amp = 2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / measure as f32;
        output_amp / amplitude
    }

    fn measure_projected_gain_for_configured_note(
        filter: &mut LadderFilter,
        freq: f32,
        note: f32,
        sample_rate: f32,
        amplitude: f32,
    ) -> f32 {
        let dt = 1.0 / sample_rate;
        let omega = 2.0 * std::f32::consts::PI * freq;
        let settle = (sample_rate * 0.1) as usize;
        let measure = (sample_rate * 0.05) as usize;

        filter.reset();

        let mut phase = 0.0f32;
        for _ in 0..settle {
            let input = WideF32::splat(phase.sin() * amplitude);
            let _ = process(filter, input, WideF32::splat(note), sample_rate);
            phase += omega * dt;
        }

        let mut sin_sum = 0.0f32;
        let mut cos_sum = 0.0f32;
        for _ in 0..measure {
            let sin = phase.sin();
            let cos = phase.cos();
            let input = WideF32::splat(sin * amplitude);
            let out = process(filter, input, WideF32::splat(note), sample_rate).to_array()[0];
            sin_sum += out * sin;
            cos_sum += out * cos;
            phase += omega * dt;
        }

        let output_amp = 2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / measure as f32;
        output_amp / amplitude
    }

    fn measure_modulated_response(
        filter: &mut LadderFilter,
        freq: f32,
        note: f32,
        filter_env: f32,
        osc1_audio: f32,
        sample_rate: f32,
        amplitude: f32,
    ) -> f32 {
        let dt = 1.0 / sample_rate;
        let omega = 2.0 * std::f32::consts::PI * freq;
        let settle = (sample_rate * 0.1) as usize;
        let measure = (sample_rate * 0.03) as usize;
        let note = WideF32::splat(note);
        let filter_env = WideF32::splat(filter_env);
        let osc1_audio = WideF32::splat(osc1_audio);

        let mut phase = 0.0f32;
        for _ in 0..settle {
            let input = WideF32::splat(phase.sin() * amplitude);
            let _ = process_modulated(
                filter,
                input,
                note,
                filter_env,
                WideF32::splat(1.0),
                osc1_audio,
                sample_rate,
            );
            phase += omega * dt;
        }

        let mut sum_sq = 0.0f32;
        for _ in 0..measure {
            let input = WideF32::splat(phase.sin() * amplitude);
            let out = process_modulated(
                filter,
                input,
                note,
                filter_env,
                WideF32::splat(1.0),
                osc1_audio,
                sample_rate,
            );
            sum_sq += out.to_array()[0].powi(2);
            phase += omega * dt;
        }
        (sum_sq / measure as f32).sqrt()
    }

    #[cfg(feature = "wide-4")]
    fn measure_velocity_lane_response(
        filter: &mut LadderFilter,
        freq: f32,
        velocities: [f32; WideF32::LANES],
        sample_rate: f32,
        amplitude: f32,
    ) -> [f32; WideF32::LANES] {
        let dt = 1.0 / sample_rate;
        let omega = 2.0 * std::f32::consts::PI * freq;
        let settle = (sample_rate * 0.1) as usize;
        let measure = (sample_rate * 0.03) as usize;
        let note = WideF32::splat(60.0);
        let filter_env = WideF32::splat(1.0);
        let velocity = WideF32::new(velocities);
        let osc1_audio = WideF32::ZERO;

        let mut phase = 0.0f32;
        for _ in 0..settle {
            let input = WideF32::splat(phase.sin() * amplitude);
            let _ = process_modulated(
                filter,
                input,
                note,
                filter_env,
                velocity,
                osc1_audio,
                sample_rate,
            );
            phase += omega * dt;
        }

        let mut sum_sq = [0.0; WideF32::LANES];
        for _ in 0..measure {
            let input = WideF32::splat(phase.sin() * amplitude);
            let out = process_modulated(
                filter,
                input,
                note,
                filter_env,
                velocity,
                osc1_audio,
                sample_rate,
            )
            .to_array();
            for lane in 0..WideF32::LANES {
                sum_sq[lane] += out[lane].powi(2);
            }
            phase += omega * dt;
        }

        sum_sq.map(|sum| (sum / measure as f32).sqrt())
    }

    #[derive(Debug, Clone, Copy)]
    struct ResponseSummary {
        peak_abs: f32,
        energy: f32,
        zero_crossings: usize,
    }

    fn summarize_response(samples: &[f32]) -> ResponseSummary {
        let mut peak_abs = 0.0f32;
        let mut energy = 0.0f32;
        let mut zero_crossings = 0usize;
        let mut prev = 0.0f32;

        for &sample in samples {
            peak_abs = peak_abs.max(sample.abs());
            energy += sample * sample;
            if prev < 0.0 && sample >= 0.0 {
                zero_crossings += 1;
            }
            prev = sample;
        }

        ResponseSummary {
            peak_abs,
            energy,
            zero_crossings,
        }
    }

    fn render_impulse_response(
        filter: &mut LadderFilter,
        cutoff: f32,
        resonance: f32,
        poles: u8,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        filter.reset();
        filter.set_cutoff(cutoff);
        filter.set_resonance(resonance);
        filter.set_poles(poles);

        (0..frames)
            .map(|i| {
                let input = if i == 0 { 1.0 } else { 0.0 };
                process(
                    filter,
                    WideF32::splat(input),
                    WideF32::splat(60.0),
                    sample_rate,
                )
                .to_array()[0]
            })
            .collect()
    }

    fn render_sine_sweep(
        filter: &mut LadderFilter,
        cutoff: f32,
        resonance: f32,
        poles: u8,
        start_freq: f32,
        end_freq: f32,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        filter.reset();
        filter.set_cutoff(cutoff);
        filter.set_resonance(resonance);
        filter.set_poles(poles);

        let mut phase = 0.0f32;
        (0..frames)
            .map(|i| {
                let t = i as f32 / frames.max(1) as f32;
                let freq = start_freq + (end_freq - start_freq) * t;
                phase += 2.0 * std::f32::consts::PI * freq / sample_rate;
                let input = phase.sin() * 0.1;
                process(
                    filter,
                    WideF32::splat(input),
                    WideF32::splat(60.0),
                    sample_rate,
                )
                .to_array()[0]
            })
            .collect()
    }

    fn render_cutoff_sweep(
        filter: &mut LadderFilter,
        input_freq: f32,
        start_cutoff: f32,
        end_cutoff: f32,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        filter.reset();
        filter.set_resonance(0.2);
        filter.set_poles(4);

        let mut phase = 0.0f32;
        (0..frames)
            .map(|i| {
                let t = i as f32 / frames.max(1) as f32;
                filter.set_cutoff(start_cutoff + (end_cutoff - start_cutoff) * t);
                phase += 2.0 * std::f32::consts::PI * input_freq / sample_rate;
                let input = phase.sin() * 0.1;
                process(
                    filter,
                    WideF32::splat(input),
                    WideF32::splat(60.0),
                    sample_rate,
                )
                .to_array()[0]
            })
            .collect()
    }

    fn render_resonance_sweep(
        filter: &mut LadderFilter,
        input_freq: f32,
        cutoff: f32,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        filter.reset();
        filter.set_cutoff(cutoff);
        filter.set_poles(4);

        let mut phase = 0.0f32;
        (0..frames)
            .map(|i| {
                let t = i as f32 / frames.max(1) as f32;
                filter.set_resonance(t);
                phase += 2.0 * std::f32::consts::PI * input_freq / sample_rate;
                let input = phase.sin() * 0.05;
                process(
                    filter,
                    WideF32::splat(input),
                    WideF32::splat(60.0),
                    sample_rate,
                )
                .to_array()[0]
            })
            .collect()
    }

    fn render_self_oscillation(
        filter: &mut LadderFilter,
        cutoff: f32,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        render_self_oscillation_with_note(filter, cutoff, 60.0, 0.0, sample_rate, frames)
    }

    fn render_self_oscillation_with_note(
        filter: &mut LadderFilter,
        cutoff: f32,
        note: f32,
        key_track: f32,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        render_self_oscillation_with_note_and_tuning(
            filter,
            cutoff,
            note,
            key_track,
            SELF_OSC_PITCH_TUNING_CENTS,
            sample_rate,
            frames,
        )
    }

    fn render_self_oscillation_with_note_and_tuning(
        filter: &mut LadderFilter,
        cutoff: f32,
        note: f32,
        key_track: f32,
        tuning_cents: f32,
        sample_rate: f32,
        frames: usize,
    ) -> Vec<f32> {
        filter.reset();
        filter.set_cutoff(cutoff);
        filter.set_resonance(1.0);
        filter.set_poles(4);
        filter.set_key_track(key_track);
        filter.set_self_osc_pitch_tuning_cents(tuning_cents);

        (0..frames)
            .map(|_| {
                process(filter, WideF32::ZERO, WideF32::splat(note), sample_rate).to_array()[0]
            })
            .collect()
    }

    fn measure_projected_component(samples: &[f32], freq: f32, sample_rate: f32) -> f32 {
        let omega = 2.0 * std::f32::consts::PI * freq / sample_rate;
        let mut phase = 0.0f32;
        let mut sin_sum = 0.0f32;
        let mut cos_sum = 0.0f32;

        for &sample in samples {
            let sin = phase.sin();
            let cos = phase.cos();
            sin_sum += sample * sin;
            cos_sum += sample * cos;
            phase += omega;
        }

        2.0 * (sin_sum * sin_sum + cos_sum * cos_sum).sqrt() / samples.len().max(1) as f32
    }

    fn fold_frequency(freq: f32, sample_rate: f32) -> f32 {
        let nyquist = sample_rate * 0.5;
        let period = sample_rate;
        let folded = freq.rem_euclid(period);
        if folded > nyquist {
            period - folded
        } else {
            folded
        }
    }

    fn estimate_frequency_from_positive_crossings(samples: &[f32], sample_rate: f32) -> f32 {
        let mut crossings = 0usize;
        let mut first_crossing = None;
        let mut last_crossing = None;
        let mut prev = samples.first().copied().unwrap_or(0.0);

        for (index, &sample) in samples.iter().enumerate().skip(1) {
            if prev < 0.0 && sample >= 0.0 {
                crossings += 1;
                first_crossing.get_or_insert(index);
                last_crossing = Some(index);
            }
            prev = sample;
        }

        let Some(first) = first_crossing else {
            return 0.0;
        };
        let Some(last) = last_crossing else {
            return 0.0;
        };
        if crossings < 2 || last <= first {
            return 0.0;
        }

        (crossings - 1) as f32 * sample_rate / (last - first) as f32
    }

    fn estimate_self_oscillation_pitch_hz(
        cutoff: f32,
        note: f32,
        key_track: f32,
        tuning_cents: f32,
        sample_rate: f32,
    ) -> f32 {
        estimate_excited_self_oscillation_pitch_hz(
            cutoff,
            note,
            key_track,
            tuning_cents,
            sample_rate,
        )
    }

    fn estimate_excited_self_oscillation_pitch_hz(
        cutoff: f32,
        note: f32,
        key_track: f32,
        tuning_cents: f32,
        sample_rate: f32,
    ) -> f32 {
        let mut filter = LadderFilter::default();
        filter.reset();
        filter.set_cutoff(cutoff);
        filter.set_resonance(1.0);
        filter.set_poles(4);
        filter.set_key_track(key_track);
        filter.set_self_osc_pitch_tuning_cents(tuning_cents);

        for _ in 0..128 {
            let _ = process(
                &mut filter,
                WideF32::splat(0.1),
                WideF32::splat(note),
                sample_rate,
            );
        }

        let mut samples = Vec::with_capacity(24_000);
        for _ in 0..24_000 {
            samples.push(
                process(
                    &mut filter,
                    WideF32::ZERO,
                    WideF32::splat(note),
                    sample_rate,
                )
                .to_array()[0],
            );
        }

        estimate_frequency_from_positive_crossings(&samples[8_000..], sample_rate)
    }

    fn measure_self_oscillation_tail_energy(resonance: f32, cutoff: f32, sample_rate: f32) -> f32 {
        let mut filter = LadderFilter::default();
        filter.set_cutoff(cutoff);
        filter.set_resonance(resonance);
        filter.set_poles(4);

        for _ in 0..128 {
            let _ = process(
                &mut filter,
                WideF32::splat(0.1),
                WideF32::splat(60.0),
                sample_rate,
            );
        }

        let mut energy = 0.0;
        for i in 0..18_000 {
            let out = process(
                &mut filter,
                WideF32::ZERO,
                WideF32::splat(60.0),
                sample_rate,
            )
            .to_array()[0];
            assert!(out.is_finite(), "NaN/Inf at i={i}");
            if i >= 14_000 {
                energy += out * out;
            }
        }

        energy
    }

    #[test]
    fn test_dc_gain_is_unity() {
        let mut f = LadderFilter::default();
        for _ in 0..5000 {
            let out = process(&mut f, WideF32::splat(1.0), WideF32::splat(60.0), 44100.0);
            let val = out.to_array()[0];
            assert!(val.is_finite() && val.abs() < 5.0, "DC out of range: {val}");
        }
        let out = process(&mut f, WideF32::splat(1.0), WideF32::splat(60.0), 44100.0);
        let val = out.to_array()[0];
        assert!((val - 1.0).abs() < 0.3, "DC gain should be ~1.0, got {val}");
    }

    #[test]
    fn test_cutoff_control_opens_filter() {
        let sr = 44100.0;
        let mut closed = LadderFilter::default();
        let mut open = LadderFilter::default();
        let closed_amp = measure_response(&mut closed, 2000.0, 500.0, 0.0, 4, sr, 0.1);
        let open_amp = measure_response(&mut open, 2000.0, 5000.0, 0.0, 4, sr, 0.1);

        assert!(
            open_amp > closed_amp * 10.0,
            "higher cutoff should pass more high-frequency energy: closed={closed_amp:.4} open={open_amp:.4}"
        );
    }

    #[test]
    fn test_four_pole_attenuates_more_than_two_pole() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let mut f4 = LadderFilter::default();
        let mut f2 = LadderFilter::default();
        let amp4 = measure_response(&mut f4, 4000.0, cutoff, 0.0, 4, sr, 0.1);
        let amp2 = measure_response(&mut f2, 4000.0, cutoff, 0.0, 2, sr, 0.1);
        assert!(
            amp4 < amp2,
            "4-pole ({amp4:.4}) should attenuate more than 2-pole ({amp2:.4})"
        );
    }

    #[test]
    fn test_two_pole_rolls_off() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let mut f = LadderFilter::default();
        let amp1k = measure_response(&mut f, 1000.0, cutoff, 0.0, 2, sr, 0.1);
        let amp4k = measure_response(&mut f, 4000.0, cutoff, 0.0, 2, sr, 0.1);
        assert!(
            amp4k < amp1k,
            "2-pole should attenuate above cutoff: 1k={amp1k:.4} 4k={amp4k:.4}"
        );
    }

    #[test]
    fn test_two_pole_resonates_without_self_oscillation() {
        let sr = 44100.0;
        let mut flat_filter = LadderFilter::default();
        let mut resonant_filter = LadderFilter::default();
        let flat = render_impulse_response(&mut flat_filter, 1000.0, 0.0, 2, sr, 4096);
        let resonant = render_impulse_response(&mut resonant_filter, 1000.0, 1.0, 2, sr, 4096);
        let flat_summary = summarize_response(&flat);
        let resonant_summary = summarize_response(&resonant);
        assert!(
            resonant_summary.zero_crossings > flat_summary.zero_crossings
                && resonant_summary.peak_abs < 0.1,
            "2-pole resonance should add ringing without self-oscillation: flat={flat_summary:?} res={resonant_summary:?}"
        );

        let mut f = LadderFilter::default();
        f.set_cutoff(440.0);
        f.set_resonance(1.0);
        f.set_poles(2);
        for _ in 0..10 {
            let _ = process(&mut f, WideF32::splat(0.5), WideF32::splat(60.0), sr);
        }

        let mut first_energy = 0.0f32;
        let mut last_energy = 0.0f32;
        for i in 0..12000 {
            let out = process(&mut f, WideF32::ZERO, WideF32::splat(60.0), sr);
            let val = out.to_array()[0];
            assert!(val.is_finite(), "NaN/Inf at i={i}");
            if i < 1024 {
                first_energy += val * val;
            } else if i >= 10976 {
                last_energy += val * val;
            }
        }
        assert!(
            last_energy < first_energy * 0.1,
            "2-pole mode should decay rather than self-oscillate: first={first_energy:.6} last={last_energy:.6}"
        );
    }

    #[test]
    fn test_resonance_creates_peak() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let mut f_flat = LadderFilter::default();
        let mut f_res = LadderFilter::default();
        let amp_flat = measure_response(&mut f_flat, 1000.0, cutoff, 0.0, 4, sr, 0.1);
        let amp_res = measure_response(&mut f_res, 1000.0, cutoff, 1.0, 4, sr, 0.1);
        assert!(
            amp_res > amp_flat * 4.0,
            "resonance should boost near cutoff: flat={amp_flat:.4} res={amp_res:.4}"
        );
    }

    #[test]
    fn test_four_pole_resonance_compensates_low_frequency_loss() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let mut flat = LadderFilter::default();
        let mut resonant = LadderFilter::default();
        let flat_amp = measure_response(&mut flat, 100.0, cutoff, 0.0, 4, sr, 0.02);
        let resonant_amp = measure_response(&mut resonant, 100.0, cutoff, 0.8, 4, sr, 0.02);
        let ratio = resonant_amp / flat_amp;

        assert!(
            (0.95..=1.25).contains(&ratio),
            "4-pole bass compensation should keep low-frequency response near unity: flat={flat_amp:.4} resonant={resonant_amp:.4} ratio={ratio:.3}"
        );
    }

    #[test]
    fn test_four_pole_resonance_compensation_survives_musical_level() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let amplitude = 0.35;
        let mut flat = LadderFilter::default();
        let mut resonant = LadderFilter::default();
        let flat_amp = measure_response(&mut flat, 100.0, cutoff, 0.0, 4, sr, amplitude);
        let resonant_amp = measure_response(&mut resonant, 100.0, cutoff, 0.8, 4, sr, amplitude);
        let ratio = resonant_amp / flat_amp;

        assert!(
            (0.9..=1.25).contains(&ratio),
            "4-pole bass compensation should stay near unity at musical signal levels: flat={flat_amp:.4} resonant={resonant_amp:.4} ratio={ratio:.3}"
        );
        assert!(
            resonant_amp.is_finite() && resonant_amp < amplitude * 4.0,
            "4-pole bass compensation should stay bounded at musical signal levels: resonant={resonant_amp:.4}"
        );
    }

    #[test]
    fn test_max_resonance_open_filter_passband_stays_near_unity() {
        let sr = 44100.0;
        let cutoff = 18000.0;
        let freq = 1000.0;
        let mut flat = LadderFilter::default();
        let mut resonant = LadderFilter::default();
        for amplitude in [0.02, 0.35] {
            let flat_gain = measure_projected_gain(&mut flat, freq, cutoff, 0.0, 4, sr, amplitude);
            let resonant_gain =
                measure_projected_gain(&mut resonant, freq, cutoff, 1.0, 4, sr, amplitude);
            let ratio = resonant_gain / flat_gain;
            let db = 20.0 * ratio.log10();

            assert!(
                (-1.0..=1.0).contains(&db),
                "max-resonance open filter passband should stay near unity at amplitude {amplitude}: flat={flat_gain:.4} resonant={resonant_gain:.4} ratio={ratio:.3} db={db:.2}"
            );
        }
    }

    #[test]
    fn test_resonance_peak_survives_self_oscillation_threshold() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let freq = cutoff;
        let amplitude = 0.05;
        for (below_resonance, above_resonance) in [(0.89, 0.91), (0.955, 0.975)] {
            let mut below = LadderFilter::default();
            let mut above = LadderFilter::default();
            let below_gain =
                measure_projected_gain(&mut below, freq, cutoff, below_resonance, 4, sr, amplitude);
            let above_gain =
                measure_projected_gain(&mut above, freq, cutoff, above_resonance, 4, sr, amplitude);
            let ratio = above_gain / below_gain;

            assert!(
                ratio > 0.75,
                "resonance peak should not collapse across resonance {below_resonance:.3}->{above_resonance:.3}: below={below_gain:.4} above={above_gain:.4} ratio={ratio:.3}"
            );
        }
    }

    #[test]
    fn test_self_oscillation_threshold_response_transition_is_smooth() {
        let sr = 44100.0;
        let cutoff = 6214.0;
        let note = 48.0;
        let probe_freq = 11_380.0;
        let amplitude = 0.02;
        let mut at_threshold = LadderFilter::default();
        let mut just_above = LadderFilter::default();

        for filter in [&mut at_threshold, &mut just_above] {
            filter.set_cutoff(cutoff);
            filter.set_poles(4);
            filter.set_key_track(1.0);
        }
        at_threshold.set_resonance(SELF_OSC_RESONANCE_START);
        just_above.set_resonance(SELF_OSC_RESONANCE_START + 0.01);

        let threshold_gain = measure_projected_gain_for_configured_note(
            &mut at_threshold,
            probe_freq,
            note,
            sr,
            amplitude,
        );
        let above_gain = measure_projected_gain_for_configured_note(
            &mut just_above,
            probe_freq,
            note,
            sr,
            amplitude,
        );
        let db_delta = 20.0 * (above_gain / threshold_gain.max(1.0e-9)).log10();

        assert!(
            db_delta.abs() < 0.75,
            "self-oscillation threshold should not abruptly move the resonant response: threshold={threshold_gain:.6} above={above_gain:.6} delta={db_delta:.2}dB"
        );
    }

    #[test]
    fn test_max_resonance_keeps_cutoff_peak() {
        let sr = 44100.0;
        let cutoff = 1000.0;
        let max_resonance_freq = cutoff * 2.0f32.powf(SELF_OSC_PITCH_TUNING_CENTS / 1200.0);
        let amplitude = 0.02;
        let mut pre_self_osc = LadderFilter::default();
        let mut max_resonance = LadderFilter::default();
        let pre_gain = measure_projected_gain(
            &mut pre_self_osc,
            cutoff,
            cutoff,
            SELF_OSC_RESONANCE_START - 0.02,
            4,
            sr,
            amplitude,
        );
        let max_gain = measure_projected_gain(
            &mut max_resonance,
            max_resonance_freq,
            cutoff,
            1.0,
            4,
            sr,
            amplitude,
        );
        let ratio = max_gain / pre_gain;

        assert!(
            ratio > 0.55 && max_gain > 5.5,
            "max resonance should keep a strong calibrated cutoff peak: pre={pre_gain:.4} max={max_gain:.4} ratio={ratio:.3}"
        );
    }

    #[test]
    fn test_filter_measurement_helpers_generate_finite_summaries() {
        let sr = 44100.0;
        let mut impulse_filter = LadderFilter::default();
        let impulse = render_impulse_response(&mut impulse_filter, 1000.0, 0.4, 4, sr, 2048);
        let impulse_summary = summarize_response(&impulse);
        assert!(impulse_summary.peak_abs > 0.0 && impulse_summary.peak_abs < 2.0);
        assert!(impulse_summary.energy.is_finite() && impulse_summary.energy > 0.0);

        let mut sine_filter = LadderFilter::default();
        let sine_sweep =
            render_sine_sweep(&mut sine_filter, 1200.0, 0.35, 4, 100.0, 4000.0, sr, 2048);
        let sine_summary = summarize_response(&sine_sweep);
        assert!(sine_summary.energy.is_finite() && sine_summary.energy > 0.0);

        let mut cutoff_filter = LadderFilter::default();
        let cutoff_sweep = render_cutoff_sweep(&mut cutoff_filter, 2500.0, 300.0, 6000.0, sr, 2048);
        let cutoff_summary = summarize_response(&cutoff_sweep);
        assert!(cutoff_summary.energy.is_finite() && cutoff_summary.energy > 0.0);

        let mut resonance_filter = LadderFilter::default();
        let resonance_sweep =
            render_resonance_sweep(&mut resonance_filter, 1000.0, 1000.0, sr, 2048);
        let resonance_summary = summarize_response(&resonance_sweep);
        assert!(resonance_summary.energy.is_finite() && resonance_summary.peak_abs < 5.0);

        let mut self_osc_filter = LadderFilter::default();
        let self_osc = render_self_oscillation(&mut self_osc_filter, 440.0, sr, 60_000);
        let self_osc_summary = summarize_response(&self_osc[58_000..]);
        assert!(self_osc_summary.peak_abs.is_finite() && self_osc_summary.peak_abs < 5.0);
        assert!(
            self_osc_summary.zero_crossings > 5,
            "self-oscillation helper should capture an oscillating tail: {self_osc_summary:?}"
        );
    }

    #[test]
    fn set_cutoff_preserves_prophet_program_zero_base() {
        let mut filter = LadderFilter::default();
        filter.set_cutoff(1.021_975);
        assert!(
            (filter.cutoff() - 1.021_975).abs() < 0.001,
            "base cutoff must keep Prophet raw 0 (~1 Hz) so key-track/env modulate from the true base"
        );
        filter.set_cutoff(0.5);
        assert_eq!(filter.cutoff(), MIN_BASE_CUTOFF_HZ);
    }

    #[test]
    fn test_key_tracking_opens_cutoff_for_higher_notes() {
        let sr = 44100.0;
        let mut low_note = LadderFilter::default();
        low_note.set_cutoff(40.0);
        low_note.set_key_track(1.0);

        let mut high_note = LadderFilter::default();
        high_note.set_cutoff(40.0);
        high_note.set_key_track(1.0);

        let low_amp = measure_modulated_response(&mut low_note, 2500.0, 48.0, 0.0, 0.0, sr, 0.1);
        let high_amp = measure_modulated_response(&mut high_note, 2500.0, 72.0, 0.0, 0.0, sr, 0.1);

        assert!(
            high_amp > low_amp * 2.0,
            "key tracking should open cutoff for high notes: low={low_amp:.4} high={high_amp:.4}"
        );
    }

    #[test]
    fn test_two_pole_modulated_cutoff_ignores_self_oscillation_pitch_trim() {
        let sr = 44100.0;
        let mut normal_trim = LadderFilter::default();
        let mut exaggerated_trim = LadderFilter::default();

        for filter in [&mut normal_trim, &mut exaggerated_trim] {
            filter.set_poles(2);
            filter.set_cutoff(700.0);
            filter.set_resonance(1.0);
            filter.set_key_track(1.0);
        }
        exaggerated_trim.set_self_osc_pitch_tuning_cents(1200.0);

        let normal_amp =
            measure_modulated_response(&mut normal_trim, 2500.0, 72.0, 0.0, 0.0, sr, 0.1);
        let exaggerated_amp =
            measure_modulated_response(&mut exaggerated_trim, 2500.0, 72.0, 0.0, 0.0, sr, 0.1);
        let ratio = exaggerated_amp / normal_amp.max(1.0e-9);

        assert!(
            (0.995..=1.005).contains(&ratio),
            "2-pole mode should ignore self-oscillation pitch trim even on the modulated cutoff path: normal={normal_amp:.6} exaggerated={exaggerated_amp:.6} ratio={ratio:.6}"
        );
    }

    #[test]
    fn test_prophet_reference_self_oscillation_pitch_without_key_tracking() {
        let sr = 44100.0;
        let pitch_hz =
            estimate_self_oscillation_pitch_hz(444.0, 69.0, 0.0, SELF_OSC_PITCH_TUNING_CENTS, sr);

        assert!(
            (440.0..=490.0).contains(&pitch_hz),
            "max-resonance self-oscillation at cutoff 444 Hz without key tracking should stay in the measured high-400 Hz region: pitch={pitch_hz:.2}Hz tuning={SELF_OSC_PITCH_TUNING_CENTS:.1}c"
        );
    }

    #[test]
    fn test_key_tracked_self_oscillation_is_close_to_c4_seventh_harmonic() {
        let sr = 44100.0;
        let c4 = midi_to_hz(60);
        let target_hz = c4 * 7.0;
        let base_cutoff_hz = 444.0 / 64.0 * 4.0;
        let pitch_hz = estimate_self_oscillation_pitch_hz(
            base_cutoff_hz,
            60.0,
            1.0,
            SELF_OSC_PITCH_TUNING_CENTS,
            sr,
        );
        let beat_hz = (pitch_hz - target_hz).abs();

        assert!(
            beat_hz <= 8.0,
            "key-tracked self-oscillation should beat slowly against C4's 7th harmonic near the audible mix point: pitch={pitch_hz:.2}Hz target={target_hz:.2}Hz beat={beat_hz:.2}Hz tuning={SELF_OSC_PITCH_TUNING_CENTS:.1}c"
        );
    }

    #[test]
    #[ignore = "prints the best cents trim for the C4/key-tracked self-oscillation beat-rate target"]
    fn calibrate_self_oscillation_pitch_tuning_cents_for_key_tracked_c4() {
        let sr = 44100.0;
        let c4 = midi_to_hz(60);
        let target_hz = c4 * 7.0;
        let base_cutoff_hz = 444.0 / 64.0 * 4.0;
        let mut best_cents = 0.0;
        let mut best_pitch_hz = 0.0;
        let mut best_beat_hz = f32::INFINITY;

        for cents in 110..=150 {
            let cents = cents as f32;
            let pitch_hz = estimate_self_oscillation_pitch_hz(base_cutoff_hz, 60.0, 1.0, cents, sr);
            let beat_hz = (pitch_hz - target_hz).abs();

            if beat_hz < best_beat_hz {
                best_cents = cents;
                best_pitch_hz = pitch_hz;
                best_beat_hz = beat_hz;
            }
        }

        std::println!(
            "best SELF_OSC_PITCH_TUNING_CENTS={best_cents:.1}, key_tracked_c4_pitch={best_pitch_hz:.3}Hz, c4_7th_harmonic={target_hz:.3}Hz, beat={best_beat_hz:.3}Hz"
        );

        assert!(
            best_beat_hz <= 3.0,
            "best self-oscillation tuning should get within the estimator resolution of C4's 7th harmonic"
        );
    }

    #[test]
    fn test_prophet_reference_key_tracking_pushes_c4_self_oscillation_high() {
        let sr = 44100.0;
        let mut filter = LadderFilter::default();
        let base_cutoff_hz = 444.0 / 64.0 * 4.0;
        let samples =
            render_self_oscillation_with_note(&mut filter, base_cutoff_hz, 60.0, 1.0, sr, 70_000);
        let pitch_hz = estimate_frequency_from_positive_crossings(&samples[50_000..], sr);

        assert!(
            (1750.0..=2050.0).contains(&pitch_hz),
            "max key tracking at C4 should move Prophet-offset self-oscillation near the measured 1.9 kHz region, got {pitch_hz:.2} Hz"
        );
    }

    #[test]
    fn test_filter_envelope_amount_modulates_cutoff() {
        let sr = 44100.0;
        let mut closed = LadderFilter::default();
        closed.set_cutoff(400.0);
        closed.set_env_amount(1.0);

        let mut opened = LadderFilter::default();
        opened.set_cutoff(400.0);
        opened.set_env_amount(1.0);

        let closed_amp = measure_modulated_response(&mut closed, 3000.0, 60.0, 0.0, 0.0, sr, 0.1);
        let opened_amp = measure_modulated_response(&mut opened, 3000.0, 60.0, 1.0, 0.0, sr, 0.1);

        assert!(
            opened_amp > closed_amp * 4.0,
            "positive filter EG amount should open cutoff: closed={closed_amp:.4} opened={opened_amp:.4}"
        );
    }

    #[test]
    fn test_prophet_env_amount_depth_is_one_semitone_per_tick() {
        let sr = 44100.0;
        let base_hz = 200.0;
        let env_amount = 12.0 / ENV_DEPTH_SEMITONES;

        let mut closed = LadderFilter::default();
        closed.set_cutoff(base_hz);
        closed.set_resonance(1.0);
        closed.set_poles(4);
        closed.set_env_amount(env_amount);
        closed.set_self_osc_pitch_tuning_cents(SELF_OSC_PITCH_TUNING_CENTS);

        let mut opened = LadderFilter::default();
        opened.set_cutoff(base_hz);
        opened.set_resonance(1.0);
        opened.set_poles(4);
        opened.set_env_amount(env_amount);
        opened.set_self_osc_pitch_tuning_cents(SELF_OSC_PITCH_TUNING_CENTS);

        let closed_samples: Vec<f32> = (0..70_000)
            .map(|_| {
                process_modulated(
                    &mut closed,
                    WideF32::ZERO,
                    WideF32::splat(60.0),
                    WideF32::ZERO,
                    WideF32::splat(1.0),
                    WideF32::ZERO,
                    sr,
                )
                .to_array()[0]
            })
            .collect();
        let opened_samples: Vec<f32> = (0..70_000)
            .map(|_| {
                process_modulated(
                    &mut opened,
                    WideF32::ZERO,
                    WideF32::splat(60.0),
                    WideF32::splat(1.0),
                    WideF32::splat(1.0),
                    WideF32::ZERO,
                    sr,
                )
                .to_array()[0]
            })
            .collect();

        let closed_hz = estimate_frequency_from_positive_crossings(&closed_samples[50_000..], sr);
        let opened_hz = estimate_frequency_from_positive_crossings(&opened_samples[50_000..], sr);
        let ratio = opened_hz / closed_hz;

        assert!(
            (1.9..=2.1).contains(&ratio),
            "Env Amount +12 should raise self-osc by one octave (ratio≈2), got closed={closed_hz:.2} opened={opened_hz:.2} ratio={ratio:.3}"
        );
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn test_filter_velocity_adds_envelope_depth_per_lane() {
        let sr = 44100.0;
        let mut filter = LadderFilter::default();
        filter.set_cutoff(400.0);
        filter.set_env_amount(0.0);
        filter.set_env_velocity_amount(1.0);

        let amps =
            measure_velocity_lane_response(&mut filter, 3000.0, [0.0, 0.25, 0.5, 1.0], sr, 0.1);

        assert!(
            amps[0] < amps[1] && amps[1] < amps[2] && amps[2] < amps[3],
            "filter velocity should open cutoff monotonically per lane, amps={amps:?}"
        );
        assert!(
            amps[3] > amps[0] * 4.0,
            "filter velocity should open the envelope with Env Amount zero, amps={amps:?}"
        );
    }

    #[test]
    fn test_audio_mod_modulates_cutoff_from_osc1() {
        let sr = 44100.0;
        let mut negative = LadderFilter::default();
        negative.set_cutoff(1000.0);
        negative.set_audio_mod(1.0);

        let mut positive = LadderFilter::default();
        positive.set_cutoff(1000.0);
        positive.set_audio_mod(1.0);

        let negative_amp =
            measure_modulated_response(&mut negative, 2500.0, 60.0, 0.0, -1.0, sr, 0.1);
        let positive_amp =
            measure_modulated_response(&mut positive, 2500.0, 60.0, 0.0, 1.0, sr, 0.1);

        assert!(
            positive_amp > negative_amp * 3.0,
            "positive Osc1 audio mod should open cutoff relative to negative mod: negative={negative_amp:.4} positive={positive_amp:.4}"
        );
    }

    #[test]
    fn test_max_resonance_self_oscillates_without_blowing_up() {
        let sr = 44100.0;
        let cutoff = 440.0;
        let resonance = 1.0;
        let mut f = LadderFilter::default();
        f.set_cutoff(cutoff);
        f.set_resonance(resonance);

        // Brief impulse
        for _ in 0..10 {
            let _ = process(&mut f, WideF32::splat(0.5), WideF32::splat(60.0), sr);
        }

        let mut prev = 0.0f32;
        let mut zero_crossings = 0usize;
        let mut max_abs = 0.0f32;
        let mut first_energy = 0.0f32;
        let mut last_energy = 0.0f32;
        for i in 0..12000 {
            let out = process(&mut f, WideF32::ZERO, WideF32::splat(60.0), sr);
            let val = out.to_array()[0];
            max_abs = max_abs.max(val.abs());
            if i < 1024 {
                first_energy += val * val;
            } else if i >= 10976 {
                last_energy += val * val;
            }
            if prev < 0.0 && val >= 0.0 {
                zero_crossings += 1;
            }
            prev = val;
        }

        assert!(
            zero_crossings > 5,
            "max resonance should ring after an impulse, got {zero_crossings} crossings"
        );
        assert!(max_abs < 5.0, "self-oscillation blew up: {max_abs:.4}");
        assert!(
            last_energy > first_energy * 0.5,
            "max resonance should sustain instead of decay: first={first_energy:.6} last={last_energy:.6}"
        );
    }

    #[test]
    fn test_max_resonance_self_oscillation_starts_from_silence() {
        let sr = 44100.0;
        let mut f = LadderFilter::default();
        f.set_cutoff(440.0);
        f.set_resonance(1.0);

        let mut first_energy = 0.0f32;
        let mut last_energy = 0.0f32;
        let mut max_abs = 0.0f32;
        for i in 0..60000 {
            let out = process(&mut f, WideF32::ZERO, WideF32::splat(60.0), sr);
            let val = out.to_array()[0];
            assert!(val.is_finite(), "NaN/Inf at i={i}");
            max_abs = max_abs.max(val.abs());
            if i < 2048 {
                first_energy += val * val;
            } else if i >= 57952 {
                last_energy += val * val;
            }
        }

        assert!(
            last_energy > first_energy * 100.0 && last_energy > 1.0e-8,
            "self-oscillation should grow from seeded silence: first={first_energy:.12} last={last_energy:.12}"
        );
        assert!(
            max_abs < 5.0,
            "seeded self-oscillation blew up: {max_abs:.4}"
        );
        assert!(
            max_abs > 0.25,
            "seeded self-oscillation should reach an audible level: {max_abs:.4}"
        );
    }

    #[test]
    fn test_max_resonance_self_oscillation_harmonics_stay_subtle() {
        let sr = 44100.0;
        let mut f = LadderFilter::default();
        f.set_cutoff(440.0);
        f.set_resonance(1.0);

        let mut samples = Vec::with_capacity(70_000);
        for _ in 0..70_000 {
            samples.push(process(&mut f, WideF32::ZERO, WideF32::splat(60.0), sr).to_array()[0]);
        }
        let tail = &samples[50_000..];
        let fundamental_hz = estimate_frequency_from_positive_crossings(tail, sr);
        let fundamental = measure_projected_component(tail, fundamental_hz, sr);
        let second = measure_projected_component(tail, fundamental_hz * 2.0, sr);
        let third = measure_projected_component(tail, fundamental_hz * 3.0, sr);
        let strongest_harmonic = second.max(third);
        let ratio = strongest_harmonic / fundamental.max(1.0e-9);

        assert!(
            ratio < 0.02,
            "self-oscillation harmonics should stay subtle: fundamental_hz={fundamental_hz:.2} fundamental={fundamental:.4} second={second:.4} third={third:.4} ratio={ratio:.3}"
        );
    }

    #[test]
    fn test_below_self_oscillation_threshold_decays() {
        let sr = 44100.0;
        let mut f = LadderFilter::default();
        f.set_cutoff(440.0);
        f.set_resonance(SELF_OSC_RESONANCE_START - 0.02);

        for _ in 0..10 {
            let _ = process(&mut f, WideF32::splat(0.5), WideF32::splat(60.0), sr);
        }

        let mut first_energy = 0.0f32;
        let mut last_energy = 0.0f32;
        for i in 0..12000 {
            let out = process(&mut f, WideF32::ZERO, WideF32::splat(60.0), sr);
            let val = out.to_array()[0];
            assert!(val.is_finite(), "NaN/Inf at i={i}");
            if i < 1024 {
                first_energy += val * val;
            } else if i >= 10976 {
                last_energy += val * val;
            }
        }

        assert!(
            last_energy < first_energy,
            "below self-oscillation threshold should decay: first={first_energy:.6} last={last_energy:.6}"
        );
    }

    #[test]
    fn test_self_oscillation_spans_wide_resonance_range_and_level_rises() {
        let sr = 44100.0;
        let cutoff = 440.0;
        let below =
            measure_self_oscillation_tail_energy(SELF_OSC_RESONANCE_START - 0.02, cutoff, sr);
        let onset =
            measure_self_oscillation_tail_energy(SELF_OSC_RESONANCE_START + 0.02, cutoff, sr);
        let mid = measure_self_oscillation_tail_energy(0.85, cutoff, sr);
        let max = measure_self_oscillation_tail_energy(1.0, cutoff, sr);

        assert!(
            onset > below * 2.0,
            "self-oscillation should begin soon after resonance start {SELF_OSC_RESONANCE_START:.2}: below={below:.6} onset={onset:.6}"
        );
        assert!(
            mid > onset * 1.5 && max > mid * 1.5,
            "self-oscillation level should rise across the resonance range: onset={onset:.6} mid={mid:.6} max={max:.6}"
        );
    }

    #[test]
    #[cfg(not(feature = "wide-1"))]
    fn test_simd_lanes_equal() {
        let mut f = LadderFilter::default();
        f.set_cutoff(500.0);
        f.set_resonance(0.3);
        let sr = 44100.0;
        for _ in 0..1000 {
            let input = WideF32::splat(0.5);
            let out = process(&mut f, input, WideF32::splat(60.0), sr);
            let arr = out.to_array();
            for i in 1..4 {
                assert!(
                    (arr[i] - arr[0]).abs() < 1e-5,
                    "SIMD lane {i} diverged: {} vs {}",
                    arr[i],
                    arr[0]
                );
            }
        }
    }

    #[test]
    fn test_output_stays_bounded() {
        let mut f = LadderFilter::default();
        let sr = 44100.0;
        let mut phase = 0.0f32;
        for i in 0..10000 {
            let freq = if i < 2500 {
                100.0
            } else if i < 5000 {
                1000.0
            } else if i < 7500 {
                5000.0
            } else {
                10000.0
            };
            let res = if i < 2500 {
                0.0
            } else if i < 5000 {
                0.5
            } else if i < 7500 {
                0.9
            } else {
                1.0
            };
            let input = WideF32::splat(phase.sin());
            f.set_cutoff(2000.0);
            f.set_resonance(res);
            let out = process(&mut f, input, WideF32::splat(60.0), sr);
            let val = out.to_array()[0];
            assert!(val.is_finite(), "NaN/Inf at i={i} res={res} freq={freq}");
            assert!(
                val.abs() < 30.0,
                "output exploded to {val} at i={i} res={res} freq={freq}"
            );
            phase += 2.0 * std::f32::consts::PI * freq / sr;
        }
    }

    #[test]
    fn test_high_cutoff_high_resonance_stays_bounded() {
        let mut f = LadderFilter::default();
        let sr = 44100.0;
        let mut phase = 0.0f32;
        f.set_cutoff(18000.0);
        f.set_resonance(1.0);

        let mut max_abs = 0.0f32;
        for i in 0..20000 {
            let input = WideF32::splat((phase.sin() * 0.5) + ((phase * 0.37).sin() * 0.25));
            let out = process(&mut f, input, WideF32::splat(84.0), sr);
            let val = out.to_array()[0];
            assert!(val.is_finite(), "NaN/Inf at i={i}");
            max_abs = max_abs.max(val.abs());
            phase += 2.0 * std::f32::consts::PI * 8000.0 / sr;
        }

        assert!(
            max_abs < 5.0,
            "high cutoff/high resonance should stay bounded, peak {max_abs:.4}"
        );
    }

    #[test]
    fn test_filter_oversampling_auto_resolution() {
        assert_eq!(FilterOversampling::Off.factor(44_100.0), 1);
        assert_eq!(FilterOversampling::Auto.factor(44_100.0), 4);
        assert_eq!(FilterOversampling::Auto.factor(48_000.0), 4);
        assert_eq!(FilterOversampling::Auto.factor(96_000.0), 2);
        assert_eq!(FilterOversampling::Auto.factor(192_000.0), 1);
        assert_eq!(FilterOversampling::X2.factor(192_000.0), 2);
        assert_eq!(FilterOversampling::X4.factor(192_000.0), 4);
    }

    #[test]
    fn test_filter_oversampling_does_not_affect_two_pole_mode() {
        let sr = 44100.0;
        let mut off = LadderFilter::default();
        let mut x4 = LadderFilter::default();
        off.set_oversampling(FilterOversampling::Off);
        x4.set_oversampling(FilterOversampling::X4);
        off.set_poles(2);
        x4.set_poles(2);
        off.set_cutoff(1800.0);
        x4.set_cutoff(1800.0);
        off.set_resonance(1.0);
        x4.set_resonance(1.0);

        let mut phase = 0.0f32;
        let mut max_diff = 0.0f32;
        for _ in 0..5000 {
            let input = WideF32::splat(phase.sin() * 0.25);
            let off_out = process(&mut off, input, WideF32::splat(60.0), sr).to_array()[0];
            let x4_out = process(&mut x4, input, WideF32::splat(60.0), sr).to_array()[0];
            max_diff = max_diff.max((off_out - x4_out).abs());
            phase += std::f32::consts::TAU * 330.0 / sr;
        }

        assert!(
            max_diff < 1.0e-6,
            "oversampling should not alter two-pole processing: max_diff={max_diff:.8}"
        );
    }

    #[test]
    fn test_filter_oversampling_does_not_affect_four_pole_below_self_oscillation() {
        let sr = 44100.0;
        let mut off = LadderFilter::default();
        let mut x4 = LadderFilter::default();
        off.set_oversampling(FilterOversampling::Off);
        x4.set_oversampling(FilterOversampling::X4);
        off.set_cutoff(1800.0);
        x4.set_cutoff(1800.0);
        off.set_resonance(SELF_OSC_RESONANCE_START - 0.02);
        x4.set_resonance(SELF_OSC_RESONANCE_START - 0.02);

        let mut phase = 0.0f32;
        let mut max_diff = 0.0f32;
        for _ in 0..5000 {
            let input = WideF32::splat(phase.sin() * 0.25);
            let off_out = process(&mut off, input, WideF32::splat(60.0), sr).to_array()[0];
            let x4_out = process(&mut x4, input, WideF32::splat(60.0), sr).to_array()[0];
            max_diff = max_diff.max((off_out - x4_out).abs());
            phase += std::f32::consts::TAU * 330.0 / sr;
        }

        assert!(
            max_diff < 1.0e-6,
            "oversampling should not alter four-pole processing below self-oscillation: max_diff={max_diff:.8}"
        );
    }

    #[test]
    fn test_filter_oversampling_reduces_high_cutoff_foldback() {
        let sr = 44100.0;
        let cutoff = 9000.0;
        let render = |mode: FilterOversampling| {
            let mut f = LadderFilter::default();
            f.set_oversampling(mode);
            f.set_cutoff(cutoff);
            f.set_resonance(1.0);
            let mut samples = Vec::with_capacity(90_000);
            for _ in 0..90_000 {
                samples
                    .push(process(&mut f, WideF32::ZERO, WideF32::splat(60.0), sr).to_array()[0]);
            }
            samples
        };

        let off = render(FilterOversampling::Off);
        let x4 = render(FilterOversampling::X4);
        let off_tail = &off[50_000..];
        let x4_tail = &x4[50_000..];
        let off_fundamental_hz = estimate_frequency_from_positive_crossings(off_tail, sr);
        let x4_fundamental_hz = estimate_frequency_from_positive_crossings(x4_tail, sr);
        let off_folded_third_hz = fold_frequency(off_fundamental_hz * 3.0, sr);
        let x4_folded_third_hz = fold_frequency(x4_fundamental_hz * 3.0, sr);
        let off_fundamental = measure_projected_component(off_tail, off_fundamental_hz, sr);
        let x4_fundamental = measure_projected_component(x4_tail, x4_fundamental_hz, sr);
        let off_alias = measure_projected_component(off_tail, off_folded_third_hz, sr);
        let x4_alias = measure_projected_component(x4_tail, x4_folded_third_hz, sr);
        let off_ratio = off_alias / off_fundamental.max(1.0e-9);
        let x4_ratio = x4_alias / x4_fundamental.max(1.0e-9);

        assert!(
            x4_fundamental > off_fundamental * 0.25,
            "4x oversampling should preserve the main self-oscillation component: off={off_fundamental:.4} x4={x4_fundamental:.4}"
        );
        assert!(
            x4_ratio < off_ratio * 0.75,
            "4x oversampling should reduce folded third-harmonic energy: off_ratio={off_ratio:.4} x4_ratio={x4_ratio:.4} off_f={off_fundamental_hz:.1}Hz x4_f={x4_fundamental_hz:.1}Hz"
        );
    }

    #[test]
    #[cfg(feature = "wide-4")]
    fn runtime_wrapper_stays_no_larger_than_the_legacy_filter() {
        assert!(core::mem::size_of::<DistributedNewtonTpt>() <= 128);
        assert!(core::mem::size_of::<ScalarFeedbackTpt>() <= 128);
        assert!(core::mem::size_of::<GainLimitedTpt>() <= 128);
        assert!(core::mem::size_of::<HuovilainenLadder>() <= 160);
        assert!(core::mem::size_of::<CascadedTptSvf>() <= 160);
        assert!(core::mem::size_of::<Filter>() <= 208);
    }
}
