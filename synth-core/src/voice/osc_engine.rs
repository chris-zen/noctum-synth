//! Retained owner for complete pre-filter oscillator engines.
//!
//! Phase 1 wraps the bit-proven production oscillator section as `BlepEngine`.
//! Later engine implementations live beside it and remain allocated while
//! dormant; voice code continues to address this owner only.

#[cfg(feature = "osc-blep")]
use core::ops::{Deref, DerefMut};

use crate::{
    GlideMode, ParamId,
    dsp::{SawMethod, Waveform},
    math::WideF32,
    patch::LayerPatch,
    profiling::RenderContext,
};

use super::oscillators::{OscillatorModulation, Oscillators, OscillatorsOutput, OscillatorsParams};

#[cfg(feature = "osc-wavetable")]
mod wavetable_banks;

/// Stable, session-level oscillator engine selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OscillatorEngineType {
    #[cfg(feature = "osc-blep")]
    Blep,
    #[cfg(feature = "osc-wavetable")]
    Wavetable,
}

impl OscillatorEngineType {
    pub const ALL: &'static [(&'static str, Self)] = &[
        #[cfg(feature = "osc-blep")]
        ("blep", Self::Blep),
        #[cfg(feature = "osc-wavetable")]
        ("wavetable", Self::Wavetable),
    ];

    pub const fn id(self) -> &'static str {
        match self {
            #[cfg(feature = "osc-blep")]
            Self::Blep => "blep",
            #[cfg(feature = "osc-wavetable")]
            Self::Wavetable => "wavetable",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find_map(|&(candidate_id, engine)| (candidate_id == id).then_some(engine))
    }

    pub const fn name(self) -> &'static str {
        match self {
            #[cfg(feature = "osc-blep")]
            Self::Blep => "BLEP",
            #[cfg(feature = "osc-wavetable")]
            Self::Wavetable => "Measured Wavetable",
        }
    }

    pub const fn default_engine() -> Self {
        #[cfg(feature = "osc-blep")]
        return Self::Blep;
        #[cfg(all(not(feature = "osc-blep"), feature = "osc-wavetable"))]
        Self::Wavetable
    }

    pub const fn blep_methods(self) -> &'static [SawMethod] {
        match self {
            #[cfg(feature = "osc-blep")]
            Self::Blep => SawMethod::ALL,
            #[cfg(feature = "osc-wavetable")]
            Self::Wavetable => &[],
        }
    }

    pub const fn wavetable_banks(self) -> &'static [(&'static str, BankId)] {
        match self {
            #[cfg(feature = "osc-blep")]
            Self::Blep => &[],
            #[cfg(feature = "osc-wavetable")]
            Self::Wavetable => BankId::ALL,
        }
    }

    pub const fn selection_name(self, _blep_method: SawMethod, _bank: BankId) -> &'static str {
        match self {
            #[cfg(feature = "osc-blep")]
            Self::Blep => _blep_method.name(),
            #[cfg(feature = "osc-wavetable")]
            Self::Wavetable => _bank.name(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BankId {
    Monologue,
    Prophet5,
}

impl BankId {
    pub const ALL: &'static [(&'static str, Self)] =
        &[("monologue", Self::Monologue), ("prophet5", Self::Prophet5)];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Monologue => "monologue",
            Self::Prophet5 => "prophet5",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .find_map(|&(candidate_id, bank)| (candidate_id == id).then_some(bank))
    }

    #[cfg(feature = "osc-wavetable")]
    pub fn compiled_bank(self) -> crate::dsp::WavetableBank {
        wavetable_banks::bank(self)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Monologue => "Wavetable (Monologue)",
            Self::Prophet5 => "Wavetable (Prophet-5 V)",
        }
    }
}

/// Complete production BLEP source section: Osc 1/2, sub, noise, mix, sync,
/// glide, and their private state.
#[cfg(feature = "osc-blep")]
pub struct BlepEngine {
    section: Oscillators,
    method: SawMethod,
}

#[cfg(feature = "osc-blep")]
impl BlepEngine {
    fn new(sample_rate: f32) -> Self {
        let method = default_blep_method();
        let mut section = Oscillators::new(sample_rate);
        section.set_saw_method(method);
        Self { section, method }
    }

    pub const fn method(&self) -> SawMethod {
        self.method
    }

    pub fn set_method(&mut self, method: SawMethod) {
        if self.method == method {
            return;
        }
        self.section.set_saw_method(method);
        self.method = method;
    }
}

