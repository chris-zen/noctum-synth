# Analog Synth Daisy firmware

Hardware proof firmware for a Daisy Seed 1.1 using `embassy-daisy`.

The default build uses the official Daisy bootloader's `BOOT_SRAM` model. The
application is stored in QSPI at `0x90040000`, copied into the 480 KiB AXI SRAM
application window, and executed from `0x24000000`. Effects memory is allocated
from external SDRAM and audio DMA remains in non-cacheable D2 SRAM.

Build and create the raw bootloader image:

```sh
cargo build --release -p analog-synth-daisy-firmware
arm-none-eabi-objcopy -O binary \
  target/thumbv7em-none-eabihf/release/analog-synth-daisy-firmware \
  analog-synth-daisy-firmware.bin
```

With the official bootloader in its USB grace period, upload it using:

```sh
dfu-util -a 0 -s 0x90040000:leave \
  -D analog-synth-daisy-firmware.bin -d ,0483:df11
```

The firmware runs one four-lane SIMD `SynthEngine` voice pack at 48 kHz,
accepts USB MIDI on all channels, and applies queued controls before each
32-frame audio block. Flashing and logging use `probe-rs` and RTT.

The probe target is `STM32H750IB`, matching the STM32H750IBK6 fitted to Daisy
Seed boards. The similarly named STM32H750VB does not expose all Seed pins.

The BSP enables the Cortex-M7 instruction and data caches after configuring
SDRAM as cacheable normal memory and D2 audio DMA SRAM as non-cacheable,
shareable memory.

## USB MIDI development identity

The firmware currently enumerates as a bidirectional USB-MIDI device and
decodes USB event packets with the no-std `wmidi` crate. The decoder honors
each packet's code-index-number length and
reassembles SysEx into a fixed 256-byte buffer without allocation. It uses
temporary VID/PID
`0xC0DE:0xCAFE` for local development only. Do not distribute firmware or
hardware using this identity.

Before the first public binary or hardware release:

1. Request a project PID under the pid.codes community VID `0x1209`.
2. Replace `DEVELOPMENT_VID` and `DEVELOPMENT_PID` in the MIDI component.
3. Verify the release identity on every supported host operating system.

Allocation requirements: <https://pid.codes/howto/>.

The MIDI component terminates at a synchronous typed-message handler. The
firmware maps performance messages into `synth_core::ControlMessage`, enqueues
up to 32 commands without blocking, and drains them immediately before
rendering each audio block. Effects and table BLEP are always enabled.
`synth-core` only receives a caller-provided mutable slice and remains
unaware that the firmware backs that storage with external SDRAM.

## Audio profiling

Build and run with the firmware-only profiling feature to measure the Cortex-M7
DWT cycle count for each DSP stage:

```sh
cargo run --release -p analog-synth-daisy-firmware --features audio-profiling
```

The profiler reports total block time and separate envelope/modulation,
oscillator, filter, amplifier/pan, effects, and master-output contributions.
It accumulates measurements in RAM and logs one snapshot every 1,500 blocks,
plus a final snapshot if the SAI transport reports an overrun.

The normal production command leaves `audio-profiling` disabled. Its DWT setup,
stage hooks, counters, report strings, and RAM storage are removed at compile
time:

```sh
cargo run --release -p analog-synth-daisy-firmware
```

For repeatable on-target DSP measurements without SAI, DMA, USB, or executor
deadlines, run the standalone benchmark binary:

```sh
cargo run --release --bin dsp-benchmark --features audio-profiling
```

It uses the production clock/cache/MPU setup and the same SDRAM-backed effects
memory. After a warm-up pass it measures idle, one-note and four-note defaults,
active and self-oscillating filter configurations, modulation-heavy synthesis,
each effect type, and a representative worst case. Output includes average and
maximum cycles per 32-frame block, deadline utilization in per-mille, deadline
overruns, and the per-stage cycle breakdown.
