#[derive(Clone, Copy)]
#[repr(u8)]
enum Register {
    LeftInputVolume = 0x00,
    RightInputVolume = 0x01,
    LeftHeadphoneVolume = 0x02,
    RightHeadphoneVolume = 0x03,
    AnalogPath = 0x04,
    DigitalPath = 0x05,
    Power = 0x06,
    Interface = 0x07,
    SampleRate = 0x08,
    Active = 0x09,
    Reset = 0x0f,
}

const CONFIG: &[(Register, u16)] = &[
    (Register::Reset, 0x000),
    (Register::LeftInputVolume, 0x017),
    (Register::RightInputVolume, 0x017),
    (Register::LeftHeadphoneVolume, 0x000),
    (Register::RightHeadphoneVolume, 0x000),
    (Register::AnalogPath, 0x012),
    (Register::DigitalPath, 0x001),
    (Register::Power, 0x042),
    (Register::Interface, 0x009),
    (Register::SampleRate, 0x000),
    (Register::Active, 0x000),
    (Register::Active, 0x001),
];

const fn command(register: Register, value: u16) -> [u8; 2] {
    let value = value & 0x01ff;
    [((register as u8) << 1) | ((value >> 8) as u8), value as u8]
}

use embassy_stm32::i2c::{self, I2c, Master};
use embassy_stm32::mode::Blocking;

const ADDRESS: u8 = 0x1a;

pub struct Wm8731 {
    bus: I2c<'static, Blocking, Master>,
}

impl Wm8731 {
    pub fn new(bus: I2c<'static, Blocking, Master>) -> Self {
        Self { bus }
    }

    pub fn start(&mut self) -> Result<(), i2c::Error> {
        for &(register, value) in CONFIG {
            self.bus
                .blocking_write(ADDRESS, &command(register, value))?;
            // At 480 MHz this is just over the WM8731's required inter-command
            // settling delay used by the proven Daisy Seed 1.1 setup.
            cortex_m::asm::delay(5_000);
        }
        Ok(())
    }
}

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::{Register, command};

    #[test]
    fn encodes_nine_bit_register_write() {
        assert_eq!(command(Register::Interface, 0x109), [0x0f, 0x09]);
        assert_eq!(command(Register::Reset, 0), [0x1e, 0x00]);
    }

    #[test]
    fn masks_value_to_nine_bits() {
        assert_eq!(
            command(Register::Reset, 0x3ff),
            command(Register::Reset, 0x1ff)
        );
    }

    #[test]
    fn register_address_in_high_byte() {
        assert_eq!(command(Register::Power, 0), [0x0c, 0x00]);
        assert_eq!(command(Register::Active, 0x001), [0x12, 0x01]);
        assert_eq!(command(Register::LeftInputVolume, 0x017), [0x00, 0x17]);
    }
}
