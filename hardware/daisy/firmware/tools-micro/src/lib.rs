#![no_std]

pub const PROGRAM_DATA_SYSEX_LEN: usize = 2346;
pub const PATCH_RECORD_SIZE: u32 = 1024;

pub const QSPI_SIZE_BYTES: u32 = 8 * 1024 * 1024;
pub const QSPI_SECTOR_SIZE: u32 = 4096;

pub const CATALOG_A_ADDRESS: u32 = 0x000c_0000;
const PROGRAMS_ADDRESS: u32 = CATALOG_A_ADDRESS + QSPI_SECTOR_SIZE;
const SLOT_COUNT: u32 = 8 * 128;
pub const PROGRAM_STORAGE_END: u32 =
    PROGRAMS_ADDRESS + SLOT_COUNT * PATCH_RECORD_SIZE + QSPI_SECTOR_SIZE;

pub const PRESET_COUNT: usize = 512;
pub const BANK_SIZE: usize = PRESET_COUNT * PROGRAM_DATA_SYSEX_LEN;
pub const BANK_CRC32: u32 = 0x3df3_3c23;

const BANK_SECTORS: u32 = (BANK_SIZE as u32).div_ceil(QSPI_SECTOR_SIZE);
pub const BANK_ADDRESS: u32 = QSPI_SIZE_BYTES - BANK_SECTORS * QSPI_SECTOR_SIZE;

const _: () = {
    assert!(CATALOG_A_ADDRESS % QSPI_SECTOR_SIZE == 0);
    assert!(PROGRAMS_ADDRESS % QSPI_SECTOR_SIZE == 0);
    assert!((PROGRAM_STORAGE_END - QSPI_SECTOR_SIZE) % QSPI_SECTOR_SIZE == 0);
    assert!(BANK_ADDRESS % QSPI_SECTOR_SIZE == 0);
    assert!(BANK_ADDRESS >= PROGRAM_STORAGE_END);
    assert!(BANK_ADDRESS + BANK_SIZE as u32 <= QSPI_SIZE_BYTES);
};

#[cfg(target_arch = "arm")]
const _: () = {
    assert!(QSPI_SIZE_BYTES == embassy_daisy::qspi::SIZE_BYTES);
    assert!(QSPI_SECTOR_SIZE == embassy_daisy::qspi::SECTOR_SIZE);
    assert!(CATALOG_A_ADDRESS == embassy_daisy::qspi::APPLICATION_RESERVED_END);
};

pub struct Crc32(u32);

impl Crc32 {
    pub const fn new() -> Self {
        Self(u32::MAX)
    }

    pub fn update(&mut self, data: &[u8]) {
        for &byte in data {
            self.0 ^= u32::from(byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(self.0 & 1);
                self.0 = (self.0 >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }

    pub const fn finish(self) -> u32 {
        !self.0
    }
}

pub fn bank_crc32(data: &[u8]) -> u32 {
    let mut crc = Crc32::new();
    crc.update(data);
    crc.finish()
}
