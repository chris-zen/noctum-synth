//! Fitted physical parameters for the Plan 12 Monologue saw-core experiment.

use super::gray_box_oscillator::{GrayBoxOutput, GrayBoxProfile};

pub(crate) const PROFILE_JSON_SHA256: &str =
    "e2cc80d25ee949803fe5f06d26a4945820c05acc83399dcd5ec60c531b852b26";

pub(crate) const KORG_MONOLOGUE_GRAY_BOX_V1: GrayBoxProfile = GrayBoxProfile {
    id: "korg-monologue-gray-box-saw-core-v1",
    target_id: "korg-monologue-v1",
    revision: 1,
    curvature: -0.85,
    saw: GrayBoxOutput {
        lowpass_hz: 60_000.0,
        gain: -0.322_257_3,
        dc: 0.107_312_08,
    },
    triangle: GrayBoxOutput {
        lowpass_hz: 3_984.99,
        gain: 1.149_772_2,
        dc: 0.230_349_67,
    },
    pulse: GrayBoxOutput {
        lowpass_hz: 800.0,
        gain: 0.396_946_7,
        dc: 0.201_093_47,
    },
};
