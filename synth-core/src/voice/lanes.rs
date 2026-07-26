use crate::math::WideF32;

use super::NoteGlide;
use super::voice_pan_position;

#[derive(Clone, Copy, Default)]
struct LaneFade {
    start: f32,
    target: f32,
    remaining: u32,
    total: u32,
}

#[derive(Clone, Copy)]
pub(crate) struct PendingNote {
    pub note: u8,
    pub velocity: f32,
    pub reset_key_synced_lfos: bool,
    pub glide: NoteGlide,
}

#[derive(Clone, Copy)]
struct Lane {
    note: u8,
    velocity: f32,
    gate: bool,
    age: u64,
    pending: Option<PendingNote>,
    tuning_cents: f32,
    pan_position: f32,
    lifecycle_gain: f32,
    lifecycle_fade: LaneFade,
}

impl Default for Lane {
    fn default() -> Self {
        Self {
            note: 60,
            velocity: 1.0,
            gate: false,
            age: 0,
            pending: None,
            tuning_cents: 0.0,
            pan_position: 0.0,
            lifecycle_gain: 0.0,
            lifecycle_fade: LaneFade::default(),
        }
    }
}

pub(crate) struct Lanes {
    lanes: [Lane; WideF32::LANES],
    pending_mask: u8,
}

impl Lanes {
    pub fn new() -> Self {
        Self {
            lanes: core::array::from_fn(|lane| Lane {
                pan_position: voice_pan_position(lane, WideF32::LANES),
                ..Lane::default()
            }),
            pending_mask: 0,
        }
    }

    pub fn velocities(&self) -> WideF32 {
        WideF32::new(self.lanes.map(|lane| lane.velocity))
    }

    pub fn notes_as_f32(&self) -> WideF32 {
        WideF32::new(self.lanes.map(|lane| lane.note as f32))
    }

    pub fn pan_positions(&self) -> WideF32 {
        WideF32::new(self.lanes.map(|lane| lane.pan_position))
    }

    pub fn note_semitones(&self) -> [f32; WideF32::LANES] {
        core::array::from_fn(|lane| {
            f32::from(self.lanes[lane].note) + self.lanes[lane].tuning_cents / 100.0
        })
    }

    pub fn tuning_cents_array(&self) -> [f32; WideF32::LANES] {
        self.lanes.map(|lane| lane.tuning_cents)
    }

    pub fn set_tuning_cents_array(&mut self, tuning_cents: [f32; WideF32::LANES]) {
        for (lane, cents) in self.lanes.iter_mut().zip(tuning_cents) {
            lane.tuning_cents = cents;
        }
    }

    pub fn set_pan_positions(&mut self, positions: [f32; WideF32::LANES]) {
        for (lane, position) in self.lanes.iter_mut().zip(positions) {
            lane.pan_position = position.clamp(-1.0, 1.0);
        }
    }

    pub fn gate(&self, lane: usize) -> bool {
        self.lanes[lane].gate
    }

    pub fn age(&self, lane: usize) -> u64 {
        self.lanes[lane].age
    }

    pub(super) fn lifecycle_gains_array(&self) -> [f32; WideF32::LANES] {
        self.lanes.map(|lane| lane.lifecycle_gain)
    }

    pub fn has_pending(&self, lane: usize) -> bool {
        self.lanes[lane].pending.is_some()
    }

    pub fn pending(&self, lane: usize) -> Option<PendingNote> {
        self.lanes[lane].pending
    }

    pub fn pending_mask(&self) -> u8 {
        self.pending_mask
    }

    pub fn set_pending(&mut self, lane: usize, pending: PendingNote, tuning_cents: f32) {
        self.lanes[lane].pending = Some(pending);
        self.lanes[lane].tuning_cents = tuning_cents;
        self.pending_mask |= 1 << lane;
    }

    pub fn clear_pending(&mut self, lane: usize) {
        self.lanes[lane].pending = None;
        self.pending_mask &= !(1 << lane);
    }

    pub fn clear_all_pending(&mut self) {
        for lane in &mut self.lanes {
            lane.pending = None;
        }
        self.pending_mask = 0;
    }

    pub fn take_pending(&mut self, lane: usize) -> Option<PendingNote> {
        self.lanes[lane].pending.take()
    }

    pub fn begin_note_on(&mut self, lane: usize, note: u8, velocity: f32, tuning_cents: f32) {
        self.lanes[lane].pending = None;
        self.pending_mask &= !(1 << lane);
        self.lanes[lane].note = note;
        self.lanes[lane].velocity = velocity;
        self.lanes[lane].gate = true;
        self.lanes[lane].age = 0;
        self.lanes[lane].tuning_cents = tuning_cents;
    }

