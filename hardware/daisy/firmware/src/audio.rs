//! Real-time audio rendering on the interrupt executor.

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::peripheral::DWT;
use embassy_daisy::audio::{Audio, AudioResources, BLOCK_LENGTH, Block, Error as AudioError};
use embassy_executor::InterruptExecutor;
use embassy_futures::yield_now;
use embassy_stm32::interrupt;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use heapless::Deque;

use synth_core::{
    ControlMessage, LayerId, LayerMode, Patch, SequencerFeedback, SynthEngineWithMemory,
};
#[cfg(feature = "audio-profiling")]
use synth_core::{RenderProfiler, RenderStage};

#[cfg(feature = "audio-profiling")]
use crate::profiling;
use crate::{diagnostics, indicator, pending_releases::PendingReleases, usb_audio::UsbAudioBuffer};

// Parameter traffic is isolated from performance events and coalesced by key.
pub use crate::control_queue::{CONTROL_QUEUE_CAPACITY, ControlQueue};

pub const PERFORMANCE_QUEUE_CAPACITY: usize = 32;
pub const PATCH_QUEUE_CAPACITY: usize = 2;
pub const BLOCK_CYCLE_BUDGET: u32 =
    embassy_daisy::clocks::SYSCLK_HZ / embassy_daisy::audio::SAMPLE_RATE_HZ * BLOCK_LENGTH as u32;

static EXECUTOR: InterruptExecutor = InterruptExecutor::new();
static OVERRUNS_COUNT: AtomicU32 = AtomicU32::new(0);
static UNDERRUNS_COUNT: AtomicU32 = AtomicU32::new(0);

pub type HardwareSynth = SynthEngineWithMemory<&'static mut [f32], 1>;
pub type PatchQueue = Channel<CriticalSectionRawMutex, Patch, PATCH_QUEUE_CAPACITY>;

pub struct PerformanceQueue {
    queue:
        Mutex<CriticalSectionRawMutex, RefCell<Deque<ControlMessage, PERFORMANCE_QUEUE_CAPACITY>>>,
}

impl PerformanceQueue {
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(RefCell::new(Deque::new())),
        }
    }

    pub fn try_send(
        &self,
        command: ControlMessage,
    ) -> Result<(), embassy_sync::channel::TrySendError<ControlMessage>> {
        self.queue.lock(|queue| {
            let mut queue = queue.borrow_mut();
            if matches!(&command, ControlMessage::AllNotesOff)
                && queue
                    .iter()
                    .any(|queued| matches!(queued, ControlMessage::AllNotesOff))
            {
                return Ok(());
            }
            queue
                .push_back(command)
                .map_err(embassy_sync::channel::TrySendError::Full)
        })
    }

    pub fn try_receive(&self) -> Result<ControlMessage, embassy_sync::channel::TryReceiveError> {
        self.queue.lock(|queue| {
            queue
                .borrow_mut()
                .pop_front()
                .ok_or(embassy_sync::channel::TryReceiveError::Empty)
        })
    }
}

pub struct AdaptiveControlBudget {
    adaptive: Option<u32>,
}

impl AdaptiveControlBudget {
    pub const fn new() -> Self {
        Self { adaptive: None }
    }

    pub fn reset(&mut self) {
        self.adaptive = None;
    }

    pub fn effective_budget(&self) -> u32 {
        self.adaptive.map_or(0, |budget| budget * 9 / 10)
    }

    pub fn observe_rendered_block(
        &mut self,
        work_cycles: u32,
        adaptive_spent: u32,
        block_budget: u32,
    ) {
        let observed_headroom =
            block_budget.saturating_sub(work_cycles.saturating_sub(adaptive_spent));
        self.adaptive = Some(match self.adaptive {
            Some(previous) => previous.min(observed_headroom),
            None => observed_headroom,
        });
    }
}

