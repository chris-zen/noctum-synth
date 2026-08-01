use directories::ProjectDirs;
use serde::Serialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use synth_core::midi::rev2;
use synth_core::{LayerMode, LayerPatch, Patch};

const PATCH_SCHEMA_VERSION: u8 = 1;

#[derive(Serialize)]
struct PatchLayersFile {
    a: LayerPatch,
    b: LayerPatch,
}

#[derive(Serialize)]
struct PatchFile {
    schema_version: u8,
    mode: LayerMode,
    split_point: u8,
    layers: PatchLayersFile,
}

impl From<&Patch> for PatchFile {
    fn from(patch: &Patch) -> Self {
        Self {
            schema_version: PATCH_SCHEMA_VERSION,
            mode: patch.mode,
            split_point: patch.split_point,
            layers: PatchLayersFile {
                a: patch.layer_a.clone(),
                b: patch.layer_b.clone(),
            },
        }
    }
}

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(input) = args.next().map(PathBuf::from) else {
        eprintln!("usage: export_rev2_patches <input.syx> [output-dir]");
        return ExitCode::FAILURE;
    };
    let output_dir = match args.next() {
        Some(path) => PathBuf::from(path),
        None => match default_patches_dir() {
            Some(path) => path,
            None => {
                eprintln!("could not resolve Noctum patches directory");
                return ExitCode::FAILURE;
            }
        },
    };
    if args.next().is_some() {
        eprintln!("usage: export_rev2_patches <input.syx> [output-dir]");
        return ExitCode::FAILURE;
    }

    let bytes = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read {}: {error}", input.display());
            return ExitCode::FAILURE;
        }
    };
    if bytes.is_empty() {
        eprintln!("input file is empty: {}", input.display());
        return ExitCode::FAILURE;
    }

    if let Err(error) = fs::create_dir_all(&output_dir) {
        eprintln!(
            "failed to create output directory {}: {error}",
            output_dir.display()
        );
        return ExitCode::FAILURE;
    }

    let mut written = 0usize;
    let mut skipped = 0usize;
    for message in sysex_messages(&bytes) {
        match rev2::decode::program_data(message) {
            Ok(program) => match write_program(&output_dir, &program) {
                Ok(path) => {
                    written += 1;
                    println!("{}", path.display());
                }
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            },
            Err(error) => {
                skipped += 1;
                eprintln!("skipped sysex message ({} bytes): {error:?}", message.len());
            }
        }
    }

    if written == 0 {
        eprintln!("no Rev2 Program Data messages found in {}", input.display());
        return ExitCode::FAILURE;
    }

    println!(
        "wrote {written} patch(es) to {} (skipped {skipped})",
        output_dir.display()
    );
    ExitCode::SUCCESS
}

fn default_patches_dir() -> Option<PathBuf> {
    ProjectDirs::from("", "", "Noctum").map(|dirs| dirs.config_dir().join("patches"))
}

fn sysex_messages(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    SysexMessages { bytes, index: 0 }
}

struct SysexMessages<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> Iterator for SysexMessages<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        while self.index < self.bytes.len() {
            if self.bytes[self.index] != 0xf0 {
                self.index += 1;
                continue;
            }
            let start = self.index;
            self.index += 1;
            while self.index < self.bytes.len() {
                let byte = self.bytes[self.index];
                self.index += 1;
                if byte == 0xf7 {
                    return Some(&self.bytes[start..self.index]);
                }
                if byte == 0xf0 {
                    self.index -= 1;
                    break;
                }
            }
        }
        None
    }
}

fn write_program(output_dir: &Path, program: &rev2::ProgramData) -> Result<PathBuf, String> {
    let name = rev2_program_filename(
        program.bank,
        program.program,
        program.patch.layer_a.name.as_str(),
    )
    .ok_or_else(|| {
        format!(
            "MIDI program bank or number is outside the factory library: bank={}, program={}",
            program.bank, program.program
        )
    })?;
    let path = output_dir.join(format!("{name}.json"));
    let json = serde_json::to_string_pretty(&PatchFile::from(&program.patch))
        .map_err(|error| format!("failed to serialize {}: {error}", path.display()))?;
    fs::write(&path, json)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    Ok(path)
}

fn rev2_program_filename(bank: u8, program: u8, patch_name: &str) -> Option<String> {
    if bank > 7 || program > 127 {
        return None;
    }
    let (bank_kind, bank_number) = if bank < 4 {
        ('F', bank + 1)
    } else {
        ('U', bank - 3)
    };
    let name = if patch_name.is_empty() {
        "LayerPatch".to_string()
    } else {
        sanitize_filename(patch_name)
    };
    Some(format!(
        "{bank_kind}{bank_number}-{:03}-{name}",
        program + 1
    ))
}

fn sanitize_filename(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            output.push(character);
            separator = false;
        } else if !separator && !output.is_empty() {
            output.push('_');
            separator = true;
        }
    }
    while output.ends_with('_') {
        output.pop();
    }
    output
}
