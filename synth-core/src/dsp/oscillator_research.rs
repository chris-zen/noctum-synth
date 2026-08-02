//! Feature-gated oscillator research registry, scalar adapter, and case runner.
//!
//! This module is deliberately independent of patches and voice allocation. It
//! gives offline tools and Oscillator Lab one stable semantic interface while
//! the production oscillator remains statically selected.

use core::mem::size_of;

use super::analog_oscillator::{AnalogOscillator, EngineOscillator};
use super::live_wavetable::LiveWavetable;
use super::target_conditioned_oscillator::{
    PARAMETERS as TARGET_CONDITIONED_PARAMETERS, TargetConditionedOscillator,
};
use super::target_conditioned_profile::{KORG_MONOLOGUE_PHASE_FILTER_V1, PROFILE_JSON_SHA256};
use super::target_conditioned_profile_v2::{
    KORG_MONOLOGUE_PHASE_FILTER_V2, PROFILE_JSON_SHA256_V2,
};
use super::wavetable_bank::WavetableBank;
use super::wavetable_bank_profile::MONOLOGUE_WAVETABLE_BANK_PROFILE;
use super::wavetable_bank_profile_prophet5::PROPHET5_WAVETABLE_BANK_PROFILE;
use super::{MipWavetableBank, SawMethod, WAVETABLE_BANK_SAMPLES, Waveform, WavetableOscillator};
use crate::math::WideF32;

/// Stable built-in model identifiers used by research artifacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResearchModelId {
    Baseline,
    TableBlep,
    PolyBlep,
    Wavetable,
    WavetableMonologue,
    WavetableProphet5,
    TargetConditioned,
    TargetConditionedV2,
}

impl ResearchModelId {
    pub const ALL: [Self; 8] = [
        Self::Baseline,
        Self::TableBlep,
        Self::PolyBlep,
        Self::Wavetable,
        Self::WavetableMonologue,
        Self::WavetableProphet5,
        Self::TargetConditioned,
        Self::TargetConditionedV2,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline-v1",
            Self::TableBlep => "table-blep-v1",
            Self::PolyBlep => "polyblep-v1",
            Self::Wavetable => "wavetable-prototype-v1",
            Self::WavetableMonologue => "korg-monologue-measured-wavetable-v1",
            Self::WavetableProphet5 => "prophet5-wavetable-v1",
            Self::TargetConditioned => "target-conditioned-phase-filter-v1",
            Self::TargetConditionedV2 => "target-conditioned-phase-filter-v2",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }
}

/// Broad implementation family; stateful models need not expose phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchModelFamily {
    PhaseKernel,
    Stateful,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResearchModelCapabilities {
    pub saw: bool,
    pub saw_triangle: bool,
    pub triangle: bool,
    pub pulse: bool,
    pub shape: bool,
    pub audio_rate_pwm: bool,
    pub hard_sync: bool,
    pub note_reset: bool,
    pub slop: bool,
    pub simd_lanes: bool,
    pub real_time_safe: bool,
}

/// Immutable model metadata shared by offline tools, Oscillator Lab, and live
/// selection. Mutable model parameters are intentionally not shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResearchModelDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub revision: u32,
    pub family: ResearchModelFamily,
    pub capabilities: ResearchModelCapabilities,
    pub requires_external_asset: bool,
    pub mutable_state_bytes: usize,
    pub immutable_asset_bytes: usize,
    pub latency_samples: u32,
    pub bounded_render_cost: bool,
    pub no_std_compatible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchParameterScale {
    Linear,
    Logarithmic,
}

/// Stable metadata for one model-owned parameter. Parameters remain outside
/// patches, `ParamId`, MIDI, and SysEx during research.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResearchParameterDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub unit: &'static str,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
    pub scale: ResearchParameterScale,
}

/// A deterministic scalar render request shared by all oscillator families.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResearchRenderCase {
    pub waveform: Waveform,
    pub sample_rate_hz: f32,
    pub frequency_hz: f32,
    pub shape: f32,
    pub warmup_samples: usize,
    pub render_samples: usize,
    pub seed: u64,
    pub reset_phase: bool,
}

