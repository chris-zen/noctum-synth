//! Per-layer Prophet '08 program-image constants and SysEx layer codec.

use core::marker::PhantomData;

use crate::{
    ClockDivision, GatedDestination, GatedSequencerMode, GatedStep, LayerId, LayerPatch,
    SequencerType,
    midi::p08::{
        map::{MidiUpdate, map_nrpn, p08_mod_destination, program_nrpn_value},
        program::PROGRAM_DATA_LEN,
    },
    midi::prophet::{MAX_BPM, MIN_BPM},
    sequencer::model::{GATED_STEP_COUNT, GATED_TRACK_COUNT},
};

const LAYER_DATA_LEN: usize = 200;
const LAYER_NAME_OFFSET: usize = 184;
const LAYER_NAME_END: usize = LAYER_DATA_LEN;
const BPM_OFFSET: usize = 91;
const CLOCK_DIVIDE_OFFSET: usize = 92;
const GATED_MODE_OFFSET: usize = 94;
const SEQUENCER_TYPE_OFFSET: usize = 101;
const GATED_DESTINATION_BASE: usize = 77;
const GATED_STEP_DATA_BASE: usize = 120;
const CLOCK_DIVIDE_MAX_INDEX: u8 = 12;
const GATED_MODE_MAX_INDEX: u8 = 4;
const MAX_LAYER_NRPN: u16 = 119;

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
    const NAME_RANGE: core::ops::Range<usize> = LAYER_NAME_OFFSET..LAYER_NAME_END;
    const NRPN_OFFSET: u16 = 0;
}

/// Layer B in the official [Prophet '08 program image].
///
/// [Prophet '08 program image]: https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf
pub struct LayerB;

impl Layer for LayerB {
    const ID: LayerId = LayerId::B;
    const DATA_OFFSET: usize = LAYER_DATA_LEN;
    /// Present in the image layout; naming uses the shared program name instead.
    const NAME_RANGE: core::ops::Range<usize> =
        LAYER_DATA_LEN + LAYER_NAME_OFFSET..LAYER_DATA_LEN * 2;
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
        for number in 0..=MAX_LAYER_NRPN {
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
    patch.bpm = f32::from(raw[offset + BPM_OFFSET].clamp(MIN_BPM, MAX_BPM));
    patch.clock_divide = ClockDivision::from_index(usize::from(
        raw[offset + CLOCK_DIVIDE_OFFSET].min(CLOCK_DIVIDE_MAX_INDEX),
    ));
    patch.sequence.sequencer_type = if raw[offset + SEQUENCER_TYPE_OFFSET] == 0 {
        SequencerType::Polyphonic
    } else {
        SequencerType::Gated
    };
    patch.sequence.gated_mode = GatedSequencerMode::from_index(usize::from(
        raw[offset + GATED_MODE_OFFSET].min(GATED_MODE_MAX_INDEX),
    ));

    for track in 0..GATED_TRACK_COUNT {
        let destination = raw[offset + GATED_DESTINATION_BASE + track];
        patch.sequence.gated.tracks[track].destination = if destination == 0 {
            GatedDestination::Off
        } else {
            GatedDestination::Modulation(p08_mod_destination(u16::from(destination)))
        };

        for step in 0..GATED_STEP_COUNT {
            let value = raw[offset + GATED_STEP_DATA_BASE + track * GATED_STEP_COUNT + step];
            patch.sequence.gated.tracks[track].steps[step] =
                GatedStep::from_rev2_raw(u16::from(value));
        }
    }
}