const fn default_blep_method() -> SawMethod {
    #[cfg(feature = "oscillator-polyblep")]
    return SawMethod::PolyBlep;
    #[cfg(not(feature = "oscillator-polyblep"))]
    SawMethod::Blep
}

#[cfg(feature = "osc-blep")]
impl Deref for BlepEngine {
    type Target = Oscillators;

    fn deref(&self) -> &Self::Target {
        &self.section
    }
}

#[cfg(feature = "osc-blep")]
impl DerefMut for BlepEngine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.section
    }
}

#[cfg(feature = "osc-wavetable")]
pub struct WavetableEngine {
    section: Oscillators<crate::dsp::live_wavetable::LiveWavetable>,
    bank: BankId,
}

#[cfg(feature = "osc-wavetable")]
impl WavetableEngine {
    fn new(sample_rate: f32) -> Self {
        let bank = BankId::Monologue;
        Self {
            section: Oscillators::new_wavetable(sample_rate, wavetable_banks::bank(bank)),
            bank,
        }
    }

    pub const fn bank(&self) -> BankId {
        self.bank
    }

    pub fn set_bank(&mut self, bank: BankId) {
        if self.bank == bank {
            return;
        }
        self.section.set_wavetable_bank(wavetable_banks::bank(bank));
        self.bank = bank;
    }
}

/// One retained owner per voice block. Engine selection does not reconstruct
/// the concrete engine or discard its phase/history.
pub struct OscillatorEngines {
    selected: OscillatorEngineType,
    #[cfg(feature = "osc-blep")]
    blep: BlepEngine,
    #[cfg(feature = "osc-wavetable")]
    wavetable: WavetableEngine,
}

macro_rules! selected {
    ($owner:expr, $method:ident($($argument:expr),* $(,)?)) => {
        match $owner.selected {
            #[cfg(feature = "osc-blep")]
            OscillatorEngineType::Blep => $owner.blep.section.$method($($argument),*),
            #[cfg(feature = "osc-wavetable")]
            OscillatorEngineType::Wavetable => $owner.wavetable.section.$method($($argument),*),
        }
    };
}

macro_rules! broadcast {
    ($owner:expr, $method:ident($($argument:expr),* $(,)?)) => {{
        #[cfg(feature = "osc-blep")]
        $owner.blep.section.$method($($argument),*);
        #[cfg(feature = "osc-wavetable")]
        $owner.wavetable.section.$method($($argument),*);
    }};
}

