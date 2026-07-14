//! Post-voice effects.

use crate::{DEFAULT_TEMPO_BPM, EffectParams, EffectType, TAU, midi_to_hz};

const DELAY_TIME_CROSSFADE_SECONDS: f32 = 0.035;
const DELAY_TIME_CHANGE_THRESHOLD_SAMPLES: f32 = 8.0;
const MIN_DELAY_SAMPLES: f32 = 1.0;
const MIN_BUFFER_SAMPLES: usize = 4;
const REVERB_SEGMENTS: usize = 14;
const REVERB_COMB_LEFT_SECONDS: [f32; 4] = [0.0297, 0.0371, 0.0411, 0.0437];
const REVERB_COMB_RIGHT_SECONDS: [f32; 4] = [0.0309, 0.0389, 0.0427, 0.0451];
const REVERB_ALLPASS_LEFT_SECONDS: [f32; 3] = [0.0050, 0.0017, 0.0063];
const REVERB_ALLPASS_RIGHT_SECONDS: [f32; 3] = [0.0054, 0.0021, 0.0069];
const PHASER_HIGH_FEEDBACK: f32 = 0.82;
const PHASER_LOW_FEEDBACK: f32 = 0.35;
const PHASER_MST_FEEDBACK: f32 = 0.58;
const FLANGER_1_FEEDBACK: f32 = 0.72;
const FLANGER_2_FEEDBACK: f32 = 0.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct EffectModulation {
    /// Additive modulation applied to the wet/dry mix.
    pub mix: f32,
    /// Additive modulation applied to the selected effect's first parameter.
    pub param1: f32,
    /// Additive modulation applied to the selected effect's second parameter.
    pub param2: f32,
}

impl EffectModulation {
    pub fn add(&mut self, other: Self) {
        self.mix += other.mix;
        self.param1 += other.param1;
        self.param2 += other.param2;
    }

