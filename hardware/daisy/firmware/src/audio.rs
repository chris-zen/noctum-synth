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
use synth_core::{ControlMessage, Patch, SynthEngineWithMemory};

use crate::pending_releases::PendingReleases;
use crate::patch_transition::PatchTransition;
#[cfg(feature = "audio-profiling")]
use crate::profiling;
use crate::usb_audio::UsbAudioBuffer;
use crate::{diagnostics, indicator};
#[cfg(feature = "audio-profiling")]
use synth_core::{RenderProfiler, RenderStage};

// Parameter traffic is isolated from performance events and coalesced by key.
// Keep this small so the bounded in-place scan spends negligible time in its
// critical section even under an NRPN flood.
pub const CONTROL_QUEUE_CAPACITY: usize = 32;
pub const PERFORMANCE_QUEUE_CAPACITY: usize = 32;
pub const PATCH_QUEUE_CAPACITY: usize = 2;
const BLOCK_CYCLE_BUDGET: u32 =
    embassy_daisy::clocks::SYSCLK_HZ / embassy_daisy::audio::SAMPLE_RATE_HZ * BLOCK_LENGTH as u32;
// Reserve measured render headroom by limiting patch/control work to ten
// percent of one audio-block deadline. Unlike a message-count cap, cheap
// commands can drain a burst quickly while expensive commands remain bounded.
const CONTROL_CYCLE_BUDGET: u32 = BLOCK_CYCLE_BUDGET / 10;

static EXECUTOR: InterruptExecutor = InterruptExecutor::new();
static OVERRUNS_COUNT: AtomicU32 = AtomicU32::new(0);
static UNDERRUNS_COUNT: AtomicU32 = AtomicU32::new(0);

pub type HardwareSynth = SynthEngineWithMemory<1, &'static mut [f32]>;
pub struct ControlQueue {
    queue: Mutex<CriticalSectionRawMutex, RefCell<Deque<ControlMessage, CONTROL_QUEUE_CAPACITY>>>,
}

impl ControlQueue {
    pub const fn new() -> Self {
        Self {
            queue: Mutex::new(RefCell::new(Deque::new())),
        }
    }

    /// Enqueue a control, replacing an older queued update to the same
    /// parameter or modulation-route field when possible.
    pub fn try_send(
        &self,
        command: ControlMessage,
    ) -> Result<(), embassy_sync::channel::TrySendError<ControlMessage>> {
        self.queue.lock(|queue| {
            let mut queue = queue.borrow_mut();
            if let Some(existing) = queue
                .iter_mut()
                .find(|existing| replaceable_same_field(existing, &command))
            {
                *existing = command;
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

fn replaceable_same_field(existing: &ControlMessage, incoming: &ControlMessage) -> bool {
    match (existing, incoming) {
        (ControlMessage::SetParam(left, _), ControlMessage::SetParam(right, _)) => left == right,
        (
            ControlMessage::SetModulationParam {
                route: left_route,
                parameter: left_parameter,
            },
            ControlMessage::SetModulationParam {
                route: right_route,
                parameter: right_parameter,
            },
        ) => {
            left_route == right_route
                && core::mem::discriminant(left_parameter)
                    == core::mem::discriminant(right_parameter)
        }
        _ => false,
    }
}
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
pub type PatchQueue = Channel<CriticalSectionRawMutex, Patch, PATCH_QUEUE_CAPACITY>;

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
    let mut audio = Audio::output(resources).expect("WM8731/SAI initialization failed");
    yield_now().await;

    let mut output: Block = [(0.0, 0.0); BLOCK_LENGTH];
    let mut interleaved = [0.0f32; BLOCK_LENGTH * 2];
    #[cfg(feature = "audio-profiling")]
    let mut profiler = profiling::AudioProfiler::new(BLOCK_CYCLE_BUDGET);
    let mut perf_monitor = diagnostics::PerfMonitor::new(BLOCK_CYCLE_BUDGET);
    let mut patch_transition = PatchTransition::default();

    // Render before starting the receive clock. The SAI input ring cannot
    // overrun while the first, comparatively expensive DSP block is prepared.
    engine.process_interleaved(&mut interleaved, 2);
    copy_output(&interleaved, &mut output);
    yield_now().await;

    audio
        .start(&output)
        .await
        .expect("SAI stream failed to start");

    diagnostics::emit(diagnostics::Event::AudioStarted);

    loop {
        match audio.transfer(&output).await {
            Ok(()) => {}
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
                let _ = error;
                panic!("unrecoverable audio transfer failure");
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
            patch_transition.enqueue(patch);
        }
        let transition_action = patch_transition.begin_block();
        if let Some(patch) = transition_action.patch {
            engine.apply_patch(&patch);
        }
        // Releases that overflowed the performance queue are correctness
        // critical and do not wait behind replaceable parameter traffic.
        apply_pending_releases(engine, pending_releases);

        while DWT::cycle_count().wrapping_sub(work_started) < CONTROL_CYCLE_BUDGET {
            if let Ok(command) = performance.try_receive() {
                apply_control(engine, command);
                continue;
            }
            if let Ok(command) = controls.try_receive() {
                queue_midi_parameter_applied(&command);
                apply_control(engine, command);
                continue;
            }
            break;
        }
        #[cfg(feature = "audio-profiling")]
        profiler.end(RenderStage::ControlDrain);

        if transition_action.render {
            #[cfg(feature = "audio-profiling")]
            engine.process_interleaved_profiled(&mut interleaved, 2, &mut profiler);
            #[cfg(not(feature = "audio-profiling"))]
            engine.process_interleaved(&mut interleaved, 2);
        }
        patch_transition.finish_block(&mut interleaved, transition_action.render);

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
        if let Some(event) = perf_monitor.observe(work_cycles) {
            diagnostics::emit(event);
        }
    }
}

fn apply_control(engine: &mut HardwareSynth, command: ControlMessage) {
    engine.handle_control(command);
}

fn apply_pending_releases(engine: &mut HardwareSynth, pending: &PendingReleases) {
    if pending.take_all_notes_off() {
        apply_control(engine, ControlMessage::AllNotesOff);
    }
    for (word_index, mut word) in pending.take().into_iter().enumerate() {
        while word != 0 {
            let note = word_index as u8 * 32 + word.trailing_zeros() as u8;
            word &= word - 1;
            apply_control(engine, ControlMessage::NoteOff { note });
        }
    }
}

#[inline]
pub(crate) fn overruns_count() -> u32 {
    OVERRUNS_COUNT.load(Ordering::Relaxed)
}

#[inline]
pub(crate) fn underruns_count() -> u32 {
    UNDERRUNS_COUNT.load(Ordering::Relaxed)
}

#[inline]
fn increment_overruns() {
    OVERRUNS_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[inline]
fn increment_underruns() {
    UNDERRUNS_COUNT.fetch_add(1, Ordering::Relaxed);
}

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

#[cfg(feature = "diagnostics")]
fn queue_midi_parameter_applied(command: &ControlMessage) {
    match command {
        ControlMessage::SetParam(param, value) => diagnostics::emit(diagnostics::Event::Param {
            param: *param,
            value: *value,
        }),
        ControlMessage::SetModulationParam { route, parameter } => {
            diagnostics::emit(diagnostics::Event::ModulationParam {
                route: *route,
                parameter: *parameter,
            })
        }
        _ => {}
    }
}

#[cfg(not(feature = "diagnostics"))]
#[inline(always)]
fn queue_midi_parameter_applied(_command: &ControlMessage) {}

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
