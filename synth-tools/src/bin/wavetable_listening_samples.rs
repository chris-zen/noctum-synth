//! Generates short listening samples for the wavetable prototype report.

use std::{fs, io::Write, path::Path};
use synth_core::dsp::{
    AnalogOscillator, SawMethod, WAVETABLE_BANK_SAMPLES, Waveform, WavetableBank,
    WavetableOscillator, generate_wavetable_bank,
};
use synth_core::math::WideF32;

const SAMPLE_RATE: u32 = 48_000;
const SAMPLES: usize = SAMPLE_RATE as usize * 3;

fn reference_wavetable_bank() -> WavetableBank {
    static BANK: std::sync::OnceLock<WavetableBank> = std::sync::OnceLock::new();
    *BANK.get_or_init(|| {
        let mut samples = vec![0.0; WAVETABLE_BANK_SAMPLES];
        generate_wavetable_bank(&mut samples).expect("generate listening bank");
        WavetableBank::new(Box::leak(samples.into_boxed_slice()))
            .expect("validate generated wavetable bank")
    })
}

fn main() {
    let output = Path::new("plans/wavetable-listening");
    fs::create_dir_all(output).expect("create listening-sample directory");

    for (name, waveform, shape) in [
        ("saw-997", Waveform::Saw, 0.0),
        ("pulse-997-width-37", Waveform::Pulse, 0.37),
        ("triangle-997", Waveform::Triangle, 0.0),
    ] {
        let mut blep = AnalogOscillator::new(SAMPLE_RATE as f32);
        blep.set_saw_method(SawMethod::Blep);
        blep.set_waveform(waveform);
        blep.set_shape(shape);
        blep.set_frequency(WideF32::splat(997.0));
        let mut ctx = synth_core::create_render_context!();
        write_wav(
            &output.join(format!("blep-{name}.wav")),
            (0..SAMPLES).map(|_| blep.next(&mut ctx).output.to_array()[0] * 0.35),
        );

        let mut wavetable =
            WavetableOscillator::new_wavetable(SAMPLE_RATE as f32, reference_wavetable_bank());
        wavetable.set_waveform(waveform);
        wavetable.set_shape(shape);
        wavetable.set_frequency(WideF32::splat(997.0));
        let mut ctx = synth_core::create_render_context!();
        write_wav(
            &output.join(format!("wavetable-{name}.wav")),
            (0..SAMPLES).map(|_| wavetable.next(&mut ctx).output.to_array()[0] * 0.35),
        );
    }

    let mut sweep =
        WavetableOscillator::new_wavetable(SAMPLE_RATE as f32, reference_wavetable_bank());
    sweep.set_waveform(Waveform::Saw);
    let mut ctx = synth_core::create_render_context!();
    write_wav(
        &output.join("wavetable-saw-mip-sweep.wav"),
        (0..SAMPLES).map(|index| {
            let position = index as f32 / (SAMPLES - 1) as f32;
            let frequency = 110.0 * 2.0_f32.powf(position * 6.0);
            sweep.set_frequency(WideF32::splat(frequency));
            sweep.next(&mut ctx).output.to_array()[0] * 0.35
        }),
    );
}

fn write_wav(path: &Path, samples: impl Iterator<Item = f32>) {
    let pcm: Vec<i16> = samples
        .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
        .collect();
    let data_bytes = (pcm.len() * 2) as u32;
    let mut file = fs::File::create(path).expect("create WAV file");
    file.write_all(b"RIFF").unwrap();
    file.write_all(&(36 + data_bytes).to_le_bytes()).unwrap();
    file.write_all(b"WAVEfmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap();
    file.write_all(&SAMPLE_RATE.to_le_bytes()).unwrap();
    file.write_all(&(SAMPLE_RATE * 2).to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&16_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&data_bytes.to_le_bytes()).unwrap();
    for sample in pcm {
        file.write_all(&sample.to_le_bytes()).unwrap();
    }
}
