//! Non-blocking status-indicator events shared by real-time producers.

use core::sync::atomic::{AtomicU32, Ordering};

use embassy_daisy::PwmUserLed;
use embassy_stm32::peripherals::TIM3;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LedStatus {
    Off = 0,
    Midi = 1,
    XRun = 2,
}

impl LedStatus {
    fn from_u32(value: u32) -> Self {
        match value {
            1 => Self::Midi,
            2 => Self::XRun,
            _ => Self::Off,
        }
    }
}

/// Shared bounded event source. Call [`split`](Indicator::split) once at startup
/// and hand each side to producers and the LED task.
pub struct Indicator {
    status: AtomicU32,
    wake: Signal<CriticalSectionRawMutex, ()>,
}

impl Indicator {
    pub const fn new() -> Self {
        Self {
            status: AtomicU32::new(LedStatus::Off as u32),
            wake: Signal::new(),
        }
    }

    pub fn split(&self) -> (Sender<'_>, Receiver<'_>) {
        (
            Sender {
                status: &self.status,
                wake: &self.wake,
            },
            Receiver {
                status: &self.status,
                wake: &self.wake,
            },
        )
    }
}

#[derive(Clone, Copy)]
pub struct Sender<'a> {
    status: &'a AtomicU32,
    wake: &'a Signal<CriticalSectionRawMutex, ()>,
}

impl Sender<'_> {
    /// Activity is deliberately lossy: one queued event is enough to wake the
    /// LED task and a burst is visually coalesced.
    #[inline]
    pub fn notify_midi(self) {
        self.promote(LedStatus::Midi);
    }

    /// Audio gaps have priority over MIDI activity.
    #[inline]
    pub fn notify_xrun(self) {
        self.promote(LedStatus::XRun);
    }

    fn promote(self, level: LedStatus) {
        let prev = self.status.fetch_max(level as u32, Ordering::AcqRel);
        if prev < level as u32 {
            self.wake.signal(());
        }
    }
}

#[derive(Clone, Copy)]
pub struct Receiver<'a> {
    status: &'a AtomicU32,
    wake: &'a Signal<CriticalSectionRawMutex, ()>,
}

impl Receiver<'_> {
    async fn wait(&self) -> LedStatus {
        loop {
            if let Some(status) = self.take() {
                return status;
            }
            self.wake.wait().await;
        }
    }

    fn take(&self) -> Option<LedStatus> {
        match LedStatus::from_u32(self.status.swap(LedStatus::Off as u32, Ordering::AcqRel)) {
            LedStatus::Off => None,
            status => Some(status),
        }
    }
}

/// Sole owner of the PWM LED. Incoming activity is coalesced so a MIDI burst
/// never leaves a delayed animation backlog; audio gaps always take priority.
#[embassy_executor::task]
pub async fn run_task(mut led: PwmUserLed<TIM3>, receiver: Receiver<'static>) -> ! {
    led.off();

    loop {
        match receiver.wait().await {
            LedStatus::XRun => flash_led_for_xrun(&mut led).await,
            LedStatus::Midi => flash_led_for_midi_activity(&mut led).await,
            LedStatus::Off => led.off(),
        }

        // Activity received while the LED was illuminated is represented by
        // that pulse and is discarded. Audio gaps retain priority and replay
        // immediately if another one arrived during the pattern.
        while matches!(receiver.take(), Some(LedStatus::XRun)) {
            flash_led_for_xrun(&mut led).await;
        }
    }
}

async fn flash_led_for_midi_activity(led: &mut PwmUserLed<TIM3>) {
    use embassy_time::Timer;

    const MIDI_LED_BRIGHTNESS: u8 = 64;
    const MIDI_LED_PULSE_MS: u64 = 60;

    led.set_brightness(MIDI_LED_BRIGHTNESS);
    Timer::after_millis(MIDI_LED_PULSE_MS).await;
    led.off();
}

async fn flash_led_for_xrun(led: &mut PwmUserLed<TIM3>) {
    use embassy_time::Timer;

    const AUDIO_GAP_LED_PULSE_MS: u64 = 25;
    const AUDIO_GAP_LED_FLASHES: usize = 3;

    for _ in 0..AUDIO_GAP_LED_FLASHES {
        led.set_brightness(u8::MAX);
        Timer::after_millis(AUDIO_GAP_LED_PULSE_MS).await;
        led.off();
        Timer::after_millis(AUDIO_GAP_LED_PULSE_MS).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{Indicator, LedStatus};

    #[test]
    fn coalesce_prioritizes_xruns() {
        let indicator = Indicator::new();
        let (sender, receiver) = indicator.split();
        sender.notify_midi();
        sender.notify_midi();
        sender.notify_xrun();

        assert_eq!(receiver.take(), Some(LedStatus::XRun));
    }

    #[test]
    fn midi_burst_coalesces_to_one_status() {
        let indicator = Indicator::new();
        let (sender, receiver) = indicator.split();
        for _ in 0..32 {
            sender.notify_midi();
        }

        assert_eq!(receiver.take(), Some(LedStatus::Midi));
        assert_eq!(receiver.take(), None);
    }

    #[test]
    fn xrun_priority_survives_a_midi_burst() {
        let indicator = Indicator::new();
        let (sender, receiver) = indicator.split();
        for _ in 0..32 {
            sender.notify_midi();
        }
        sender.notify_xrun();

        assert_eq!(receiver.take(), Some(LedStatus::XRun));
    }
}
