use std::process::ExitCode;

use clap::Parser;

use synth_capture::cli::{Cli, run};

fn main() -> ExitCode {
    run(Cli::parse())
}