impl ResearchRenderCase {
    pub fn validate(self) -> Result<Self, ResearchError> {
        if !self.sample_rate_hz.is_finite() || self.sample_rate_hz <= 0.0 {
            return Err(ResearchError::InvalidSampleRate);
        }
        if !self.frequency_hz.is_finite()
            || self.frequency_hz <= 0.0
            || self.frequency_hz >= self.sample_rate_hz * 0.49
        {
            return Err(ResearchError::InvalidFrequency);
        }
        if !self.shape.is_finite() || !(0.0..=1.0).contains(&self.shape) {
            return Err(ResearchError::InvalidShape);
        }
        if self.render_samples == 0 {
            return Err(ResearchError::EmptyRender);
        }
        Ok(self)
    }
}

/// Semantic events understood by research models without exposing internal
/// phase, integrator, comparator, or solver state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResearchEvent {
    Reset { reset_phase: bool },
    SetFrequency(f32),
    SetShape(f32),
    HardSync { subsample_offset: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResearchError {
    UnknownModel,
    MissingMipWavetableBank,
    MissingWavetableBank,
    UnexpectedMipWavetableBank,
    UnexpectedWavetableBank,
    InvalidSampleRate,
    InvalidFrequency,
    InvalidShape,
    EmptyRender,
    WrongOutputLength,
    NonFiniteOutput { sample_index: usize },
    UnsupportedEvent,
    ComparisonLengthMismatch,
    UnknownParameter,
    InvalidParameterValue,
}

/// Common interface implemented by both phase kernels and fully stateful
/// desktop research models.
pub trait OscillatorResearchModel {
    fn descriptor(&self) -> ResearchModelDescriptor;
    fn configure(&mut self, case: ResearchRenderCase) -> Result<(), ResearchError>;
    fn reset(&mut self, reset_phase: bool);
    fn apply_event(&mut self, event: ResearchEvent) -> Result<(), ResearchError>;
    fn next_sample(&mut self) -> f32;

    fn parameter_descriptors(&self) -> &'static [ResearchParameterDescriptor] {
        &[]
    }

    fn set_parameter(&mut self, _id: &str, _value: f32) -> Result<(), ResearchError> {
        Err(ResearchError::UnknownParameter)
    }

    fn parameter_value(&self, _id: &str) -> Option<f32> {
        None
    }
}

enum RegisteredSource {
    Baseline(EngineOscillator),
    TableBlep(AnalogOscillator),
    PolyBlep(AnalogOscillator),
    Wavetable(WavetableOscillator),
    PitchWavetable(LiveWavetable),
    TargetConditioned(TargetConditionedOscillator),
}

/// Built-in adapter used by the first comparison harness. New stateful models
/// can implement [`OscillatorResearchModel`] without modifying production
/// voice code.
pub struct RegisteredResearchModel {
    id: ResearchModelId,
    source: RegisteredSource,
    sample_rate_hz: f32,
    mip_wavetable_bank: Option<MipWavetableBank>,
    wavetable_bank: Option<WavetableBank>,
    configured_case: Option<ResearchRenderCase>,
    target_phase_amount: f32,
    target_filter_amount: f32,
}

macro_rules! with_source_mut {
    ($source:expr, $oscillator:ident => $body:expr) => {
        match $source {
            RegisteredSource::Baseline($oscillator) => $body,
            RegisteredSource::TableBlep($oscillator) | RegisteredSource::PolyBlep($oscillator) => {
                $body
            }
            RegisteredSource::Wavetable($oscillator) => $body,
            RegisteredSource::PitchWavetable(_) => {
                unreachable!("pitch wavetable source uses its scalar adapter")
            }
            RegisteredSource::TargetConditioned(_) => {
                unreachable!("target-conditioned source uses its scalar adapter")
            }
        }
    };
}