impl OscillatorEngines {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            selected: OscillatorEngineType::default_engine(),
            #[cfg(feature = "osc-blep")]
            blep: BlepEngine::new(sample_rate),
            #[cfg(feature = "osc-wavetable")]
            wavetable: WavetableEngine::new(sample_rate),
        }
    }

    pub const fn selected(&self) -> OscillatorEngineType {
        self.selected
    }

    pub fn select(&mut self, next: OscillatorEngineType) {
        if self.selected == next {
            return;
        }
        #[cfg(all(feature = "osc-blep", feature = "osc-wavetable"))]
        match (self.selected, next) {
            (OscillatorEngineType::Blep, OscillatorEngineType::Wavetable) => self
                .wavetable
                .section
                .synchronize_runtime_from(&self.blep.section),
            (OscillatorEngineType::Wavetable, OscillatorEngineType::Blep) => self
                .blep
                .section
                .synchronize_runtime_from(&self.wavetable.section),
            _ => {}
        }
        self.selected = next;
    }

    pub const fn blep_method(&self) -> SawMethod {
        #[cfg(feature = "osc-blep")]
        return self.blep.method();
        #[cfg(not(feature = "osc-blep"))]
        default_blep_method()
    }

    pub fn set_blep_method(&mut self, method: SawMethod) {
        #[cfg(feature = "osc-blep")]
        self.blep.set_method(method);
        #[cfg(not(feature = "osc-blep"))]
        let _ = method;
    }

    pub const fn wavetable_bank(&self) -> BankId {
        #[cfg(feature = "osc-wavetable")]
        return self.wavetable.bank();
        #[cfg(not(feature = "osc-wavetable"))]
        BankId::Monologue
    }

    pub fn set_wavetable_bank(&mut self, bank: BankId) {
        #[cfg(feature = "osc-wavetable")]
        self.wavetable.set_bank(bank);
        #[cfg(not(feature = "osc-wavetable"))]
        let _ = bank;
    }

    pub fn params(&self) -> &OscillatorsParams {
        selected!(self, params())
    }

    pub fn osc1_frequency_hz(&self) -> WideF32 {
        selected!(self, osc1_frequency_hz())
    }

    pub(crate) fn current_keyboard_semitones(&self) -> WideF32 {
        selected!(self, current_keyboard_semitones())
    }

    pub fn apply_params(&mut self, patch: &LayerPatch) {
        broadcast!(self, apply_params(patch));
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) -> bool {
        #[cfg(not(feature = "osc-blep"))]
        let handled = false;
        #[cfg(feature = "osc-blep")]
        let handled = self.blep.section.set_param(id, value);
        #[cfg(feature = "osc-wavetable")]
        let handled = self.wavetable.section.set_param(id, value) || handled;
        handled
    }

    pub fn next(
        &mut self,
        modulation: OscillatorModulation,
        shape_modulation: [f32; 2],
        context: &mut RenderContext<'_>,
    ) -> OscillatorsOutput {
        selected!(self, next(modulation, shape_modulation, context))
    }

    pub fn set_note_frequency(&mut self, frequency: WideF32) {
        selected!(self, set_note_frequency(frequency));
    }

    pub fn note_on(&mut self, lane: usize, frequency: WideF32) {
        selected!(self, note_on(lane, frequency));
    }

    pub(crate) fn set_note_semitones_preserving_glide(&mut self, semitones: [f32; WideF32::LANES]) {
        selected!(self, set_note_semitones_preserving_glide(semitones));
    }

    pub(crate) fn note_on_with_glide(
        &mut self,
        lane: usize,
        semitones: [f32; WideF32::LANES],
        start: Option<f32>,
        glide: bool,
    ) {
        selected!(self, note_on_with_glide(lane, semitones, start, glide));
    }

    pub(crate) fn retune_with_glide(
        &mut self,
        lane: usize,
        semitones: [f32; WideF32::LANES],
        glide: bool,
    ) {
        selected!(self, retune_with_glide(lane, semitones, glide));
    }

    pub fn set_osc1_enabled(&mut self, value: bool) {
        broadcast!(self, set_osc1_enabled(value));
    }

    pub fn set_osc2_enabled(&mut self, value: bool) {
        broadcast!(self, set_osc2_enabled(value));
    }

    pub fn set_osc1_waveform(&mut self, value: Waveform) {
        broadcast!(self, set_osc1_waveform(value));
    }

    pub fn set_osc2_waveform(&mut self, value: Waveform) {
        broadcast!(self, set_osc2_waveform(value));
    }

    pub fn set_osc1_frequency_semitones(&mut self, value: f32) {
        broadcast!(self, set_osc1_frequency_semitones(value));
    }

    pub fn set_osc2_frequency_semitones(&mut self, value: f32) {
        broadcast!(self, set_osc2_frequency_semitones(value));
    }

    pub fn set_osc1_fine_tune_cents(&mut self, value: f32) {
        broadcast!(self, set_osc1_fine_tune_cents(value));
    }

    pub fn set_osc2_fine_tune_cents(&mut self, value: f32) {
        broadcast!(self, set_osc2_fine_tune_cents(value));
    }

    pub fn set_osc1_shape_mod(&mut self, value: f32) {
        broadcast!(self, set_osc1_shape_mod(value));
    }

    pub fn set_osc2_shape_mod(&mut self, value: f32) {
        broadcast!(self, set_osc2_shape_mod(value));
    }

    pub fn set_osc1_note_reset(&mut self, value: bool) {
        broadcast!(self, set_osc1_note_reset(value));
    }

    pub fn set_osc2_note_reset(&mut self, value: bool) {
        broadcast!(self, set_osc2_note_reset(value));
    }

    pub fn set_osc1_keyboard_on(&mut self, value: bool) {
        broadcast!(self, set_osc1_keyboard_on(value));
    }

    pub fn set_osc2_keyboard_on(&mut self, value: bool) {
        broadcast!(self, set_osc2_keyboard_on(value));
    }

    pub fn set_osc1_glide(&mut self, value: f32) {
        broadcast!(self, set_osc1_glide(value));
    }

    pub fn set_osc2_glide(&mut self, value: f32) {
        broadcast!(self, set_osc2_glide(value));
    }

    pub fn set_glide_mode(&mut self, value: GlideMode) {
        broadcast!(self, set_glide_mode(value));
    }

    pub fn set_glide_enabled(&mut self, value: bool) {
        broadcast!(self, set_glide_enabled(value));
    }

    pub fn set_sync(&mut self, value: bool) {
        broadcast!(self, set_sync(value));
    }

    pub fn set_mix(&mut self, value: f32) {
        broadcast!(self, set_mix(value));
    }

    pub fn set_sub_octave(&mut self, value: f32) {
        broadcast!(self, set_sub_octave(value));
    }

    pub fn set_noise(&mut self, value: f32) {
        broadcast!(self, set_noise(value));
    }

    pub fn set_slop(&mut self, value: f32) {
        broadcast!(self, set_slop(value));
    }
}

