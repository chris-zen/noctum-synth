# Common Evaluation and Hardware Selection

## Objective

Provide one comparison protocol for every oscillator experiment. Keep target
accuracy, alias suppression, modulation behavior, subjective preference,
desktop cost, and hardware feasibility as separate axes. Select candidates from
a Pareto frontier rather than one opaque score.

## Implementation status

Metric revision 1 of the minimum evaluator is implemented:

- Common signal measurements: DC, RMS, peak, crest factor, midpoint duty, and
  crossing-interpolated measured fundamental.
- Pair comparison: normalized RMS error, maximum absolute error, and centered
  correlation.
- Hann-window spectral residual reporting with legal-harmonic guards,
  fundamental level, and worst residual component. It is deliberately labelled
  residual rather than blindly calling measured non-harmonic energy aliasing.
- Self-describing JSON records model/case revisions, Git state, host/build,
  seed, unnormalized levels, state/assets/latency, timings, hashes, metrics,
  comparison model, and generated source-only float WAV.
- Analytic tests cover known sine level/frequency, known delay error, clean tone
  versus injected non-harmonic spur, deterministic repetition, and WAV format.
- Plan 04 adds a reproducible static log-pitch scheduler at 48/96 kHz. It
  compares guarded non-harmonic residuals against the baseline and records both
  relative regressions and an absolute warning floor. A dense target-model
  triangle sweep also checks interpolation continuity.
- Plan 04 also provides the first fixed-seed, RMS-matched named/ABX/target-match
  listening package with hashed manifests, a separate answer key, and a
  response template. Listener results are not yet available.

Automated full-matrix scheduling, target complex-harmonic metrics, randomized
listening-set/ABX support, dynamic saw/pulse/PWM scheduling, percentile timing,
and hardware benchmark aggregation remain pending. Metric revisions must remain
explicit as those are added.

Plan 07 now contributes the first compact dynamic extension without changing
metric revision 1. A versioned renderer schedules pitch/shape, audio-rate PWM,
fractional hard sync, and a combined event stream at 48/192 kHz. Filtered
high-rate disagreement is explicitly labelled an alias/implementation proxy.
The same tool records median/p95/p99/maximum 64-frame desktop timings for one
voice, four voices, and a combined slop/detune/glide/PWM/sync profile. This does
not replace the still-pending full matrix or hardware aggregation.

## Candidate set

Always include:

- Current table-BLEP baseline.
- Current PolyBLEP comparison.
- Any retained wavetable prototype relevant to the case.
- Target recording when available.
- High-rate/offline reference for the method under test.
- Pass Through (Raw) for all playable full-voice comparisons, followed by the
  same representative existing filter settings for every candidate.

Candidate versions and target checksums are frozen before a formal comparison.

## Deterministic case matrix

### Static

- Waveforms: saw, triangle, pulse.
- Notes: E0-E6 reference grid plus MIDI 84, 96, 108 stress points.
- Pulse widths: 10, 25, 50, 75, 90 percent.
- Sample rates: 44.1, 48, 96, 192 kHz; 24 kHz for Daisy characterization.
- Shape/SawTri compatibility cases at 0, 0.25, 0.5, 0.75, 1.

### Dynamic

- Logarithmic pitch sweep.
- Slow and audio-rate PWM sweeps.
- Hard-sync ratios and master sweeps.
- Note reset and free-running phase.
- Abrupt pitch changes and smooth glide.
- Target/profile and model-parameter transitions where supported.

### Polyphonic/stochastic

- One and four lanes with matched and spread notes.
- Chords for beating/common drift.
- Fixed-seed and long-run statistical cases.
- Full synth path first through Pass Through, then through representative
  open/closed/resonant filter settings.

## Measurement preparation

- Discard documented model warm-up.
- Estimate actual fundamental; do not assume requested frequency.
- Preserve raw level measurements, then create level-matched copies for
  timbre/listening.
- Use integer-cycle or phase-aware windows where possible.
- Align latency and phase only for metrics that require it.
- Analyze enough samples to separate narrow harmonics from drift/sidebands.

## Objective metrics

### Pitch and level

- Frequency/cents error and long-run slope.
- DC, peak, RMS, fundamental level, crest factor.
- Algorithmic latency and startup transient.

### Deterministic target character

- Phase-aligned normalized time-domain error.
- Complex harmonic error: magnitude and phase.
- Log-spectral error over legal harmonic bins.
- Edge time, overshoot, settling, plateau droop, ramp/triangle curvature,
  duty, and slope asymmetry.
- Held-out-pitch and held-out-width error separate from fitted conditions.

### Aliasing and spurious output

- Integrated residual energy excluding legal harmonics and known target noise.
- Worst non-harmonic/folded component.
- Alias spectrogram during pitch/PWM/sync sweeps.
- Separate target-recording bandwidth/alias from algorithm-added components.

Do not call all non-harmonic target energy aliasing; measured drift and noise
require separate analysis.

### Dynamic behavior

- Click/transient energy during coefficient, mip, pitch, PW, and model changes.
- Pitch and amplitude continuity.
- PWM/sync sideband comparison.
- Maximum event/solver/transition work.

### Stochastic behavior