impl RegisteredResearchModel {
    fn create_source(
        id: ResearchModelId,
        sample_rate_hz: f32,
        mip_wavetable_bank: Option<MipWavetableBank>,
        wavetable_bank: Option<WavetableBank>,
    ) -> Result<RegisteredSource, ResearchError> {
        if id != ResearchModelId::Wavetable && mip_wavetable_bank.is_some() {
            return Err(ResearchError::UnexpectedMipWavetableBank);
        }
        if !matches!(
            id,
            ResearchModelId::WavetableMonologue | ResearchModelId::WavetableProphet5
        ) && wavetable_bank.is_some()
        {
            return Err(ResearchError::UnexpectedWavetableBank);
        }
        match id {
            ResearchModelId::Baseline => Ok(RegisteredSource::Baseline(
                EngineOscillator::new_engine(sample_rate_hz),
            )),
            ResearchModelId::TableBlep | ResearchModelId::PolyBlep => {
                let mut oscillator = AnalogOscillator::new(sample_rate_hz);
                if id == ResearchModelId::PolyBlep {
                    oscillator.set_saw_method(SawMethod::PolyBlep);
                    Ok(RegisteredSource::PolyBlep(oscillator))
                } else {
                    oscillator.set_saw_method(SawMethod::Blep);
                    Ok(RegisteredSource::TableBlep(oscillator))
                }
            }
            ResearchModelId::Wavetable => {
                let bank = mip_wavetable_bank.ok_or(ResearchError::MissingMipWavetableBank)?;
                Ok(RegisteredSource::Wavetable(
                    WavetableOscillator::new_wavetable(sample_rate_hz, bank),
                ))
            }
            ResearchModelId::WavetableMonologue | ResearchModelId::WavetableProphet5 => {
                let bank = wavetable_bank.ok_or(ResearchError::MissingWavetableBank)?;
                Ok(RegisteredSource::PitchWavetable(LiveWavetable::new(
                    bank,
                    sample_rate_hz,
                )))
            }
            ResearchModelId::TargetConditioned | ResearchModelId::TargetConditionedV2 => {
                let profile = if id == ResearchModelId::TargetConditionedV2 {
                    &KORG_MONOLOGUE_PHASE_FILTER_V2
                } else {
                    &KORG_MONOLOGUE_PHASE_FILTER_V1
                };
                Ok(RegisteredSource::TargetConditioned(
                    TargetConditionedOscillator::new(profile, sample_rate_hz),
                ))
            }
        }
    }

    fn rebuild(&mut self) {
        self.source = Self::create_source(
            self.id,
            self.sample_rate_hz,
            self.mip_wavetable_bank,
            self.wavetable_bank,
        )
        .expect("validated research model assets must remain available");
        self.apply_target_parameters();
    }

    fn apply_target_parameters(&mut self) {
        if let RegisteredSource::TargetConditioned(oscillator) = &mut self.source {
            oscillator
                .set_parameter("phase-amount", self.target_phase_amount)
                .expect("stored research parameter is validated");
            oscillator
                .set_parameter("filter-amount", self.target_filter_amount)
                .expect("stored research parameter is validated");
        }
    }

    fn configure_source(&mut self, case: ResearchRenderCase) -> Result<(), ResearchError> {
        if let RegisteredSource::TargetConditioned(oscillator) = &mut self.source {
            return oscillator.configure(case);
        }
        if let RegisteredSource::PitchWavetable(oscillator) = &mut self.source {
            if case.sample_rate_hz < 43_200.0 {
                return Err(ResearchError::UnsupportedEvent);
            }
            oscillator.set_waveform(case.waveform);
            oscillator.set_shape(case.shape);
            oscillator.set_frequency(WideF32::splat(case.frequency_hz));
            if case.reset_phase {
                for lane in 0..WideF32::LANES {
                    oscillator.trigger_lane(lane, true);
                }
            }
            return if oscillator.uses_measured_tables() {
                Ok(())
            } else {
                Err(ResearchError::UnsupportedEvent)
            };
        }
        with_source_mut!(&mut self.source, oscillator => {
            oscillator.set_waveform(case.waveform);
            oscillator.set_shape(case.shape);
            oscillator.set_frequency(WideF32::splat(case.frequency_hz));
            oscillator.set_enabled(true);
            oscillator.set_slop_amount(0.0);
            if case.reset_phase {
                for lane in 0..WideF32::LANES {
                    oscillator.start_phase_lane(lane);
                }
            }
        });
        Ok(())
    }
}

