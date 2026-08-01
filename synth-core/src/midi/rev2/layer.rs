//! Per-layer Rev2 program-image constants and SysEx layer codec.

use core::marker::PhantomData;

use crate::{
    LayerId, LayerPatch, ParamId, SequencerType,
    midi::rev2::{
        encoder::ControllerEncoder,
        ids::{
            NRPN_GATED_DESTINATION_START, NRPN_GATED_MODE, NRPN_GATED_STEP_START,
            NRPN_POLY_NOTE_START, NRPN_SEQUENCER_TYPE, POLY_LANE_NRPN_STRIDE,
            POLY_VELOCITY_NRPN_OFFSET,
        },
        map::{
            LfoPairingState, MappedUpdate, map_nrpn_with_lfo, program_nrpn_value, quantize,
            store_nrpn, store_program_nrpn, unit,
        },
        program::PROGRAM_DATA_LEN,
    },
    patch::decode_patch_name,
    sequencer::model::GATED_STEP_COUNT,
};

const LAYER_IMAGE_LEN: usize = 1024;
const LAYER_NAME_OFFSET: usize = 235;
const LAYER_NAME_LEN: usize = 20;
const SEQUENCER_TYPE_IMAGE_OFFSET: usize = 139;
const VCA_INITIAL_LEVEL_OFFSET: usize = 27;
const MAX_LAYER_NRPN: u16 = 1043;

/// Layer-specific Rev2 program-image and NRPN addressing.
pub trait Layer {
    const ID: LayerId;
    const DATA_OFFSET: usize;
    const NAME_RANGE: core::ops::Range<usize>;
    const NRPN_OFFSET: u16;
}

/// Layer A in the official [Rev2 SysEx format] / [MIDI implementation].
///
/// [Rev2 SysEx format]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
/// [MIDI implementation]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
pub struct LayerA;

/// Layer B in the official [Rev2 SysEx format] / [MIDI implementation].
///
/// [Rev2 SysEx format]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
/// [MIDI implementation]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
pub struct LayerB;

impl Layer for LayerA {
    const ID: LayerId = LayerId::A;
    const DATA_OFFSET: usize = 0;
    /// Verified against Sequential's official [Rev2 factory bank].
    ///
    /// [Rev2 factory bank]: https://sequential.com/support/download/prophet-rev2-sounds/
    const NAME_RANGE: core::ops::Range<usize> =
        LAYER_NAME_OFFSET..LAYER_NAME_OFFSET + LAYER_NAME_LEN;
    const NRPN_OFFSET: u16 = 0;
}

impl Layer for LayerB {
    const ID: LayerId = LayerId::B;
    const DATA_OFFSET: usize = LAYER_IMAGE_LEN;
    /// Verified against Sequential's official [Rev2 factory bank].
    ///
    /// [Rev2 factory bank]: https://sequential.com/support/download/prophet-rev2-sounds/
    const NAME_RANGE: core::ops::Range<usize> = Self::DATA_OFFSET + LAYER_NAME_OFFSET
        ..Self::DATA_OFFSET + LAYER_NAME_OFFSET + LAYER_NAME_LEN;
    const NRPN_OFFSET: u16 = LAYER_IMAGE_LEN as u16 * 2;
}

/// Decodes and encodes one layer half of a Rev2 program image.
pub struct LayerDecoder<L: Layer> {
    _layer: PhantomData<L>,
}

impl<L: Layer> LayerDecoder<L> {
    /// Decode one layer from an unpacked Rev2 program image.
    pub fn decode(raw: &[u8; PROGRAM_DATA_LEN]) -> LayerPatch {
        let mut patch = LayerPatch::default();
        let mut state = LfoPairingState::default();
        for number in 0..=MAX_LAYER_NRPN {
            // Program byte 139 is a Type enum (0 Gated, 1 Polyphonic), while
            // live NRPN 183 is an on/off switch with the opposite polarity.
            if number == NRPN_SEQUENCER_TYPE {
                continue;
            }
            if let Some(value) = program_nrpn_value(raw, number, L::DATA_OFFSET) {
                map_nrpn_with_lfo(number, value, &mut state, &mut |update| match update {
                    MappedUpdate::Param(param, value) => patch.set_param(param, value),
                    MappedUpdate::MasterVolume(_)
                    | MappedUpdate::MidiClockMode(_)
                    | MappedUpdate::LayerMode(_)
                    | MappedUpdate::SplitPoint(_) => {}
                    MappedUpdate::Modulation { route, parameter } => {
                        patch.set_modulation_param(route, parameter)
                    }
                    MappedUpdate::Sequence(update) => patch.sequence.apply(update),
                    MappedUpdate::SequencerRunning(_) => {}
                    MappedUpdate::SequencerRecording(_) => {}
                });
            }
        }
        patch.sequence.sequencer_type = SequencerType::from_index(usize::from(
            raw[L::DATA_OFFSET + SEQUENCER_TYPE_IMAGE_OFFSET].min(1),
        ));
        patch.name = decode_patch_name(&raw[L::NAME_RANGE]);
        patch.set_param(
            ParamId::VcaInitialLevel,
            unit(
                u16::from(raw[L::DATA_OFFSET + VCA_INITIAL_LEVEL_OFFSET]),
                127,
            ),
        );
        patch
    }

