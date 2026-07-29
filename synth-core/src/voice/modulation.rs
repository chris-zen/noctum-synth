use crate::math::WideF32;
use crate::patch::{
    AuxEnvelopeParams, DedicatedModSlot, DedicatedModSource, LFO_COUNT, LayerPatch, LfoParams,
    MOD_MATRIX_FREE_SLOT_COUNT, ModDestination, ModMatrix, ModMatrixSlot, ModRoute,
};
use crate::{ModSource, ModulationParam};

use super::VoiceBlock;

const MAX_COMPILED_MOD_ROUTES: usize =
    LFO_COUNT + 1 + MOD_MATRIX_FREE_SLOT_COUNT + DedicatedModSource::COUNT;

pub struct PatchModulation {
    matrix: ModMatrix,
    lfo_patch: [LfoParams; LFO_COUNT],
    aux: AuxEnvelopeParams,
    plan: ModulationExecutionPlan,
    defer_rebuild: bool,
}

impl Default for PatchModulation {
    fn default() -> Self {
        Self {
            matrix: ModMatrix::default(),
            lfo_patch: [LfoParams::default(); LFO_COUNT],
            aux: AuxEnvelopeParams::default(),
            plan: ModulationExecutionPlan::default(),
            defer_rebuild: false,
        }
    }
}

impl PatchModulation {
    pub(crate) fn plan(&self) -> &ModulationExecutionPlan {
        &self.plan
    }

    pub fn aux_amount(&self) -> f32 {
        self.aux.amount
    }

    #[cfg(test)]
    pub(crate) fn aux_destination(&self) -> ModDestination {
        self.aux.destination
    }

    #[cfg(test)]
    pub(crate) fn matrix_free_slots(&self) -> &[ModMatrixSlot; MOD_MATRIX_FREE_SLOT_COUNT] {
        &self.matrix.free_slots
    }

    #[cfg(test)]
    pub(crate) fn matrix_dedicated_slots(&self) -> &[DedicatedModSlot; DedicatedModSource::COUNT] {
        &self.matrix.dedicated
    }

    pub fn begin_patch_update(&mut self) {
        self.defer_rebuild = true;
    }

    pub fn finish_patch_update(&mut self) {
        self.defer_rebuild = false;
        self.rebuild();
    }

    pub fn apply_from_patch(&mut self, patch: &LayerPatch) {
        self.begin_patch_update();
        self.matrix = patch.mod_matrix.clone();
        self.lfo_patch = patch.lfos;
        self.aux = patch.aux_envelope;
        self.finish_patch_update();
    }

    pub fn set_mod_route(
        &mut self,
        route: ModRoute,
        enabled: bool,
        source: ModSource,
        destination: ModDestination,
        amount: f32,
    ) {
        match route {
            ModRoute::Free(index) => {
                if let Some(slot) = self.matrix.free_slots.get_mut(index) {
                    *slot = ModMatrixSlot {
                        enabled,
                        source,
                        destination,
                        amount: amount.clamp(-1.0, 1.0),
                    };
                }
            }
            ModRoute::Dedicated(source) => {
                let index = source.index();
                if let Some(slot) = self.matrix.dedicated.get_mut(index) {
                    *slot = DedicatedModSlot {
                        enabled,
                        destination,
                        amount: amount.clamp(-1.0, 1.0),
                    };
                }
            }
        }
        self.refresh();
    }

    pub fn set_mod_route_param(&mut self, route: ModRoute, parameter: ModulationParam) {
        match route {
            ModRoute::Free(index) => {
                if let Some(slot) = self.matrix.free_slots.get_mut(index) {
                    match parameter {
                        ModulationParam::Source(source) => slot.source = source,
                        ModulationParam::Destination(destination) => {
                            slot.destination = destination;
                        }
                        ModulationParam::Amount(amount) => {
                            slot.amount = amount.clamp(-1.0, 1.0);
                        }
                    }
                    if !matches!(parameter, ModulationParam::Amount(_)) {
                        slot.enabled = slot.source != ModSource::Off
                            && slot.destination != ModDestination::Off;
                    }
                }
            }
            ModRoute::Dedicated(source) => {
                let index = source.index();
                if let Some(slot) = self.matrix.dedicated.get_mut(index) {
                    match parameter {
                        ModulationParam::Destination(destination) => {
                            slot.destination = destination;
                            slot.enabled = destination != ModDestination::Off;
                        }
                        ModulationParam::Amount(amount) => {
                            slot.amount = amount.clamp(-1.0, 1.0);
                        }
                        ModulationParam::Source(_) => {}
                    }
                }
            }
        }
        self.refresh();
    }

