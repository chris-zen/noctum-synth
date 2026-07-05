//! Low-frequency oscillators for periodic modulation.

use wide::f32x4;

use crate::rng::DspRng;
use crate::{LANES, wrap01};

/// Minimum LFO rate in Hz.
pub const MIN_LFO_RATE_HZ: f32 = 0.022;
/// Maximum LFO rate in Hz.
pub const MAX_LFO_RATE_HZ: f32 = 500.0;

/// LFO waveform shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LfoWaveform {
    Triangle,
    Saw,
    ReverseSaw,
    Square,
    SampleAndHold,
}

/// Four-lane LFO with configurable rate, depth, waveform, and key sync.
pub struct Lfo {
    phase: f32x4,
    phase_inc: f32x4,
    waveform: LfoWaveform,
    key_sync: bool,
    depth: f32,
    sample_and_hold: f32x4,
    rng: [DspRng; LANES],
    sample_rate: f32,
}

impl Default for Lfo {
    fn default() -> Self {
        Self::new(crate::DEFAULT_SAMPLE_RATE)
    }
}

impl Lfo {
    pub fn new(sample_rate: f32) -> Self {
        let mut lfo = Self {
            phase: f32x4::splat(0.0),
            phase_inc: f32x4::splat(0.0),
            waveform: LfoWaveform::Triangle,
            key_sync: true,
            depth: 0.0,
            sample_and_hold: f32x4::splat(0.0),
            rng: [
                DspRng::new(0x4c46_4f01, 0x5148_0001),
                DspRng::new(0x4c46_4f02, 0x5148_0002),
                DspRng::new(0x4c46_4f03, 0x5148_0003),
                DspRng::new(0x4c46_4f04, 0x5148_0004),
            ],
            sample_rate,
        };
        lfo.set_rate_hz(MIN_LFO_RATE_HZ);
        lfo.refresh_sample_and_hold();
        lfo
    }

    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate.max(1.0);
        let rate = self.phase_inc.to_array()[0] * self.sample_rate;
        self.set_rate_hz(rate);
    }

    pub fn set_rate_hz(&mut self, rate_hz: f32) {
        let rate_hz = rate_hz.clamp(MIN_LFO_RATE_HZ, MAX_LFO_RATE_HZ);
        self.phase_inc = f32x4::splat(rate_hz / self.sample_rate.max(1.0));
    }

    pub fn set_depth(&mut self, depth: f32) {
        self.depth = depth.clamp(0.0, 1.0);
    }

    pub fn set_waveform(&mut self, waveform: LfoWaveform) {
        self.waveform = waveform;
    }

    pub fn set_key_sync(&mut self, key_sync: bool) {
        self.key_sync = key_sync;
    }

    pub fn key_sync(&self) -> bool {
        self.key_sync
    }

    pub fn reset_all(&mut self) {
        self.phase = f32x4::splat(0.0);
        if self.waveform == LfoWaveform::SampleAndHold {
            self.refresh_sample_and_hold();
        }
    }

    pub fn reset_lane(&mut self, lane: usize) {
        let mut phases = self.phase.to_array();
        phases[lane] = 0.0;
        self.phase = f32x4::new(phases);

        if self.waveform == LfoWaveform::SampleAndHold {
            let mut values = self.sample_and_hold.to_array();
            values[lane] = bipolar_random(&mut self.rng[lane]);
            self.sample_and_hold = f32x4::new(values);
        }
    }

    pub fn next(&mut self) -> f32x4 {
        let output = self.raw_output() * f32x4::splat(self.depth);
        self.advance();
        output
    }

    pub fn raw_output(&self) -> f32x4 {
        match self.waveform {
            LfoWaveform::Triangle => triangle(self.phase),
            LfoWaveform::Saw => self.phase,
            LfoWaveform::ReverseSaw => f32x4::splat(1.0) - self.phase,
            LfoWaveform::Square => square(self.phase),
            LfoWaveform::SampleAndHold => self.sample_and_hold,
        }
    }

    fn advance(&mut self) {
        let next = self.phase + self.phase_inc;
        self.phase = wrap01(next);

        if self.waveform == LfoWaveform::SampleAndHold {
            let next_lanes = next.to_array();
            let mut sample_and_hold = self.sample_and_hold.to_array();
            for lane in 0..LANES {
                if next_lanes[lane] >= 1.0 {
                    sample_and_hold[lane] = bipolar_random(&mut self.rng[lane]);
                }
            }
            self.sample_and_hold = f32x4::new(sample_and_hold);
        }
    }

    fn refresh_sample_and_hold(&mut self) {
        self.sample_and_hold = f32x4::new(core::array::from_fn(|lane| {
            bipolar_random(&mut self.rng[lane])
        }));
    }
}

fn triangle(phase: f32x4) -> f32x4 {
    let four = f32x4::splat(4.0);
    let scaled = phase * four;
    let rising = scaled;
    let falling = f32x4::splat(2.0) - scaled;
    let wrapping = scaled - four;
    phase.simd_lt(f32x4::splat(0.25)).blend(
        rising,
        phase.simd_lt(f32x4::splat(0.75)).blend(falling, wrapping),
    )
}

fn square(phase: f32x4) -> f32x4 {
    phase
        .simd_lt(f32x4::splat(0.5))
        .blend(f32x4::splat(1.0), f32x4::splat(0.0))
}

fn bipolar_random(rng: &mut DspRng) -> f32 {
    rng.f32() * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_lane(value: f32x4) -> f32 {
        value.to_array()[0]
    }

    #[test]
    fn triangle_is_bipolar() {
        let mut lfo = Lfo::new(4.0);
        lfo.set_rate_hz(1.0);
        lfo.set_depth(1.0);
        lfo.set_waveform(LfoWaveform::Triangle);

        let values = [
            first_lane(lfo.next()),
            first_lane(lfo.next()),
            first_lane(lfo.next()),
            first_lane(lfo.next()),
        ];

        assert_eq!(values, [0.0, 1.0, 0.0, -1.0]);
    }

    #[test]
    fn saw_square_and_reverse_saw_are_positive_only() {
        for waveform in [
            LfoWaveform::Saw,
            LfoWaveform::ReverseSaw,
            LfoWaveform::Square,
        ] {
            let mut lfo = Lfo::new(16.0);
            lfo.set_rate_hz(1.0);
            lfo.set_depth(1.0);
            lfo.set_waveform(waveform);

            for _ in 0..32 {
                let value = first_lane(lfo.next());
                assert!(
                    (0.0..=1.0).contains(&value),
                    "{waveform:?} produced {value}"
                );
            }
        }
    }

    #[test]
    fn sample_and_hold_updates_once_per_cycle() {
        let mut lfo = Lfo::new(4.0);
        lfo.set_rate_hz(1.0);
        lfo.set_depth(1.0);
        lfo.set_waveform(LfoWaveform::SampleAndHold);

        let held = first_lane(lfo.next());
        for _ in 0..3 {
            assert_eq!(first_lane(lfo.next()), held);
        }
        assert_ne!(first_lane(lfo.next()), held);
    }

    #[test]
    fn max_audio_rate_remains_finite() {
        let mut lfo = Lfo::new(44_100.0);
        lfo.set_rate_hz(500.0);
        lfo.set_depth(1.0);

        for _ in 0..44_100 {
            let value = first_lane(lfo.next());
            assert!(value.is_finite());
            assert!((-1.0..=1.0).contains(&value));
        }
    }
}