pub fn spawn(
    resources: AudioResources,
    engine: &'static mut HardwareSynth,
    controls: &'static ControlQueue,
    performance: &'static PerformanceQueue,
    pending_releases: &'static PendingReleases,
    patches: &'static PatchQueue,
    indicator: indicator::Sender<'static>,
    usb_audio: &'static UsbAudioBuffer,
) -> Result<(), embassy_executor::SpawnError> {
    EXECUTOR.start(interrupt::I2C4_EV).spawn(run_task(
        resources,
        engine,
        controls,
        performance,
        pending_releases,
        patches,
        indicator,
        usb_audio,
    )?);
    Ok(())
}

#[embassy_executor::task]
pub async fn run_task(
    resources: AudioResources,
    engine: &'static mut HardwareSynth,
    controls: &'static ControlQueue,
    performance: &'static PerformanceQueue,
    pending_releases: &'static PendingReleases,
    patches: &'static PatchQueue,
    indicator: indicator::Sender<'static>,
    usb_audio: &'static UsbAudioBuffer,
) -> ! {
    let mut audio = match Audio::output(resources) {
        Ok(audio) => audio,
        Err(error) => audio_unavailable(error.category()).await,
    };
    yield_now().await;

    let mut output: Block = [(0.0, 0.0); BLOCK_LENGTH];
    let mut interleaved = [0.0f32; BLOCK_LENGTH * 2];
    #[cfg(feature = "audio-profiling")]
    let mut profiler = profiling::AudioProfiler::new(BLOCK_CYCLE_BUDGET);
    let mut perf_monitor = diagnostics::PerfMonitor::new(BLOCK_CYCLE_BUDGET);
    let mut adaptive_control_budget = AdaptiveControlBudget::new();

    // Render before starting the receive clock. The SAI input ring cannot
    // overrun while the first, comparatively expensive DSP block is prepared.
    engine.process_interleaved(&mut interleaved, 2);
    copy_output(&interleaved, &mut output);
    yield_now().await;

    if let Err(error) = audio.start(&output).await {
        audio_unavailable(error.category()).await;
    }

    diagnostics::emit(diagnostics::Event::AudioStarted);
    let mut unexpected_transfer_error_reported = false;

    loop {
        match audio.transfer(&output).await {
            Ok(()) => unexpected_transfer_error_reported = false,
            Err(AudioError::SaiReceive(_)) => {
                increment_overruns();
                diagnostics::emit_xrun(overruns_count(), underruns_count());
                indicator.notify_xrun();
                fade_stereo_to_silence(&mut output);
                continue;
            }
            Err(AudioError::SaiTransmit(_)) => {
                // The Embassy writable DMA ring resets its cursor on this
                // underrun, just as the readable ring does after an overrun.
                increment_underruns();
                diagnostics::emit_xrun(overruns_count(), underruns_count());
                indicator.notify_xrun();
                fade_stereo_to_silence(&mut output);
                continue;
            }
            Err(error) => {
                if !unexpected_transfer_error_reported {
                    diagnostics::emit(diagnostics::Event::AudioUnavailable {
                        reason: error.category(),
                    });
                    unexpected_transfer_error_reported = true;
                }
                increment_underruns();
                diagnostics::emit_xrun(overruns_count(), underruns_count());
                indicator.notify_xrun();
                fade_stereo_to_silence(&mut output);
                embassy_time::Timer::after_millis(1).await;
                continue;
            }
        }

        #[cfg(feature = "audio-profiling")]
        if profiler.report_due() {
            queue_profile(profiler.take_snapshot());
        }

        let work_started = DWT::cycle_count();

        #[cfg(feature = "audio-profiling")]
        {
            profiler.begin_block();
            profiler.begin(RenderStage::ControlDrain);
        }
        if let Ok(patch) = patches.try_receive() {
            engine.apply_patch(&patch);
            emit_playback_status(engine);
            adaptive_control_budget.reset();
        }
        apply_pending_releases(engine, pending_releases);

        if let Ok(command) = performance.try_receive() {
            let topology = is_topology_control(&command);
            engine.handle_control(command);
            if topology {
                emit_playback_status(engine);
            }
        } else if let Ok(command) = controls.try_receive() {
            emit_control_diagnostics(&command);
            let topology = is_topology_control(&command);
            engine.handle_control(command);
            if topology {
                emit_playback_status(engine);
            }
        }
        let extras_started = DWT::cycle_count();
        let effective_budget = adaptive_control_budget.effective_budget();
        while DWT::cycle_count().wrapping_sub(extras_started) < effective_budget {
            if let Ok(command) = performance.try_receive() {
                let topology = is_topology_control(&command);
                engine.handle_control(command);
                if topology {
                    emit_playback_status(engine);
                }
                continue;
            }
            if let Ok(command) = controls.try_receive() {
                emit_control_diagnostics(&command);
                let topology = is_topology_control(&command);
                engine.handle_control(command);
                if topology {
                    emit_playback_status(engine);
                }
                continue;
            }
            break;
        }
        let adaptive_spent = DWT::cycle_count().wrapping_sub(extras_started);
        #[cfg(feature = "audio-profiling")]
        profiler.end(RenderStage::ControlDrain);

        #[cfg(feature = "audio-profiling")]
        engine.process_interleaved_profiled(&mut interleaved, 2, &mut profiler);
        #[cfg(not(feature = "audio-profiling"))]
        engine.process_interleaved(&mut interleaved, 2);

        while let Some(feedback) = engine.pop_sequencer_feedback() {
            match feedback {
                SequencerFeedback::RecordStatus {
                    layer,
                    recording,
                    cursor,
                } => diagnostics::emit(diagnostics::Event::SequencerRecordStatus {
                    layer: u8::from(layer == LayerId::B),
                    recording,
                    cursor,
                }),
                SequencerFeedback::StepChanged { layer, step, .. } => {
                    diagnostics::emit(diagnostics::Event::SequencerStepChanged {
                        layer: u8::from(layer == LayerId::B),
                        step,
                    });
                }
                SequencerFeedback::RecordOverflow { layer, cursor } => {
                    diagnostics::emit(diagnostics::Event::SequencerRecordOverflow {
                        layer: u8::from(layer == LayerId::B),
                        cursor,
                    });
                }
            }
        }

        #[cfg(feature = "audio-profiling")]
        profiler.begin(RenderStage::OutputCopy);
        copy_output(&interleaved, &mut output);
        usb_audio.push_block(&output);
        #[cfg(feature = "audio-profiling")]
        {
            profiler.end(RenderStage::OutputCopy);
            profiler.end_block();
        }

        let work_cycles = DWT::cycle_count().wrapping_sub(work_started);
        adaptive_control_budget.observe_rendered_block(
            work_cycles,
            adaptive_spent,
            BLOCK_CYCLE_BUDGET,
        );
        if let Some(event) = perf_monitor.observe(work_cycles) {
            diagnostics::emit(event);
        }
    }
}