    pub fn scale(self, amount: f32) -> Self {
        Self {
            mix: self.mix * amount,
            param1: self.param1 * amount,
            param2: self.param2 * amount,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectedEffect {
    DelayMono,
    DdlStereo,
    BucketBrigadeDelay,
    Chorus,
    PhaserHigh,
    PhaserLow,
    PhaserMst,
    Flanger1,
    Flanger2,
    Reverb,
    RingMod,
    Distortion,
    HighPass,
}

impl SelectedEffect {
    fn effect_type(self) -> EffectType {
        match self {
            Self::DelayMono => EffectType::DelayMono,
            Self::DdlStereo => EffectType::DdlStereo,
            Self::BucketBrigadeDelay => EffectType::BucketBrigadeDelay,
            Self::Chorus => EffectType::Chorus,
            Self::PhaserHigh => EffectType::PhaserHigh,
            Self::PhaserLow => EffectType::PhaserLow,
            Self::PhaserMst => EffectType::PhaserMst,
            Self::Flanger1 => EffectType::Flanger1,
            Self::Flanger2 => EffectType::Flanger2,
            Self::Reverb => EffectType::Reverb,
            Self::RingMod => EffectType::RingMod,
            Self::Distortion => EffectType::Distortion,
            Self::HighPass => EffectType::HighPassFilter,
        }
    }
}

impl From<EffectType> for SelectedEffect {
    fn from(value: EffectType) -> Self {
        match value {
            EffectType::DelayMono => Self::DelayMono,
            EffectType::DdlStereo => Self::DdlStereo,
            EffectType::BucketBrigadeDelay => Self::BucketBrigadeDelay,
            EffectType::Chorus => Self::Chorus,
            EffectType::PhaserHigh => Self::PhaserHigh,
            EffectType::PhaserLow => Self::PhaserLow,
            EffectType::PhaserMst => Self::PhaserMst,
            EffectType::Flanger1 => Self::Flanger1,
            EffectType::Flanger2 => Self::Flanger2,
            EffectType::Reverb => Self::Reverb,
            EffectType::RingMod => Self::RingMod,
            EffectType::Distortion => Self::Distortion,
            EffectType::HighPassFilter => Self::HighPass,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RuntimeParams {
    mix: f32,
    clock_sync: bool,
    param1: f32,
    param2: f32,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        let params = EffectParams::default();
        Self::from_patch(params)
    }
}

impl RuntimeParams {
    fn from_patch(params: EffectParams) -> Self {
        Self {
            mix: params.mix.clamp(0.0, 1.0),
            clock_sync: params.clock_sync,
            param1: params.param1.clamp(0.0, 1.0),
            param2: params.param2.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Copy)]
struct ProcessContext {
    sample_rate: f32,
    tempo_bpm: f32,
    param1: f32,
    param2: f32,
    clock_sync: bool,
    lowest_note: Option<u8>,
}

/// Effects processor backed by caller-selected mutable sample storage.
///
/// `Memory` is deliberately expressed only in terms of ordinary slice access. A
/// host can use an array or `Vec`, while embedded firmware can borrow memory
/// initialized by its board support package.
pub struct EffectsWithMemory<Memory> {
    sample_rate: f32,
    tempo_bpm: f32,
    enabled: bool,
    selected: SelectedEffect,
    buffer: Memory,
    delay_mono: MonoDelay,
    ddl_stereo: StereoDelay,
    bucket_brigade_delay: BucketBrigadeDelay,
    chorus: Chorus,
    phaser_high: Phaser,
    phaser_low: Phaser,
    phaser_mst: Phaser,
    flanger1: Flanger,
    flanger2: Flanger,
    reverb: Reverb,
    ring_mod: RingMod,
    distortion: Distortion,
    high_pass: HighPass,
}

/// Effects processor with inline, statically sized sample storage.
pub type Effects<const SAMPLES: usize> = EffectsWithMemory<[f32; SAMPLES]>;

impl<const SAMPLES: usize> EffectsWithMemory<[f32; SAMPLES]> {
    /// Creates an effects processor with an inline, statically sized buffer.
    pub fn new(sample_rate: f32) -> Self {
        // Construct the large inline array in place. Passing it through the
        // generic constructor creates an additional debug-build stack copy.
        Self {
            sample_rate: sample_rate.max(1.0),
            tempo_bpm: DEFAULT_TEMPO_BPM,
            enabled: false,
            selected: SelectedEffect::DelayMono,
            buffer: [0.0; SAMPLES],
            delay_mono: MonoDelay::default(),
            ddl_stereo: StereoDelay::default(),
            bucket_brigade_delay: BucketBrigadeDelay::default(),
            chorus: Chorus::default(),
            phaser_high: Phaser::default(),
            phaser_low: Phaser::default(),
            phaser_mst: Phaser::default(),
            flanger1: Flanger::new(FLANGER_1_FEEDBACK),
            flanger2: Flanger::new(FLANGER_2_FEEDBACK),
            reverb: Reverb::default(),
            ring_mod: RingMod::default(),
            distortion: Distortion::default(),
            high_pass: HighPass::default(),
        }
    }
}

impl<Memory> EffectsWithMemory<Memory>
where
    Memory: AsRef<[f32]> + AsMut<[f32]>,
{
    /// Creates an effects processor using `buffer` as its shared delay memory.
    pub fn new_with_memory(sample_rate: f32, mut buffer: Memory) -> Self {
        buffer.as_mut().fill(0.0);
        Self {
            sample_rate: sample_rate.max(1.0),
            tempo_bpm: DEFAULT_TEMPO_BPM,
            enabled: false,
            selected: SelectedEffect::DelayMono,
            buffer,
            delay_mono: MonoDelay::default(),
            ddl_stereo: StereoDelay::default(),
            bucket_brigade_delay: BucketBrigadeDelay::default(),
            chorus: Chorus::default(),
            phaser_high: Phaser::default(),
            phaser_low: Phaser::default(),
            phaser_mst: Phaser::default(),
            flanger1: Flanger::new(FLANGER_1_FEEDBACK),
            flanger2: Flanger::new(FLANGER_2_FEEDBACK),
            reverb: Reverb::default(),
            ring_mod: RingMod::default(),
            distortion: Distortion::default(),
            high_pass: HighPass::default(),
        }
    }

    pub fn set_params(&mut self, params: EffectParams) {
        let selected = SelectedEffect::from(params.effect_type);
        let changed = self.enabled != params.enabled || self.selected != selected;
        self.enabled = params.enabled;
        self.selected = selected;
        *self.selected_params_mut() = RuntimeParams::from_patch(params);
        if changed {
            self.clear_audio_memory();
        }
    }

    pub fn params(&self) -> EffectParams {
        let params = self.selected_params();
        EffectParams {
            enabled: self.enabled,
            effect_type: self.selected.effect_type(),
            mix: params.mix,
            clock_sync: params.clock_sync,
            param1: params.param1,
            param2: params.param2,
        }
    }

    pub fn set_tempo_bpm(&mut self, tempo_bpm: f32) {
        self.tempo_bpm = tempo_bpm.clamp(30.0, 250.0);
    }

    pub fn tempo_bpm(&self) -> f32 {
        self.tempo_bpm
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.clear_audio_memory();
        }
    }

    pub fn set_type(&mut self, effect_type: EffectType) {
        let selected = SelectedEffect::from(effect_type);
        if self.selected != selected {
            self.selected = selected;
            self.clear_audio_memory();
        }
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.selected_params_mut().mix = mix.clamp(0.0, 1.0);
    }

    pub fn set_clock_sync(&mut self, clock_sync: bool) {
        self.selected_params_mut().clock_sync = clock_sync;
    }

    pub fn set_param1(&mut self, value: f32) {
        self.selected_params_mut().param1 = value.clamp(0.0, 1.0);
    }

    pub fn set_param2(&mut self, value: f32) {
        self.selected_params_mut().param2 = value.clamp(0.0, 1.0);
    }

    pub fn next(
        &mut self,
        left: f32,
        right: f32,
        modulation: EffectModulation,
        lowest_note: Option<u8>,
    ) -> (f32, f32) {
        if !self.enabled {
            return (left, right);
        }

        let params = self.selected_params();
        let mix = (params.mix + modulation.mix).clamp(0.0, 1.0);
        let context = ProcessContext {
            sample_rate: self.sample_rate,
            tempo_bpm: self.tempo_bpm,
            param1: (params.param1 + modulation.param1).clamp(0.0, 1.0),
            param2: (params.param2 + modulation.param2).clamp(0.0, 1.0),
            clock_sync: params.clock_sync,
            lowest_note,
        };

        let wet = match self.selected {
            SelectedEffect::DelayMono => {
                self.delay_mono
                    .next(left, right, self.buffer.as_mut(), context)
            }
            SelectedEffect::DdlStereo => {
                self.ddl_stereo
                    .next(left, right, self.buffer.as_mut(), context)
            }
            SelectedEffect::BucketBrigadeDelay => {
                self.bucket_brigade_delay
                    .next(left, right, self.buffer.as_mut(), context)
            }
            SelectedEffect::Chorus => self.chorus.next(left, right, self.buffer.as_mut(), context),
            SelectedEffect::PhaserHigh => {
                Some(
                    self.phaser_high
                        .next(left, right, context, PHASER_HIGH_FEEDBACK),
                )
            }
            SelectedEffect::PhaserLow => {
                Some(
                    self.phaser_low
                        .next(left, right, context, PHASER_LOW_FEEDBACK),
                )
            }
            SelectedEffect::PhaserMst => {
                Some(
                    self.phaser_mst
                        .next(left, right, context, PHASER_MST_FEEDBACK),
                )
            }
            SelectedEffect::Flanger1 => {
                self.flanger1
                    .next(left, right, self.buffer.as_mut(), context)
            }
            SelectedEffect::Flanger2 => {
                self.flanger2
                    .next(left, right, self.buffer.as_mut(), context)
            }
            SelectedEffect::Reverb => self.reverb.next(left, right, self.buffer.as_mut(), context),
            SelectedEffect::RingMod => Some(self.ring_mod.next(left, right, context)),
            SelectedEffect::Distortion => Some(self.distortion.next(left, right, context)),
            SelectedEffect::HighPass => Some(self.high_pass.next(left, right, context)),
        };

        let Some((wet_left, wet_right)) = wet else {
            return (left, right);
        };
        (
            crossfade(left, wet_left, mix).clamp(-1.0, 1.0),
            crossfade(right, wet_right, mix).clamp(-1.0, 1.0),
        )
    }

    fn clear_audio_memory(&mut self) {
        self.buffer.as_mut().fill(0.0);
        self.delay_mono.clear();
        self.ddl_stereo.clear();
        self.bucket_brigade_delay.clear();
        self.chorus.clear();
        self.phaser_high.clear();
        self.phaser_low.clear();
        self.phaser_mst.clear();
        self.flanger1.clear();
        self.flanger2.clear();
        self.reverb.clear();
        self.ring_mod.clear();
        self.distortion.clear();
        self.high_pass.clear();
    }

    fn selected_params(&self) -> RuntimeParams {
        match self.selected {
            SelectedEffect::DelayMono => self.delay_mono.params,
            SelectedEffect::DdlStereo => self.ddl_stereo.params,
            SelectedEffect::BucketBrigadeDelay => self.bucket_brigade_delay.params,
            SelectedEffect::Chorus => self.chorus.params,
            SelectedEffect::PhaserHigh => self.phaser_high.params,
            SelectedEffect::PhaserLow => self.phaser_low.params,
            SelectedEffect::PhaserMst => self.phaser_mst.params,
            SelectedEffect::Flanger1 => self.flanger1.params,
            SelectedEffect::Flanger2 => self.flanger2.params,
            SelectedEffect::Reverb => self.reverb.params,
            SelectedEffect::RingMod => self.ring_mod.params,
            SelectedEffect::Distortion => self.distortion.params,
            SelectedEffect::HighPass => self.high_pass.params,
        }
    }

    fn selected_params_mut(&mut self) -> &mut RuntimeParams {
        match self.selected {
            SelectedEffect::DelayMono => &mut self.delay_mono.params,
            SelectedEffect::DdlStereo => &mut self.ddl_stereo.params,
            SelectedEffect::BucketBrigadeDelay => &mut self.bucket_brigade_delay.params,
            SelectedEffect::Chorus => &mut self.chorus.params,
            SelectedEffect::PhaserHigh => &mut self.phaser_high.params,
            SelectedEffect::PhaserLow => &mut self.phaser_low.params,
            SelectedEffect::PhaserMst => &mut self.phaser_mst.params,
            SelectedEffect::Flanger1 => &mut self.flanger1.params,
            SelectedEffect::Flanger2 => &mut self.flanger2.params,
            SelectedEffect::Reverb => &mut self.reverb.params,
            SelectedEffect::RingMod => &mut self.ring_mod.params,
            SelectedEffect::Distortion => &mut self.distortion.params,
            SelectedEffect::HighPass => &mut self.high_pass.params,
        }
    }
}

#[derive(Default)]
struct DelayTransition {
    active: f32,
    next: f32,
    pending: f32,
    crossfade: f32,
    initialized: bool,
}

impl DelayTransition {
    fn retarget(&mut self, target: f32, max_delay: f32, sample_rate: f32) -> (f32, f32, f32) {
        let target = target.clamp(MIN_DELAY_SAMPLES, max_delay.max(MIN_DELAY_SAMPLES));
        if !self.initialized {
            self.initialized = true;
            self.active = target;
            self.next = target;
            self.pending = target;
            self.crossfade = 1.0;
            return (target, target, 1.0);
        }
        self.pending = target;
        if self.crossfade >= 1.0 {
            self.active = self.next.clamp(MIN_DELAY_SAMPLES, max_delay);
            if (self.pending - self.active).abs() > DELAY_TIME_CHANGE_THRESHOLD_SAMPLES {
                self.next = self.pending;
                self.crossfade = 0.0;
            }
        }
        self.active = self.active.clamp(MIN_DELAY_SAMPLES, max_delay);
        self.next = self.next.clamp(MIN_DELAY_SAMPLES, max_delay);
        let fade = smoothstep(self.crossfade);
        if self.crossfade < 1.0 {
            let step = (1.0 / (sample_rate * DELAY_TIME_CROSSFADE_SECONDS)).clamp(0.0001, 1.0);
            self.crossfade = (self.crossfade + step).min(1.0);
        }
        (self.active, self.next, fade)
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Mono feedback delay.
///
/// Parameter 1 selects delay time, or a beat division when clock sync is
/// enabled. Parameter 2 controls feedback.
#[derive(Default)]
struct MonoDelay {
    params: RuntimeParams,
    index: usize,
    transition: DelayTransition,
}

impl MonoDelay {
    fn next(
        &mut self,
        left: f32,
        right: f32,
        buffer: &mut [f32],
        context: ProcessContext,
    ) -> Option<(f32, f32)> {
        let mut delay = DelayBuffer::new(buffer, &mut self.index)?;
        let target = delay_seconds(context) * context.sample_rate;
        let (old, new, fade) =
            self.transition
                .retarget(target, delay.max_delay(), context.sample_rate);
        let delayed = crossfade(delay.read(old), delay.read(new), fade);
        delay.write((left + right) * 0.5 + delayed * context.param2 * 0.85);
        Some((delayed, delayed))
    }

    fn clear(&mut self) {
        self.index = 0;
        self.transition.clear();
    }
}

/// Independent left/right digital delay lines.
///
/// Parameter 1 selects delay time, or a beat division when clock sync is
/// enabled. Parameter 2 controls feedback in each channel.
#[derive(Default)]
struct StereoDelay {
    params: RuntimeParams,
    left_index: usize,
    right_index: usize,
    transition: DelayTransition,
}

impl StereoDelay {
    fn next(
        &mut self,
        left: f32,
        right: f32,
        buffer: &mut [f32],
        context: ProcessContext,
    ) -> Option<(f32, f32)> {
        let (left_buffer, right_buffer) = split_stereo(buffer)?;
        let mut left_delay = DelayBuffer::new(left_buffer, &mut self.left_index)?;
        let mut right_delay = DelayBuffer::new(right_buffer, &mut self.right_index)?;
        let target = delay_seconds(context) * context.sample_rate;
        let max_delay = left_delay.max_delay().min(right_delay.max_delay());
        let (old, new, fade) = self
            .transition
            .retarget(target, max_delay, context.sample_rate);
        let delayed_left = crossfade(left_delay.read(old), left_delay.read(new), fade);
        let delayed_right = crossfade(right_delay.read(old), right_delay.read(new), fade);
        let feedback = context.param2 * 0.85;
        left_delay.write(left + soft_clip(delayed_left * feedback, 1.4));
        right_delay.write(right + soft_clip(delayed_right * feedback, 1.4));
        Some((delayed_left, delayed_right))
    }

    fn clear(&mut self) {
        self.left_index = 0;
        self.right_index = 0;
        self.transition.clear();
    }
}

/// Darkened feedback delay inspired by analogue bucket-brigade devices.
///
/// Parameter 1 selects delay time, or a beat division when clock sync is
/// enabled. Parameter 2 controls feedback. A fixed low-pass stage softens each
/// repeat to approximate the bandwidth loss of a bucket-brigade circuit.
#[derive(Default)]
struct BucketBrigadeDelay {
    params: RuntimeParams,
    left_index: usize,
    right_index: usize,
    transition: DelayTransition,
    tone_left: f32,
    tone_right: f32,
}

impl BucketBrigadeDelay {
    fn next(
        &mut self,
        left: f32,
        right: f32,
        buffer: &mut [f32],
        context: ProcessContext,
    ) -> Option<(f32, f32)> {
        let (left_buffer, right_buffer) = split_stereo(buffer)?;
        let mut left_delay = DelayBuffer::new(left_buffer, &mut self.left_index)?;
        let mut right_delay = DelayBuffer::new(right_buffer, &mut self.right_index)?;
        let target = delay_seconds(context) * context.sample_rate;
        let max_delay = left_delay.max_delay().min(right_delay.max_delay());
        let (old, new, fade) = self
            .transition
            .retarget(target, max_delay, context.sample_rate);
        let raw_left = crossfade(left_delay.read(old), left_delay.read(new), fade);
        let raw_right = crossfade(right_delay.read(old), right_delay.read(new), fade);
        self.tone_left += (raw_left - self.tone_left) * 0.12;
        self.tone_right += (raw_right - self.tone_right) * 0.12;
        let feedback = context.param2 * 0.78;
        left_delay.write(left + soft_clip(self.tone_left * feedback, 1.4));
        right_delay.write(right + soft_clip(self.tone_right * feedback, 1.4));
        Some((self.tone_left, self.tone_right))
    }

    fn clear(&mut self) {
        self.left_index = 0;
        self.right_index = 0;
        self.transition.clear();
        self.tone_left = 0.0;
        self.tone_right = 0.0;
    }
}

/// Stereo modulated-delay chorus.
///
/// Parameter 1 controls modulation rate and parameter 2 controls delay-depth.
/// The right channel uses a quarter-cycle phase offset for stereo width.
#[derive(Default)]
struct Chorus {
    params: RuntimeParams,
    left_index: usize,
    right_index: usize,
    phase: f32,
}

impl Chorus {
    fn next(
        &mut self,
        left: f32,
        right: f32,
        buffer: &mut [f32],
        context: ProcessContext,
    ) -> Option<(f32, f32)> {
        let (left_buffer, right_buffer) = split_stereo(buffer)?;
        let mut left_delay = DelayBuffer::new(left_buffer, &mut self.left_index)?;
        let mut right_delay = DelayBuffer::new(right_buffer, &mut self.right_index)?;
        let rate = 0.05 + context.param1 * 7.5;
        let depth_ms = 1.0 + context.param2 * 12.0;
        let phase_left = self.phase;
        let phase_right = wrap01(self.phase + 0.25);
        self.phase = advance_phase(self.phase, rate, context.sample_rate);
        let delay_left = (18.0 + sine(phase_left) * depth_ms) * 0.001 * context.sample_rate;
        let delay_right = (18.0 + sine(phase_right) * depth_ms) * 0.001 * context.sample_rate;
        let wet_left = left_delay.read(delay_left);
        let wet_right = right_delay.read(delay_right);
        left_delay.write(left);
        right_delay.write(right);
        Some((wet_left, wet_right))
    }

    fn clear(&mut self) {
        self.left_index = 0;
        self.right_index = 0;
        self.phase = 0.0;
    }
}

/// Short modulated-delay flanger.
///
/// Parameter 1 controls modulation rate and parameter 2 controls delay-depth.
/// The two public flanger variants select different fixed feedback amounts.
struct Flanger {
    params: RuntimeParams,
    left_index: usize,
    right_index: usize,
    phase: f32,
    feedback: f32,
}

impl Flanger {
    fn new(feedback: f32) -> Self {
        Self {
            params: RuntimeParams::default(),
            left_index: 0,
            right_index: 0,
            phase: 0.0,
            feedback,
        }
    }

    fn next(
        &mut self,
        left: f32,
        right: f32,
        buffer: &mut [f32],
        context: ProcessContext,
    ) -> Option<(f32, f32)> {
        let (left_buffer, right_buffer) = split_stereo(buffer)?;
        let mut left_delay = DelayBuffer::new(left_buffer, &mut self.left_index)?;
        let mut right_delay = DelayBuffer::new(right_buffer, &mut self.right_index)?;
        let rate = 0.03 + context.param1 * 4.0;
        let depth_ms = 0.2 + context.param2 * 4.8;
        let phase_left = self.phase;
        let phase_right = wrap01(self.phase + 0.5);
        self.phase = advance_phase(self.phase, rate, context.sample_rate);
        let delay_left = (3.0 + sine(phase_left) * depth_ms) * 0.001 * context.sample_rate;
        let delay_right = (3.0 + sine(phase_right) * depth_ms) * 0.001 * context.sample_rate;
        let wet_left = left_delay.read(delay_left);
        let wet_right = right_delay.read(delay_right);
        left_delay.write(left + wet_left * self.feedback);
        right_delay.write(right + wet_right * self.feedback);
        Some((left + wet_left, right + wet_right))
    }

    fn clear(&mut self) {
        self.left_index = 0;
        self.right_index = 0;
        self.phase = 0.0;
    }
}

/// Six-stage stereo all-pass phaser.
///
/// Parameter 1 controls sweep rate and parameter 2 controls sweep depth. The
/// High, Low, and Mst variants select named fixed feedback characteristics.
#[derive(Default)]
struct Phaser {
    params: RuntimeParams,
    phase: f32,
    left: PhaserState,
    right: PhaserState,
}

impl Phaser {
    fn next(
        &mut self,
        left: f32,
        right: f32,
        context: ProcessContext,
        feedback: f32,
    ) -> (f32, f32) {
        let rate = 0.03 + context.param1 * 5.5;
        let phase_left = self.phase;
        let phase_right = wrap01(self.phase + 0.33);
        self.phase = advance_phase(self.phase, rate, context.sample_rate);
        let coeff_left = 0.12 + 0.72 * (0.5 + 0.5 * sine(phase_left)) * context.param2;
        let coeff_right = 0.12 + 0.72 * (0.5 + 0.5 * sine(phase_right)) * context.param2;
        (
            self.left.process(left, coeff_left, feedback),
            self.right.process(right, coeff_right, feedback),
        )
    }

    fn clear(&mut self) {
        self.phase = 0.0;
        self.left = PhaserState::default();
        self.right = PhaserState::default();
    }
}

/// Stereo Schroeder-style reverb using parallel combs and serial all-passes.
///
/// Parameter 1 controls decay feedback and parameter 2 controls damping. The
/// left and right delay tunings differ to produce a diffuse stereo tail.
#[derive(Default)]
struct Reverb {
    params: RuntimeParams,
    comb_left_indices: [usize; 4],
    comb_right_indices: [usize; 4],
    allpass_left_indices: [usize; 3],
    allpass_right_indices: [usize; 3],
    tone_left: [f32; 4],
    tone_right: [f32; 4],
}

impl Reverb {
    fn next(
        &mut self,
        left: f32,
        right: f32,
        buffer: &mut [f32],
        context: ProcessContext,
    ) -> Option<(f32, f32)> {
        let segment_len = buffer.len() / REVERB_SEGMENTS;
        if segment_len < MIN_BUFFER_SAMPLES {
            return None;
        }
        let input = (left + right) * 0.5;
        let feedback = 0.70 + context.param1 * 0.24;
        let tone = 0.08 + context.param2 * 0.34;
        let mut remaining = buffer;
        let mut sum_left = 0.0;
        let mut sum_right = 0.0;
        for index in 0..4 {
            let segment = take_segment(&mut remaining, segment_len);
            sum_left += reverb_comb(
                segment,
                &mut self.comb_left_indices[index],
                &mut self.tone_left[index],
                input,
                REVERB_COMB_LEFT_SECONDS[index] * context.sample_rate,
                feedback,
                tone,
            );
        }
        for index in 0..4 {
            let segment = take_segment(&mut remaining, segment_len);
            sum_right += reverb_comb(
                segment,
                &mut self.comb_right_indices[index],
                &mut self.tone_right[index],
                input,
                REVERB_COMB_RIGHT_SECONDS[index] * context.sample_rate,
                feedback,
                tone,
            );
        }
        let mut wet_left = sum_left * 0.18;
        let mut wet_right = sum_right * 0.18;
        for index in 0..3 {
            let segment = take_segment(&mut remaining, segment_len);
            wet_left = reverb_allpass(
                segment,
                &mut self.allpass_left_indices[index],
                wet_left,
                REVERB_ALLPASS_LEFT_SECONDS[index] * context.sample_rate,
            );
        }
        for index in 0..3 {
            let segment = take_segment(&mut remaining, segment_len);
            wet_right = reverb_allpass(
                segment,
                &mut self.allpass_right_indices[index],
                wet_right,
                REVERB_ALLPASS_RIGHT_SECONDS[index] * context.sample_rate,
            );
        }
        Some((wet_left * 0.7, wet_right * 0.7))
    }

    fn clear(&mut self) {
        self.comb_left_indices = [0; 4];
        self.comb_right_indices = [0; 4];
        self.allpass_left_indices = [0; 3];
        self.allpass_right_indices = [0; 3];
        self.tone_left = [0.0; 4];
        self.tone_right = [0.0; 4];
    }
}

/// Sine-carrier ring modulation.
///
/// Parameter 1 controls carrier frequency. Parameter 2 switches between a
/// free-running carrier and note-tracked carrier ratios at its midpoint.
#[derive(Default)]
struct RingMod {
    params: RuntimeParams,
    phase: f32,
}

impl RingMod {
    fn next(&mut self, left: f32, right: f32, context: ProcessContext) -> (f32, f32) {
        let frequency = if context.param2 >= 0.5 {
            context.lowest_note.map(midi_to_hz).unwrap_or(110.0) * (0.25 + context.param1 * 4.0)
        } else {
            20.0 + context.param1 * 1980.0
        };
        self.phase = advance_phase(self.phase, frequency, context.sample_rate);
        let carrier = sine(self.phase);
        (left * carrier, right * carrier)
    }

    fn clear(&mut self) {
        self.phase = 0.0;
    }
}

/// Soft-clipping distortion followed by a one-pole tone stage.
///
/// Parameter 1 controls drive and parameter 2 controls tone bandwidth.
#[derive(Default)]
struct Distortion {
    params: RuntimeParams,
    tone_left: f32,
    tone_right: f32,
}

impl Distortion {
    fn next(&mut self, left: f32, right: f32, context: ProcessContext) -> (f32, f32) {
        let gain = 1.0 + context.param1 * 40.0;
        let tone = 0.03 + context.param2 * 0.55;
        self.tone_left += (soft_clip(left, gain) - self.tone_left) * tone;
        self.tone_right += (soft_clip(right, gain) - self.tone_right) * tone;
        (self.tone_left, self.tone_right)
    }

    fn clear(&mut self) {
        self.tone_left = 0.0;
        self.tone_right = 0.0;
    }
}

/// One-pole stereo high-pass effect.
///
/// Parameter 1 controls logarithmic cutoff frequency and parameter 2 controls
/// post-filter output emphasis.
#[derive(Default)]
struct HighPass {
    params: RuntimeParams,
    left: OnePoleHighPass,
    right: OnePoleHighPass,
}

impl HighPass {
    fn next(&mut self, left: f32, right: f32, context: ProcessContext) -> (f32, f32) {
        let cutoff = 20.0 * crate::math::powf(600.0, context.param1);
        let resonance_gain = 1.0 + context.param2 * 0.75;
        (
            self.left.process(left, cutoff, context.sample_rate) * resonance_gain,
            self.right.process(right, cutoff, context.sample_rate) * resonance_gain,
        )
    }

    fn clear(&mut self) {
        self.left = OnePoleHighPass::default();
        self.right = OnePoleHighPass::default();
    }
}

struct DelayBuffer<'a> {
    samples: &'a mut [f32],
    index: &'a mut usize,
}

impl<'a> DelayBuffer<'a> {
    fn new(samples: &'a mut [f32], index: &'a mut usize) -> Option<Self> {
        if samples.len() < MIN_BUFFER_SAMPLES {
            return None;
        }
        if *index >= samples.len() {
            *index = 0;
        }
        Some(Self { samples, index })
    }

    fn max_delay(&self) -> f32 {
        (self.samples.len() - 2) as f32
    }

    fn read(&self, delay_samples: f32) -> f32 {
        let len = self.samples.len();
        let delay = delay_samples.clamp(MIN_DELAY_SAMPLES, self.max_delay());
        let read_position = *self.index as f32 + len as f32 - delay;
        let read_floor = crate::math::floor(read_position);
        let base = read_floor as usize % len;
        let fraction = read_position - read_floor;
        let next = (base + 1) % len;
        self.samples[base] * (1.0 - fraction) + self.samples[next] * fraction
    }

    fn write(&mut self, sample: f32) {
        self.samples[*self.index] = sample.clamp(-2.0, 2.0);
        *self.index = (*self.index + 1) % self.samples.len();
    }
}

#[derive(Default)]
struct PhaserState {
    stages: [f32; 6],
    feedback_state: f32,
}

impl PhaserState {
    fn process(&mut self, input: f32, coefficient: f32, feedback: f32) -> f32 {
        let mut value = input + self.feedback_state * feedback;
        for stage in &mut self.stages {
            let output = -coefficient * value + *stage;
            *stage = value + coefficient * output;
            value = output;
        }
        self.feedback_state = value;
        value
    }
}

#[derive(Default)]
struct OnePoleHighPass {
    previous_input: f32,
    previous_output: f32,
}

impl OnePoleHighPass {
    fn process(&mut self, input: f32, cutoff: f32, sample_rate: f32) -> f32 {
        let rc = 1.0 / (TAU * cutoff.clamp(20.0, sample_rate * 0.45));
        let dt = 1.0 / sample_rate.max(1.0);
        let alpha = rc / (rc + dt);
        let output = alpha * (self.previous_output + input - self.previous_input);
        self.previous_input = input;
        self.previous_output = output;
        output
    }
}

fn split_stereo(buffer: &mut [f32]) -> Option<(&mut [f32], &mut [f32])> {
    let midpoint = buffer.len() / 2;
    let (left, right) = buffer.split_at_mut(midpoint);
    (left.len() >= MIN_BUFFER_SAMPLES && right.len() >= MIN_BUFFER_SAMPLES).then_some((left, right))
}

fn take_segment<'a>(remaining: &mut &'a mut [f32], length: usize) -> &'a mut [f32] {
    let samples = core::mem::take(remaining);
    let (segment, rest) = samples.split_at_mut(length);
    *remaining = rest;
    segment
}

fn delay_seconds(context: ProcessContext) -> f32 {
    if context.clock_sync {
        let index = crate::math::round(context.param1 * 10.0) as usize;
        let beats = [
            4.0,
            3.0,
            2.0,
            1.5,
            1.0,
            4.0 / 3.0,
            0.75,
            0.5,
            1.0 / 3.0,
            0.375,
            0.25,
        ][index.min(10)];
        let mut seconds = beats * 60.0 / context.tempo_bpm.max(1.0);
        while seconds > 1.0 {
            seconds *= 0.5;
        }
        seconds
    } else {
        0.001 + context.param1 * 0.999
    }
}

fn reverb_comb(
    buffer: &mut [f32],
    index: &mut usize,
    tone_state: &mut f32,
    input: f32,
    delay_samples: f32,
    feedback: f32,
    tone: f32,
) -> f32 {
    let mut delay = DelayBuffer::new(buffer, index).expect("reverb segments are validated");
    let delayed = delay.read(delay_samples);
    *tone_state += (delayed - *tone_state) * tone.clamp(0.01, 1.0);
    delay.write(input * 0.55 + *tone_state * feedback.clamp(0.0, 0.97));
    *tone_state
}

fn reverb_allpass(buffer: &mut [f32], index: &mut usize, input: f32, delay_samples: f32) -> f32 {
    let mut delay = DelayBuffer::new(buffer, index).expect("reverb segments are validated");
    let delayed = delay.read(delay_samples);
    let output = delayed - input;
    delay.write(input + delayed * 0.5);
    output
}

fn advance_phase(phase: f32, frequency: f32, sample_rate: f32) -> f32 {
    wrap01(phase + frequency.max(0.0) / sample_rate.max(1.0))
}

fn wrap01(value: f32) -> f32 {
    value - crate::math::floor(value)
}

fn sine(phase: f32) -> f32 {
    crate::math::effect_sin(TAU * phase)
}

fn smoothstep(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

fn crossfade(left: f32, right: f32, amount: f32) -> f32 {
    left + (right - left) * amount.clamp(0.0, 1.0)
}

fn soft_clip(input: f32, gain: f32) -> f32 {
    crate::math::tanh(input * gain)
}
