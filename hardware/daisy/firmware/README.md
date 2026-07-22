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

MIDI clock defaults to Off (patch BPM). NRPN 4099 selects Off, Slave, or Slave
No S/S; Master and Slave Thru fall back to Off. Continue is ignored. The mode
is runtime-only. System Real-Time bytes may arrive during SysEx without
interrupting assembly.

The probe target is `STM32H750IB`, matching the STM32H750IBK6 fitted to Daisy
Seed boards. The similarly named STM32H750VB does not expose all Seed pins.

The BSP enables the Cortex-M7 instruction and data caches after configuring
SDRAM as cacheable normal memory and D2 audio DMA SRAM as non-cacheable,
shareable memory.

## USB MIDI development identity

The firmware enumerates as a composite bidirectional USB-MIDI device and UAC1
audio source. The MIDI decoder honors each packet's code-index-number length
and reassembles SysEx into a fixed buffer sized for the larger Rev2
stored-program envelope without allocation.
It uses
temporary VID/PID
`0xC0DE:0xCAFE` for local development only. Do not distribute firmware or
hardware using this identity.

Before the first public binary or hardware release:

1. Request a project PID under the pid.codes community VID `0x1209`.
2. Replace `DEVELOPMENT_VID` and `DEVELOPMENT_PID` in the MIDI component.
3. Verify the release identity on every supported host operating system.

Allocation requirements: <https://pid.codes/howto/>.

## USB audio

The analog DAC is always active. When a host opens the USB audio capture
interface, the firmware additionally mirrors the same post-effects,
post-master-volume signal as stereo packed 24-bit PCM at 48 kHz. Merely
attaching a cable or opening USB MIDI does not start audio streaming.

SAI remains the synth clock and never waits for USB. A lock-free frame ring
bridges the 32-frame render blocks to 1 ms USB packets; the USB packetizer uses
47, 48, or 49 frames to absorb clock drift. It primes with silence before a
5 ms fade-in. USB class and packet work runs on a P2 software-interrupt
executor, below P1 audio rendering but above thread-mode diagnostics. USB
disconnects, underruns, resets, and suspend/resume transitions flush and
re-prime only the USB copy and cannot mute or restart the DAC.

While the capture interface is active, the firmware enables the Synopsys OTG
end-of-periodic-frame recovery path. This prevents a missed isochronous frame
from leaving the endpoint writer blocked indefinitely; it is disabled again
when streaming stops.

The buffering and packed-PCM logic has host tests. Because the Daisy workspace
defaults to the Cortex-M target, supply the development machine's host target
explicitly, for example on Apple Silicon:

```sh
cargo test -p analog-synth-daisy-firmware --lib \
  --target aarch64-apple-darwin
```

`tools/usb-audio-smoke.sh` opens the CoreAudio input, sends three USB-MIDI
notes, records a WAV, prints signal statistics, and fails if the capture is
silent. For transport-only diagnosis, build with `usb-audio-test-tone`; that
feature substitutes a USB-only 1 kHz reference signal without changing DAC
output and is not enabled in normal firmware builds.

macOS can open a CoreAudio input while returning only zero-filled samples when
microphone privacy is denied. The smoke test probes the default input after an
all-zero capture and reports that condition separately.

For a privacy-independent raw USB transport check, build the diagnostic
firmware so AppleUSBAudio does not claim its streaming interface, then run the
libusb harness:

```sh
cargo build --release -p analog-synth-daisy-firmware \
  --features diagnostics,usb-audio-test-tone,usb-audio-raw-test
tools/usb-audio-raw-smoke.sh
```

`usb-audio-raw-test` is diagnostic-only: normal firmware omits it and binds to
the operating system USB Audio Class driver.

