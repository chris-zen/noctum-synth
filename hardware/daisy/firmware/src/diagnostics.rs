//! Non-blocking diagnostics boundary between real-time producers and logging.

#[cfg(feature = "audio-profiling")]
use synth_core::RenderStage;
use synth_core::{ModRoute, ModulationParam, ParamId};

#[derive(Clone, Copy)]
#[cfg_attr(
    all(feature = "diagnostics", target_arch = "arm"),
    derive(defmt::Format)
)]
pub enum InvalidMidiReason {
    UnsupportedCable,
    UnsupportedCodeIndex,
    UnexpectedSysExContinuation,
    NestedSysExStart,
    SysExTooLong,
    InvalidMessage,
    InvalidSysExLength,
    InvalidSysExFraming,
    InvalidSysExManufacturer,
    InvalidSysExModel,
    UnsupportedSysExCommand,
    InvalidSysExBank,
    NonSevenBitSysExData,
    InvalidSysExProgramData,
    SysExOutputTooSmall,
}

#[derive(Clone, Copy)]
#[cfg_attr(
    all(feature = "diagnostics", target_arch = "arm"),
    derive(defmt::Format)
)]
pub enum StorageOperation {
    Load,
    Save,
    PersistSelection,
}

#[derive(Clone, Copy)]
#[cfg_attr(
    all(feature = "diagnostics", target_arch = "arm"),
    derive(defmt::Format)
)]
pub enum StorageFailureReason {
    Flash,
    InvalidAddress,
    InvalidRecord,
    VerifyFailed,
}

#[cfg(feature = "diagnostics")]
pub use enabled::{emit, emit_xrun, init, run_task, PerfMonitor};

#[cfg(not(feature = "diagnostics"))]
pub use disabled::{emit, emit_xrun, init, PerfMonitor};

#[derive(Clone, Copy)]
pub enum Event {
    AudioStarted,
    AudioUnavailable {
        reason: &'static str,
    },
    Param {
        param: ParamId,
        value: f32,
    },
    ModulationParam {
        route: ModRoute,
        parameter: ModulationParam,
    },
    Xrun {
        overruns: u32,
        underruns: u32,
    },
    Perf {
        blocks: u32,
        over_budget_blocks: u32,
        average_cycles: u32,
        p95_cycles: u32,
        p99_cycles: u32,
        maximum_cycles: u32,
        budget_cycles: u32,
    },
    #[cfg(feature = "audio-profiling")]
    PerfStages {
        worst_block: bool,
        cycles: [u32; RenderStage::COUNT],
    },
    ProfileBlock {
        blocks: u32,
        over_budget_blocks: u32,
        average_cycles: u32,
        maximum_cycles: u32,
    },
    NrpnRx {
        channel: u8,
        number: u16,
        value: u16,
    },
    ControlQueueFull,
    PatchQueueFull,
    ProgramStorageQueueFull,
    ProgramEditBufferReceived,
    ProgramDataReceived {
        bank: u8,
        program: u8,
    },
    ProgramChangeReceived {
        bank: u8,
        program: u8,
    },
    ProgramLoaded {
        bank: u8,
        program: u8,
        elapsed_micros: u64,
    },
    ProgramSaved {
        bank: u8,
        program: u8,
    },
    ProgramStorageFailed {
        operation: StorageOperation,
        reason: StorageFailureReason,
        bank: u8,
        program: u8,
    },
    InvalidMidi {
        cable: u8,
        reason: InvalidMidiReason,
        length: u16,
    },
    UsbMidiConnected,
    UsbMidiDisconnected,
    UsbMidiTrailingBytes {
        count: u8,
    },
    UsbMidiBufferOverflow,
    UsbAudioStarted,
    UsbAudioPrimed,
    UsbAudioStopped,
    UsbAudioConfigurationInvalid {
        reason: &'static str,
    },
    UsbAudioRecoveryUnavailable {
        endpoint: u8,
    },
}

#[cfg(feature = "diagnostics")]
mod enabled {
    use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
    use embassy_sync::channel::Channel;
    #[cfg(feature = "audio-profiling")]
    use synth_core::RenderStage;
    use synth_core::{ModRoute, ModulationParam};

