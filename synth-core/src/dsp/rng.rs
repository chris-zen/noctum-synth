use rand_core::Rng;
use rand_pcg::Pcg32;

pub(crate) struct DspRng {
    rng: Pcg32,
}

impl DspRng {
    pub(crate) fn new(state: u64, stream: u64) -> Self {
        Self {
            rng: Pcg32::new(state, stream),
        }
    }

    pub(crate) fn f32(&mut self) -> f32 {
        const SCALE: f32 = 1.0 / ((u32::MAX as f32) + 1.0);
        self.rng.next_u32() as f32 * SCALE
    }

    pub(crate) fn u32_inclusive(&mut self, min: u32, max: u32) -> u32 {
        if min >= max {
            return min;
        }

        let range = max - min + 1;
        min + (self.rng.next_u32() % range)
    }
}
