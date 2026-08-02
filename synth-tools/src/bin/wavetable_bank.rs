use std::{env, path::PathBuf, process::ExitCode};

use synth_tools::wavetable_bank::{BankRequest, build_bank, default_research_root};

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let mut request = BankRequest::prophet5_defaults();
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--derived-root" => {
                index += 1;
                request.derived_root = required_path(&args, index, "--derived-root")?;
            }
            "--output-dir" => {
                index += 1;
                request.output_dir = required_path(&args, index, "--output-dir")?;
            }
            "--profile-id" => {
                index += 1;
                request.profile_id = required_value(&args, index, "--profile-id")?;
            }
            "--target-id" => {
                index += 1;
                request.target_id = required_value(&args, index, "--target-id")?;
            }
            "--reference-sample-rate" => {
                index += 1;
                let value = required_value(&args, index, "--reference-sample-rate")?;
                request.reference_sample_rate_hz = value
                    .parse()
                    .map_err(|_| format!("invalid --reference-sample-rate {value}"))?;
            }
            "--rust-output" => {
                index += 1;
                request.rust_profile_path = Some(required_path(&args, index, "--rust-output")?);
            }
            "--no-rust-output" => request.rust_profile_path = None,
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }

    let result = build_bank(&request).map_err(|err| err.to_string())?;
    println!(
        "wrote {} ({} samples, {} training pitches/waveform)",
        result.binary_path.display(),
        result.sample_count,
        result.pitch_count_per_waveform
    );
    println!("wrote {}", result.manifest_path.display());
    if let Some(path) = result.rust_profile_path {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn required_path(args: &[String], index: usize, flag: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(required_value(args, index, flag)?))
}

fn required_value(args: &[String], index: usize, flag: &str) -> Result<String, String> {
    args.get(index)
        .cloned()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn print_help() {
    let research = default_research_root();
    println!(
        "\
wavetable_bank — build a measured pitch-conditioned wavetable bank from synth-capture NPZs

USAGE:
  cargo run --release -p synth-tools --bin wavetable_bank -- [OPTIONS]

OPTIONS:
  --derived-root <dir>           Directory containing {{saw,triangle,pulse50}}-cycles-v1.npz
  --output-dir <dir>             Directory for .f32le + .json outputs
  --profile-id <id>              Bank profile id (default prophet5-wavetable-bank-v1)
  --target-id <id>               Target id (default prophet5-v1)
  --reference-sample-rate <hz>   Playback-bank Nyquist reference (default 48000)
  --rust-output <file>           Generated synth-core profile source
  --no-rust-output               Do not generate Rust profile source

Defaults assume research tree:
  derived: {}/captures/arturia-prophet5-v1-r7/derived
  output:  {}/banks
",
        research.display(),
        research.display()
    );
}
