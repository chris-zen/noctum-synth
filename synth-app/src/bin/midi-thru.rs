//! Forwards every MIDI message from one input port to one output port.

use std::{env, process, thread, time::Duration};

use midir::{Ignore, MidiInput, MidiOutput};

#[derive(Default)]
struct Args {
    input: Option<String>,
    output: Option<String>,
    list: bool,
}

fn usage() -> &'static str {
    "Usage:\n  midi-thru --list\n  midi-thru <input-port> <output-port>\n  midi-thru --input <input-port> --output <output-port>\n\nA port may be its zero-based index or its name."
}

fn parse_args() -> Result<Args, String> {
    let mut args = env::args().skip(1);
    let mut parsed = Args::default();
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--list" | "-l" => parsed.list = true,
            "--input" | "-i" => parsed.input = Some(args.next().ok_or("--input requires a port")?),
            "--output" | "-o" => {
                parsed.output = Some(args.next().ok_or("--output requires a port")?)
            }
            "--help" | "-h" => return Err(String::new()),
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => positional.push(arg),
        }
    }

    if positional.len() > 2 {
        return Err("expected at most an input and output port".into());
    }
    if parsed.input.is_none() {
        parsed.input = positional.first().cloned();
    }
    if parsed.output.is_none() {
        parsed.output = positional.get(1).cloned();
    }
    Ok(parsed)
}

fn select_port<T>(
    ports: Vec<T>,
    selector: &str,
    name: impl Fn(&T) -> Result<String, midir::PortInfoError>,
) -> Result<T, String> {
    if let Ok(index) = selector.parse::<usize>() {
        return ports
            .into_iter()
            .nth(index)
            .ok_or_else(|| format!("port index {index} is out of range"));
    }

    let selector = selector.to_lowercase();
    let mut matches = ports.into_iter().filter_map(|port| match name(&port) {
        Ok(port_name) if port_name.to_lowercase() == selector => Some(Ok(port)),
        Ok(_) => None,
        Err(err) => Some(Err(err.to_string())),
    });

    let port = matches
        .next()
        .ok_or_else(|| format!("no port named {selector:?}"))??;
    if matches.next().is_some() {
        return Err(format!(
            "port name {selector:?} is ambiguous; use its index"
        ));
    }
    Ok(port)
}

fn list_ports() -> Result<(), String> {
    let midi_in = MidiInput::new("analog-synth-midi-thru").map_err(|err| err.to_string())?;
    let midi_out = MidiOutput::new("analog-synth-midi-thru").map_err(|err| err.to_string())?;

    println!("Input ports:");
    for (index, port) in midi_in.ports().iter().enumerate() {
        println!(
            "  {index}: {}",
            midi_in
                .port_name(port)
                .unwrap_or_else(|_| "<unknown>".into())
        );
    }
    println!("Output ports:");
    for (index, port) in midi_out.ports().iter().enumerate() {
        println!(
            "  {index}: {}",
            midi_out
                .port_name(port)
                .unwrap_or_else(|_| "<unknown>".into())
        );
    }
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    if args.list {
        return list_ports();
    }
    let input = args.input.ok_or("missing input port")?;
    let output = args.output.ok_or("missing output port")?;

    let mut midi_in = MidiInput::new("analog-synth-midi-thru").map_err(|err| err.to_string())?;
    midi_in.ignore(Ignore::None);
    let midi_out = MidiOutput::new("analog-synth-midi-thru").map_err(|err| err.to_string())?;
    let input_port = select_port(midi_in.ports(), &input, |port| midi_in.port_name(port))?;
    let output_port = select_port(midi_out.ports(), &output, |port| midi_out.port_name(port))?;
    let input_name = midi_in
        .port_name(&input_port)
        .unwrap_or_else(|_| input.clone());
    let output_name = midi_out
        .port_name(&output_port)
        .unwrap_or_else(|_| output.clone());

    let mut output_connection = midi_out
        .connect(&output_port, "midi-thru-output")
        .map_err(|err| err.to_string())?;
    let _input_connection = midi_in
        .connect(
            &input_port,
            "midi-thru-input",
            move |_timestamp, message, _| {
                if let Err(err) = output_connection.send(message) {
                    eprintln!("Failed to forward MIDI message: {err}");
                }
            },
            (),
        )
        .map_err(|err| err.to_string())?;

    eprintln!("Forwarding MIDI from {input_name:?} to {output_name:?}. Press Ctrl-C to stop.");
    loop {
        thread::sleep(Duration::from_secs(3600));
    }
}

fn main() {
    match parse_args().and_then(run) {
        Ok(()) => {}
        Err(message) if message.is_empty() => println!("{}", usage()),
        Err(message) => {
            eprintln!("Error: {message}\n\n{}", usage());
            process::exit(2);
        }
    }
}