/// Isolated single-oscillator renderer used by desktop analysis views.
/// Selection and DSP both go through the same retained engine owner as live audio.
pub struct OscillatorPreview {
    engines: OscillatorEngines,
    frequency_hz: f32,
}

impl OscillatorPreview {
    pub fn new(
        sample_rate: f32,
        engine: OscillatorEngineType,
        blep_method: SawMethod,
        bank: BankId,
    ) -> Self {
        let mut engines = OscillatorEngines::new(sample_rate);
        engines.set_blep_method(blep_method);
        engines.set_wavetable_bank(bank);
        engines.select(engine);
        engines.set_osc1_enabled(true);
        engines.set_osc2_enabled(false);
        engines.set_mix(0.0);
        engines.set_sub_octave(0.0);
        engines.set_noise(0.0);
        engines.set_slop(0.0);
        engines.set_osc1_note_reset(true);
        engines.set_osc1_keyboard_on(true);
        Self {
            engines,
            frequency_hz: 440.0,
        }
    }

    pub fn set_waveform(&mut self, waveform: Waveform) {
        self.engines.set_osc1_waveform(waveform);
    }

    pub fn set_shape(&mut self, shape: f32) {
        self.engines.set_osc1_shape_mod(shape);
    }

    pub fn set_frequency(&mut self, frequency_hz: f32) {
        self.frequency_hz = frequency_hz.max(0.0);
        self.engines
            .set_note_frequency(WideF32::splat(self.frequency_hz));
    }

    pub fn reset(&mut self) {
        self.engines.note_on(0, WideF32::splat(self.frequency_hz));
    }

    pub fn next_sample(&mut self, context: &mut RenderContext<'_>) -> f32 {
        self.engines
            .next(OscillatorModulation::default(), [0.0; 2], context)
            .audio
            .to_array()[0]
    }
}

#[cfg(test)]
mod tests {
    use super::{BankId, OscillatorEngineType, OscillatorEngines, OscillatorPreview};
    use crate::{
        GlideMode,
        dsp::Waveform,
        math::WideF32,
        voice::oscillators::{OscillatorModulation, Oscillators},
    };

