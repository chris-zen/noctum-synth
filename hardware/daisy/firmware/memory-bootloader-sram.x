/* Official Daisy bootloader BOOT_SRAM-compatible execution layout. */
MEMORY
{
    /* The bootloader reserves the final 32 KiB of the 512 KiB AXI SRAM. */
    FLASH  (RX)  : ORIGIN = 0x24000000, LENGTH = 480K
    RAM    (RWX) : ORIGIN = 0x20000000, LENGTH = 128K
    RAM_D2 (RWX) : ORIGIN = 0x30000000, LENGTH = 288K
}

SECTIONS
{
    .sram1_bss (NOLOAD) : ALIGN(32)
    {
        . = ALIGN(32);
        __sram1_bss_start__ = .;
        *(.sram1_bss)
        *(.sram1_bss.*)
        . = ALIGN(32);
        __sram1_bss_end__ = .;
    } > RAM_D2
}