    pub fn set_lfo_depth(&mut self, index: usize, depth: f32) {
        if let Some(params) = self.lfo_patch.get_mut(index) {
            params.depth = depth.clamp(0.0, 1.0);
            self.refresh();
        }
    }

    pub fn set_lfo_destination(&mut self, index: usize, destination: ModDestination) {
        if let Some(params) = self.lfo_patch.get_mut(index) {
            params.destination = destination;
            self.refresh();
        }
    }

    pub fn set_aux_destination(&mut self, destination: ModDestination) {
        self.aux.destination = destination;
        self.refresh();
    }

    pub fn set_aux_amount(&mut self, amount: f32) {
        self.aux.amount = amount.clamp(-1.0, 1.0);
        self.refresh();
    }

    fn refresh(&mut self) {
        if !self.defer_rebuild {
            self.rebuild();
        }
    }

    fn rebuild(&mut self) {
        self.plan = ModulationExecutionPlan::compile(
            lfo_depths(&self.lfo_patch),
            lfo_destinations(&self.lfo_patch),
            self.aux.destination,
            self.aux.amount,
            self.matrix.free_slots,
            self.matrix.dedicated,
        );
    }

    #[cfg(test)]
    pub(crate) fn test_matrix_slot(&self, index: usize) -> ModMatrixSlot {
        self.matrix.free_slots[index]
    }

    #[cfg(test)]
    pub(crate) fn test_plan_mut(&mut self) -> &mut ModulationExecutionPlan {
        &mut self.plan
    }
}

fn lfo_depths(params: &[LfoParams; LFO_COUNT]) -> [f32; LFO_COUNT] {
    core::array::from_fn(|index| params[index].depth)
}

fn lfo_destinations(params: &[LfoParams; LFO_COUNT]) -> [ModDestination; LFO_COUNT] {
    core::array::from_fn(|index| params[index].destination)
}

#[derive(Clone, Copy)]
pub(crate) struct ModSignalContext {
    pub performance: super::PerformanceModulation,
    pub velocities: WideF32,
    pub filter_env: WideF32,
    pub amp_env: WideF32,
    pub aux_env: WideF32,
    pub aux_signal: WideF32,
}

#[derive(Clone, Copy)]
pub(crate) struct ModulationExecutionPlan {
    pub control_routes: [CompiledModRoute; MAX_COMPILED_MOD_ROUTES],
    pub audio_routes: [CompiledModRoute; MAX_COMPILED_MOD_ROUTES],
    pub control_count: u8,
    pub audio_count: u8,
    pub active_lfo_mask: u8,
    pub rate_target_mask: u8,
    pub depth_target_mask: u8,
    pub total_route_count: u8,
    pub single_pwm_route: Option<SinglePwmRoute>,
    pub single_filter_cutoff_route: Option<SingleFilterCutoffRoute>,
    pub any_modulation: bool,
}

impl Default for ModulationExecutionPlan {
    fn default() -> Self {
        Self {
            control_routes: [CompiledModRoute::EMPTY; MAX_COMPILED_MOD_ROUTES],
            audio_routes: [CompiledModRoute::EMPTY; MAX_COMPILED_MOD_ROUTES],
            control_count: 0,
            audio_count: 0,
            active_lfo_mask: 0,
            rate_target_mask: 0,
            depth_target_mask: 0,
            total_route_count: 0,
            single_pwm_route: None,
            single_filter_cutoff_route: None,
            any_modulation: false,
        }
    }
}

