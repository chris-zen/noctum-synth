use midir::{MidiOutput, MidiOutputConnection};

use crate::midi::{MidiError, MidiTransport};

pub struct MidirOutputTransport {
    connection: MidiOutputConnection,
    port_name: String,
}

impl MidirOutputTransport {
    pub fn open_exact(port_name: &str) -> Result<Self, MidiError> {
        let midi_out =
            MidiOutput::new("synth-capture").map_err(|err| MidiError::Init(err.to_string()))?;
        let ports = midi_out.ports();
        let mut names = Vec::with_capacity(ports.len());
        for port in &ports {
            let name = midi_out
                .port_name(port)
                .map_err(|err| MidiError::Init(err.to_string()))?;
            names.push(name);
        }
        let index = select_exact_port_indices(&names, port_name)?;
        let connection = midi_out
            .connect(&ports[index], "synth-capture-out")
            .map_err(|err| MidiError::Init(err.to_string()))?;
        Ok(Self {
            connection,
            port_name: port_name.to_string(),
        })
    }

    pub fn port_name(&self) -> &str {
        &self.port_name
    }
}

impl MidiTransport for MidirOutputTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.connection
            .send(bytes)
            .map_err(|err| MidiError::Send(err.to_string()))
    }

    fn flush(&mut self) -> Result<(), MidiError> {
        Ok(())
    }
}

pub fn list_output_names() -> Result<Vec<String>, MidiError> {
    let midi_out =
        MidiOutput::new("synth-capture-list").map_err(|err| MidiError::Init(err.to_string()))?;
    let mut names = Vec::new();
    for port in midi_out.ports() {
        let name = midi_out
            .port_name(&port)
            .map_err(|err| MidiError::Init(err.to_string()))?;
        names.push(name);
    }
    names.sort();
    Ok(names)
}

pub fn select_exact_port_indices(
    port_names: &[String],
    requested: &str,
) -> Result<usize, MidiError> {
    let matches: Vec<usize> = port_names
        .iter()
        .enumerate()
        .filter(|(_, name)| name.as_str() == requested)
        .map(|(index, _)| index)
        .collect();
    match matches.len() {
        0 => Err(MidiError::PortNotFound {
            requested: requested.to_string(),
            available: port_names.join(", "),
        }),
        1 => Ok(matches[0]),
        count => Err(MidiError::AmbiguousPort {
            requested: requested.to_string(),
            count,
        }),
    }
}

#[cfg(test)]
mod tests {
    use crate::midi::{MidiError, midir_output::select_exact_port_indices};

    #[test]
    fn exact_port_match_rejects_substring() {
        let ports = vec![
            "Noctum Capture In".to_string(),
            "Noctum Capture".to_string(),
        ];
        assert_eq!(
            select_exact_port_indices(&ports, "Noctum Capture").unwrap(),
            1
        );
        assert!(matches!(
            select_exact_port_indices(&ports, "Noctum"),
            Err(MidiError::PortNotFound { .. })
        ));
    }

    #[test]
    fn exact_port_match_detects_ambiguity() {
        let ports = vec!["Port A".to_string(), "Port A".to_string()];
        assert!(matches!(
            select_exact_port_indices(&ports, "Port A"),
            Err(MidiError::AmbiguousPort { count: 2, .. })
        ));
    }
}