impl OscillatorResearchModel for RegisteredResearchModel {
    fn descriptor(&self) -> ResearchModelDescriptor {
        ResearchRegistry::descriptor(self.id)
    }

    fn configure(&mut self, case: ResearchRenderCase) -> Result<(), ResearchError> {
        let case = case.validate()?;
        self.sample_rate_hz = case.sample_rate_hz;
        // Every case is isolated. Reusing a model instance must produce the
        // same result as constructing a fresh instance with the same case.
        self.rebuild();
        self.configure_source(case)?;
        self.configured_case = Some(case);
        Ok(())
    }

    fn reset(&mut self, reset_phase: bool) {
        if let RegisteredSource::PitchWavetable(oscillator) = &mut self.source {
            for lane in 0..WideF32::LANES {
                oscillator.trigger_lane(lane, reset_phase);
            }
            return;
        }
        self.rebuild();
        if let Some(mut case) = self.configured_case {
            case.reset_phase = reset_phase;
            self.configure_source(case)
                .expect("previously validated research case must remain valid");
            self.configured_case = Some(case);
        }
    }

    fn apply_event(&mut self, event: ResearchEvent) -> Result<(), ResearchError> {
        match event {
            ResearchEvent::Reset { reset_phase } => self.reset(reset_phase),
            ResearchEvent::SetFrequency(frequency_hz) => {
                if !frequency_hz.is_finite()
                    || frequency_hz <= 0.0
                    || frequency_hz >= self.sample_rate_hz * 0.49
                {
                    return Err(ResearchError::InvalidFrequency);
                }
                match &mut self.source {
                    RegisteredSource::TargetConditioned(oscillator) => {
                        oscillator.set_frequency(frequency_hz)
                    }
                    RegisteredSource::PitchWavetable(oscillator) => {
                        oscillator.set_frequency(WideF32::splat(frequency_hz));
                        if !oscillator.uses_measured_tables() {
                            return Err(ResearchError::UnsupportedEvent);
                        }
                    }
                    source => with_source_mut!(source, oscillator => {
                        oscillator.set_frequency(WideF32::splat(frequency_hz));
                    }),
                }
                if let Some(case) = self.configured_case.as_mut() {
                    case.frequency_hz = frequency_hz;
                }
            }
            ResearchEvent::SetShape(shape) => {
                if !shape.is_finite() || !(0.0..=1.0).contains(&shape) {
                    return Err(ResearchError::InvalidShape);
                }
                match &mut self.source {
                    RegisteredSource::TargetConditioned(oscillator) => oscillator.set_shape(shape),
                    RegisteredSource::PitchWavetable(oscillator) => oscillator.set_shape(shape),
                    source => with_source_mut!(source, oscillator => oscillator.set_shape(shape)),
                }
                if let Some(case) = self.configured_case.as_mut() {
                    case.shape = shape;
                }
            }
            ResearchEvent::HardSync { subsample_offset } => {
                if !subsample_offset.is_finite() || !(0.0..=1.0).contains(&subsample_offset) {
                    return Err(ResearchError::UnsupportedEvent);
                }
                match &mut self.source {
                    RegisteredSource::TargetConditioned(oscillator) => {
                        oscillator.hard_sync(subsample_offset)
                    }
                    RegisteredSource::PitchWavetable(oscillator) => {
                        oscillator.hard_sync_reset(
                            WideF32::splat(1.0).simd_gt(WideF32::ZERO),
                            WideF32::splat(subsample_offset),
                        );
                    }
                    source => with_source_mut!(source, oscillator => {
                        oscillator.hard_sync_reset(
                            WideF32::splat(1.0).simd_gt(WideF32::ZERO),
                            WideF32::splat(subsample_offset),
                        );
                    }),
                }
            }
        }
        Ok(())
    }