impl ModulationExecutionPlan {
    fn compile(
        lfo_base_depths: [f32; LFO_COUNT],
        lfo_destinations: [ModDestination; LFO_COUNT],
        aux_destination: ModDestination,
        aux_amount: f32,
        matrix_slots: [ModMatrixSlot; MOD_MATRIX_FREE_SLOT_COUNT],
        dedicated_slots: [DedicatedModSlot; DedicatedModSource::COUNT],
    ) -> Self {
        let mut plan = Self::default();

        for (index, depth) in lfo_base_depths.iter().enumerate() {
            if *depth != 0.0 {
                plan.active_lfo_mask |= 1 << index;
            }
        }

        for slot in matrix_slots {
            if slot.enabled && slot.amount != 0.0 {
                plan.active_lfo_mask |= Self::lfo_depth_target_mask(slot.destination);
            }
        }
        for slot in dedicated_slots {
            if slot.enabled && slot.amount != 0.0 {
                plan.active_lfo_mask |= Self::lfo_depth_target_mask(slot.destination);
            }
        }

        let lfo_sources: [ModSource; LFO_COUNT] = [
            ModSource::Lfo1,
            ModSource::Lfo2,
            ModSource::Lfo3,
            ModSource::Lfo4,
        ];
        for (index, destination) in lfo_destinations.iter().copied().enumerate() {
            if destination != ModDestination::Off && plan.active_lfo_mask & (1 << index) != 0 {
                plan.add_route(CompiledModRoute {
                    source: CompiledModSource::Standard(lfo_sources[index]),
                    destination,
                    amount: 1.0,
                });
            }
        }
        if aux_destination != ModDestination::Off && aux_amount != 0.0 {
            plan.add_route(CompiledModRoute {
                source: CompiledModSource::AuxSignal,
                destination: aux_destination,
                amount: 1.0,
            });
        }
        for slot in matrix_slots {
            if slot.enabled && slot.amount != 0.0 {
                plan.add_route(CompiledModRoute {
                    source: CompiledModSource::Standard(slot.source),
                    destination: slot.destination,
                    amount: slot.amount,
                });
            }
        }
        for (index, slot) in dedicated_slots.iter().copied().enumerate() {
            if slot.enabled && slot.amount != 0.0 {
                plan.add_route(CompiledModRoute {
                    source: CompiledModSource::Standard(DedicatedModSource::ALL[index].source()),
                    destination: slot.destination,
                    amount: slot.amount,
                });
            }
        }
        plan.single_pwm_route = plan.detect_single_pwm_route();
        plan.single_filter_cutoff_route = plan.detect_single_filter_cutoff_route();
        plan.any_modulation = plan.total_route_count > 0 || plan.active_lfo_mask != 0;
        plan
    }

    pub fn control_routes(&self) -> &[CompiledModRoute] {
        &self.control_routes[..self.control_count as usize]
    }

    pub fn audio_routes(&self) -> &[CompiledModRoute] {
        &self.audio_routes[..self.audio_count as usize]
    }

    fn add_route(&mut self, route: CompiledModRoute) {
        self.total_route_count += 1;
        if let CompiledModSource::Standard(source) = route.source {
            self.active_lfo_mask |= Self::lfo_source_mask(source);
        }
        self.rate_target_mask |= Self::lfo_rate_target_mask(route.destination);
        self.depth_target_mask |= Self::lfo_depth_target_mask(route.destination);

        if Self::is_lfo_control_destination(route.destination) {
            let index = self.control_count as usize;
            self.control_routes[index] = route;
            self.control_count += 1;
        } else if Self::is_audio_destination(route.destination) {
            let index = self.audio_count as usize;
            self.audio_routes[index] = route;
            self.audio_count += 1;
        }
    }

    fn detect_single_pwm_route(&self) -> Option<SinglePwmRoute> {
        if self.total_route_count != 1 || self.control_count != 0 || self.audio_count != 1 {
            return None;
        }
        let route = self.audio_routes[0];
        if route.destination != ModDestination::Osc1ShapeMod {
            return None;
        }
        let CompiledModSource::Standard(source) = route.source else {
            return None;
        };
        let lfo_index = match source {
            ModSource::Lfo1 => 0,
            ModSource::Lfo2 => 1,
            ModSource::Lfo3 => 2,
            ModSource::Lfo4 => 3,
            _ => return None,
        };
        Some(SinglePwmRoute {
            lfo_index,
            amount: route.amount,
        })
    }

