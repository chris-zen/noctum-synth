//! Real-time audio rendering on the interrupt executor.

use core::sync::atomic::{AtomicU32, Ordering};

use cortex_m::peripheral::DWT;
use embassy_daisy::audio::{Audio, AudioResources, BLOCK_LENGTH, Block, Error as AudioError};
use embassy_executor::InterruptExecutor;
use embassy_futures::yield_now;
use embassy_stm32::interrupt;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use synth_core::{ControlMessage, SynthEngineWithMemory};

#[cfg(feature = "audio-profiling")]
use crate::profiling;
use crate::{diagnostics, indicator};

pub const CONTROL_QUEUE_CAPACITY: usize = 256;
// Bound non-audio work between DMA transfers. Four commands per block still
// sustains 6,000 parameter updates/second at 48 kHz with 32-frame blocks.
const MAX_CONTROLS_PER_BLOCK: usize = 4;
const BLOCK_CYCLE_BUDGET: u32 =
    embassy_daisy::clocks::SYSCLK_HZ / embassy_daisy::audio::SAMPLE_RATE_HZ * BLOCK_LENGTH as u32;

static EXECUTOR: InterruptExecutor = InterruptExecutor::new();
static OVERRUNS_COUNT: AtomicU32 = AtomicU32::new(0);
static UNDERRUNS_COUNT: AtomicU32 = AtomicU32::new(0);

pub type HardwareSynth = SynthEngineWithMemory<1, &'static mut [f32]>;
pub type ControlQueue = Channel<CriticalSectionRawMutex, ControlMessage, CONTROL_QUEUE_CAPACITY>;

pub fn spawn(
    resources: AudioResources,
    engine: &'static mut HardwareSynth,
    controls: &'static ControlQueue,
    indicator: indicator::Sender<'static>,
) -> Result<(), embassy_executor::SpawnError> {
    EXECUTOR
        .start(interrupt::I2C4_EV)
        .spawn(run_task(resources, engine, controls, indicator)?);
    Ok(())
}

#[embassy_executor::task]
pub async fn run_task(
    resources: AudioResources,
    engine: &'static mut HardwareSynth,
    controls: &'static ControlQueue,
    indicator: indicator::Sender<'static>,
) -> ! {
    let mut audio = Audio::output(resources).expect("WM8731/SAI initialization failed");
    yield_now().await;

    let mut output: Block = [(0.0, 0.0); BLOCK_LENGTH];
    let mut interleaved = [0.0f32; BLOCK_LENGTH * 2];
    #[cfg(feature = "audio-profiling")]
    let mut profiler = profiling::AudioProfiler::new(BLOCK_CYCLE_BUDGET);
    let mut perf_monitor = diagnostics::PerfMonitor::new(BLOCK_CYCLE_BUDGET);

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

        for _ in 0..MAX_CONTROLS_PER_BLOCK {
            let Ok(command) = controls.try_receive() else {
                break;
            };
            queue_midi_parameter_applied(&command);
            engine.handle_control(command);
        }

        #[cfg(feature = "audio-profiling")]
        {
            profiler.begin_block();
            engine.process_interleaved_profiled(&mut interleaved, 2, &mut profiler);
            profiler.end_block();
        }
        #[cfg(not(feature = "audio-profiling"))]
        engine.process_interleaved(&mut interleaved, 2);
        copy_output(&interleaved, &mut output);

        let work_cycles = DWT::cycle_count().wrapping_sub(work_started);
        if let Some(event) = perf_monitor.observe(work_cycles) {
            diagnostics::emit(event);
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
        maximum: false,
        cycles: snapshot.stage_average,
    });
    diagnostics::emit(diagnostics::Event::PerfStages {
        maximum: true,
        cycles: snapshot.stage_max,
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
