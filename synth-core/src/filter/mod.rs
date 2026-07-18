//! Heapless runtime-selectable low-pass filter models.

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
mod cascaded_tpt_svf;
mod coefficient_math;
#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
mod distributed_newton_tpt;
mod gain_limited_tpt;
#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
mod huovilainen_ladder;
#[cfg(any(test, all(feature = "embedded-math", target_os = "none")))]
mod prewarp_table;
mod resonance_math;
#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
mod scalar_feedback_tpt;

use crate::f32x4;

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
use cascaded_tpt_svf::CascadedTptSvf;
#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
use distributed_newton_tpt::DistributedNewtonTpt;
use gain_limited_tpt::GainLimitedTpt;
#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
use huovilainen_ladder::HuovilainenLadder;
#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
use scalar_feedback_tpt::ScalarFeedbackTpt;

/// Lowest cutoff accepted by every filter model.
pub(crate) const MIN_CUTOFF_HZ: f32 = 20.0;
/// Highest cutoff accepted by every filter model.
pub(crate) const MAX_CUTOFF_HZ: f32 = 18_000.0;
/// Full filter-envelope modulation depth in semitones.
const ENV_DEPTH_SEMITONES: f32 = 96.0;
/// Full audio-rate filter modulation depth in semitones.
const AUDIO_MOD_DEPTH_SEMITONES: f32 = 48.0;
/// MIDI note that produces zero semitones of filter keyboard tracking.
const KEY_TRACK_REFERENCE_NOTE: f32 = 36.0;
/// Exponent applied to the public resonance control before model calibration.
#[cfg(any(test, not(all(feature = "embedded-math", target_os = "none"))))]
const RESONANCE_CONTROL_EXPONENT: f32 = 1.75;

/// Public resonance value where 4-pole nonlinear self-oscillation begins.
pub const SELF_OSC_RESONANCE_START: f32 = 0.71;
/// Baseline self-oscillation pitch trim, in cents.
pub const SELF_OSC_PITCH_TUNING_CENTS: f32 = 133.0;

/// Runtime-selectable filter model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum FilterType {
    /// Existing Rev2-inspired distributed-Newton TPT model.
    #[default]
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

impl FilterType {
    pub const ALL: [Self; 5] = [
        Self::DistributedNewtonTpt,
        Self::ScalarFeedbackTpt,
        Self::GainLimitedTpt,
        Self::HuovilainenLadder,
        Self::CascadedTptSvf,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::DistributedNewtonTpt => "Distributed Newton TPT",
            Self::ScalarFeedbackTpt => "Scalar Feedback TPT",
            Self::GainLimitedTpt => "Gain-Limited TPT",
            Self::HuovilainenLadder => "Huovilainen Ladder",
            Self::CascadedTptSvf => "Cascaded TPT SVF",
        }
    }

    /// Whether this model has its own implementation in the current phase.
    pub const fn is_implemented(self) -> bool {
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
    pub input: f32x4,
    pub cutoff_hz: f32,
    pub cutoff_mod_semitones: f32x4,
    pub cutoff_mod_uniform_semitones: Option<f32>,
    pub resonance_control: f32x4,
    pub shaped_resonance: f32x4,
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
    fn process(&mut self, frame: FilterFrame) -> f32x4;
}

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
enum FilterAlgorithmState {
    DistributedNewtonTpt(DistributedNewtonTpt),
    ScalarFeedbackTpt(ScalarFeedbackTpt),
    GainLimitedTpt(GainLimitedTpt),
    HuovilainenLadder(HuovilainenLadder),
    CascadedTptSvf(CascadedTptSvf),
}

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
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

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
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

