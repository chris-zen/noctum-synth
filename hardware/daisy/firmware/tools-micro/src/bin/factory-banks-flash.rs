#![no_std]
#![no_main]

use {defmt_rtt as _, panic_probe as _};

use embassy_daisy::{
    Board,
    qspi::{QspiFlash, SECTOR_SIZE},
};
use tools_micro::{self as factory_banks, Crc32};

const COMPRESSED_BANK: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../target/factory-bank.zlib"
));

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::info!(
        "flashing factory bank bytes={} compressed={} qspi_offset={:#x}",
        factory_banks::BANK_SIZE,
        COMPRESSED_BANK.len(),
        factory_banks::BANK_ADDRESS
    );

    let mut core = cortex_m::Peripherals::take().expect("Cortex-M peripherals already initialized");
    core.DCB.enable_trace();
    core.DWT.enable_cycle_counter();
    core.SCB.disable_dcache(&mut core.CPUID);
    core.SCB.enable_icache();

    let parts = Board::take().expect("Daisy board already initialized");
    let mut qspi = QspiFlash::new(parts.qspi);
    qspi.prepare_storage();
    let mut sdram = parts
        .sdram
        .init(&mut core.MPU, &mut core.SCB, &mut core.CPUID)
        .expect("SDRAM data/address-line test failed");

    let words = factory_banks::BANK_SIZE.div_ceil(core::mem::size_of::<f32>());
    let scratch = sdram
        .allocate_f32(words)
        .expect("SDRAM factory-bank allocation failed");
    let bank = unsafe {
        core::slice::from_raw_parts_mut(scratch.as_mut_ptr().cast::<u8>(), factory_banks::BANK_SIZE)
    };

    let decompressed = match miniz_oxide::inflate::decompress_slice_iter_to_slice(
        bank,
        core::iter::once(COMPRESSED_BANK),
        true,
        false,
    ) {
        Ok(len) => len,
        Err(_status) => {
            defmt::error!("factory bank decompress failed");
            loop {
                cortex_m::asm::wfi();
            }
        }
    };
    if decompressed != factory_banks::BANK_SIZE {
        defmt::error!(
            "factory bank size mismatch expected={} actual={}",
            factory_banks::BANK_SIZE,
            decompressed
        );
        loop {
            cortex_m::asm::wfi();
        }
    }

    let end = factory_banks::BANK_ADDRESS + factory_banks::BANK_SIZE as u32;
    let mut address = factory_banks::BANK_ADDRESS;
    while address < end {
        qspi.erase_sector(address)
            .expect("factory bank sector erase failed");
        address += SECTOR_SIZE;
    }

    qspi.program(factory_banks::BANK_ADDRESS, bank)
        .expect("factory bank program failed");

    let mut crc = Crc32::new();
    let mut message = [0_u8; factory_banks::PROGRAM_DATA_SYSEX_LEN];
    for index in 0..factory_banks::PRESET_COUNT {
        let offset = index * factory_banks::PROGRAM_DATA_SYSEX_LEN;
        message.copy_from_slice(&bank[offset..offset + factory_banks::PROGRAM_DATA_SYSEX_LEN]);
        crc.update(&message);
    }
    let actual_crc = crc.finish();
    if actual_crc != factory_banks::BANK_CRC32 {
        defmt::error!(
            "factory bank CRC mismatch expected={:#x} actual={:#x}",
            factory_banks::BANK_CRC32,
            actual_crc
        );
        loop {
            cortex_m::asm::wfi();
        }
    }

    defmt::info!("factory bank flashed and verified CRC32={:#x}", actual_crc);
    loop {
        cortex_m::asm::wfi();
    }
}
