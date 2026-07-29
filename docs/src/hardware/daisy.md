# Daisy Seed

Build, flash, and debug guide for the Daisy Seed 1.1 (STM32H750IBK6, Cortex-M7
at 480 MHz). The firmware crate is `noctum-micro` under `hardware/daisy/`.

## Prerequisites

- Rust 1.93 or newer with the `thumbv7em-none-eabihf` target:
  ```sh
  rustup target add thumbv7em-none-eabihf
  ```
- `arm-none-eabi-objcopy` (arm-none-eabi-binutils) for creating raw binaries
- A Daisy Seed 1.1 with the official bootloader installed

Optional:
- [probe-rs](https://probe.rs/) for flashing and RTT logging without DFU
- `dfu-util` for uploading via the Daisy bootloader

## Building

```sh
cd hardware/daisy
cargo build --release -p noctum-micro
```

Create the raw binary for DFU upload:

```sh
arm-none-eabi-objcopy -O binary \
  target/thumbv7em-none-eabihf/release/noctum-micro \
  noctum-micro.bin
```

## Feature flags

Features are additive. The default set is `fast-math`, `wide-4`,
`downsampling`, and `filter-gain-limited` (Micro 4). Use
`--no-default-features` when selecting a different model:

| Model | Features |
| --- | --- |
| Micro 4 | `fast-math,wide-4,downsampling,filter-gain-limited` |
| Micro 1 | `fast-math,wide-1,filter-huovilainen` |

| Feature | Effect |
| --- | --- |
| `fast-math` | Uses scalar `micromath` operations for the Cortex-M7 (default) |
| `wide-4` | Processes four voices per SIMD-style block (default) |
| `wide-1` | Processes one voice per scalar block; use instead of `wide-4` |
| `downsampling` | Runs DSP at 24 kHz and reconstructs 48 kHz output (default for four voices) |
| `filter-gain-limited` | Compiles Gain-Limited TPT (Micro 4; default) |
| `filter-huovilainen` | Compiles Huovilainen Ladder (Micro 1) |
| `oscillator-polyblep` | Uses PolyBLEP anti-aliasing instead of the default BLEP |
| `diagnostics` | Enables RTT logging, MIDI debug output, and overrun warnings |
| `audio-profiling` | Enables DWT cycle-count profiling per DSP stage |
| `usb-audio-test-tone` | Substitutes USB audio with a 1 kHz reference tone (DAC unaffected) |
| `usb-audio-raw-test` | Exposes raw USB interface without UAC1 class claim (diagnostic only) |

Exactly one `filter-*` feature must be enabled (or the build fails). Model
defaults live in `firmware/src/model.rs` and are driven by these features.
Makefile targets `run-micro-4` / `run-micro-1` (and the matching
`bench-*-micro-*` targets) pass the feature sets above.

Examples:

```sh
cargo build --release -p noctum-micro --features diagnostics
cargo build --release -p noctum-micro --no-default-features \
  --features fast-math,wide-4,downsampling,filter-gain-limited,diagnostics
```

## Flashing

### Via Daisy bootloader (DFU)

Put the Daisy in bootloader mode (press BOOT + RESET, release RESET, release
BOOT) and upload:

```sh
dfu-util -a 0 -s 0x90040000:leave \
  -D noctum-micro.bin -d ,0483:df11
```

### Via probe-rs

With a debug probe attached:

```sh
cd hardware/daisy
cargo run --release -p noctum-micro
```

The `.cargo/config.toml` sets the runner to `probe-rs run --chip STM32H750IB`.

## Diagnostics

Build with the `diagnostics` feature to enable RTT logging:

```sh
DEFMT_LOG=debug cargo run --release -p noctum-micro \
  --features diagnostics
```

The firmware logs parameter changes (`PARAM:`), queue overflows, and overrun
events. `PERF` warnings appear when the audio task reaches 90% of the
320,000-cycle deadline in any 1,500-block window.

Production builds omit diagnostics.

## Profiling

The `audio-profiling` feature measures Cortex-M7 DWT cycle counts per DSP stage
(envelopes/modulation, oscillators, filter, amplifier/pan, effects, master
output). The oscillator total is further split into control updates, waveform
generation, and sub/noise/mix cycles.

```sh
cargo run --release -p noctum-micro --features audio-profiling
```

### Standalone DSP benchmark

Runs a fixed set of test cases without SAI, DMA, USB, or executor overhead:

```sh
cargo run --release -p noctum-micro \
  --features audio-profiling --bin bench-dsp
```

### Factory-preset benchmark

Evaluates all 512 Layer A programs from the Prophet Rev2 v1.0 factory bank.
Each program receives a four-note chord (C4, E4, G4, C5) at full velocity and
measures block time across attack, steady-state, and parameter-change scenarios.

The bank must already live in QSPI at offset `0x006DA000` (end of the 8 MiB
flash, clear of MIDI program storage). Flash it once via probe-rs (ST-Link),
then run the benchmark:

```sh
cd hardware/daisy
make factory-banks-flash
make bench-factory-banks-micro-4
```

`make factory-banks-flash` compresses the Rev2 factory `.syx` via
`tools-host` (`factory-banks-compress`) into `target/factory-bank.zlib`, then
runs the on-target `factory-banks-flash` binary from `tools-micro`. That image
decompresses into SDRAM, programs the end-of-flash QSPI region, and verifies
the expected CRC32. It does not erase MIDI program catalogs or slots.

The benchmark reports average, p95, p99, maximum cycles, headroom, and
over-budget block counts per program.

## Audio reliability

SAI receive overruns and transmit underruns are recoverable. Both DMA rings
resynchronize automatically, and the firmware fades to silence before resuming.
MIDI control processing is bounded per audio block so a burst cannot
indefinitely delay DMA servicing.

The user LED shows overruns as three full-brightness flashes (25 ms on/off).

## USB audio capture

When a host opens the USB audio capture interface, post-effects stereo output
is mirrored as packed 24-bit PCM at 48 kHz. SAI remains the synth clock and
never waits for USB. A lock-free frame ring bridges 32-frame render blocks to
1 ms USB packets with 47–49 frames per packet to absorb clock drift.

## USB MIDI identity

Firmware uses temporary VID/PID `0xC0DE:0xCAFE` for local development. Before
public release:
1. Request a project PID under pid.codes community VID `0x1209`.
2. Update `DEVELOPMENT_VID` and `DEVELOPMENT_PID` in the USB MIDI component.
3. Verify the release identity on every supported host OS.

## Program storage

8 banks × 128 complete two-layer programs (1024-byte records) are stored in
QSPI. Catalog A is at `0x000C0000`, records occupy
`0x000C1000..0x001C1000`, and Catalog B is at `0x001C1000`. The
bootloader/application reservation below this region and the high-address raw
factory bank are never erased. On first boot or when neither versioned catalog
is valid, firmware formats only this region. Empty slots load the default
complete patch. Until layered rendering lands, firmware renders Layer A from
the loaded record. One thread-mode task owns QSPI; neither flash erase nor page
programming runs in the audio path.

Bank Select (CC0/CC32, values 0–7) followed by Program Change loads a program.
Rev2 and Prophet '08 Program Data SysEx messages save to their addressed slot.

## Tests

The Daisy workspace defaults to the Cortex-M target. Supply the host target for
unit tests:

```sh
cargo test -p noctum-micro --lib --target aarch64-apple-darwin
```

Board-level tests for `embassy-daisy`:

```sh
cargo test -p embassy-daisy --features seed_1_1
```