    use super::Event;

    const QUEUE_CAPACITY: usize = 32;
    const PERF_REPORT_INTERVAL_BLOCKS: u32 = 1_500;
    const PERF_WARNING_THRESHOLD_PERMILLE: u32 = 850;
    const PERF_HISTOGRAM_BINS: usize = 128;
    const PERF_HISTOGRAM_RANGE_PERMILLE: u32 = 1_250;

    static EVENTS: Channel<CriticalSectionRawMutex, Event, QUEUE_CAPACITY> = Channel::new();
    static DROPPED_EVENTS: AtomicU32 = AtomicU32::new(0);
    static XRUN_EVENT_PENDING: AtomicBool = AtomicBool::new(false);

    /// Low-overhead accumulator for full audio-task work, including controls and
    /// output copying but excluding time asleep waiting for DMA.
    pub struct PerfMonitor {
        budget_cycles: u32,
        blocks: u32,
        over_budget_blocks: u32,
        total_cycles: u64,
        maximum_cycles: u32,
        histogram: [u16; PERF_HISTOGRAM_BINS],
    }

    impl PerfMonitor {
        pub const fn new(budget_cycles: u32) -> Self {
            Self {
                budget_cycles,
                blocks: 0,
                over_budget_blocks: 0,
                total_cycles: 0,
                maximum_cycles: 0,
                histogram: [0; PERF_HISTOGRAM_BINS],
            }
        }

        /// Observe one completed block and return a report only when the window's
        /// maximum reaches the warning threshold.
        #[inline]
        pub fn observe(&mut self, cycles: u32) -> Option<Event> {
            self.blocks += 1;
            self.over_budget_blocks += u32::from(cycles > self.budget_cycles);
            self.total_cycles += u64::from(cycles);
            self.maximum_cycles = self.maximum_cycles.max(cycles);
            let range = histogram_range(self.budget_cycles);
            let bin = ((u64::from(cycles.min(range)) * PERF_HISTOGRAM_BINS as u64)
                / u64::from(range.max(1)))
            .min((PERF_HISTOGRAM_BINS - 1) as u64) as usize;
            self.histogram[bin] = self.histogram[bin].saturating_add(1);

            if self.blocks < PERF_REPORT_INTERVAL_BLOCKS {
                return None;
            }

            let blocks = self.blocks;
            let over_budget_blocks = self.over_budget_blocks;
            let average_cycles = (self.total_cycles / u64::from(blocks)) as u32;
            let p95_cycles = histogram_quantile(&self.histogram, blocks, 95, range);
            let p99_cycles = histogram_quantile(&self.histogram, blocks, 99, range);
            let maximum_cycles = self.maximum_cycles;
            self.blocks = 0;
            self.over_budget_blocks = 0;
            self.total_cycles = 0;
            self.maximum_cycles = 0;
            self.histogram = [0; PERF_HISTOGRAM_BINS];

            let near_budget = u64::from(maximum_cycles) * 1_000
                >= u64::from(self.budget_cycles) * u64::from(PERF_WARNING_THRESHOLD_PERMILLE);
            near_budget.then_some(Event::Perf {
                blocks,
                over_budget_blocks,
                average_cycles,
                p95_cycles,
                p99_cycles,
                maximum_cycles,
                budget_cycles: self.budget_cycles,
            })
        }
    }

    fn histogram_range(budget_cycles: u32) -> u32 {
        (u64::from(budget_cycles) * u64::from(PERF_HISTOGRAM_RANGE_PERMILLE) / 1_000)
            .min(u64::from(u32::MAX)) as u32
    }

    fn histogram_quantile(
        histogram: &[u16; PERF_HISTOGRAM_BINS],
        blocks: u32,
        percentile: u32,
        range: u32,
    ) -> u32 {
        let target = (u64::from(blocks) * u64::from(percentile) + 99) / 100;
        let mut cumulative = 0u64;
        for (index, count) in histogram.iter().enumerate() {
            cumulative += u64::from(*count);
            if cumulative >= target {
                return ((index as u64 + 1) * u64::from(range) / PERF_HISTOGRAM_BINS as u64) as u32;
            }
        }
        range
    }