fn is_topology_control(command: &ControlMessage) -> bool {
    matches!(
        command,
        ControlMessage::SetLayerMode(_)
            | ControlMessage::SetSplitPoint(_)
            | ControlMessage::SetEditLayer(_)
    )
}

fn emit_playback_status(engine: &HardwareSynth) {
    let status = engine.playback_status();
    diagnostics::emit(diagnostics::Event::LayerPlayback {
        mode: match status.mode {
            LayerMode::Normal => 0,
            LayerMode::Stack => 1,
            LayerMode::Split => 2,
        },
        edit_layer: match status.edit_layer {
            LayerId::A => 0,
            LayerId::B => 1,
        },
        rendered_mask: status.rendered_mask,
        degraded: status.degraded,
    });
}

#[inline]
pub(crate) fn overruns_count() -> u32 {
    OVERRUNS_COUNT.load(Ordering::Relaxed)
}

#[inline]
pub(crate) fn underruns_count() -> u32 {
    UNDERRUNS_COUNT.load(Ordering::Relaxed)
}

async fn audio_unavailable(reason: &'static str) -> ! {
    diagnostics::emit(diagnostics::Event::AudioUnavailable { reason });
    loop {
        embassy_time::Timer::after_secs(3_600).await;
    }
}

fn apply_pending_releases(engine: &mut HardwareSynth, pending: &PendingReleases) {
    if pending.take_all_notes_off() {
        engine.handle_control(ControlMessage::AllNotesOff);
    }
    for (word_index, mut word) in pending.take().into_iter().enumerate() {
        while word != 0 {
            let note = word_index as u8 * 32 + word.trailing_zeros() as u8;
            word &= word - 1;
            engine.handle_control(ControlMessage::NoteOff { note });
        }
    }
}

