use std::{env, path::PathBuf, process::ExitCode};

use synth_tools::measured_wavetable_bank::{BankRequest, build_bank, default_research_root};

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
    let mut request = BankRequest::arturia_defaults();
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
measured_wavetable_bank — build a measured pitch-conditioned wavetable bank from synth-capture NPZs

USAGE:
  cargo run --release -p synth-tools --bin measured_wavetable_bank -- [OPTIONS]

OPTIONS:
  --derived-root <dir>           Directory containing {{saw,triangle,pulse50}}-cycles-v1.npz
  --output-dir <dir>             Directory for .f32le + .json outputs
  --profile-id <id>              Bank profile id (default arturia-prophet5-measured-bank-v1)
  --target-id <id>               Target id (default arturia-prophet5-v1)
  --reference-sample-rate <hz>   Bank Nyquist reference rate (default 96000)

Defaults assume research tree:
  derived: {}/captures/arturia-prophet5-v1/derived
  output:  {}/banks
",
        research.display(),
        research.display()
    );
}
