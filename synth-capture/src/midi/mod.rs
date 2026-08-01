use std::time::Instant;

use thiserror::Error;

pub mod midir_output;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MidiError {
    #[error("midi port `{requested}` not found; available: {available}")]
    PortNotFound {
        requested: String,
        available: String,
    },
    #[error("ambiguous midi port name `{requested}` matched {count} ports")]
    AmbiguousPort { requested: String, count: usize },
    #[error("midi init failed: {0}")]
    Init(String),
    #[error("midi send failed: {0}")]
    Send(String),
    #[error("midi flush failed: {0}")]
    Flush(String),
}

pub trait MidiTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError>;
    fn flush(&mut self) -> Result<(), MidiError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptEntry {
    pub offset_ms: u64,
    pub bytes: Vec<u8>,
}

pub struct TranscriptTransport<T> {
    inner: T,
    started: Instant,
    entries: Vec<TranscriptEntry>,
}

impl<T> TranscriptTransport<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            started: Instant::now(),
            entries: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[TranscriptEntry] {
        &self.entries
    }

    pub fn clear_entries(&mut self) {
        self.entries.clear();
        self.started = Instant::now();
    }

    pub fn into_inner(self) -> (T, Vec<TranscriptEntry>) {
        (self.inner, self.entries)
    }

    pub fn fingerprint(&self) -> String {
        let mut material = String::new();
        for entry in &self.entries {
            material.push_str(&format!("{};", entry.offset_ms));
            for byte in &entry.bytes {
                material.push_str(&format!("{byte:02x}"));
            }
            material.push('\n');
        }
        crate::project::sha256_bytes(material.as_bytes())
    }
}

impl<T: MidiTransport> MidiTransport for TranscriptTransport<T> {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.inner.send(bytes)?;
        self.entries.push(TranscriptEntry {
            offset_ms: self.started.elapsed().as_millis() as u64,
            bytes: bytes.to_vec(),
        });
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MidiError> {
        self.inner.flush()
    }
}

#[derive(Default)]
pub struct FakeMidiTransport {
    pub sent: Vec<Vec<u8>>,
    pub flushed: u32,
}

impl MidiTransport for FakeMidiTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), MidiError> {
        self.sent.push(bytes.to_vec());
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MidiError> {
        self.flushed += 1;
        Ok(())
    }
}

pub fn list_midi_output_names() -> Result<Vec<String>, MidiError> {
    midir_output::list_output_names()
}
