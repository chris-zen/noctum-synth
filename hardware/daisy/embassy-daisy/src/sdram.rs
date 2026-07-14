//! External SDRAM initialization and bounded storage allocation.

use core::mem::{align_of, size_of};
use cortex_m::peripheral::{CPUID, MPU, SCB};
use embassy_stm32::{Peri, fmc::Fmc, peripherals};
use stm32_fmc::devices::as4c16m32msa_6::As4c16m32msa;

pub const BASE_ADDRESS: usize = 0xC000_0000;
pub const SIZE_BYTES: usize = 64 * 1024 * 1024;

pub struct SdramResources {
    pub(crate) fmc: Peri<'static, peripherals::FMC>,
    pub(crate) a0: Peri<'static, peripherals::PF0>,
    pub(crate) a1: Peri<'static, peripherals::PF1>,
    pub(crate) a2: Peri<'static, peripherals::PF2>,
    pub(crate) a3: Peri<'static, peripherals::PF3>,
    pub(crate) a4: Peri<'static, peripherals::PF4>,
    pub(crate) a5: Peri<'static, peripherals::PF5>,
    pub(crate) a6: Peri<'static, peripherals::PF12>,
    pub(crate) a7: Peri<'static, peripherals::PF13>,
    pub(crate) a8: Peri<'static, peripherals::PF14>,
    pub(crate) a9: Peri<'static, peripherals::PF15>,
    pub(crate) a10: Peri<'static, peripherals::PG0>,
    pub(crate) a11: Peri<'static, peripherals::PG1>,
    pub(crate) a12: Peri<'static, peripherals::PG2>,
    pub(crate) ba0: Peri<'static, peripherals::PG4>,
    pub(crate) ba1: Peri<'static, peripherals::PG5>,
    pub(crate) d0: Peri<'static, peripherals::PD14>,
    pub(crate) d1: Peri<'static, peripherals::PD15>,
    pub(crate) d2: Peri<'static, peripherals::PD0>,
    pub(crate) d3: Peri<'static, peripherals::PD1>,
    pub(crate) d4: Peri<'static, peripherals::PE7>,
    pub(crate) d5: Peri<'static, peripherals::PE8>,
    pub(crate) d6: Peri<'static, peripherals::PE9>,
    pub(crate) d7: Peri<'static, peripherals::PE10>,
    pub(crate) d8: Peri<'static, peripherals::PE11>,
    pub(crate) d9: Peri<'static, peripherals::PE12>,
    pub(crate) d10: Peri<'static, peripherals::PE13>,
    pub(crate) d11: Peri<'static, peripherals::PE14>,
    pub(crate) d12: Peri<'static, peripherals::PE15>,
    pub(crate) d13: Peri<'static, peripherals::PD8>,
    pub(crate) d14: Peri<'static, peripherals::PD9>,
    pub(crate) d15: Peri<'static, peripherals::PD10>,
    pub(crate) d16: Peri<'static, peripherals::PH8>,
    pub(crate) d17: Peri<'static, peripherals::PH9>,
    pub(crate) d18: Peri<'static, peripherals::PH10>,
    pub(crate) d19: Peri<'static, peripherals::PH11>,
    pub(crate) d20: Peri<'static, peripherals::PH12>,
    pub(crate) d21: Peri<'static, peripherals::PH13>,
    pub(crate) d22: Peri<'static, peripherals::PH14>,
    pub(crate) d23: Peri<'static, peripherals::PH15>,
    pub(crate) d24: Peri<'static, peripherals::PI0>,
    pub(crate) d25: Peri<'static, peripherals::PI1>,
    pub(crate) d26: Peri<'static, peripherals::PI2>,
    pub(crate) d27: Peri<'static, peripherals::PI3>,
    pub(crate) d28: Peri<'static, peripherals::PI6>,
    pub(crate) d29: Peri<'static, peripherals::PI7>,
    pub(crate) d30: Peri<'static, peripherals::PI9>,
    pub(crate) d31: Peri<'static, peripherals::PI10>,
    pub(crate) nbl0: Peri<'static, peripherals::PE0>,
    pub(crate) nbl1: Peri<'static, peripherals::PE1>,
    pub(crate) nbl2: Peri<'static, peripherals::PI4>,
    pub(crate) nbl3: Peri<'static, peripherals::PI5>,
    pub(crate) sdcke: Peri<'static, peripherals::PH2>,
    pub(crate) sdclk: Peri<'static, peripherals::PG8>,
    pub(crate) sdncas: Peri<'static, peripherals::PG15>,
    pub(crate) sdne: Peri<'static, peripherals::PH3>,
    pub(crate) sdnras: Peri<'static, peripherals::PF11>,
    pub(crate) sdnwe: Peri<'static, peripherals::PH5>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllocationError {
    ZeroLength,
    OutOfMemory,
    SizeOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryTestError {
    pub word_offset: usize,
    pub expected: u32,
    pub actual: u32,
}

/// Initialized external SDRAM with exclusive monotonic allocation authority.
pub struct Sdram {
    next: usize,
}

impl SdramResources {
    pub fn init(
        self,
        mpu: &mut MPU,
        scb: &mut SCB,
        cpuid: &mut CPUID,
    ) -> Result<Sdram, MemoryTestError> {
        let mut device = Fmc::sdram_a13bits_d32bits_4banks_bank1(
            self.fmc,
            self.a0,
            self.a1,
            self.a2,
            self.a3,
            self.a4,
            self.a5,
            self.a6,
            self.a7,
            self.a8,
            self.a9,
            self.a10,
            self.a11,
            self.a12,
            self.ba0,
            self.ba1,
            self.d0,
            self.d1,
            self.d2,
            self.d3,
            self.d4,
            self.d5,
            self.d6,
            self.d7,
            self.d8,
            self.d9,
            self.d10,
            self.d11,
            self.d12,
            self.d13,
            self.d14,
            self.d15,
            self.d16,
            self.d17,
            self.d18,
            self.d19,
            self.d20,
            self.d21,
            self.d22,
            self.d23,
            self.d24,
            self.d25,
            self.d26,
            self.d27,
            self.d28,
            self.d29,
            self.d30,
            self.d31,
            self.nbl0,
            self.nbl1,
            self.nbl2,
            self.nbl3,
            self.sdcke,
            self.sdclk,
            self.sdncas,
            self.sdne,
            self.sdnras,
            self.sdnwe,
            As4c16m32msa {},
        );
        let base = device.init(&mut embassy_time::Delay);
        debug_assert_eq!(base as usize, BASE_ADDRESS);
        test_data_and_address_lines(base)?;
        configure_memory_attributes(mpu, scb, cpuid);
        Ok(Sdram { next: 0 })
    }
}

fn test_data_and_address_lines(base: *mut u32) -> Result<(), MemoryTestError> {
    const WORDS: usize = SIZE_BYTES / size_of::<u32>();
    // Test every data bit at the first word before caches are enabled.
    for bit in 0..32 {
        let expected = 1u32 << bit;
        unsafe { base.write_volatile(expected) };
        let actual = unsafe { base.read_volatile() };
        if actual != expected {
            return Err(MemoryTestError {
                word_offset: 0,
                expected,
                actual,
            });
        }
    }

    // Distinct values at power-of-two offsets expose aliased address lines.
    unsafe { base.write_volatile(0xA5A5_5A5A) };
    let mut offset = 1usize;
    while offset < WORDS {
        let expected = 0x5A5A_A5A5 ^ offset as u32;
        unsafe { base.add(offset).write_volatile(expected) };
        offset <<= 1;
    }
    offset = 1;
    while offset < WORDS {
        let expected = 0x5A5A_A5A5 ^ offset as u32;
        let actual = unsafe { base.add(offset).read_volatile() };
        if actual != expected {
            return Err(MemoryTestError {
                word_offset: offset,
                expected,
                actual,
            });
        }
        offset <<= 1;
    }
    let actual = unsafe { base.read_volatile() };
    if actual != 0xA5A5_5A5A {
        return Err(MemoryTestError {
            word_offset: 0,
            expected: 0xA5A5_5A5A,
            actual,
        });
    }
    Ok(())
}

impl Sdram {
    pub const fn capacity(&self) -> usize {
        SIZE_BYTES
    }
    pub const fn used(&self) -> usize {
        self.next
    }

