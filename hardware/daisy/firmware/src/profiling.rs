use cortex_m::peripheral::DWT;
use synth_core::{RenderProfiler, RenderStage};

pub const REPORT_INTERVAL_BLOCKS: u32 = 1_500;

pub struct Snapshot {
    pub blocks: u32,
    pub overruns: u32,
    pub block_average: u32,
    pub block_max: u32,
    pub stage_average: [u32; RenderStage::COUNT],
    /// Stage attribution captured from the same block as `block_max`.
    pub stage_worst_block: [u32; RenderStage::COUNT],
}

pub struct AudioProfiler {
    block_cycle_budget: u32,
    block_started: u32,
    blocks: u32,
    overruns: u32,
    block_cycles: u32,
    block_max: u32,
    stage_started: [u32; RenderStage::COUNT],
    current_stage_cycles: [u32; RenderStage::COUNT],
    stage_cycles: [u32; RenderStage::COUNT],
    stage_worst_block: [u32; RenderStage::COUNT],
}

impl AudioProfiler {
    pub const fn new(block_cycle_budget: u32) -> Self {
        Self {
            block_cycle_budget,
            block_started: 0,
            blocks: 0,
            overruns: 0,
            block_cycles: 0,
            block_max: 0,
            stage_started: [0; RenderStage::COUNT],
            current_stage_cycles: [0; RenderStage::COUNT],
            stage_cycles: [0; RenderStage::COUNT],
            stage_worst_block: [0; RenderStage::COUNT],
        }
    }

    pub fn report_due(&self) -> bool {
        self.blocks >= REPORT_INTERVAL_BLOCKS
    }

    pub fn take_snapshot(&mut self) -> Snapshot {
        let divisor = self.blocks.max(1);
        let snapshot = Snapshot {
            blocks: self.blocks,
            overruns: self.overruns,
            block_average: self.block_cycles / divisor,
            block_max: self.block_max,
            stage_average: self.stage_cycles.map(|cycles| cycles / divisor),
            stage_worst_block: self.stage_worst_block,
        };
        self.blocks = 0;
        self.overruns = 0;
        self.block_cycles = 0;
        self.block_max = 0;
        self.stage_cycles = [0; RenderStage::COUNT];
        self.stage_worst_block = [0; RenderStage::COUNT];
        snapshot
    }

    #[inline]
    pub fn begin_block(&mut self) {
        self.current_stage_cycles = [0; RenderStage::COUNT];
        self.block_started = DWT::cycle_count();
    }

    #[inline]
    pub fn end_block(&mut self) {
        let cycles = DWT::cycle_count().wrapping_sub(self.block_started);
        self.blocks += 1;
        self.block_cycles = self.block_cycles.wrapping_add(cycles);
        if cycles > self.block_max {
            self.block_max = cycles;
            self.stage_worst_block = self.current_stage_cycles;
        }
        if cycles > self.block_cycle_budget {
            self.overruns += 1;
        }
    }
}

impl RenderProfiler for AudioProfiler {
    #[inline]
    fn begin(&mut self, stage: RenderStage) {
        self.stage_started[stage.index()] = DWT::cycle_count();
    }

    #[inline]
    fn end(&mut self, stage: RenderStage) {
        let index = stage.index();
        let cycles = DWT::cycle_count().wrapping_sub(self.stage_started[index]);
        self.current_stage_cycles[index] = self.current_stage_cycles[index].wrapping_add(cycles);
        self.stage_cycles[index] = self.stage_cycles[index].wrapping_add(cycles);
    }
}
