//! Complete two-layer synthesizer patch model.
//!
//! Format decisions follow this source hierarchy, in descending authority:
//!
//! 1. Sequential's official [Prophet Rev2 User's Guide], [Prophet '08 manual],
//!    and the official SysEx fixtures committed with this repository.
//! 2. Behavior verified across the complete official factory-program corpus.
//! 3. Independent editor implementations such as [Edisyn].
//! 4. Forum research, used only to corroborate the sources above.
//!
//! [Prophet Rev2 User's Guide]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
//! [Prophet '08 manual]: https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf
//! [Edisyn]: https://github.com/eclab/edisyn

use crate::LayerPatch;

#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize};

/// Lowest split-point note accepted by the official [Prophet Rev2 range].
///
/// [Prophet Rev2 range]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
pub const MIN_SPLIT_POINT: u8 = 0;

/// Highest split-point note accepted by the official [Prophet Rev2 range].
///
/// [Prophet Rev2 range]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
pub const MAX_SPLIT_POINT: u8 = 120;

/// Default split point used by a new program, within the official [Prophet Rev2 range].
///
/// [Prophet Rev2 range]: https://www.sequential.com/wp-content/uploads/2019/05/Prophet-Rev2-Users-Guide-1.2.2.pdf
pub const DEFAULT_SPLIT_POINT: u8 = 60;

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerId {
    #[default]
    A,
    B,
}

/// Resolves a patch edit either against the engine's current edit layer or an
/// explicitly addressed layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerTarget {
    Edit,
    Explicit(LayerId),
}

/// Prophet two-layer keyboard mode. Raw values follow the official [Prophet '08
/// manual], the official Rev2 factory corpus, and [Edisyn's live value order].
///
/// [Prophet '08 manual]: https://www.sequential.com/downloads/prophet_keyboard/doc/Prophet_08_Manual_v1.3.pdf
/// [Edisyn's live value order]: https://github.com/eclab/edisyn
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerMode {
    #[default]
    Normal,
    Stack,
    Split,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone)]
pub struct Patch {
    pub layer_a: LayerPatch,
    pub layer_b: LayerPatch,
    pub mode: LayerMode,
    #[cfg_attr(feature = "serde", serde(deserialize_with = "deserialize_split_point"))]
    pub split_point: u8,
}

impl Patch {
    pub fn new(layer_a: LayerPatch, layer_b: LayerPatch, mode: LayerMode, split_point: u8) -> Self {
        Self {
            layer_a,
            layer_b,
            mode,
            split_point: clamp_split_point(split_point),
        }
    }

    pub fn layer(&self, layer: LayerId) -> &LayerPatch {
        match layer {
            LayerId::A => &self.layer_a,
            LayerId::B => &self.layer_b,
        }
    }

    pub fn layer_mut(&mut self, layer: LayerId) -> &mut LayerPatch {
        match layer {
            LayerId::A => &mut self.layer_a,
            LayerId::B => &mut self.layer_b,
        }
    }

    pub fn set_split_point(&mut self, split_point: u8) {
        self.split_point = clamp_split_point(split_point);
    }

    /// Clamp externally populated fields to the supported patch contract.
    pub fn validate(&mut self) {
        self.set_split_point(self.split_point);
        self.layer_a.sequence.validate();
        self.layer_b.sequence.validate();
    }
}

impl Default for Patch {
    fn default() -> Self {
        Self::new(
            LayerPatch::default(),
            LayerPatch::default(),
            LayerMode::Normal,
            DEFAULT_SPLIT_POINT,
        )
    }
}

const fn clamp_split_point(split_point: u8) -> u8 {
    if split_point > MAX_SPLIT_POINT {
        MAX_SPLIT_POINT
    } else {
        split_point
    }
}

#[cfg(feature = "serde")]
fn deserialize_split_point<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    u8::deserialize(deserializer).map(clamp_split_point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_patch_has_two_default_layers() {
        let patch = Patch::default();

        assert_eq!(patch.mode, LayerMode::Normal);
        assert_eq!(patch.split_point, DEFAULT_SPLIT_POINT);
        assert_eq!(patch.layer(LayerId::A).name, LayerPatch::default().name);
        assert_eq!(patch.layer(LayerId::B).name, LayerPatch::default().name);
    }

    #[test]
    fn split_point_is_clamped_at_model_boundaries() {
        let mut patch = Patch::new(
            LayerPatch::default(),
            LayerPatch::default(),
            LayerMode::Split,
            u8::MAX,
        );
        assert_eq!(patch.split_point, MAX_SPLIT_POINT);

        patch.split_point = u8::MAX;
        patch.validate();
        assert_eq!(patch.split_point, MAX_SPLIT_POINT);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn patch_serde_round_trip_preserves_both_layers() {
        let mut patch = Patch::default();
        patch.layer_a.name.push_str("Layer A").unwrap();
        patch.layer_b.name.push_str("Layer B").unwrap();
        patch.layer_a.filter.cutoff = 321.0;
        patch.layer_b.filter.cutoff = 4_321.0;
        patch.mode = LayerMode::Stack;
        patch.set_split_point(72);

        let json = serde_json::to_string(&patch).unwrap();
        let decoded: Patch = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.layer_a.name, patch.layer_a.name);
        assert_eq!(decoded.layer_b.name, patch.layer_b.name);
        assert_eq!(decoded.layer_a.filter.cutoff, patch.layer_a.filter.cutoff);
        assert_eq!(decoded.layer_b.filter.cutoff, patch.layer_b.filter.cutoff);
        assert_eq!(decoded.mode, patch.mode);
        assert_eq!(decoded.split_point, patch.split_point);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_clamps_an_out_of_range_split_point() {
        let json = serde_json::to_string(&Patch::default())
            .unwrap()
            .replace("\"split_point\":60", "\"split_point\":255");

        let decoded: Patch = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.split_point, MAX_SPLIT_POINT);
    }
}
