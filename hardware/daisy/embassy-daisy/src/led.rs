//! Onboard user LED control.

use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::timer::{Ch2, GeneralInstance4Channel, TimerPin};
use embassy_stm32::{Peri, peripherals};

use crate::pwm::{PwmChannel, PwmOutput};

/// Unconfigured pin connected to the Daisy Seed's onboard user LED.
///
/// Convert it into either a binary [`UserLed`] or a brightness-controlled
/// [`PwmUserLed`]. Keeping the pin unconfigured leaves all timer peripherals
/// available to the application.
pub struct UserLedPin {
    pin: Peri<'static, peripherals::PC7>,
}

impl UserLedPin {
    pub(crate) fn new(pin: Peri<'static, peripherals::PC7>) -> Self {
        Self { pin }
    }

    /// Converts the pin into a binary, manually controlled LED.
    pub fn into_led(self) -> UserLed {
        UserLed::new(self.pin)
    }

    /// Attaches the LED to channel 2 of a configured PWM timer.
    pub fn into_pwm_led<T>(self, channel: PwmChannel<T, Ch2>) -> PwmUserLed<T>
    where
        T: GeneralInstance4Channel,
        peripherals::PC7: TimerPin<T, Ch2>,
    {
        PwmUserLed::new(channel.into_output(self.pin))
    }
}

/// Active-high controller for the Daisy Seed's red user LED on PC7.
///
/// The LED is initialized off. This type retains ownership of the GPIO output
/// for its lifetime so the pin does not return to a floating state.
pub struct UserLed {
    output: Output<'static>,
}

impl UserLed {
    fn new(pin: Peri<'static, peripherals::PC7>) -> Self {
        Self {
            output: Output::new(pin, Level::Low, Speed::Low),
        }
    }

    /// Turns the LED on.
    pub fn on(&mut self) {
        self.output.set_high();
    }

    /// Turns the LED off.
    pub fn off(&mut self) {
        self.output.set_low();
    }

    /// Sets the LED to the requested state.
    pub fn set(&mut self, on: bool) {
        if on {
            self.on();
        } else {
            self.off();
        }
    }

    /// Inverts the current LED state.
    pub fn toggle(&mut self) {
        self.output.toggle();
    }

    /// Returns whether the LED is currently on.
    pub fn is_on(&self) -> bool {
        self.output.is_set_high()
    }
}

/// Active-high, brightness-controlled Daisy Seed user LED.
///
/// Turning the LED off preserves its remembered brightness; turning it on
/// restores that level. The LED starts off with full brightness remembered.
pub struct PwmUserLed<T: GeneralInstance4Channel> {
    output: PwmOutput<T, Ch2>,
    brightness: u8,
    on: bool,
}

impl<T: GeneralInstance4Channel> PwmUserLed<T> {
    fn new(mut output: PwmOutput<T, Ch2>) -> Self {
        output.set_duty_cycle_fully_off();
        output.enable();
        Self {
            output,
            brightness: u8::MAX,
            on: false,
        }
    }

    /// Turns the LED on at its remembered brightness.
    ///
    /// If its brightness was explicitly set to zero, full brightness is
    /// restored so that `on()` always produces visible output.
    pub fn on(&mut self) {
        if self.brightness == 0 {
            self.brightness = u8::MAX;
        }
        self.on = true;
        self.apply_brightness();
    }

    /// Turns the LED off while preserving its remembered brightness.
    pub fn off(&mut self) {
        self.on = false;
        self.output.set_duty_cycle_fully_off();
    }

    /// Sets whether the LED is on.
    pub fn set(&mut self, on: bool) {
        if on {
            self.on();
        } else {
            self.off();
        }
    }

    /// Toggles the LED while preserving its remembered brightness.
    pub fn toggle(&mut self) {
        self.set(!self.on);
    }

    /// Returns whether the LED is logically on.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Sets and applies linear brightness in `0..=255`.
    ///
    /// A nonzero value turns the LED on. Zero turns it off and becomes the new
    /// remembered brightness; a subsequent [`Self::on`] restores full
    /// brightness.
    pub fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
        self.on = brightness != 0;
        self.apply_brightness();
    }

    /// Returns the remembered linear brightness in `0..=255`.
    pub fn brightness(&self) -> u8 {
        self.brightness
    }

    fn apply_brightness(&mut self) {
        if !self.on || self.brightness == 0 {
            self.output.set_duty_cycle_fully_off();
            return;
        }

        let max = u64::from(self.output.max_duty_cycle());
        let duty = max * u64::from(self.brightness) / u64::from(u8::MAX);
        self.output.set_duty_cycle(duty as u32);
    }
}