    /// Allocate zeroed SDRAM for DSP sample memory.
    ///
    /// Every successful call advances the only allocator cursor, so returned
    /// mutable slices cannot overlap. The board singleton ensures only one
    /// allocator exists for the physical SDRAM.
    pub fn allocate_f32(&mut self, samples: usize) -> Result<&'static mut [f32], AllocationError> {
        if samples == 0 {
            return Err(AllocationError::ZeroLength);
        }
        let aligned =
            align_up(self.next, align_of::<f32>()).ok_or(AllocationError::SizeOverflow)?;
        let bytes = samples
            .checked_mul(size_of::<f32>())
            .ok_or(AllocationError::SizeOverflow)?;
        let end = aligned
            .checked_add(bytes)
            .ok_or(AllocationError::SizeOverflow)?;
        if end > SIZE_BYTES {
            return Err(AllocationError::OutOfMemory);
        }
        self.next = end;
        let ptr = (BASE_ADDRESS + aligned) as *mut f32;
        // SAFETY: FMC is initialized; bounds and alignment are checked; the
        // monotonic cursor prevents overlap; SDRAM has static physical life.
        let slice = unsafe { core::slice::from_raw_parts_mut(ptr, samples) };
        slice.fill(0.0);
        Ok(slice)
    }
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    value
        .checked_add(alignment - 1)
        .map(|v| v & !(alignment - 1))
}

fn configure_memory_attributes(mpu: &mut MPU, scb: &mut SCB, cpuid: &mut CPUID) {
    const ENABLE: u32 = 1;
    const PRIVDEFENA: u32 = 1 << 2;
    const FULL_ACCESS: u32 = 0b011 << 24;
    const XN: u32 = 1 << 28;
    const CACHEABLE: u32 = 1 << 17;
    const BUFFERABLE: u32 = 1 << 16;
    const SHAREABLE: u32 = 1 << 18;
    const NORMAL_NON_CACHEABLE: u32 = 1 << 19; // TEX=001, C=0, B=0

    cortex_m::asm::dmb();
    unsafe {
        mpu.ctrl.write(0);
        // Region 0: 64 MiB SDRAM, normal cacheable write-back memory.
        mpu.rnr.write(0);
        mpu.rbar.write(BASE_ADDRESS as u32);
        mpu.rasr
            .write(XN | FULL_ACCESS | CACHEABLE | BUFFERABLE | (25 << 1) | ENABLE);
        // Region 1: complete D2 window, normal non-cacheable DMA memory.
        mpu.rnr.write(1);
        mpu.rbar.write(0x3000_0000);
        mpu.rasr
            .write(XN | FULL_ACCESS | SHAREABLE | NORMAL_NON_CACHEABLE | (18 << 1) | ENABLE);
        mpu.ctrl.write(PRIVDEFENA | ENABLE);
        scb.shcsr.modify(|v| v | (1 << 16));
    }
    cortex_m::asm::dsb();
    cortex_m::asm::isb();
    scb.enable_dcache(cpuid);
}
