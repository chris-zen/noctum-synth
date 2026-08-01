# Oscillator research foundation

The minimum shared foundation for candidate experiments lives behind the
`synth-core` feature `oscillator-research`. It does not alter normal production
feature defaults, patches, MIDI/SysEx, or firmware model selection.

List registered analysis models:

```bash
cargo run -p synth-tools --release --bin analog_osc_research -- --list
```

Render and compare a deterministic case:

```bash
cargo run -p synth-tools --release --bin analog_osc_research -- \
  --model polyblep-v1 \
  --reference-model baseline-v1 \
  --waveform pulse \
  --frequency 223.7 \
  --shape 0.37
```

Candidate-owned parameters use repeatable `--param id=value` arguments. Their
stable IDs and complete effective values are recorded in the artifact; they do
not enter production synth parameters.

The runner writes an unnormalized mono IEEE-float WAV under
`target/analog-osc/renders/<model>/` and a versioned JSON record under
`target/analog-osc/metrics/<model>/`. Generated artifacts remain ignored by
Git. Use `--release` for meaningful timing; debug timing is retained but marked
as such in the artifact.

The core runner accepts a caller-owned output slice and performs no allocation.
Models receive semantic waveform/frequency/shape/reset/sync controls rather
than access to production voice internals. A model appearing in the analysis
registry is not automatically playable: only models with a separately tested
live adapter share an `ExperimentalOscillatorModel` identity and appear in the
single Params-view dropdown.

The existing Osc Design tab will evolve into Oscillator Lab. It will consume
the same immutable descriptors and case semantics while owning separate model
instances and experimental parameters. It will not send live engine-control
messages.
