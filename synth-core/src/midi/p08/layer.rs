//! Per-layer Prophet '08 program-image constants and SysEx layer codec.

use core::marker::PhantomData;

use crate::{
    ClockDivision, GATED_STEP_COUNT, GATED_TRACK_COUNT, GatedDestination, GatedSequencerMode,
    GatedStep, LayerId, LayerPatch, SequencerType,
    midi::p08::{
        map::{MidiUpdate, map_nrpn, p08_mod_destination, program_nrpn_value},
        program::PROGRAM_DATA_LEN,
    },
};

/// Layer-specific Prophet '08 program-image addressing.
pub trait Layer {
    #[allow(dead_code)]
    const ID: LayerId;
    const DATA_OFFSET: usize;
    const NAME_RANGE: core::ops::Range<usize>;
    #[allow(dead_code)]
    const NRPN_OFFSET: u16;
}

/// Layer A in the official [Prophet '08 program image].
///
/// [Prophet '08 program image]: https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf
pub struct LayerA;

impl Layer for LayerA {
    const ID: LayerId = LayerId::A;
    const DATA_OFFSET: usize = 0;
    const NAME_RANGE: core::ops::Range<usize> = 184..200;
    const NRPN_OFFSET: u16 = 0;
}

/// Layer B in the official [Prophet '08 program image].
///
/// [Prophet '08 program image]: https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf
pub struct LayerB;

impl Layer for LayerB {
    const ID: LayerId = LayerId::B;
    const DATA_OFFSET: usize = 200;
    /// Present in the image layout; naming uses the shared program name instead.
    const NAME_RANGE: core::ops::Range<usize> = 368..384;
    const NRPN_OFFSET: u16 = 0;
}

/// Decodes one layer half of a Prophet '08 program image.
///
/// Does not set [`LayerPatch::name`]; [`super::program::decode`] applies the
/// shared program name to both layers.
pub struct LayerDecoder<L: Layer> {
    _layer: PhantomData<L>,
}

impl<L: Layer> LayerDecoder<L> {
    /// Decode one layer from an unpacked Prophet '08 program image.
    pub fn decode(raw: &[u8; PROGRAM_DATA_LEN]) -> LayerPatch {
        let mut patch = LayerPatch::default();
        for number in 0..=119 {
            if let Some(value) = program_nrpn_value(raw, number, L::DATA_OFFSET) {
                map_nrpn(number, value, &mut |update| match update {
                    MidiUpdate::Param(param, value) => patch.set_param(param, value),
                    MidiUpdate::Modulation { route, parameter } => {
                        patch.set_modulation_param(route, parameter);
                    }
                });
            }
        }
        decode_sequence::<L>(raw, &mut patch);
        patch.glide_enabled = patch.osc1.glide > 0.0 || patch.osc2.glide > 0.0;
        patch
    }
}

fn decode_sequence<L: Layer>(raw: &[u8; PROGRAM_DATA_LEN], patch: &mut LayerPatch) {
    let offset = L::DATA_OFFSET;
    patch.bpm = f32::from(raw[offset + 91].clamp(30, 250));
    patch.clock_divide = ClockDivision::from_index(usize::from(raw[offset + 92].min(12)));
    patch.sequence.sequencer_type = if raw[offset + 101] == 0 {
        SequencerType::Polyphonic
    } else {
        SequencerType::Gated
    };
    patch.sequence.gated_mode =
        GatedSequencerMode::from_index(usize::from(raw[offset + 94].min(4)));

    for track in 0..GATED_TRACK_COUNT {
        let destination = raw[offset + 77 + track];
        patch.sequence.gated.tracks[track].destination = if destination == 0 {
            GatedDestination::Off
        } else {
            GatedDestination::Modulation(p08_mod_destination(u16::from(destination)))
        };

        for step in 0..GATED_STEP_COUNT {
            let value = raw[offset + 120 + track * GATED_STEP_COUNT + step];
            patch.sequence.gated.tracks[track].steps[step] =
                GatedStep::from_rev2_raw(u16::from(value));
        }
    }
}
