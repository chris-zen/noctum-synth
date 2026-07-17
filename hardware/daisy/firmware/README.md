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
reassembles SysEx into a fixed buffer sized for a complete Rev2 Program Edit
Buffer dump without allocation. It uses
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
up to 256 commands without blocking, and drains them immediately before
rendering each audio block. Effects and table BLEP are always enabled.
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