#[cfg(not(all(feature = "embedded-math", target_os = "none")))]
impl FilterAlgorithmState {
    fn new(filter_type: FilterType) -> Self {
        match filter_type {
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
    fn process(&mut self, frame: FilterFrame) -> f32x4 {
        with_filter_algorithm_mut!(self, algorithm => algorithm.process(frame))
    }
}

/// Daisy production filter bank. The public control surface is retained, but
/// the hot path contains only the firmware's selected Gain-Limited TPT model.
#[cfg(all(feature = "embedded-math", target_os = "none"))]
struct FilterAlgorithmState(GainLimitedTpt);

#[cfg(all(feature = "embedded-math", target_os = "none"))]
impl FilterAlgorithmState {
    fn new(_filter_type: FilterType) -> Self {
        Self(GainLimitedTpt::default())
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
    fn process(&mut self, frame: FilterFrame) -> f32x4 {
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
            // Later model variants intentionally use the bit-identical baseline
            // until their sequential implementation phase lands.
            algorithm: FilterAlgorithmState::new(filter_type),
        }
    }

    pub const fn filter_type(&self) -> FilterType {
        self.filter_type
    }

    /// Selects a fresh model state while retaining all common controls.
    pub fn set_filter_type(&mut self, filter_type: FilterType) {
        if self.filter_type == filter_type {
            return;
        }
        self.filter_type = filter_type;
        self.algorithm = FilterAlgorithmState::new(filter_type);
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = cutoff.clamp(MIN_CUTOFF_HZ, MAX_CUTOFF_HZ);
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
        self.key_track = key_track.clamp(0.0, 1.0);
    }

    pub fn set_env_amount(&mut self, env_amount: f32) {
        self.env_amount = env_amount.clamp(-1.0, 1.0);
    }

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
        input: f32x4,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        resonance_mod: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
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
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn process_prepared(
        &mut self,
        input: f32x4,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        cutoff_mod_uniform_semitones: Option<f32>,
        resonance_mod: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
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
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn process_inner(
        &mut self,
        input: f32x4,
        note: f32x4,
        filter_env: f32x4,
        velocity: f32x4,
        osc1_audio: f32x4,
        cutoff_mod_semitones: f32x4,
        cutoff_mod_uniform_semitones: Option<f32>,
        resonance_mod: f32x4,
        audio_mod: f32x4,
        sample_rate: f32,
    ) -> f32x4 {
        let resonance_control = (f32x4::splat(self.resonance) + resonance_mod)
            .clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let shaped_resonance = if resonance_mod == f32x4::ZERO {
            f32x4::splat(self.shaped_resonance)
        } else {
            shape_resonance_control(resonance_control)
        };
        let key_semitones =
            (note - f32x4::splat(KEY_TRACK_REFERENCE_NOTE)) * f32x4::splat(self.key_track);
        let velocity_scale = f32x4::splat(1.0 - self.env_velocity_amount)
            + velocity.clamp(f32x4::splat(0.0), f32x4::splat(1.0))
                * f32x4::splat(self.env_velocity_amount);
        let env_semitones = filter_env.clamp(f32x4::splat(0.0), f32x4::splat(1.0))
            * velocity_scale
            * f32x4::splat(self.env_amount * ENV_DEPTH_SEMITONES);
        let audio_mod_amount =
            (f32x4::splat(self.audio_mod) + audio_mod).clamp(f32x4::splat(0.0), f32x4::splat(1.0));
        let audio_semitones = osc1_audio.clamp(f32x4::splat(-1.0), f32x4::splat(1.0))
            * audio_mod_amount
            * f32x4::splat(AUDIO_MOD_DEPTH_SEMITONES);
        let cutoff_mod_semitones =
            key_semitones + env_semitones + audio_semitones + cutoff_mod_semitones;
        let cutoff_mod_uniform_semitones = cutoff_mod_uniform_semitones.filter(|_| {
            self.key_track == 0.0
                && self.env_amount == 0.0
                && self.audio_mod == 0.0
                && all_lanes_near_zero(audio_mod)
        });
        let static_cutoff = self.key_track == 0.0
            && self.env_amount == 0.0
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

/// Compatibility alias for existing callers.
pub type LadderFilter = Filter;

fn shape_resonance_control(value: f32x4) -> f32x4 {
    let mut values = value.to_array();
    for value in &mut values {
        *value = shape_resonance_scalar(*value);
    }
    f32x4::new(values)
}

fn shape_resonance_scalar(value: f32) -> f32 {
    resonance_math::shape(value.clamp(0.0, 1.0))
}

fn all_lanes_near_zero(value: f32x4) -> bool {
    value.abs().simd_lt(f32x4::splat(f32::EPSILON)).all()
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn runtime_wrapper_stays_no_larger_than_the_legacy_filter() {
        assert!(core::mem::size_of::<DistributedNewtonTpt>() <= 128);
        assert!(core::mem::size_of::<ScalarFeedbackTpt>() <= 128);
        assert!(core::mem::size_of::<GainLimitedTpt>() <= 128);
        assert!(core::mem::size_of::<HuovilainenLadder>() <= 160);
        assert!(core::mem::size_of::<CascadedTptSvf>() <= 160);
        assert!(core::mem::size_of::<Filter>() <= 208);
    }
}
