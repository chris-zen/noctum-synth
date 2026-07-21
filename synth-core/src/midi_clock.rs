//! MIDI real-time clock mode, tempo following, and transport state.

/// Prophet Rev2-compatible MIDI clock mode.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MidiClockMode {
    #[default]
    Off,
    Master,
    Slave,
    SlaveThru,
    SlaveNoStartStop,
}

impl MidiClockMode {
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Master,
        Self::Slave,
        Self::SlaveThru,
        Self::SlaveNoStartStop,
    ];

    pub const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Master,
            2 => Self::Slave,
            3 => Self::SlaveThru,
            4 => Self::SlaveNoStartStop,
            _ => Self::Off,
        }
    }

    pub const fn index(self) -> usize {
        match self {
            Self::Off => 0,
            Self::Master => 1,
            Self::Slave => 2,
            Self::SlaveThru => 3,
            Self::SlaveNoStartStop => 4,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Master => "Master",
            Self::Slave => "Slave",
            Self::SlaveThru => "Slave Thru",
            Self::SlaveNoStartStop => "Slave No S/S",
        }
    }

    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Off | Self::Slave | Self::SlaveNoStartStop)
    }

    pub const fn effective(self) -> Self {
        if self.is_supported() { self } else { Self::Off }
    }

    pub const fn receives_clock(self) -> bool {
        matches!(self.effective(), Self::Slave | Self::SlaveNoStartStop)
    }

    pub const fn receives_start_stop(self) -> bool {
        matches!(self.effective(), Self::Slave)
    }
}

/// Supported MIDI System Real-Time input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiRealtimeEvent {
    TimingClock { timestamp_micros: u64 },
    Start,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MidiTransportState {
    #[default]
    Stopped,
    Running,
}

/// Read-only snapshot of the engine's MIDI clock follower.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiClockStatus {
    pub configured_mode: MidiClockMode,
    pub effective_mode: MidiClockMode,
    pub live: bool,
    pub learned_bpm: Option<f32>,
    pub effective_bpm: f32,
    pub transport: MidiTransportState,
    pub pulse_position: u64,
}

const WINDOW: usize = 5;
const MICROS_PER_MINUTE: f32 = 60_000_000.0;

pub(crate) struct MidiClockFollower {
    mode: MidiClockMode,
    live: bool,
    learned_bpm: Option<f32>,
    last_timestamp: Option<u64>,
    intervals: [u64; WINDOW],
    interval_count: usize,
    interval_index: usize,
    frames_since_tick: u64,
    loss_frames: u64,
    transport: MidiTransportState,
    pulse_position: u64,
}

impl MidiClockFollower {
    pub(crate) fn new(sample_rate: f32) -> Self {
        Self {
            mode: MidiClockMode::Off,
            live: false,
            learned_bpm: None,
            last_timestamp: None,
            intervals: [0; WINDOW],
            interval_count: 0,
            interval_index: 0,
            frames_since_tick: 0,
            loss_frames: (sample_rate.max(1.0) * 0.5) as u64,
            transport: MidiTransportState::Stopped,
            pulse_position: 0,
        }
    }

    pub(crate) fn set_mode(&mut self, mode: MidiClockMode) -> bool {
        if self.mode == mode {
            return false;
        }
        self.mode = mode;
        self.reset();
        true
    }

    fn reset(&mut self) {
        self.live = false;
        self.learned_bpm = None;
        self.last_timestamp = None;
        self.interval_count = 0;
        self.interval_index = 0;
        self.frames_since_tick = 0;
        self.transport = MidiTransportState::Stopped;
        self.pulse_position = 0;
    }

    pub(crate) fn handle(&mut self, event: MidiRealtimeEvent) -> Option<f32> {
        match event {
            MidiRealtimeEvent::TimingClock { timestamp_micros } => self.tick(timestamp_micros),
            MidiRealtimeEvent::Start if self.mode.receives_start_stop() => {
                self.transport = MidiTransportState::Running;
                self.pulse_position = 0;
                None
            }
            MidiRealtimeEvent::Stop if self.mode.receives_start_stop() => {
                self.transport = MidiTransportState::Stopped;
                None
            }
            _ => None,
        }
    }

