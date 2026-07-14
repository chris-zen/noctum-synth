//! Board memory regions not covered by cortex-m-rt startup.

unsafe extern "C" {
    static __sram1_bss_start__: u8;
    static __sram1_bss_end__: u8;
}

pub(crate) fn zero_sram1_bss() {
    let start = core::ptr::addr_of!(__sram1_bss_start__) as usize;
    let end = core::ptr::addr_of!(__sram1_bss_end__) as usize;
    if end > start {
        unsafe {
            core::ptr::write_bytes(start as *mut u8, 0, end - start);
        }
    }
}