    pub fn update_sounding_lane(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        tuning_cents: f32,
    ) {
        self.lanes[lane].note = note;
        self.lanes[lane].velocity = velocity;
        self.lanes[lane].tuning_cents = tuning_cents;
    }

    pub fn update_pending_lane(
        &mut self,
        lane: usize,
        note: u8,
        velocity: f32,
        tuning_cents: f32,
        should_glide: bool,
    ) -> bool {
        let Some(pending) = &mut self.lanes[lane].pending else {
            return false;
        };
        pending.note = note;
        pending.velocity = velocity;
        pending.glide.enabled = should_glide;
        self.lanes[lane].tuning_cents = tuning_cents;
        true
    }

    pub fn release_gate(&mut self, lane: usize) {
        self.lanes[lane].gate = false;
    }

    pub fn release_all_gates(&mut self) {
        for lane in &mut self.lanes {
            lane.gate = false;
        }
    }

    pub fn advance_ages(&mut self) {
        for lane in &mut self.lanes {
            if lane.gate {
                lane.age += 1;
            }
        }
    }

    pub fn active_note(&self, lane: usize) -> Option<u8> {
        self.lanes[lane]
            .pending
            .map(|pending| pending.note)
            .or_else(|| self.lanes[lane].gate.then_some(self.lanes[lane].note))
    }

    pub fn for_each_active_note(&self, mut f: impl FnMut(u8)) {
        for lane in &self.lanes {
            if let Some(pending) = lane.pending {
                f(pending.note);
            } else if lane.gate {
                f(lane.note);
            }
        }
    }

    pub fn lifecycle_gain(&self, lane: usize) -> f32 {
        self.lanes[lane].lifecycle_gain
    }

    pub fn lifecycle_fade_remaining(&self, lane: usize) -> u32 {
        self.lanes[lane].lifecycle_fade.remaining
    }

    pub fn activate_lifecycle_lane(
        &mut self,
        lane: usize,
        vca_initial_level: f32,
        shutdown_samples: u32,
    ) {
        if vca_initial_level > 0.0 && self.lanes[lane].lifecycle_gain < 1.0 {
            self.start_lifecycle_fade(lane, 1.0, shutdown_samples);
        } else {
            self.set_lifecycle_gain(lane, 1.0);
        }
    }

    pub fn fade_out_lifecycle_lane(&mut self, lane: usize, shutdown_samples: u32) {
        self.start_lifecycle_fade(lane, 0.0, shutdown_samples);
    }

    pub fn next_lifecycle_gain(&mut self) -> WideF32 {
        let mut gains = self.lifecycle_gains_array();
        for (lane, gain) in gains.iter_mut().enumerate() {
            let fade = &mut self.lanes[lane].lifecycle_fade;
            let remaining = fade.remaining;
            if remaining == 0 {
                continue;
            }
            let next_remaining = remaining - 1;
            fade.remaining = next_remaining;
            if next_remaining == 0 {
                *gain = fade.target;
                self.lanes[lane].lifecycle_gain = fade.target;
                continue;
            }
            let progress = 1.0 - next_remaining as f32 / fade.total as f32;
            let smooth = progress * progress * (3.0 - 2.0 * progress);
            let start = fade.start;
            *gain = start + (fade.target - start) * smooth;
            self.lanes[lane].lifecycle_gain = *gain;
        }
        WideF32::new(gains)
    }

    fn start_lifecycle_fade(&mut self, lane: usize, target: f32, shutdown_samples: u32) {
        let current = self.lanes[lane].lifecycle_gain;
        if current == target {
            self.set_lifecycle_gain(lane, target);
            return;
        }
        let samples = shutdown_samples.max(1);
        let fade = &mut self.lanes[lane].lifecycle_fade;
        fade.start = current;
        fade.target = target;
        fade.remaining = samples;
        fade.total = samples;
    }

    fn set_lifecycle_gain(&mut self, lane: usize, gain: f32) {
        self.lanes[lane].lifecycle_gain = gain;
        let fade = &mut self.lanes[lane].lifecycle_fade;
        fade.start = gain;
        fade.target = gain;
        fade.remaining = 0;
        fade.total = 0;
    }
}

#[cfg(test)]
impl Lanes {
    pub(crate) fn pan_positions_array(&self) -> [f32; WideF32::LANES] {
        self.lanes.map(|lane| lane.pan_position)
    }

    pub(crate) fn gates_array(&self) -> [bool; WideF32::LANES] {
        self.lanes.map(|lane| lane.gate)
    }

    pub(crate) fn notes_array(&self) -> [u8; WideF32::LANES] {
        self.lanes.map(|lane| lane.note)
    }

    pub(crate) fn note(&self, lane: usize) -> u8 {
        self.lanes[lane].note
    }
}
