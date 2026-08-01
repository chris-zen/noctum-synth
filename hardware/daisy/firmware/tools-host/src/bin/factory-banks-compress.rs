use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use tools_micro as factory_banks;

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(input) = args.next().map(PathBuf::from) else {
        eprintln!("usage: factory-banks-compress <input.syx> <output.zlib>");
        return ExitCode::FAILURE;
    };
    let Some(output) = args.next().map(PathBuf::from) else {
        eprintln!("usage: factory-banks-compress <input.syx> <output.zlib>");
        return ExitCode::FAILURE;
    };
    if args.next().is_some() {
        eprintln!("usage: factory-banks-compress <input.syx> <output.zlib>");
        return ExitCode::FAILURE;
    }

    let raw = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("failed to read {}: {error}", input.display());
            return ExitCode::FAILURE;
        }
    };

    if raw.len() != factory_banks::BANK_SIZE {
        eprintln!(
            "unexpected bank size: {} (expected {})",
            raw.len(),
            factory_banks::BANK_SIZE
        );
        return ExitCode::FAILURE;
    }

    let crc = factory_banks::bank_crc32(&raw);
    if crc != factory_banks::BANK_CRC32 {
        eprintln!(
            "unexpected bank CRC32: {crc:#010x} (expected {:#010x})",
            factory_banks::BANK_CRC32
        );
        return ExitCode::FAILURE;
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    if let Err(error) = encoder.write_all(&raw) {
        eprintln!("compress failed: {error}");
        return ExitCode::FAILURE;
    }
    let compressed = match encoder.finish() {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("compress failed: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(parent) = output.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!("failed to create {}: {error}", parent.display());
            return ExitCode::FAILURE;
        }
    }
    if let Err(error) = fs::write(&output, &compressed) {
        eprintln!("failed to write {}: {error}", output.display());
        return ExitCode::FAILURE;
    }

    println!(
        "compressed {} -> {} ({} -> {} bytes, CRC32={crc:#010x})",
        input.display(),
        output.display(),
        raw.len(),
        compressed.len()
    );
    ExitCode::SUCCESS
}
