//! Bounded patch changes for the real-time audio task.

use synth_core::Patch;

const FADE_BLOCKS: u8 = 3;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    FadeOut { remaining: u8 },
    Apply,
    FadeIn { remaining: u8 },
}

pub struct PatchTransition {
    pending: Option<Patch>,
    state: State,
    gain: f32,
}

pub struct BlockAction {
    pub patch: Option<Patch>,
    pub render: bool,
}

impl Default for PatchTransition {
    fn default() -> Self {
        Self {
            pending: None,
            state: State::Idle,
            gain: 1.0,
        }
    }
}

impl PatchTransition {
    /// Queues the newest complete patch. Repeated updates replace the pending
    /// snapshot without allocating or lengthening an active fade-out.
    pub fn enqueue(&mut self, patch: Patch) {
        self.pending = Some(patch);
        match self.state {
            State::Idle | State::FadeIn { .. } => {
                self.state = State::FadeOut {
                    remaining: FADE_BLOCKS,
                };
            }
            State::FadeOut { .. } | State::Apply => {}
        }
    }

    /// Chooses the work for one audio block. Program mutation receives a whole
    /// silent block and is never combined with DSP rendering.
    pub fn begin_block(&mut self) -> BlockAction {
        if self.state != State::Apply {
            return BlockAction {
                patch: None,
                render: true,
            };
        }

        let patch = self.pending.take();
        self.gain = 0.0;
        self.state = if patch.is_some() {
            State::FadeIn {
                remaining: FADE_BLOCKS,
            }
        } else {
            State::Idle
        };
        BlockAction {
            patch,
            render: false,
        }
    }

    pub fn finish_block(&mut self, interleaved: &mut [f32], rendered: bool) {
        if !rendered {
            interleaved.fill(0.0);
            return;
        }

        let start_gain = self.gain;
        match self.state {
            State::Idle => self.gain = 1.0,
            State::FadeOut { remaining } => {
                self.gain -= self.gain / f32::from(remaining);
                self.state = if remaining == 1 {
                    State::Apply
                } else {
                    State::FadeOut {
                        remaining: remaining - 1,
                    }
                };
            }
            State::FadeIn { remaining } => {
                self.gain += (1.0 - self.gain) / f32::from(remaining);
                self.state = if remaining == 1 {
                    State::Idle
                } else {
                    State::FadeIn {
                        remaining: remaining - 1,
                    }
                };
            }
            State::Apply => {}
        }

        if start_gain != 1.0 || self.gain != 1.0 {
            let frames = (interleaved.len() / 2).max(1) as f32;
            let step = (self.gain - start_gain) / frames;
            let mut gain = start_gain;
            for frame in interleaved.chunks_exact_mut(2) {
                gain += step;
                frame[0] *= gain;
                frame[1] *= gain;
            }
        }
    }

    pub const fn is_idle(&self) -> bool {
        matches!(self.state, State::Idle) && self.pending.is_none()
    }
}

#[cfg(test)]
mod tests {
    use synth_core::{LayerMode, Patch};

    use super::PatchTransition;

    #[test]
    fn transition_delivers_the_complete_two_layer_program() {
        let mut program = Patch {
            mode: LayerMode::Split,
            split_point: 71,
            ..Patch::default()
        };
        program.layer_a.filter.cutoff = 1_000.0;
        program.layer_b.filter.cutoff = 4_000.0;
        let mut transition = PatchTransition::default();
        transition.enqueue(program);
        let mut output = [1.0; 96];

        let delivered = loop {
            let action = transition.begin_block();
            if let Some(program) = action.patch {
                break program;
            }
            transition.finish_block(&mut output, action.render);
        };

        assert_eq!(delivered.layer_a.filter.cutoff, 1_000.0);
        assert_eq!(delivered.layer_b.filter.cutoff, 4_000.0);
        assert_eq!(delivered.mode, LayerMode::Split);
        assert_eq!(delivered.split_point, 71);
    }
}