    fn next_sample(&mut self) -> f32 {
        if let RegisteredSource::TargetConditioned(oscillator) = &mut self.source {
            return oscillator.next_sample();
        }
        if let RegisteredSource::PitchWavetable(oscillator) = &mut self.source {
            let mut context = crate::create_render_context!();
            return oscillator.next(&mut context).output.to_array()[0];
        }
        let mut context = crate::create_render_context!();
        with_source_mut!(&mut self.source, oscillator => {
            oscillator.next(&mut context).output.to_array()[0]
        })
    }

    fn parameter_descriptors(&self) -> &'static [ResearchParameterDescriptor] {
        match self.source {
            RegisteredSource::TargetConditioned(_) => &TARGET_CONDITIONED_PARAMETERS,
            _ => &[],
        }
    }

    fn set_parameter(&mut self, id: &str, value: f32) -> Result<(), ResearchError> {
        match &mut self.source {
            RegisteredSource::TargetConditioned(oscillator) => {
                oscillator.set_parameter(id, value)?;
                match id {
                    "phase-amount" => self.target_phase_amount = value,
                    "filter-amount" => self.target_filter_amount = value,
                    _ => return Err(ResearchError::UnknownParameter),
                }
                Ok(())
            }
            _ => Err(ResearchError::UnknownParameter),
        }
    }

    fn parameter_value(&self, id: &str) -> Option<f32> {
        match &self.source {
            RegisteredSource::TargetConditioned(oscillator) => oscillator.parameter_value(id),
            _ => None,
        }
    }
}

/// Deterministic built-in registry. The production voice does not enumerate or
/// dispatch through this registry.
pub struct ResearchRegistry;

impl ResearchRegistry {
    pub fn descriptors() -> impl ExactSizeIterator<Item = ResearchModelDescriptor> {
        ResearchModelId::ALL.into_iter().map(Self::descriptor)
    }

