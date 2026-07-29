//! Per-layer Rev2 program-image constants and SysEx layer codec.

use core::marker::PhantomData;

use crate::patch::decode_patch_name;
use crate::{LayerId, LayerPatch, ParamId};

use super::encoder::ControllerEncoder;
use super::map::{
    LfoPairingState, MappedUpdate, map_nrpn_with_lfo, program_nrpn_value, quantize, store_nrpn,
    unit,
};
use super::program::PROGRAM_DATA_LEN;

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
    const NAME_RANGE: core::ops::Range<usize> = 235..255;
    const NRPN_OFFSET: u16 = 0;
}

impl Layer for LayerB {
    const ID: LayerId = LayerId::B;
    const DATA_OFFSET: usize = 1024;
    /// Verified against Sequential's official [Rev2 factory bank].
    ///
    /// [Rev2 factory bank]: https://sequential.com/support/download/prophet-rev2-sounds/
    const NAME_RANGE: core::ops::Range<usize> = 1259..1279;
    const NRPN_OFFSET: u16 = 2048;
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
        for number in 0..=179 {
            if let Some(value) = program_nrpn_value(raw, number, L::DATA_OFFSET) {
                map_nrpn_with_lfo(number, value, &mut state, &mut |update| match update {
                    MappedUpdate::Param(param, value) => patch.set_param(param, value),
                    MappedUpdate::MasterVolume(_) | MappedUpdate::MidiClockMode(_) => {}
                    MappedUpdate::Modulation { route, parameter } => {
                        patch.set_modulation_param(route, parameter)
                    }
                });
            }
        }
        patch.name = decode_patch_name(&raw[L::NAME_RANGE]);
        patch.set_param(
            ParamId::VcaInitialLevel,
            unit(u16::from(raw[L::DATA_OFFSET + 27]), 127),
        );
        patch
    }

    /// Encode one layer into an unpacked Rev2 program image.
    pub fn encode(patch: &LayerPatch, raw: &mut [u8; PROGRAM_DATA_LEN]) {
        let mut encoder = ControllerEncoder::default();
        patch.for_each_param(|param, value| {
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
        raw[L::DATA_OFFSET + 27] = quantize(patch.amplifier.initial_level, 0.0, 1.0, 127) as u8;
        raw[L::NAME_RANGE].fill(b' ');
        raw[L::NAME_RANGE.start..L::NAME_RANGE.start + patch.name.len()]
            .copy_from_slice(patch.name.as_bytes());
    }
}
