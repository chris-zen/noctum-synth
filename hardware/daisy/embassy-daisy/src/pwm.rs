//! Typed PWM channel resources.

use core::marker::PhantomData;

use embassy_stm32::Peri;
use embassy_stm32::gpio::OutputType;
pub use embassy_stm32::time::Hertz as PwmFrequency;
use embassy_stm32::timer::low_level::CountingMode;
use embassy_stm32::timer::simple_pwm::{PwmPin, SimplePwm, SimplePwmChannel};
use embassy_stm32::timer::{Ch1, Ch2, Ch3, Ch4, GeneralInstance4Channel, TimerChannel, TimerPin};

/// Independently owned channels from one configured four-channel PWM timer.
///
/// Frequency and counting mode belong to the timer and are consequently shared
/// by all four channels. Pins are attached later by consuming an individual
/// [`PwmChannel`], so unused channels do not configure any GPIOs.
pub struct PwmChannels<T: GeneralInstance4Channel> {
    /// Timer channel 1.
    pub ch1: PwmChannel<T, Ch1>,
    /// Timer channel 2.
    pub ch2: PwmChannel<T, Ch2>,
    /// Timer channel 3.
    pub ch3: PwmChannel<T, Ch3>,
    /// Timer channel 4.
    pub ch4: PwmChannel<T, Ch4>,
}

impl<T: GeneralInstance4Channel> PwmChannels<T> {
    /// Configures `timer` for edge-aligned, up-counting PWM and splits it into
    /// independently owned channels.
    pub fn new(timer: Peri<'static, T>, frequency: PwmFrequency) -> Self {
        let pwm = SimplePwm::new(
            timer,
            None,
            None,
            None,
            None,
            frequency,
            CountingMode::EdgeAlignedUp,
        );
        let channels = pwm.split();
        Self {
            ch1: PwmChannel::new(channels.ch1),
            ch2: PwmChannel::new(channels.ch2),
            ch3: PwmChannel::new(channels.ch3),
            ch4: PwmChannel::new(channels.ch4),
        }
    }
}

/// Ownership token for one channel of a configured PWM timer.
///
/// `C` retains the channel number at the type level, unlike Embassy's erased
/// [`SimplePwmChannel`]. Consuming this token prevents the channel from being
/// assigned to more than one output.
pub struct PwmChannel<T: GeneralInstance4Channel, C: TimerChannel> {
    channel: SimplePwmChannel<'static, T>,
    channel_marker: PhantomData<C>,
}

impl<T: GeneralInstance4Channel, C: TimerChannel> PwmChannel<T, C> {
    fn new(channel: SimplePwmChannel<'static, T>) -> Self {
        Self {
            channel,
            channel_marker: PhantomData,
        }
    }

    /// Attaches `pin` to this channel as a push-pull PWM output.
    ///
    /// The returned output starts disabled with a zero duty cycle.
    pub fn into_output(self, pin: Peri<'static, impl TimerPin<T, C>>) -> PwmOutput<T, C> {
        let pin = PwmPin::new(pin, OutputType::PushPull);
        let mut output = PwmOutput {
            channel: self.channel,
            _pin: pin,
        };
        output.set_duty_cycle_fully_off();
        output
    }
}

/// A PWM channel with its configured pin retained for the output's lifetime.
pub struct PwmOutput<T: GeneralInstance4Channel, C: TimerChannel> {
    channel: SimplePwmChannel<'static, T>,
    _pin: PwmPin<'static, T, C>,
}

impl<T: GeneralInstance4Channel, C: TimerChannel> PwmOutput<T, C> {
    /// Enables waveform output on this channel.
    pub fn enable(&mut self) {
        self.channel.enable();
    }

    /// Disables waveform output on this channel.
    pub fn disable(&mut self) {
        self.channel.disable();
    }

    /// Returns whether waveform output is enabled.
    pub fn is_enabled(&self) -> bool {
        self.channel.is_enabled()
    }

    /// Returns the timer's shared PWM frequency.
    pub fn frequency(&self) -> PwmFrequency {
        self.channel.get_frequency()
    }

    /// Returns the largest accepted duty-cycle value.
    pub fn max_duty_cycle(&self) -> u32 {
        self.channel.max_duty_cycle()
    }

    /// Sets the raw duty cycle in `0..=max_duty_cycle()`.
    pub fn set_duty_cycle(&mut self, duty: u32) {
        self.channel.set_duty_cycle(duty);
    }

    /// Sets the output fully inactive.
    pub fn set_duty_cycle_fully_off(&mut self) {
        self.channel.set_duty_cycle_fully_off();
    }

    /// Sets the output fully active.
    pub fn set_duty_cycle_fully_on(&mut self) {
        self.channel.set_duty_cycle_fully_on();
    }

    /// Sets the duty cycle to `numerator / denominator`.
    ///
    /// `denominator` must be nonzero and `numerator` must not exceed it.
    pub fn set_duty_cycle_fraction(&mut self, numerator: u32, denominator: u32) {
        self.channel.set_duty_cycle_fraction(numerator, denominator);
    }

    /// Sets the duty cycle in `0..=100` percent.
    pub fn set_duty_cycle_percent(&mut self, percent: u8) {
        self.channel.set_duty_cycle_percent(percent);
    }

    /// Returns the current raw duty-cycle value.
    pub fn current_duty_cycle(&self) -> u16 {
        self.channel.current_duty_cycle()
    }
}

impl<T: GeneralInstance4Channel, C: TimerChannel> Drop for PwmOutput<T, C> {
    fn drop(&mut self) {
        self.channel.disable();
    }
}
