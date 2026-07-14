# embassy-daisy

An Embassy-based, `no_std` board support package for Electro-Smith Daisy
hardware.

The first release supports the Daisy Seed 1.1, built around the
STM32H750IBK6. Enable it explicitly:

```toml
embassy-daisy = { version = "0.1", features = ["seed_1_1"] }
```

The crate owns Daisy-specific clocking, connector pin names, codec setup, audio
transport, USB hardware resources, external SDRAM, and QSPI flash. USB classes,
memory partition policy, and application behavior remain in the consuming
firmware.

## Status

- Daisy Seed 1.1 (WM8731): initial audio support
- Daisy Seed, Seed 1.2, and Patch SM: reserved, not implemented
- 48 kHz, 32-frame stereo audio: supported
- 64 MiB external SDRAM: FMC initialization, address/data-line test, MPU/cache
  configuration, and bounds-checked monotonic `f32` allocation
- 8 MiB IS25LP064 QSPI: explicit read/program/erase access with the bootloader
  and maximum BOOT_SRAM image window protected from writes
- 96 kHz and 64-frame blocks: reserved, not implemented

## External-memory safety

`Board::take()` is the only source of FMC, QUADSPI, and their pins. SDRAM does
not expose mutable memory until FMC initialization and the physical line test
complete. Its allocator advances a single cursor and checks alignment and
bounds before creating a static slice, preventing overlapping mutable regions.

The MPU marks SDRAM as cacheable normal memory and the complete D2 DMA window
as non-cacheable, shareable memory before D-cache is enabled. Audio DMA buffers
must remain in the `.sram1_bss` section; moving them requires revisiting this
cache-coherency contract.