    /// Encode one layer into an unpacked Rev2 program image.
    pub fn encode(patch: &LayerPatch, raw: &mut [u8; PROGRAM_DATA_LEN]) {
        let mut encoder = ControllerEncoder::default();
        patch.for_each_param(|param, value| {
            // Program byte 139 uses enum polarity, not live NRPN 183 polarity.
            if param == ParamId::SequencerType {
                return;
            }
            let inactive_lfo_rate = match param {
                ParamId::Lfo1Rate => patch.lfos[0].clock_sync,
                ParamId::Lfo2Rate => patch.lfos[1].clock_sync,
                ParamId::Lfo3Rate => patch.lfos[2].clock_sync,
                ParamId::Lfo4Rate => patch.lfos[3].clock_sync,
                ParamId::Lfo1SyncDivision => !patch.lfos[0].clock_sync,
                ParamId::Lfo2SyncDivision => !patch.lfos[1].clock_sync,
                ParamId::Lfo3SyncDivision => !patch.lfos[2].clock_sync,
                ParamId::Lfo4SyncDivision => !patch.lfos[3].clock_sync,
                _ => false,
            };
            if inactive_lfo_rate {
                return;
            }
            let mut messages = [[0_u8; 3]; 4];
            let mut len = 0;
            if encoder.param(0, param, value, |message| {
                messages[len] = message;
                len += 1;
            }) {
                store_nrpn(raw, &messages[..len], L::DATA_OFFSET);
            }
        });
        patch.for_each_modulation(|route, slot| {
            let mut messages = [[0_u8; 3]; 12];
            let mut len = 0;
            encoder.modulation(
                0,
                route,
                slot.enabled,
                slot.source,
                slot.destination,
                slot.amount,
                |message| {
                    messages[len] = message;
                    len += 1;
                },
            );
            for sequence in messages[..len].chunks_exact(4) {
                store_nrpn(raw, sequence, L::DATA_OFFSET);
            }
        });
        store_program_nrpn(
            raw,
            NRPN_GATED_MODE,
            patch.sequence.gated_mode.index() as u16,
            L::DATA_OFFSET,
        );
        store_program_nrpn(
            raw,
            NRPN_SEQUENCER_TYPE,
            patch.sequence.sequencer_type.index() as u16,
            L::DATA_OFFSET,
        );
        for (track_index, track) in patch.sequence.gated.tracks.iter().enumerate() {
            store_program_nrpn(
                raw,
                NRPN_GATED_DESTINATION_START + track_index as u16,
                track.destination.rev2_raw(),
                L::DATA_OFFSET,
            );
            for (step_index, step) in track.steps.iter().copied().enumerate() {
                store_program_nrpn(
                    raw,
                    NRPN_GATED_STEP_START + (track_index * GATED_STEP_COUNT + step_index) as u16,
                    step.rev2_raw(),
                    L::DATA_OFFSET,
                );
            }
        }
        for (step_index, step) in patch.sequence.poly.steps.iter().enumerate() {
            for (lane_index, lane) in step.lanes.iter().copied().enumerate() {
                let base = NRPN_POLY_NOTE_START + lane_index as u16 * POLY_LANE_NRPN_STRIDE;
                store_program_nrpn(
                    raw,
                    base + step_index as u16,
                    lane.note.rev2_raw(),
                    L::DATA_OFFSET,
                );
                store_program_nrpn(
                    raw,
                    base + POLY_VELOCITY_NRPN_OFFSET + step_index as u16,
                    lane.velocity.rev2_raw(),
                    L::DATA_OFFSET,
                );
            }
        }
        raw[L::DATA_OFFSET + VCA_INITIAL_LEVEL_OFFSET] =
            quantize(patch.amplifier.initial_level, 0.0, 1.0, 127) as u8;
        raw[L::NAME_RANGE].fill(b' ');
        raw[L::NAME_RANGE.start..L::NAME_RANGE.start + patch.name.len()]
            .copy_from_slice(patch.name.as_bytes());
    }
}