    #[test]
    #[cfg(feature = "osc-blep")]
    fn blep_owner_is_bit_identical_to_the_existing_section() {
        let mut reference = Oscillators::new(48_000.0);
        let mut owner = OscillatorEngines::new(48_000.0);
        reference.set_osc1_waveform(Waveform::Saw);
        reference.set_osc2_waveform(Waveform::Pulse);
        reference.set_osc1_shape_mod(0.23);
        reference.set_osc2_shape_mod(0.67);
        reference.set_mix(0.41);
        reference.set_sub_octave(0.2);
        reference.set_note_frequency(WideF32::splat(220.0));
        owner.set_osc1_waveform(Waveform::Saw);
        owner.set_osc2_waveform(Waveform::Pulse);
        owner.set_osc1_shape_mod(0.23);
        owner.set_osc2_shape_mod(0.67);
        owner.set_mix(0.41);
        owner.set_sub_octave(0.2);
        owner.set_note_frequency(WideF32::splat(220.0));

        for _ in 0..512 {
            let mut reference_context = crate::create_render_context!();
            let mut owner_context = crate::create_render_context!();
            let reference = reference.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut reference_context,
            );
            let owned = owner.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut owner_context,
            );
            assert_eq!(
                reference.audio.to_array().map(f32::to_bits),
                owned.audio.to_array().map(f32::to_bits)
            );
            assert_eq!(
                reference.osc1.to_array().map(f32::to_bits),
                owned.osc1.to_array().map(f32::to_bits)
            );
        }
    }

    #[test]
    #[cfg(feature = "osc-blep")]
    fn changing_blep_method_retains_the_engine_and_phase() {
        let mut reference = Oscillators::new(48_000.0);
        let mut owner = OscillatorEngines::new(48_000.0);
        reference.set_note_frequency(WideF32::splat(220.0));
        owner.set_note_frequency(WideF32::splat(220.0));
        for _ in 0..127 {
            let mut reference_context = crate::create_render_context!();
            let mut owner_context = crate::create_render_context!();
            let _ = reference.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut reference_context,
            );
            let _ = owner.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut owner_context,
            );
        }

        reference.set_saw_method(crate::dsp::SawMethod::PolyBlep);
        owner.set_blep_method(crate::dsp::SawMethod::PolyBlep);
        assert_eq!(owner.blep_method(), crate::dsp::SawMethod::PolyBlep);
        for _ in 0..32 {
            let mut reference_context = crate::create_render_context!();
            let mut owner_context = crate::create_render_context!();
            let expected = reference.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut reference_context,
            );
            let actual = owner.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut owner_context,
            );
            assert_eq!(
                expected.audio.to_array().map(f32::to_bits),
                actual.audio.to_array().map(f32::to_bits),
                "BLEP method change discarded oscillator history"
            );
        }
    }

    #[cfg(feature = "osc-wavetable")]
    #[test]
    fn wavetable_bank_round_trip_retains_phase_history() {
        fn configured() -> OscillatorEngines {
            let mut owner = OscillatorEngines::new(96_000.0);
            owner.select(OscillatorEngineType::Wavetable);
            owner.set_osc1_waveform(Waveform::Pulse);
            owner.set_note_frequency(WideF32::splat(220.0));
            owner
        }

        let mut reference = configured();
        let mut changed = configured();
        for _ in 0..127 {
            let mut reference_context = crate::create_render_context!();
            let mut changed_context = crate::create_render_context!();
            let _ = reference.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut reference_context,
            );
            let _ = changed.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut changed_context,
            );
        }
        changed.set_wavetable_bank(BankId::Prophet5);
        changed.set_wavetable_bank(BankId::Monologue);

        for _ in 0..32 {
            let mut reference_context = crate::create_render_context!();
            let mut changed_context = crate::create_render_context!();
            let expected = reference.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut reference_context,
            );
            let actual = changed.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut changed_context,
            );
            assert_eq!(
                expected.audio.to_array().map(f32::to_bits),
                actual.audio.to_array().map(f32::to_bits),
                "bank round trip discarded wavetable phase history"
            );
        }
    }

    #[cfg(all(feature = "osc-blep", feature = "osc-wavetable"))]
    #[test]
    fn engine_switch_synchronizes_active_glide_without_discarding_blep_history() {
        fn configured() -> OscillatorEngines {
            let mut owner = OscillatorEngines::new(48_000.0);
            owner.set_osc1_frequency_semitones(24.0);
            owner.set_osc1_glide(0.5);
            owner.set_glide_mode(GlideMode::FixedTime);
            owner.set_glide_enabled(true);
            owner.note_on_with_glide(0, [72.0; WideF32::LANES], Some(48.0), true);
            owner
        }

        let mut reference = configured();
        let mut switched = configured();
        for _ in 0..128 {
            let mut reference_context = crate::create_render_context!();
            let mut switched_context = crate::create_render_context!();
            let _ = reference.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut reference_context,
            );
            let _ = switched.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut switched_context,
            );
        }

        let pitch_before = switched.current_keyboard_semitones();
        switched.select(OscillatorEngineType::Wavetable);
        assert_eq!(switched.current_keyboard_semitones(), pitch_before);
        switched.select(OscillatorEngineType::Blep);

        let mut reference_context = crate::create_render_context!();
        let mut switched_context = crate::create_render_context!();
        let expected = reference.next(
            OscillatorModulation::default(),
            [0.0; 2],
            &mut reference_context,
        );
        let actual = switched.next(
            OscillatorModulation::default(),
            [0.0; 2],
            &mut switched_context,
        );
        assert_eq!(
            expected.audio.to_array().map(f32::to_bits),
            actual.audio.to_array().map(f32::to_bits),
            "select-away/select-back discarded retained BLEP history"
        );
    }

    #[cfg(feature = "osc-wavetable")]
    #[test]
    fn wavetable_engine_applies_detune_and_glide() {
        let mut owner = OscillatorEngines::new(48_000.0);
        owner.select(OscillatorEngineType::Wavetable);
        owner.set_osc1_frequency_semitones(24.0);
        owner.set_osc1_fine_tune_cents(37.0);
        owner.set_note_frequency(WideF32::splat(crate::tuning::midi_to_hz(60)));
        let expected = crate::tuning::midi_to_hz(60) * libm::exp2f(0.37 / 12.0);
        let actual = owner.osc1_frequency_hz().to_array()[0];
        assert!(
            (actual - expected).abs() < 0.02,
            "detuned frequency {actual}"
        );

        owner.set_osc1_glide(0.5);
        owner.set_glide_mode(GlideMode::FixedTime);
        owner.set_glide_enabled(true);
        owner.note_on_with_glide(0, [72.0; WideF32::LANES], Some(48.0), true);
        let before = owner.osc1_frequency_hz().to_array()[0];
        let mut context = crate::create_render_context!();
        let _ = owner.next(OscillatorModulation::default(), [0.0; 2], &mut context);
        assert!(owner.osc1_frequency_hz().to_array()[0] > before);
    }

    #[cfg(feature = "osc-wavetable")]
    #[test]
    fn oscillator_preview_matches_the_live_engine_path() {
        let mut preview = OscillatorPreview::new(
            48_000.0,
            OscillatorEngineType::Wavetable,
            crate::dsp::SawMethod::Blep,
            BankId::Monologue,
        );
        preview.set_waveform(Waveform::Pulse);
        preview.set_shape(0.63);
        preview.set_frequency(220.0);
        preview.reset();

        let mut live = OscillatorEngines::new(48_000.0);
        live.set_wavetable_bank(BankId::Monologue);
        live.select(OscillatorEngineType::Wavetable);
        live.set_osc1_enabled(true);
        live.set_osc2_enabled(false);
        live.set_mix(0.0);
        live.set_sub_octave(0.0);
        live.set_noise(0.0);
        live.set_slop(0.0);
        live.set_osc1_note_reset(true);
        live.set_osc1_keyboard_on(true);
        live.set_osc1_waveform(Waveform::Pulse);
        live.set_osc1_shape_mod(0.63);
        live.set_note_frequency(WideF32::splat(220.0));
        live.note_on(0, WideF32::splat(220.0));

        for _ in 0..256 {
            let mut preview_context = crate::create_render_context!();
            let mut live_context = crate::create_render_context!();
            let expected = live
                .next(OscillatorModulation::default(), [0.0; 2], &mut live_context)
                .audio
                .to_array()[0];
            let actual = preview.next_sample(&mut preview_context);
            assert_eq!(expected.to_bits(), actual.to_bits());
        }
    }

    #[cfg(feature = "osc-wavetable")]
    #[test]
    fn compiled_bank_selection_changes_wavetable_output() {
        fn configured(bank: super::BankId) -> OscillatorEngines {
            let mut owner = OscillatorEngines::new(96_000.0);
            owner.set_wavetable_bank(bank);
            owner.select(OscillatorEngineType::Wavetable);
            owner.set_osc1_waveform(Waveform::Saw);
            owner.set_osc2_enabled(false);
            owner.set_mix(0.0);
            owner.set_note_frequency(WideF32::splat(220.0));
            owner
        }

        let mut monologue = configured(super::BankId::Monologue);
        let mut prophet5 = configured(super::BankId::Prophet5);
        let mut maximum_difference = 0.0_f32;
        for _ in 0..512 {
            let mut monologue_context = crate::create_render_context!();
            let mut prophet5_context = crate::create_render_context!();
            let first = monologue.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut monologue_context,
            );
            let second = prophet5.next(
                OscillatorModulation::default(),
                [0.0; 2],
                &mut prophet5_context,
            );
            maximum_difference = maximum_difference
                .max((first.audio.to_array()[0] - second.audio.to_array()[0]).abs());
        }

        assert!(maximum_difference > 1.0e-4);
    }

    #[test]
    fn stable_engine_and_bank_ids_round_trip_through_descriptors() {
        for &(id, engine) in OscillatorEngineType::ALL {
            assert_eq!(OscillatorEngineType::from_id(id), Some(engine));
        }
        assert_eq!(OscillatorEngineType::from_id("unknown"), None);

        for &(id, bank) in super::BankId::ALL {
            assert_eq!(super::BankId::from_id(id), Some(bank));
        }
        assert_eq!(super::BankId::from_id("unknown"), None);
    }
}
