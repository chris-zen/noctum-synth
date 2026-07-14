//! IS25LP064 QSPI flash access for Daisy Seed 1.1.

use embassy_stm32::{Peri, mode::Blocking, peripherals, qspi};
use qspi::enums::{AddressSize, MemorySize, QspiWidth};

pub const SIZE_BYTES: u32 = 8 * 1024 * 1024;
/// The official Daisy bootloader reserves the first four 64 KiB sectors.
pub const BOOTLOADER_RESERVED_BYTES: u32 = 0x0004_0000;
/// Reserve the complete 480 KiB BOOT_SRAM image window, rounded to 64 KiB.
pub const APPLICATION_RESERVED_END: u32 = 0x000C_0000;
pub const PAGE_SIZE: usize = 256;
pub const SECTOR_SIZE: u32 = 4 * 1024;

pub struct QspiFlashResources {
    pub(crate) qspi: Peri<'static, peripherals::QUADSPI>,
    pub(crate) io0: Peri<'static, peripherals::PF8>,
    pub(crate) io1: Peri<'static, peripherals::PF9>,
    pub(crate) io2: Peri<'static, peripherals::PF7>,
    pub(crate) io3: Peri<'static, peripherals::PF6>,
    pub(crate) sck: Peri<'static, peripherals::PF10>,
    pub(crate) cs: Peri<'static, peripherals::PG6>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlashError {
    OutOfBounds,
    EmptyWrite,
    Protected,
    UnalignedErase,
}

pub struct QspiFlash {
    driver: qspi::Qspi<'static, peripherals::QUADSPI, Blocking>,
}

impl QspiFlashResources {
    pub fn init(self) -> QspiFlash {
        let mut config = qspi::Config::default();
        config.memory_size = MemorySize::_8MiB;
        config.address_size = AddressSize::_24bit;
        config.prescaler = 1; // 240 MHz AHB / 2 = 120 MHz, below the 133 MHz part limit.
        let driver = qspi::Qspi::new_blocking_bank1(
            self.qspi, self.io0, self.io1, self.io2, self.io3, self.sck, self.cs, config,
        );
        QspiFlash { driver }
    }
}

impl QspiFlash {
    pub fn jedec_id(&mut self) -> [u8; 3] {
        let mut id = [0; 3];
        self.driver
            .blocking_read(&mut id, transfer(0x9F, None, QspiWidth::SING));
        id
    }

    pub fn read(&mut self, address: u32, output: &mut [u8]) -> Result<(), FlashError> {
        if output.is_empty() {
            return Ok(());
        }
        check_range(address, output.len())?;
        self.driver
            .blocking_read(output, transfer(0x03, Some(address), QspiWidth::SING));
        Ok(())
    }

    pub fn erase_sector(&mut self, address: u32) -> Result<(), FlashError> {
        if address < APPLICATION_RESERVED_END {
            return Err(FlashError::Protected);
        }
        if address % SECTOR_SIZE != 0 {
            return Err(FlashError::UnalignedErase);
        }
        check_range(address, SECTOR_SIZE as usize)?;
        self.write_enable();
        self.driver
            .blocking_command(transfer(0xD7, Some(address), QspiWidth::NONE));
        self.wait_ready();
        Ok(())
    }

    /// Program bytes without erasing. Writes are split at page boundaries.
    pub fn program(&mut self, mut address: u32, mut data: &[u8]) -> Result<(), FlashError> {
        if data.is_empty() {
            return Err(FlashError::EmptyWrite);
        }
        if address < APPLICATION_RESERVED_END {
            return Err(FlashError::Protected);
        }
        check_range(address, data.len())?;
        while !data.is_empty() {
            let remaining = PAGE_SIZE - address as usize % PAGE_SIZE;
            let count = remaining.min(data.len());
            self.write_enable();
            self.driver.blocking_write(
                &data[..count],
                transfer(0x02, Some(address), QspiWidth::SING),
            );
            self.wait_ready();
            address += count as u32;
            data = &data[count..];
        }
        Ok(())
    }

    fn write_enable(&mut self) {
        self.driver
            .blocking_command(transfer(0x06, None, QspiWidth::NONE));
    }

    fn wait_ready(&mut self) {
        loop {
            let mut status = [0xFF];
            self.driver
                .blocking_read(&mut status, transfer(0x05, None, QspiWidth::SING));
            if status[0] & 1 == 0 {
                break;
            }
        }
    }
}

fn transfer(instruction: u8, address: Option<u32>, dwidth: QspiWidth) -> qspi::TransferConfig {
    qspi::TransferConfig {
        instruction,
        address,
        iwidth: QspiWidth::SING,
        awidth: if address.is_some() {
            QspiWidth::SING
        } else {
            QspiWidth::NONE
        },
        dwidth,
        ..Default::default()
    }
}

fn check_range(address: u32, length: usize) -> Result<(), FlashError> {
    let end = address
        .checked_add(length.try_into().map_err(|_| FlashError::OutOfBounds)?)
        .ok_or(FlashError::OutOfBounds)?;
    if end > SIZE_BYTES {
        Err(FlashError::OutOfBounds)
    } else {
        Ok(())
    }
}