The MIDI component terminates at a synchronous typed-message handler. The
firmware maps performance messages into `synth_core::ControlMessage`, enqueues
up to 256 commands without blocking, and drains them immediately before
rendering each audio block. Program Edit Buffer dumps instead cross the
real-time boundary as one `Patch` through a dedicated two-slot queue; the audio
task passes that patch directly to `SynthEngineWithMemory::apply_patch`.
Effects and table BLEP are always enabled.
`synth-core` only receives a caller-provided mutable slice and remains
unaware that the firmware backs that storage with external SDRAM.

## Diagnostics

Build and run with the `diagnostics` feature when checking desktop-to-Daisy
parameter control, audio health, or USB MIDI bring-up:

```sh
DEFMT_LOG=debug cargo run --release -p analog-synth-daisy-firmware \
  --features diagnostics
```

The firmware assembles CC 99, 98, 6, and 38 for diagnostics and emits one
`NRPN: ch=… id=… raw=…` line at debug level for the completed transport value.
`PARAM: Amp Attack v=…` follows at info level after the parameter has been
decoded, queued, and drained by the audio task. Modulation logs include the
route, field, and named source or destination. If `USB MIDI connected` appears
but `NRPN` does not, check the desktop output selection or whether the sequence
is complete. `NRPN` without `PARAM` points to an unsupported parameter or a
dropped control. Queue overflow is reported separately. The startup log must
contain `diagnostics enabled`; if it does not, the running image was built
without the feature or an older image is still flashed.

Production firmware omits the diagnostics reporter by default. XRUNs are still
visible on the user LED; enable `diagnostics` when RTT logging is attached.

## Audio profiling

Build and run with the firmware-only profiling feature to measure the Cortex-M7
DWT cycle count for each DSP stage:

```sh
cargo run --release -p analog-synth-daisy-firmware --features audio-profiling
```

The profiler reports total block time and separate envelope/modulation,
oscillator, filter, amplifier/pan, effects, and master-output contributions.
The oscillator total is further split into control/modulation updates,
waveform-specific generation, and sub/noise/mix cycles. The waveform bucket is
measured inside each enabled `AnalogOscillator` around base-wave generation,
shape morphing, and waveform-specific wrap handling. It accumulates
measurements in RAM and queues one snapshot every 1,500 blocks. The thread-mode
diagnostics task performs the eventual RTT writes.

The standalone DSP benchmark includes paired `four-note-osc1-saw` and
`four-note-osc1-triangle` cases. Both explicitly disable OSC2, sub, noise, hard
sync, slop, and shape morphing so waveform type is the only changed parameter.
General benchmark cases use the production Gain-Limited TPT filter with
oversampling disabled; filter-comparison cases override that model explicitly.

```sh
cargo run --release -p analog-synth-daisy-firmware \
  --features audio-profiling --bin dsp-benchmark
```

### Factory-preset benchmark

`factory-preset-benchmark` evaluates all 512 Layer A programs from the Prophet
Rev2 v1.0 factory bank without measuring MIDI transport, QSPI reads, patch
decoding, logging, or engine initialization. Its timed callback-equivalent path
does include production queue polling/control draining, engine rendering,
limiting, patch-transition gain, and output copying. The bank remains in its
original stored-program SysEx representation and is packaged after the maximum
BOOT_SRAM application storage window, so its 1.2 MiB payload cannot change
executable code placement or consume the 480 KiB execution region.

Build the combined bootloader image from the repository's factory bank:

```sh
firmware/build-factory-preset-image.sh
```

The resulting image contains the SRAM executable at QSPI `0x90040000`, padding
through the complete application reservation, and the factory bank beginning at
QSPI `0x900c0000`. Upload it through the normal Daisy bootloader flow:

```sh
dfu-util -a 0 -s 0x90040000:leave \
  -D target/factory-preset-benchmark-with-bank.bin \
  -d ,0483:df11
```