    pub const fn descriptor(id: ResearchModelId) -> ResearchModelDescriptor {
        let analysis_only_capabilities = ResearchModelCapabilities {
            saw: true,
            saw_triangle: true,
            triangle: true,
            pulse: true,
            shape: true,
            audio_rate_pwm: true,
            hard_sync: true,
            note_reset: true,
            slop: false,
            simd_lanes: false,
            real_time_safe: false,
        };
        match id {
            ResearchModelId::Baseline => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Production Baseline",
                revision: 1,
                family: ResearchModelFamily::PhaseKernel,
                capabilities: ResearchModelCapabilities {
                    simd_lanes: true,
                    real_time_safe: true,
                    slop: true,
                    ..analysis_only_capabilities
                },
                requires_external_asset: false,
                mutable_state_bytes: size_of::<EngineOscillator>(),
                immutable_asset_bytes: 0,
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::TableBlep => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Table BLEP",
                revision: 1,
                family: ResearchModelFamily::PhaseKernel,
                capabilities: ResearchModelCapabilities {
                    simd_lanes: true,
                    real_time_safe: true,
                    slop: true,
                    ..analysis_only_capabilities
                },
                requires_external_asset: false,
                mutable_state_bytes: size_of::<AnalogOscillator>(),
                immutable_asset_bytes: 0,
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::PolyBlep => ResearchModelDescriptor {
                id: id.as_str(),
                name: "PolyBLEP / PolyBLAMP",
                revision: 1,
                family: ResearchModelFamily::PhaseKernel,
                capabilities: ResearchModelCapabilities {
                    simd_lanes: true,
                    real_time_safe: true,
                    slop: true,
                    ..analysis_only_capabilities
                },
                requires_external_asset: false,
                mutable_state_bytes: size_of::<AnalogOscillator>(),
                immutable_asset_bytes: 0,
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::Wavetable => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Retained Wavetable Prototype",
                revision: 1,
                family: ResearchModelFamily::PhaseKernel,
                capabilities: analysis_only_capabilities,
                requires_external_asset: true,
                mutable_state_bytes: size_of::<WavetableOscillator>(),
                immutable_asset_bytes: WAVETABLE_BANK_SAMPLES * size_of::<f32>(),
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::WavetableMonologue => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Wavetable (Monologue)",
                revision: 1,
                family: ResearchModelFamily::PhaseKernel,
                capabilities: ResearchModelCapabilities {
                    real_time_safe: true,
                    slop: true,
                    ..analysis_only_capabilities
                },
                requires_external_asset: true,
                mutable_state_bytes: size_of::<LiveWavetable>(),
                immutable_asset_bytes: MONOLOGUE_WAVETABLE_BANK_PROFILE.sample_count
                    * size_of::<f32>(),
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::WavetableProphet5 => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Wavetable (Prophet-5 V)",
                revision: 1,
                family: ResearchModelFamily::PhaseKernel,
                capabilities: ResearchModelCapabilities {
                    real_time_safe: true,
                    slop: true,
                    ..analysis_only_capabilities
                },
                requires_external_asset: true,
                mutable_state_bytes: size_of::<LiveWavetable>(),
                immutable_asset_bytes: PROPHET5_WAVETABLE_BANK_PROFILE.sample_count
                    * size_of::<f32>(),
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::TargetConditioned => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Monologue Phase + Linear Color",
                revision: KORG_MONOLOGUE_PHASE_FILTER_V1.revision,
                family: ResearchModelFamily::Stateful,
                capabilities: ResearchModelCapabilities {
                    saw: true,
                    saw_triangle: false,
                    triangle: true,
                    pulse: true,
                    shape: true,
                    audio_rate_pwm: false,
                    hard_sync: true,
                    note_reset: true,
                    slop: false,
                    simd_lanes: false,
                    real_time_safe: false,
                },
                requires_external_asset: false,
                mutable_state_bytes: size_of::<TargetConditionedOscillator>(),
                immutable_asset_bytes: (KORG_MONOLOGUE_PHASE_FILTER_V1.saw.len()
                    + KORG_MONOLOGUE_PHASE_FILTER_V1.triangle.len()
                    + KORG_MONOLOGUE_PHASE_FILTER_V1.pulse.len())
                    * size_of::<super::target_conditioned_oscillator::PhaseFilterKnot>(),
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
            ResearchModelId::TargetConditionedV2 => ResearchModelDescriptor {
                id: id.as_str(),
                name: "Monologue Phase + Linear Color v2",
                revision: KORG_MONOLOGUE_PHASE_FILTER_V2.revision,
                family: ResearchModelFamily::Stateful,
                capabilities: ResearchModelCapabilities {
                    saw: true,
                    saw_triangle: false,
                    triangle: true,
                    pulse: true,
                    shape: true,
                    audio_rate_pwm: false,
                    hard_sync: true,
                    note_reset: true,
                    slop: false,
                    simd_lanes: false,
                    real_time_safe: false,
                },
                requires_external_asset: false,
                mutable_state_bytes: size_of::<TargetConditionedOscillator>(),
                immutable_asset_bytes: (KORG_MONOLOGUE_PHASE_FILTER_V2.saw.len()
                    + KORG_MONOLOGUE_PHASE_FILTER_V2.triangle.len()
                    + KORG_MONOLOGUE_PHASE_FILTER_V2.pulse.len())
                    * size_of::<super::target_conditioned_oscillator::PhaseFilterKnot>(),
                latency_samples: 0,
                bounded_render_cost: true,
                no_std_compatible: true,
            },
        }
    }

    /// Returns the fitted profile identity and target provenance for models
    /// whose behavior is conditioned by measured reference data.
    pub const fn target_profile_metadata(
        id: ResearchModelId,
    ) -> Option<(&'static str, &'static str, &'static str)> {
        match id {
            ResearchModelId::TargetConditioned => Some((
                KORG_MONOLOGUE_PHASE_FILTER_V1.id,
                KORG_MONOLOGUE_PHASE_FILTER_V1.target_id,
                PROFILE_JSON_SHA256,
            )),
            ResearchModelId::TargetConditionedV2 => Some((
                KORG_MONOLOGUE_PHASE_FILTER_V2.id,
                KORG_MONOLOGUE_PHASE_FILTER_V2.target_id,
                PROFILE_JSON_SHA256_V2,
            )),
            ResearchModelId::WavetableMonologue => Some((
                MONOLOGUE_WAVETABLE_BANK_PROFILE.id,
                MONOLOGUE_WAVETABLE_BANK_PROFILE.target_id,
                MONOLOGUE_WAVETABLE_BANK_PROFILE.manifest_sha256,
            )),
            ResearchModelId::WavetableProphet5 => Some((
                PROPHET5_WAVETABLE_BANK_PROFILE.id,
                PROPHET5_WAVETABLE_BANK_PROFILE.target_id,
                PROPHET5_WAVETABLE_BANK_PROFILE.manifest_sha256,
            )),
            _ => None,
        }
    }

    pub fn create(
        id: ResearchModelId,
        sample_rate_hz: f32,
        mip_wavetable_bank: Option<MipWavetableBank>,
    ) -> Result<RegisteredResearchModel, ResearchError> {
        if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
            return Err(ResearchError::InvalidSampleRate);
        }
        Ok(RegisteredResearchModel {
            id,
            source: RegisteredResearchModel::create_source(
                id,
                sample_rate_hz,
                mip_wavetable_bank,
                None,
            )?,
            sample_rate_hz,
            mip_wavetable_bank,
            wavetable_bank: None,
            configured_case: None,
            target_phase_amount: TARGET_CONDITIONED_PARAMETERS[0].default,
            target_filter_amount: TARGET_CONDITIONED_PARAMETERS[1].default,
        })
    }