- Period, amplitude, duty, and descriptor distributions.
- Autocorrelation/power spectra.
- Common versus differential voice correlation.
- Deterministic reproducibility for a fixed seed.

## Listening protocol

Generate randomized, level-matched files:

- Oscillator single notes at low, middle, high pitch through Pass Through.
- Slow sweeps and PWM.
- Four-note chords and beating.
- The same events through representative filter/VCA settings.
- Short musical phrases where differences survive the subtractive chain.

Raw means bit-transparent at the filter boundary; VCA, pan, master output, and
the explicitly documented downstream settings still apply. Export the direct
offline oscillator render separately when a literal source-only file is needed.

Run:

- ABX detectability against baseline.
- Target-match choice when a reference exists.
- Preference choice without target.

Keep target match and preference separate. Record listener, playback chain,
trial order, normalization, and confidence. Do not promote a candidate only
because its unmatched output is louder/brighter.

## Performance measurements

### Desktop

- Construction/loading time.
- Nanoseconds or cycles per sample/voice and per block.
- One/four/maximum intended polyphony.
- Average, p95, p99, maximum, and parameter-change spikes.
- Mutable state, immutable asset, scratch, code, and peak working-set bytes.
- Scalar versus available SIMD behavior.
- Offline and real-time factors at 48, 96, 192 kHz.
- Full-voice cost with Pass Through and with each representative filter so
  oscillator cost and oscillator/filter interaction are both visible.

Desktop exploration may be offline. A desktop real-time label requires bounded
render cost and measured real-time margin at the intended polyphony.

### Daisy

Only evaluate candidates explicitly nominated for Daisy:

- Existing 24 kHz internal and 48 kHz codec configuration.
- Standalone DSP and factory-preset corpus benchmarks.
- Raw production-matched timing for acceptance; profiling for attribution.
- Existing reliability target and overrun criteria from
  plans/DAISY_SAMPLE_RATE_QUALITY_REPORT.md and
  plans/DAISY_FACTORY_PRESET_PERFORMANCE_PLAN.md.
- Flash/RAM placement and no blocking storage access.

Do not relax existing production gates to admit an experiment.

### Future powerful hardware

Record platform-independent work/state/assets now. Define concrete deadlines,
polyphony, memory, SIMD, and storage gates only after a platform is selected.
Do not label a method embedded-ready solely because it runs on desktop.

## Decision classes

At the end of each experiment assign one:

- Archive: no meaningful quality value or unsafe behavior.
- Analysis reference: useful ceiling/diagnostic, not a runtime candidate.
- Desktop offline.
- Desktop real-time.
- Powerful embedded candidate pending platform qualification.
- Daisy candidate pending full corpus and live soak.
- Production promotion candidate requiring a separate change plan.

A model can have multiple placements with different kernels/assets.

## Minimum retention gates

A candidate remains in comparison when:

- It is finite, deterministic when seeded, and stable across the case matrix.
- It does not regress the unchanged production baseline.
- It offers a measurable target-match, alias, dynamic, or listening advantage
  that is not solely level difference.
- Its costs and unsupported capabilities are fully reported.

A named-target model must improve held-out target metrics over baseline on the
waveforms it claims to model. An abstract character oscillator may instead be
retained through repeatable preference plus acceptable alias behavior.

## Reporting

Every report contains:

- Executive outcome and decision class.
- Exact model/target/case revisions.
- Metrics without a combined quality score.
- Listening method/results.
- CPU/state/assets by platform.
- Failure cases and unsupported features.
- Recommended next action or explicit stop.

Store machine-readable results beside the human report. Add summary CSV only
after metric schemas stabilize.

## Verification of the evaluator

- Test metrics on ideal analytic tones with known harmonic/alias content.
- Check phase/time errors using known delays and gains.
- Verify window leakage and legal-harmonic masks at exact and non-exact bins.
- Cross-check a subset with an independent high-rate/FFT implementation.
- Keep baseline historical results reproducible after metric changes by
  versioning the evaluator.

## References

- Existing quality harness: synth-core/examples/sample_rate_quality.rs
- Existing listening generator:
  synth-core/examples/wavetable_listening_samples.rs
- Daisy benchmark: hardware/daisy/firmware/src/bin/bench-dsp.rs
- Factory benchmark:
  hardware/daisy/firmware/src/bin/bench-factory-presets.rs
- Existing quality/performance decisions:
  plans/DAISY_SAMPLE_RATE_QUALITY_REPORT.md,
  plans/DAISY_WAVETABLE_PROTOTYPE_REPORT.md, and
  plans/DAISY_FACTORY_PRESET_PERFORMANCE_PLAN.md
- Pekonen et al., measured time/spectral evaluation:
  <https://link.springer.com/article/10.1155/2011/785103>
- Välimäki, Pekonen, and Nam, perceptually informed aliasing evaluation:
  <https://pubmed.ncbi.nlm.nih.gov/22280720/>
- Simionato and Fasciani, neural VCO metrics and frequency-conditioned tests:
  <https://dafx.de/paper-archive/2025/DAFx25_paper_33.pdf>
- Public reference dataset:
  <https://zenodo.org/records/15196138>
- Playable comparison and exact raw-filter semantics:
  plans/analog-osc/14-desktop-audition-and-pass-through-filter.md