The benchmark expects exactly 512 concatenated 2,346-byte messages and verifies
the complete payload against CRC32 `3df33c23` before measuring anything. It
reuses one SDRAM allocation but reconstructs the engine for every program, so
voices, envelopes, limiter state, and effect history cannot leak between
presets. Each program receives notes C4, E4, G4, and C5 at full velocity. The
benchmark measures the complete seven-block patch transition, a 128-block
four-note attack, 512 warmed steady-state blocks, and 128 blocks that each drain
four expensive parameter updates. Programs at or above the 272,000-cycle target
receive an additional 256-block profiling pass.

Run it with a debug probe attached:

```sh
cargo run --release -p analog-synth-daisy-firmware \
  --features audio-profiling --bin factory-preset-benchmark
```

Every `FACTORY raw` record contains bank/program/scenario, average, p95, p99,
maximum, target/deadline headroom, and over-budget block count. Near-budget
programs also emit `FACTORY profile` attribution. The final summary reports
failure counts, waveform/filter/effect/route groupings, and the sixteen slowest
cases. Bank and program numbers are one-based and match the factory-preset list.

On Daisy, the selected uniform quality tier runs the engine at 24 kHz and
reconstructs its stereo output at the external 48 kHz rate with the fixed
three-tap half-band interpolator owned by `render_rate`. Desktop builds retain
the unchanged full-rate path. This preserves four voices, all modulation routes,
and patch/MIDI/SysEx compatibility while intentionally reducing Daisy bandwidth.

Triangle PolyBLAMP is backend-specialized: Daisy's scalar `embedded-math`
implementation skips inactive correction windows per lane, while SIMD hosts
retain branchless evaluation with one reciprocal shared by both corners. Across
three STM32H750 runs, `four-note-osc1-triangle` measured at most 286,969 raw
cycles (89.6% of budget, zero overruns), down from the 318,941-cycle baseline.

Normal firmware with `diagnostics` enabled keeps a lightweight DWT window around
all audio-task work. It queues a `PERF` warning only when the maximum in a
1,500-block window reaches 90% of the 320,000-cycle deadline. Run without the
more expensive per-stage profiling hooks for day-to-day development:

```sh
cargo run --release -p analog-synth-daisy-firmware --features diagnostics
```

Ship builds omit diagnostics entirely:

```sh
cargo run --release -p analog-synth-daisy-firmware
```

## Audio overrun recovery

SAI receive overruns and transmit underruns are treated as recoverable audio
dropouts. Both DMA rings resynchronize automatically, and the firmware queues a
short fade from the last transmitted sample to silence before resuming
synthesis. MIDI control processing is bounded per audio block so a burst cannot
indefinitely delay DMA servicing.

Audio runs on a P1 interrupt executor and preempts the thread-mode USB MIDI task
and, when enabled, the diagnostics reporter. Real-time code only updates
atomics and attempts bounded queue writes; it never formats or emits a log. With
the `diagnostics` feature, a typed queue carries `PARAM`, `XRUN`, `PERF`, MIDI,
and queue-health events to a reporter which:

- logs RX-overrun and TX-underrun totals plus changes since the previous report;
- coalesces sustained overruns to at most one report per second.

A separate non-blocking indicator drives the Daisy user LED from its own async
task. The LED uses `PwmUserLed` on TIM3 channel 2 at 1 kHz and starts off.
Every valid MIDI message produces a 60 ms pulse at 25% brightness; activity
arriving during a pulse is coalesced rather than replayed as a delayed animation.
An audio stream gap takes priority and produces three full-brightness flashes,
each 25 ms on and 25 ms off. An atomic status word preserves that warning even
when a MIDI burst would otherwise crowd out lower-priority activity.

RTT uses non-blocking trim mode when diagnostics are enabled. This is required
because blocking `defmt-rtt` holds a global critical section while waiting for
its host, which would mask the audio executor interrupt regardless of task
priority. Diagnostic events and RTT bytes may therefore be dropped under
sustained logging pressure; dropped queue events are counted and reported when
capacity becomes available.

The monotonic counts are also available through
`audio::overruns_count()` and `audio::underruns_count()` for
a future MIDI SysEx or CC status query.

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
