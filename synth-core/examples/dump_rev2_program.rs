use std::{env, fs, process};

use synth_core::{REV2_PROGRAM_DATA_SYSEX_LEN, Rev2MidiDecoder};

fn main() {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: dump_rev2_program <bank.syx> [bank] [program]");
        process::exit(2);
    };
    let bank: usize = args.next().as_deref().unwrap_or("0").parse().unwrap();
    let program: usize = args.next().as_deref().unwrap_or("0").parse().unwrap();
    let bytes = fs::read(path).unwrap();
    let offset = (bank * 128 + program) * REV2_PROGRAM_DATA_SYSEX_LEN;
    let end = offset + REV2_PROGRAM_DATA_SYSEX_LEN;
    let decoded = Rev2MidiDecoder::program_data(&bytes[offset..end]).unwrap();
    println!(
        "bank={} program={}\n{:#?}",
        decoded.bank, decoded.program, decoded.patch
    );
}