    fn detect_single_filter_cutoff_route(&self) -> Option<SingleFilterCutoffRoute> {
        if self.total_route_count != 1 || self.control_count != 0 || self.audio_count != 1 {
            return None;
        }
        let route = self.audio_routes[0];
        if route.destination != ModDestination::FilterCutoff {
            return None;
        }
        let CompiledModSource::Standard(source) = route.source else {
            return None;
        };
        let lfo_index = match source {
            ModSource::Lfo1 => 0,
            ModSource::Lfo2 => 1,
            ModSource::Lfo3 => 2,
            ModSource::Lfo4 => 3,
            _ => return None,
        };
        Some(SingleFilterCutoffRoute {
            lfo_index,
            amount: route.amount,
        })
    }

    fn lfo_source_mask(source: ModSource) -> u8 {
        match source {
            ModSource::Lfo1 => 1 << 0,
            ModSource::Lfo2 => 1 << 1,
            ModSource::Lfo3 => 1 << 2,
            ModSource::Lfo4 => 1 << 3,
            _ => 0,
        }
    }

    fn lfo_rate_target_mask(destination: ModDestination) -> u8 {
        match destination {
            ModDestination::Lfo1Frequency => 1 << 0,
            ModDestination::Lfo2Frequency => 1 << 1,
            ModDestination::Lfo3Frequency => 1 << 2,
            ModDestination::Lfo4Frequency => 1 << 3,
            ModDestination::LfoAllFrequency => 0b1111,
            _ => 0,
        }
    }

    fn lfo_depth_target_mask(destination: ModDestination) -> u8 {
        match destination {
            ModDestination::Lfo1Amount => 1 << 0,
            ModDestination::Lfo2Amount => 1 << 1,
            ModDestination::Lfo3Amount => 1 << 2,
            ModDestination::Lfo4Amount => 1 << 3,
            ModDestination::LfoAllAmount => 0b1111,
            _ => 0,
        }
    }

    fn is_lfo_control_destination(destination: ModDestination) -> bool {
        Self::lfo_rate_target_mask(destination) != 0
            || Self::lfo_depth_target_mask(destination) != 0
    }

    fn is_audio_destination(destination: ModDestination) -> bool {
        matches!(
            destination,
            ModDestination::Osc1Frequency
                | ModDestination::Osc2Frequency
                | ModDestination::OscAllFrequency
                | ModDestination::OscMix
                | ModDestination::NoiseLevel
                | ModDestination::SubOscLevel
                | ModDestination::Osc1ShapeMod
                | ModDestination::Osc2ShapeMod
                | ModDestination::OscAllShapeMod
                | ModDestination::FilterCutoff
                | ModDestination::FilterResonance
                | ModDestination::FilterAudioMod
                | ModDestination::Vca
                | ModDestination::Pan
                | ModDestination::FxMix
                | ModDestination::FxParam1
                | ModDestination::FxParam2
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CompiledModRoute {
    source: CompiledModSource,
    destination: ModDestination,
    amount: f32,
}

impl CompiledModRoute {
    const EMPTY: Self = Self {
        source: CompiledModSource::Standard(ModSource::Off),
        destination: ModDestination::Off,
        amount: 0.0,
    };

    pub fn signal(self, block: &VoiceBlock, context: ModSignalContext) -> WideF32 {
        let signal = match self.source {
            CompiledModSource::Standard(source) => block.mod_source_signal(source, context),
            CompiledModSource::AuxSignal => context.aux_signal,
        };
        signal * WideF32::splat(self.amount)
    }

    pub fn destination(self) -> ModDestination {
        self.destination
    }
}

#[derive(Clone, Copy)]
enum CompiledModSource {
    Standard(ModSource),
    AuxSignal,
}

#[derive(Clone, Copy)]
pub(crate) struct SinglePwmRoute {
    pub lfo_index: u8,
    pub amount: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct SingleFilterCutoffRoute {
    pub lfo_index: u8,
    pub amount: f32,
}