    pub fn create_wavetable(
        sample_rate_hz: f32,
        bank: WavetableBank,
    ) -> Result<RegisteredResearchModel, ResearchError> {
        if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
            return Err(ResearchError::InvalidSampleRate);
        }
        let id = match bank.profile().id {
            id if id == MONOLOGUE_WAVETABLE_BANK_PROFILE.id => ResearchModelId::WavetableMonologue,
            id if id == PROPHET5_WAVETABLE_BANK_PROFILE.id => ResearchModelId::WavetableProphet5,
            _ => return Err(ResearchError::MissingWavetableBank),
        };
        Ok(RegisteredResearchModel {
            id,
            source: RegisteredResearchModel::create_source(id, sample_rate_hz, None, Some(bank))?,
            sample_rate_hz,
            mip_wavetable_bank: None,
            wavetable_bank: Some(bank),
            configured_case: None,
            target_phase_amount: TARGET_CONDITIONED_PARAMETERS[0].default,
            target_filter_amount: TARGET_CONDITIONED_PARAMETERS[1].default,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResearchSignalMetrics {
    pub sample_count: usize,
    pub dc: f64,
    pub rms: f64,
    pub peak: f32,
    pub crest_factor: f64,
    pub duty_above_midpoint: f64,
    pub measured_frequency_hz: Option<f64>,
}

impl ResearchSignalMetrics {
    pub fn measure(samples: &[f32], sample_rate_hz: f32) -> Result<Self, ResearchError> {
        if samples.is_empty() {
            return Err(ResearchError::EmptyRender);
        }
        let mut sum = 0.0_f64;
        let mut sum_squared = 0.0_f64;
        let mut peak = 0.0_f32;
        let mut minimum = f32::INFINITY;
        let mut maximum = f32::NEG_INFINITY;
        for (sample_index, &sample) in samples.iter().enumerate() {
            if !sample.is_finite() {
                return Err(ResearchError::NonFiniteOutput { sample_index });
            }
            let sample64 = f64::from(sample);
            sum += sample64;
            sum_squared += sample64 * sample64;
            peak = peak.max(sample.abs());
            minimum = minimum.min(sample);
            maximum = maximum.max(sample);
        }
        let count = samples.len() as f64;
        let dc = sum / count;
        let rms = libm::sqrt(sum_squared / count);
        let midpoint = (minimum + maximum) * 0.5;
        let above = samples.iter().filter(|sample| **sample > midpoint).count();
        Ok(Self {
            sample_count: samples.len(),
            dc,
            rms,
            peak,
            crest_factor: f64::from(peak) / rms.max(f64::MIN_POSITIVE),
            duty_above_midpoint: above as f64 / count,
            measured_frequency_hz: estimate_frequency(samples, sample_rate_hz),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResearchComparisonMetrics {
    pub normalized_rms_error: f64,
    pub maximum_absolute_error: f32,
    pub correlation: f64,
}

impl ResearchComparisonMetrics {
    pub fn measure(reference: &[f32], candidate: &[f32]) -> Result<Self, ResearchError> {
        if reference.len() != candidate.len() || reference.is_empty() {
            return Err(ResearchError::ComparisonLengthMismatch);
        }
        let reference_metrics = ResearchSignalMetrics::measure(reference, 1.0)?;
        let candidate_metrics = ResearchSignalMetrics::measure(candidate, 1.0)?;
        let mut squared_error = 0.0_f64;
        let mut covariance = 0.0_f64;
        let mut reference_variance = 0.0_f64;
        let mut candidate_variance = 0.0_f64;
        let mut maximum_absolute_error = 0.0_f32;
        for (&reference, &candidate) in reference.iter().zip(candidate) {
            let error = candidate - reference;
            squared_error += f64::from(error) * f64::from(error);
            maximum_absolute_error = maximum_absolute_error.max(error.abs());
            let reference_centered = f64::from(reference) - reference_metrics.dc;
            let candidate_centered = f64::from(candidate) - candidate_metrics.dc;
            covariance += reference_centered * candidate_centered;
            reference_variance += reference_centered * reference_centered;
            candidate_variance += candidate_centered * candidate_centered;
        }
        let count = reference.len() as f64;
        let error_rms = libm::sqrt(squared_error / count);
        let denominator = libm::sqrt(reference_variance * candidate_variance);
        Ok(Self {
            normalized_rms_error: error_rms / reference_metrics.rms.max(f64::MIN_POSITIVE),
            maximum_absolute_error,
            correlation: if denominator > f64::MIN_POSITIVE {
                covariance / denominator
            } else {
                0.0
            },
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResearchRenderSummary {
    pub descriptor: ResearchModelDescriptor,
    pub case: ResearchRenderCase,
    pub signal: ResearchSignalMetrics,
    pub sample_hash_fnv1a64: u64,
}

/// Configures, warms, and renders one model without allocating. A failed model
/// affects only this output buffer and returns a typed error.
pub fn render_research_case<M: OscillatorResearchModel + ?Sized>(
    model: &mut M,
    case: ResearchRenderCase,
    output: &mut [f32],
) -> Result<ResearchRenderSummary, ResearchError> {
    let case = case.validate()?;
    if output.len() != case.render_samples {
        return Err(ResearchError::WrongOutputLength);
    }
    model.configure(case)?;
    for _ in 0..case.warmup_samples {
        let _ = model.next_sample();
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for (sample_index, sample) in output.iter_mut().enumerate() {
        *sample = model.next_sample();
        if !sample.is_finite() {
            return Err(ResearchError::NonFiniteOutput { sample_index });
        }
        for byte in sample.to_bits().to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Ok(ResearchRenderSummary {
        descriptor: model.descriptor(),
        case,
        signal: ResearchSignalMetrics::measure(output, case.sample_rate_hz)?,
        sample_hash_fnv1a64: hash,
    })
}

fn estimate_frequency(samples: &[f32], sample_rate_hz: f32) -> Option<f64> {
    if !sample_rate_hz.is_finite() || sample_rate_hz <= 0.0 {
        return None;
    }
    let mean = samples.iter().map(|sample| f64::from(*sample)).sum::<f64>() / samples.len() as f64;
    let mut first = None;
    let mut last = None;
    let mut crossings = 0_u32;
    for index in 1..samples.len() {
        let left = f64::from(samples[index - 1]) - mean;
        let right = f64::from(samples[index]) - mean;
        if left <= 0.0 && right > 0.0 {
            let denominator = right - left;
            let position = index as f64 - 1.0 - left / denominator;
            first.get_or_insert(position);
            last = Some(position);
            crossings += 1;
        }
    }
    match (first, last, crossings) {
        (Some(first), Some(last), count) if count >= 2 && last > first => {
            Some(f64::from(sample_rate_hz) * f64::from(count - 1) / (last - first))
        }
        _ => None,
    }
}