    fn tick(&mut self, timestamp: u64) -> Option<f32> {
        if !self.mode.receives_clock() {
            return None;
        }
        self.live = true;
        self.frames_since_tick = 0;
        if self.transport == MidiTransportState::Running {
            self.pulse_position = self.pulse_position.saturating_add(1);
        }
        let previous = self.last_timestamp.replace(timestamp)?;
        let Some(interval) = timestamp.checked_sub(previous).filter(|value| *value > 0) else {
            self.interval_count = 0;
            self.interval_index = 0;
            return None;
        };
        let bpm = MICROS_PER_MINUTE / (24.0 * interval as f32);
        // Allow small timestamp jitter at the supported range boundaries,
        // then clamp the learned result to the engine's actual limits.
        if !(29.0..=260.0).contains(&bpm) {
            self.interval_count = 0;
            self.interval_index = 0;
            return None;
        }
        self.intervals[self.interval_index] = interval;
        self.interval_index = (self.interval_index + 1) % WINDOW;
        self.interval_count = (self.interval_count + 1).min(WINDOW);
        let mut sorted = self.intervals;
        sorted[..self.interval_count].sort_unstable();
        let median = sorted[self.interval_count / 2];
        let learned = (MICROS_PER_MINUTE / (24.0 * median as f32)).clamp(30.0, 250.0);
        self.learned_bpm = Some(learned);
        Some(learned)
    }

    pub(crate) fn advance(&mut self, frames: usize) {
        if !self.live && self.transport != MidiTransportState::Running {
            return;
        }
        self.frames_since_tick = self.frames_since_tick.saturating_add(frames as u64);
        if self.frames_since_tick >= self.loss_frames {
            self.live = false;
            self.last_timestamp = None;
            self.interval_count = 0;
            self.interval_index = 0;
            self.transport = MidiTransportState::Stopped;
        }
    }

    pub(crate) const fn learned_bpm(&self) -> Option<f32> {
        self.learned_bpm
    }

    pub(crate) fn status(&self, effective_bpm: f32) -> MidiClockStatus {
        MidiClockStatus {
            configured_mode: self.mode,
            effective_mode: self.mode.effective(),
            live: self.live,
            learned_bpm: self.learned_bpm,
            effective_bpm,
            transport: self.transport,
            pulse_position: self.pulse_position,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_keep_rev2_indices() {
        for (index, mode) in MidiClockMode::ALL.into_iter().enumerate() {
            assert_eq!(mode.index(), index);
            assert_eq!(MidiClockMode::from_index(index), mode);
        }
        assert_eq!(MidiClockMode::SlaveThru.effective(), MidiClockMode::Off);
    }

    #[test]
    fn follower_learns_and_times_out_without_discarding_tempo() {
        let mut follower = MidiClockFollower::new(48_000.0);
        follower.set_mode(MidiClockMode::Slave);
        for timestamp in [0, 20_833, 41_666, 62_499, 83_332, 104_165] {
            follower.handle(MidiRealtimeEvent::TimingClock {
                timestamp_micros: timestamp,
            });
        }
        assert!((follower.learned_bpm().unwrap() - 120.0).abs() < 0.01);
        follower.advance(24_000);
        assert!(!follower.status(120.0).live);
        assert!(follower.learned_bpm().is_some());
    }

    #[test]
    fn follower_covers_supported_tempo_range_and_rejects_jitter_outlier() {
        for (interval, expected) in [(83_333, 30.0), (10_000, 250.0)] {
            let mut follower = MidiClockFollower::new(48_000.0);
            follower.set_mode(MidiClockMode::Slave);
            for tick in 0..6 {
                follower.handle(MidiRealtimeEvent::TimingClock {
                    timestamp_micros: tick * interval,
                });
            }
            assert!((follower.learned_bpm().unwrap() - expected).abs() < 0.01);
        }

        let mut follower = MidiClockFollower::new(48_000.0);
        follower.set_mode(MidiClockMode::Slave);
        for timestamp in [0, 20_800, 41_700, 62_533, 83_366, 104_199] {
            follower.handle(MidiRealtimeEvent::TimingClock {
                timestamp_micros: timestamp,
            });
        }
        assert!((follower.learned_bpm().unwrap() - 120.0).abs() < 0.2);
    }

    #[test]
    fn start_without_following_clock_times_out() {
        let mut follower = MidiClockFollower::new(48_000.0);
        follower.set_mode(MidiClockMode::Slave);
        follower.handle(MidiRealtimeEvent::Start);
        follower.advance(24_000);
        assert_eq!(
            follower.status(120.0).transport,
            MidiTransportState::Stopped
        );
    }

    #[test]
    fn no_start_stop_mode_ignores_transport() {
        let mut follower = MidiClockFollower::new(48_000.0);
        follower.set_mode(MidiClockMode::SlaveNoStartStop);
        follower.handle(MidiRealtimeEvent::Start);
        follower.handle(MidiRealtimeEvent::TimingClock {
            timestamp_micros: 0,
        });
        assert_eq!(
            follower.status(120.0).transport,
            MidiTransportState::Stopped
        );
        assert_eq!(follower.status(120.0).pulse_position, 0);
    }
}
