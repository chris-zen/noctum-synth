use crate::f32x4;

use crate::ParamId;
use crate::patch::PanModMode;

const CENTERED_PAN_SCALE: f32 = 0.7071067811865476;
const CENTERED_PAN_SIN: f32x4 = f32x4::splat(CENTERED_PAN_SCALE);
const CENTERED_PAN_COS: f32x4 = CENTERED_PAN_SIN;

pub struct Pan {
    spread: f32,
    mode: PanModMode,
}

impl Pan {
    pub fn new() -> Self {
        Self {
            spread: 0.0,
            mode: PanModMode::Alternate,
        }
    }

    pub fn apply_params(&mut self, spread: f32, mode: PanModMode) {
        self.set_spread(spread);
        self.set_mod_mode(mode);
    }

    pub fn set_param(&mut self, id: ParamId, value: f32) -> bool {
        match id {
            ParamId::PanSpread => self.set_spread(value),
            ParamId::PanModMode => self.set_mod_mode(PanModMode::from_param(value)),
            _ => return false,
        }
        true
    }

    pub fn set_spread(&mut self, spread: f32) {
        self.spread = spread.clamp(0.0, 1.0);
    }

    pub fn set_mod_mode(&mut self, mode: PanModMode) {
        self.mode = mode;
    }

    #[cfg(test)]
    pub(crate) fn spread(&self) -> f32 {
        self.spread
    }

    #[cfg(test)]
    pub(crate) fn mod_mode(&self) -> PanModMode {
        self.mode
    }

    pub fn pan_lanes(&self, lanes: f32x4, pan_mod: f32x4, voice_position: f32x4) -> (f32, f32) {
        if self.spread == 0.0 && (self.mode == PanModMode::Alternate || pan_mod == f32x4::ZERO) {
            return (
                (lanes * CENTERED_PAN_COS).reduce_add(),
                (lanes * CENTERED_PAN_SIN).reduce_add(),
            );
        }

        let position = match self.mode {
            PanModMode::Alternate => {
                let spread =
                    (f32x4::splat(self.spread) + pan_mod).clamp(f32x4::ZERO, f32x4::splat(1.0));
                voice_position * spread
            }
            PanModMode::Fixed => (voice_position * f32x4::splat(self.spread) + pan_mod)
                .clamp(f32x4::splat(-1.0), f32x4::splat(1.0)),
        };
        let angle = (position + f32x4::splat(1.0)) * f32x4::splat(core::f32::consts::FRAC_PI_4);
        let (sin, cos) = angle.sin_cos();

        ((lanes * cos).reduce_add(), (lanes * sin).reduce_add())
    }
}

impl Default for Pan {
    fn default() -> Self {
        Self::new()
    }
}
