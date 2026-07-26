use crate::dsp::{LfoWaveform, MAX_LFO_RATE_HZ, MIN_LFO_RATE_HZ};
use crate::math::WideF32;
use crate::patch::{ClockDivision, LFO_COUNT, LfoParams, LfoSyncDivision, ModDestination};

pub struct Lfo {
    engine: crate::dsp::Lfo,
    destination: ModDestination,
    clock_sync: bool,
    sync_division: LfoSyncDivision,
    base_rate_hz: f32,
    base_depth: f32,
    last_output: WideF32,
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            engine: crate::dsp::Lfo::new(sample_rate),
            destination: ModDestination::Off,
            clock_sync: false,
            sync_division: LfoSyncDivision::default(),
            base_rate_hz: MIN_LFO_RATE_HZ,
            base_depth: 0.0,
            last_output: WideF32::ZERO,
        }
    }

    pub fn apply_params(
        &mut self,
        params: &LfoParams,
        tempo_bpm: f32,
        clock_division: ClockDivision,
    ) {
        self.base_rate_hz = params.rate_hz.clamp(MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ);
        self.base_depth = params.depth.clamp(0.0, 1.0);
        self.destination = params.destination;
        self.clock_sync = params.clock_sync;
        self.sync_division = params.sync_division;
        self.engine.set_waveform(params.waveform);
        self.engine.set_key_sync(params.key_sync);
        self.engine.set_depth(self.base_depth);
        self.refresh_engine_rate(tempo_bpm, clock_division);
    }

    pub fn output(&self) -> WideF32 {
        self.last_output
    }

    #[cfg(test)]
    pub(crate) fn destination(&self) -> ModDestination {
        self.destination
    }

    pub fn base_depth(&self) -> f32 {
        self.base_depth
    }

    pub fn base_rate_hz(&self) -> f32 {
        self.base_rate_hz
    }

    pub fn clock_sync(&self) -> bool {
        self.clock_sync
    }

    pub fn set_base_rate_hz(&mut self, rate_hz: f32) -> f32 {
        self.base_rate_hz = rate_hz.clamp(MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ);
        self.base_rate_hz
    }

    pub fn set_depth(&mut self, depth: f32) -> f32 {
        self.base_depth = depth.clamp(0.0, 1.0);
        self.engine.set_depth(self.base_depth);
        self.base_depth
    }

    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.engine.set_waveform(waveform);
    }

    pub fn set_destination(&mut self, destination: ModDestination) {
        self.destination = destination;
    }

    pub fn set_clock_sync(&mut self, clock_sync: bool) {
        self.clock_sync = clock_sync;
    }

    pub fn set_sync_division(&mut self, division: LfoSyncDivision) {
        self.sync_division = division;
    }

    pub fn set_key_sync(&mut self, key_sync: bool) {
        self.engine.set_key_sync(key_sync);
    }

    pub fn output_is_uniform(&self) -> bool {
        self.engine.output_is_uniform()
    }

    pub fn effective_rate_hz(&self, tempo_bpm: f32, clock_division: ClockDivision) -> f32 {
        if self.clock_sync {
            self.sync_division
                .rate_hz(tempo_bpm, clock_division)
                .min(MAX_LFO_RATE_HZ)
        } else {
            self.base_rate_hz
        }
    }

    pub fn refresh_engine_rate(&mut self, tempo_bpm: f32, clock_division: ClockDivision) {
        self.engine
            .set_rate_hz(self.effective_rate_hz(tempo_bpm, clock_division));
    }

    pub(crate) fn apply_engine_rate(&mut self, rate_hz: f32) {
        self.engine.set_rate_hz(rate_hz.min(MAX_LFO_RATE_HZ));
    }

    pub(crate) fn apply_engine_depth(&mut self, depth: f32) {
        self.engine.set_depth(depth);
    }

    pub fn generate(&mut self) -> WideF32 {
        self.last_output = self.engine.next();
        self.last_output
    }

    pub fn advance_idle(&mut self) {
        self.engine.advance_silent();
        self.last_output = WideF32::ZERO;
    }

    pub fn reset_if_key_synced(&mut self) {
        if self.engine.key_sync() {
            self.engine.reset_all();
        }
    }
}

pub(crate) fn base_rates(lfos: &[Lfo; LFO_COUNT]) -> [f32; LFO_COUNT] {
    core::array::from_fn(|index| lfos[index].base_rate_hz())
}

pub(crate) fn base_depths(lfos: &[Lfo; LFO_COUNT]) -> [f32; LFO_COUNT] {
    core::array::from_fn(|index| lfos[index].base_depth())
}