    /// Attempt to enqueue a diagnostic without ever waiting for the reporter.
    #[inline]
    pub fn emit(event: Event) {
        if EVENTS.try_send(event).is_err() {
            DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Coalesce any number of audio gaps into one queued reporter wakeup.
    #[inline]
    pub fn emit_xrun(overruns: u32, underruns: u32) {
        if XRUN_EVENT_PENDING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
            && EVENTS
                .try_send(Event::Xrun {
                    overruns,
                    underruns,
                })
                .is_err()
        {
            XRUN_EVENT_PENDING.store(false, Ordering::Release);
            DROPPED_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn init() {
        defmt::info!("diagnostics enabled");
    }

    /// All formatting, RTT writes, and rate limiting run in thread mode. Audio runs
    /// on a higher-priority interrupt executor, and RTT itself is configured to
    /// drop bytes instead of blocking with interrupts masked.
    #[embassy_executor::task]
    pub async fn run_task() -> ! {
        use crate::audio;
        use embassy_time::Timer;

        let mut last_overruns_reported = 0u32;
        let mut last_underruns_reported = 0u32;

        loop {
            match receive().await {
                Event::AudioStarted => {
                    defmt::info!("running four-voice synth engine at 48 kHz")
                }
                Event::AudioUnavailable { reason } => {
                    defmt::error!("DAC audio unavailable: {=str}", reason)
                }
                Event::Param { param, value } => {
                    defmt::info!("PARAM: {=str} v={=f32}", param.name(), value)
                }
                Event::ModulationParam { route, parameter } => {
                    log_midi_modulation_parameter(route, parameter)
                }
                Event::Xrun {
                    overruns,
                    underruns,
                } => {
                    let overruns_since_last = overruns.wrapping_sub(last_overruns_reported);
                    let underruns_since_last = underruns.wrapping_sub(last_underruns_reported);
                    last_overruns_reported = overruns;
                    last_underruns_reported = underruns;

                    defmt::warn!(
                        "XRUN: rx_overruns={} (+{}) tx_underruns={} (+{}); recovering with silence",
                        overruns,
                        overruns_since_last,
                        underruns,
                        underruns_since_last
                    );
                    Timer::after_millis(900).await;
                    complete_xrun_report();
                    let overruns = audio::overruns_count();
                    let underruns = audio::underruns_count();
                    if overruns != last_overruns_reported || underruns != last_underruns_reported {
                        emit_xrun(overruns, underruns);
                    }
                }
                Event::Perf {
                    blocks,
                    over_budget_blocks,
                    average_cycles,
                    p95_cycles,
                    p99_cycles,
                    maximum_cycles,
                    budget_cycles,
                } => defmt::warn!(
                    "PERF: blocks={} over_budget={} avg={} p95={} p99={} max={} max_permille={} headroom={} budget={}",
                    blocks,
                    over_budget_blocks,
                    average_cycles,
                    p95_cycles,
                    p99_cycles,
                    maximum_cycles,
                    budget_permille(maximum_cycles, budget_cycles),
                    budget_cycles.saturating_sub(maximum_cycles),
                    budget_cycles
                ),
                #[cfg(feature = "audio-profiling")]
                Event::PerfStages {
                    worst_block,
                    cycles,
                } => {
                    defmt::info!(
                        "PERF stages worst={} controls={} env_mod={} osc={} filter={} amp_pan={} effects={} output={} copy={}",
                        worst_block,
                        cycles[RenderStage::ControlDrain.index()],
                        cycles[RenderStage::EnvelopesAndModulation.index()],
                        cycles[RenderStage::Oscillators.index()],
                        cycles[RenderStage::Filter.index()],
                        cycles[RenderStage::AmplifierAndPan.index()],
                        cycles[RenderStage::Effects.index()],
                        cycles[RenderStage::MasterOutput.index()],
                        cycles[RenderStage::OutputCopy.index()]
                    );
                    defmt::info!(
                        "PERF modulation worst={} envelopes={} lfo_control={} lfo_generation={} audio_routes={} control_routes={} interpolation={}",
                        worst_block,
                        cycles[RenderStage::EnvelopeAdvance.index()],
                        cycles[RenderStage::LfoControlRouting.index()],
                        cycles[RenderStage::LfoGeneration.index()],
                        cycles[RenderStage::AudioModulationRouting.index()],
                        cycles[RenderStage::ControlRateRouting.index()],
                        cycles[RenderStage::ControlRateInterpolation.index()]
                    );
                    defmt::info!(
                        "PERF oscillators worst={} control={} waveform={} mix={}",
                        worst_block,
                        cycles[RenderStage::OscillatorControl.index()],
                        cycles[RenderStage::OscillatorWaveform.index()],
                        cycles[RenderStage::OscillatorMix.index()]
                    );
                    defmt::info!(
                        "PERF effects worst={} prepare={} reverb_combs={} reverb_allpasses={} mix={}",
                        worst_block,
                        cycles[RenderStage::EffectsPreparation.index()],
                        cycles[RenderStage::ReverbCombs.index()],
                        cycles[RenderStage::ReverbAllpasses.index()],
                        cycles[RenderStage::EffectsMix.index()]
                    );
                }
                Event::ProfileBlock {
                    blocks,
                    over_budget_blocks,
                    average_cycles,
                    maximum_cycles,
                } => defmt::info!(
                    "PERF profile blocks={} over_budget={} avg_cycles={} max_cycles={}",
                    blocks,
                    over_budget_blocks,
                    average_cycles,
                    maximum_cycles
                ),
                Event::NrpnRx {
                    channel,
                    number,
                    value,
                } => defmt::debug!("NRPN: ch={} id={} raw={}", channel, number, value),
                Event::ControlQueueFull => {
                    defmt::warn!("synth control queue full; dropping newest MIDI command")
                }
                Event::PatchQueueFull => {
                    defmt::warn!("synth patch queue full; dropping newest patch")
                }
                Event::ProgramStorageQueueFull => {
                    defmt::warn!("program storage overflow full; dropping newest request")
                }
                Event::ProgramEditBufferReceived => {
                    defmt::info!("received Rev2 Program Edit Buffer")
                }
                Event::ProgramDataReceived { bank, program } => defmt::info!(
                    "saving Rev2 Program Data bank={} program={}",
                    bank,
                    program
                ),
                Event::ProgramChangeReceived { bank, program } => defmt::info!(
                    "program change bank={} program={}",
                    bank,
                    program
                ),
                Event::ProgramLoaded {
                    bank,
                    program,
                    elapsed_micros,
                } => defmt::info!(
                    "loaded program bank={} program={} flash_us={}",
                    bank,
                    program,
                    elapsed_micros
                ),
                Event::ProgramSaved { bank, program } => {
                    defmt::info!("saved program bank={} program={}", bank, program)
                }
                Event::ProgramStorageFailed {
                    operation,
                    reason,
                    bank,
                    program,
                } => defmt::error!(
                    "program storage failed operation={:?} reason={:?} bank={} program={}",
                    operation,
                    reason,
                    bank,
                    program
                ),
                Event::InvalidMidi {
                    cable,
                    reason,
                    length,
                } => {
                    defmt::warn!(
                        "invalid MIDI message on cable {}: {:?}, length={}",
                        cable,
                        reason,
                        length
                    )
                }
                Event::UsbMidiConnected => defmt::info!("USB MIDI connected"),
                Event::UsbMidiDisconnected => defmt::info!("USB MIDI disconnected"),
                Event::UsbMidiTrailingBytes { count } => {
                    defmt::warn!("USB MIDI transfer had {} trailing byte(s)", count)
                }
                Event::UsbMidiBufferOverflow => {
                    defmt::error!("USB MIDI endpoint buffer overflow")
                }
                Event::UsbAudioStarted => defmt::info!("USB audio capture opened; priming"),
                Event::UsbAudioPrimed => defmt::info!("USB audio capture primed"),
                Event::UsbAudioStopped => defmt::info!("USB audio capture closed"),
                Event::UsbAudioConfigurationInvalid { reason } => {
                    defmt::error!("USB audio disabled: invalid configuration ({=str})", reason)
                }
                Event::UsbAudioRecoveryUnavailable { endpoint } => {
                    defmt::error!("USB audio recovery disabled: invalid endpoint {}", endpoint)
                }
            }

            let dropped = take_dropped_events();
            if dropped != 0 {
                defmt::warn!("diagnostic queue dropped {} events", dropped);
            }
        }
    }

    fn log_midi_modulation_parameter(route: ModRoute, parameter: ModulationParam) {
        match (route, parameter) {
            (ModRoute::Free(index), ModulationParam::Source(source)) => {
                defmt::info!("PARAM: Mod {} Source v={=str}", index + 1, source.name())
            }
            (ModRoute::Free(index), ModulationParam::Destination(destination)) => defmt::info!(
                "PARAM: Mod {} Destination v={=str}",
                index + 1,
                destination.name()
            ),
            (ModRoute::Free(index), ModulationParam::Amount(value)) => {
                defmt::info!("PARAM: Mod {} Amount v={=f32}", index + 1, value)
            }
            (ModRoute::Dedicated(source), ModulationParam::Source(value)) => {
                defmt::info!("PARAM: {=str} Source v={=str}", source.name(), value.name())
            }
            (ModRoute::Dedicated(source), ModulationParam::Destination(destination)) => {
                defmt::info!(
                    "PARAM: {=str} Destination v={=str}",
                    source.name(),
                    destination.name()
                )
            }
            (ModRoute::Dedicated(source), ModulationParam::Amount(value)) => {
                defmt::info!("PARAM: {=str} Amount v={=f32}", source.name(), value)
            }
        }
    }

    fn budget_permille(cycles: u32, budget_cycles: u32) -> u32 {
        (u64::from(cycles) * 1_000 / u64::from(budget_cycles.max(1))) as u32
    }

    async fn receive() -> Event {
        EVENTS.receive().await
    }

    fn complete_xrun_report() {
        XRUN_EVENT_PENDING.store(false, Ordering::Release);
    }

    fn take_dropped_events() -> u32 {
        DROPPED_EVENTS.swap(0, Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(super) const PERF_REPORT_INTERVAL_BLOCKS: u32 = PERF_REPORT_INTERVAL_BLOCKS;
}

#[cfg(not(feature = "diagnostics"))]
mod disabled {
    use super::Event;

    pub struct PerfMonitor;

    impl PerfMonitor {
        pub const fn new(_budget_cycles: u32) -> Self {
            Self
        }

        #[inline(always)]
        pub fn observe(&mut self, _cycles: u32) -> Option<Event> {
            None
        }
    }

    #[inline(always)]
    pub fn emit(_event: Event) {}

    #[inline(always)]
    pub fn emit_xrun(_overruns: u32, _underruns: u32) {}

    #[inline(always)]
    pub fn init() {}
}

#[cfg(all(test, feature = "diagnostics"))]
mod tests {
    use super::{enabled::PERF_REPORT_INTERVAL_BLOCKS, Event, PerfMonitor};

    #[test]
    fn performance_monitor_reports_only_near_the_budget() {
        let mut monitor = PerfMonitor::new(100);
        for _ in 1..PERF_REPORT_INTERVAL_BLOCKS {
            assert!(monitor.observe(50).is_none());
        }
        assert!(monitor.observe(89).is_none());

        for _ in 1..PERF_REPORT_INTERVAL_BLOCKS {
            assert!(monitor.observe(50).is_none());
        }
        match monitor.observe(90) {
            Some(Event::Perf {
                maximum_cycles,
                budget_cycles,
                ..
            }) => {
                assert_eq!(maximum_cycles, 90);
                assert_eq!(budget_cycles, 100);
            }
            _ => panic!("expected near-budget performance report"),
        }
    }
}