#[cfg(feature = "diagnostics")]
fn emit_control_diagnostics(command: &ControlMessage) {
    match command {
        ControlMessage::SetParam { param, value, .. } => {
            diagnostics::emit(diagnostics::Event::Param {
                param: *param,
                value: *value,
            })
        }
        ControlMessage::SetModulationParam {
            route, parameter, ..
        } => diagnostics::emit(diagnostics::Event::ModulationParam {
            route: *route,
            parameter: *parameter,
        }),
        _ => {}
    }
}

#[cfg(not(feature = "diagnostics"))]
#[inline(always)]
fn emit_control_diagnostics(_command: &ControlMessage) {}

fn fade_stereo_to_silence(output: &mut [(f32, f32)]) {
    let Some(&(left, right)) = output.last() else {
        return;
    };
    let length = output.len() as f32;
    for (index, frame) in output.iter_mut().enumerate() {
        let gain = 1.0 - (index + 1) as f32 / length;
        *frame = (left * gain, right * gain);
    }
}

#[cfg(feature = "audio-profiling")]
fn queue_profile(snapshot: profiling::Snapshot) {
    diagnostics::emit(diagnostics::Event::ProfileBlock {
        blocks: snapshot.blocks,
        over_budget_blocks: snapshot.overruns,
        average_cycles: snapshot.block_average,
        maximum_cycles: snapshot.block_max,
    });
    diagnostics::emit(diagnostics::Event::PerfStages {
        worst_block: false,
        cycles: snapshot.stage_average,
    });
    diagnostics::emit(diagnostics::Event::PerfStages {
        worst_block: true,
        cycles: snapshot.stage_worst_block,
    });
}

fn copy_output(interleaved: &[f32; BLOCK_LENGTH * 2], output: &mut Block) {
    for (frame, samples) in output.iter_mut().zip(interleaved.chunks_exact(2)) {
        *frame = (samples[0], samples[1]);
    }
}

#[inline]
fn increment_overruns() {
    OVERRUNS_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn increment_underruns() {
    UNDERRUNS_COUNT.fetch_add(1, Ordering::Relaxed);
}

// I2C4 is not used by the Daisy BSP. Its event vector is reserved as a
// software-pended executor interrupt for deadline-critical audio work.
#[allow(non_snake_case)]
#[unsafe(no_mangle)]
unsafe extern "C" fn I2C4_EV() {
    unsafe { EXECUTOR.on_interrupt() }
}

#[cfg(test)]
mod tests {
    use super::fade_stereo_to_silence;

    #[test]
    fn recovery_block_fades_last_frame_to_zero() {
        let mut output = [(10.0, 10.0), (20.0, 20.0), (30.0, 30.0), (1.0, -1.0)];
        fade_stereo_to_silence(&mut output);

        assert_eq!(output[0], (0.75, -0.75));
        assert_eq!(output[1], (0.5, -0.5));
        assert_eq!(output[2], (0.25, -0.25));
        assert_eq!(output[3], (0.0, -0.0));
    }
}
