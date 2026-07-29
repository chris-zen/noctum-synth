//! Compact QSPI-backed MIDI program catalog and sector-safe slot updates.

use synth_core::{LayerPatch, LayerPatchRecord, LayerPatchRecordError, LAYER_PATCH_RECORD_SIZE};

pub trait ProgramFlash {
    type Error;

    fn read(&mut self, address: u32, output: &mut [u8]) -> Result<(), Self::Error>;
    fn erase_sector(&mut self, address: u32) -> Result<(), Self::Error>;
    fn program(&mut self, address: u32, data: &[u8]) -> Result<(), Self::Error>;
}

#[cfg(target_arch = "arm")]
impl ProgramFlash for embassy_daisy::qspi::QspiFlash {
    type Error = embassy_daisy::qspi::FlashError;

    fn read(&mut self, address: u32, output: &mut [u8]) -> Result<(), Self::Error> {
        embassy_daisy::qspi::QspiFlash::read(self, address, output)
    }

    fn erase_sector(&mut self, address: u32) -> Result<(), Self::Error> {
        embassy_daisy::qspi::QspiFlash::erase_sector(self, address)
    }

    fn program(&mut self, address: u32, data: &[u8]) -> Result<(), Self::Error> {
        embassy_daisy::qspi::QspiFlash::program(self, address, data)
    }
}

pub const BANK_COUNT: u8 = 8;
pub const PROGRAMS_PER_BANK: u8 = 128;
pub const SLOT_COUNT: usize = BANK_COUNT as usize * PROGRAMS_PER_BANK as usize;
pub const SLOT_STRIDE: usize = LAYER_PATCH_RECORD_SIZE;
pub const SECTOR_SIZE: usize = 4096;
pub const CATALOG_A_ADDRESS: u32 = 0x000c_0000;
pub const PROGRAMS_ADDRESS: u32 = CATALOG_A_ADDRESS + SECTOR_SIZE as u32;
pub const CATALOG_B_ADDRESS: u32 = PROGRAMS_ADDRESS + (SLOT_COUNT * SLOT_STRIDE) as u32;
pub const STORAGE_END: u32 = CATALOG_B_ADDRESS + SECTOR_SIZE as u32;
const CATALOG_ADDRESSES: [u32; 2] = [CATALOG_A_ADDRESS, CATALOG_B_ADDRESS];

#[cfg(target_arch = "arm")]
const _: () = {
    assert!(CATALOG_A_ADDRESS == embassy_daisy::qspi::APPLICATION_RESERVED_END);
    assert!(SECTOR_SIZE as u32 == embassy_daisy::qspi::SECTOR_SIZE);
    assert!(STORAGE_END <= embassy_daisy::qspi::SIZE_BYTES);
};

const CATALOG_MAGIC: [u8; 4] = *b"NMPG";
const CATALOG_VERSION: u8 = 2;
const CATALOG_HEADER_LEN: usize = 20;
const SELECTION_ENTRY_LEN: usize = 12;

static mut SECTOR_BUFFER: [u8; SECTOR_SIZE] = [0u8; SECTOR_SIZE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitStatus {
    Opened,
    Formatted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramStoreError<E> {
    Flash(E),
    InvalidAddress,
    Record(LayerPatchRecordError),
    VerifyFailed,
}

impl<E> From<LayerPatchRecordError> for ProgramStoreError<E> {
    fn from(error: LayerPatchRecordError) -> Self {
        Self::Record(error)
    }
}

pub struct ProgramStore<F> {
    flash: F,
    last_bank: u8,
    last_program: u8,
    generation: u32,
    selection_sequence: u32,
    next_selection_offset: usize,
    active_catalog_address: u32,
}

impl<F: ProgramFlash> ProgramStore<F> {
    pub fn open(mut flash: F) -> Result<(Self, InitStatus), ProgramStoreError<F::Error>> {
        let mut best_address = 0_u32;
        let mut best_catalog: Option<(Catalog, [u8; SECTOR_SIZE])> = None;

        for &address in &CATALOG_ADDRESSES {
            let mut sector = [0; SECTOR_SIZE];
            flash
                .read(address, &mut sector)
                .map_err(ProgramStoreError::Flash)?;
            let mut header = [0; CATALOG_HEADER_LEN];
            header.copy_from_slice(&sector[..CATALOG_HEADER_LEN]);
            if let Some(catalog) = Catalog::decode(&header) {
                if best_catalog
                    .as_ref()
                    .map_or(true, |(best, _)| catalog.generation.wrapping_sub(best.generation) < u32::MAX / 2)
                {
                    best_catalog = Some((catalog, sector));
                    best_address = address;
                }
            }
        }

        if let Some((catalog, sector)) = best_catalog {
            let journal = scan_selection_journal(&sector, catalog.bank, catalog.program);
            return Ok((
                Self {
                    flash,
                    last_bank: journal.bank,
                    last_program: journal.program,
                    generation: catalog.generation,
                    selection_sequence: journal.sequence,
                    next_selection_offset: journal.next_offset,
                    active_catalog_address: best_address,
                },
                InitStatus::Opened,
            ));
        }

        #[cfg(target_arch = "arm")]
        defmt::warn!(
            "invalid program catalog headers, formatting"
        );

        let generation = 1;
        let catalog = Catalog {
            bank: 0,
            program: 0,
            generation,
        };
        let mut address = CATALOG_A_ADDRESS;
        while address < STORAGE_END {
            flash
                .erase_sector(address)
                .map_err(ProgramStoreError::Flash)?;
            address += SECTOR_SIZE as u32;
        }
        write_catalog(&mut flash, CATALOG_A_ADDRESS, catalog)?;
        Ok((
            Self {
                flash,
                last_bank: 0,
                last_program: 0,
                generation,
                selection_sequence: 0,
                next_selection_offset: CATALOG_HEADER_LEN,
                active_catalog_address: CATALOG_A_ADDRESS,
            },
            InitStatus::Formatted,
        ))
    }

    pub fn last_load(&self) -> (u8, u8) {
        (self.last_bank, self.last_program)
    }

    pub fn load(&mut self, bank: u8, program: u8) -> Result<LayerPatch, ProgramStoreError<F::Error>> {
        let address = slot_address(bank, program)?;
        let mut record = [0; LAYER_PATCH_RECORD_SIZE];
        self.flash
            .read(address, &mut record)
            .map_err(ProgramStoreError::Flash)?;
        LayerPatchRecord::decode(&record).map_err(ProgramStoreError::Record)
    }

    pub fn save(
        &mut self,
        bank: u8,
        program: u8,
        patch: &LayerPatch,
    ) -> Result<(), ProgramStoreError<F::Error>> {
        let address = slot_address(bank, program)?;
        let sector_address = address - address % SECTOR_SIZE as u32;
        let offset = (address - sector_address) as usize;
        #[allow(static_mut_refs)]
        let sector = unsafe { &mut SECTOR_BUFFER };
        self.flash
            .read(sector_address, sector)
            .map_err(ProgramStoreError::Flash)?;
        let mut record = [0xff; LAYER_PATCH_RECORD_SIZE];
        LayerPatchRecord::encode(patch, &mut record)?;
        sector[offset..offset + LAYER_PATCH_RECORD_SIZE].copy_from_slice(&record);

        self.flash
            .erase_sector(sector_address)
            .map_err(ProgramStoreError::Flash)?;
        self.flash
            .program(sector_address, sector)
            .map_err(ProgramStoreError::Flash)?;
        let mut verify = [0; LAYER_PATCH_RECORD_SIZE];
        self.flash
            .read(address, &mut verify)
            .map_err(ProgramStoreError::Flash)?;
        if verify != record {
            return Err(ProgramStoreError::VerifyFailed);
        }
        Ok(())
    }

    pub fn persist_last_load(
        &mut self,
        bank: u8,
        program: u8,
    ) -> Result<(), ProgramStoreError<F::Error>> {
        validate_address(bank, program)?;
        if (bank, program) == (self.last_bank, self.last_program) {
            return Ok(());
        }
        if self.selection_sequence != u32::MAX
            && self.next_selection_offset + SELECTION_ENTRY_LEN <= SECTOR_SIZE
        {
            let entry = SelectionEntry {
                bank,
                program,
                sequence: self.selection_sequence + 1,
            }
            .encode();
            let address = self.active_catalog_address + self.next_selection_offset as u32;
            self.flash
                .program(address, &entry)
                .map_err(ProgramStoreError::Flash)?;
            let mut verify = [0; SELECTION_ENTRY_LEN];
            self.flash
                .read(address, &mut verify)
                .map_err(ProgramStoreError::Flash)?;
            if verify != entry {
                return Err(ProgramStoreError::VerifyFailed);
            }
            self.last_bank = bank;
            self.last_program = program;
            self.selection_sequence += 1;
            self.next_selection_offset += SELECTION_ENTRY_LEN;
            return Ok(());
        }

        let generation = self.generation.wrapping_add(1);
        let catalog = Catalog {
            bank,
            program,
            generation,
        };
        let alternate = if self.active_catalog_address == CATALOG_A_ADDRESS {
            CATALOG_B_ADDRESS
        } else {
            CATALOG_A_ADDRESS
        };
        self.flash
            .erase_sector(alternate)
            .map_err(ProgramStoreError::Flash)?;
        write_catalog(&mut self.flash, alternate, catalog)?;
        self.last_bank = bank;
        self.last_program = program;
        self.generation = generation;
        self.selection_sequence = 0;
        self.next_selection_offset = CATALOG_HEADER_LEN;
        self.active_catalog_address = alternate;
        Ok(())
    }

    #[cfg(test)]
    fn into_flash(self) -> F {
        self.flash
    }
}

#[derive(Clone, Copy)]
struct Catalog {
    bank: u8,
    program: u8,
    generation: u32,
}

impl Catalog {
    fn encode(self) -> [u8; CATALOG_HEADER_LEN] {
        let mut bytes = [0xff; CATALOG_HEADER_LEN];
        bytes[..4].copy_from_slice(&CATALOG_MAGIC);
        bytes[4] = CATALOG_VERSION;
        bytes[5] = self.bank;
        bytes[6] = self.program;
        bytes[7] = 0;
        bytes[8..10].copy_from_slice(&(SLOT_COUNT as u16).to_le_bytes());
        bytes[10..12].copy_from_slice(&(SLOT_STRIDE as u16).to_le_bytes());
        bytes[12..16].copy_from_slice(&self.generation.to_le_bytes());
        let checksum = synth_core::patch_storage::crc32(&bytes[..16]);
        bytes[16..20].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8; CATALOG_HEADER_LEN]) -> Option<Self> {
        if bytes[..4] != CATALOG_MAGIC || bytes[4] != CATALOG_VERSION {
            return None;
        }
        if u16::from_le_bytes([bytes[8], bytes[9]]) != SLOT_COUNT as u16
            || u16::from_le_bytes([bytes[10], bytes[11]]) != SLOT_STRIDE as u16
        {
            return None;
        }
        let checksum = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        if checksum != synth_core::patch_storage::crc32(&bytes[..16]) {
            return None;
        }
        let bank = bytes[5];
        let program = bytes[6];
        if bank >= BANK_COUNT || program >= PROGRAMS_PER_BANK {
            return None;
        }
        Some(Self {
            bank,
            program,
            generation: u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        })
    }
}

#[derive(Clone, Copy)]
struct SelectionEntry {
    bank: u8,
    program: u8,
    sequence: u32,
}

impl SelectionEntry {
    fn encode(self) -> [u8; SELECTION_ENTRY_LEN] {
        let mut bytes = [0xff; SELECTION_ENTRY_LEN];
        bytes[..2].copy_from_slice(b"SL");
        bytes[2] = self.bank;
        bytes[3] = self.program;
        bytes[4..8].copy_from_slice(&self.sequence.to_le_bytes());
        let checksum = synth_core::patch_storage::crc32(&bytes[..8]);
        bytes[8..12].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8; SELECTION_ENTRY_LEN]) -> Option<Self> {
        if bytes[..2] != *b"SL"
            || synth_core::patch_storage::crc32(&bytes[..8]) != u32::from_le_bytes(bytes[8..12].try_into().ok()?)
        {
            return None;
        }
        let bank = bytes[2];
        let program = bytes[3];
        if bank >= BANK_COUNT || program >= PROGRAMS_PER_BANK {
            return None;
        }
        Some(Self {
            bank,
            program,
            sequence: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
        })
    }
}

struct SelectionJournal {
    bank: u8,
    program: u8,
    sequence: u32,
    next_offset: usize,
}

fn scan_selection_journal(sector: &[u8; SECTOR_SIZE], bank: u8, program: u8) -> SelectionJournal {
    let mut journal = SelectionJournal {
        bank,
        program,
        sequence: 0,
        next_offset: CATALOG_HEADER_LEN,
    };
    while journal.next_offset + SELECTION_ENTRY_LEN <= SECTOR_SIZE {
        let mut bytes = [0; SELECTION_ENTRY_LEN];
        bytes.copy_from_slice(
            &sector[journal.next_offset..journal.next_offset + SELECTION_ENTRY_LEN],
        );
        if bytes.iter().all(|byte| *byte == 0xff) {
            break;
        }
        let Some(entry) = SelectionEntry::decode(&bytes) else {
            journal.next_offset = SECTOR_SIZE;
            break;
        };
        if entry.sequence != journal.sequence.wrapping_add(1) {
            journal.next_offset = SECTOR_SIZE;
            break;
        }
        journal.bank = entry.bank;
        journal.program = entry.program;
        journal.sequence = entry.sequence;
        journal.next_offset += SELECTION_ENTRY_LEN;
    }
    journal
}

fn write_catalog<F: ProgramFlash>(
    flash: &mut F,
    address: u32,
    catalog: Catalog,
) -> Result<(), ProgramStoreError<F::Error>> {
    let header = catalog.encode();
    flash
        .program(address, &header)
        .map_err(ProgramStoreError::Flash)?;
    let mut verify = [0; CATALOG_HEADER_LEN];
    flash
        .read(address, &mut verify)
        .map_err(ProgramStoreError::Flash)?;
    if verify != header {
        return Err(ProgramStoreError::VerifyFailed);
    }
    Ok(())
}

fn slot_address<E>(bank: u8, program: u8) -> Result<u32, ProgramStoreError<E>> {
    validate_address(bank, program)?;
    let index = usize::from(bank) * usize::from(PROGRAMS_PER_BANK) + usize::from(program);
    Ok(PROGRAMS_ADDRESS + (index * SLOT_STRIDE) as u32)
}

fn validate_address<E>(bank: u8, program: u8) -> Result<(), ProgramStoreError<E>> {
    if bank >= BANK_COUNT || program >= PROGRAMS_PER_BANK {
        Err(ProgramStoreError::InvalidAddress)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;
    use std::vec::Vec;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockError {
        OutOfBounds,
        Unaligned,
        NeedsErase,
    }

    struct MockFlash {
        bytes: Vec<u8>,
        erased: Vec<u32>,
    }

    impl MockFlash {
        fn erased() -> Self {
            Self {
                bytes: vec![0xff; STORAGE_END as usize],
                erased: Vec::new(),
            }
        }

        fn range(&self, address: u32, len: usize) -> Result<core::ops::Range<usize>, MockError> {
            let start = address as usize;
            let end = start.checked_add(len).ok_or(MockError::OutOfBounds)?;
            if end > self.bytes.len() {
                return Err(MockError::OutOfBounds);
            }
            Ok(start..end)
        }
    }

    impl ProgramFlash for MockFlash {
        type Error = MockError;

        fn read(&mut self, address: u32, output: &mut [u8]) -> Result<(), Self::Error> {
            let range = self.range(address, output.len())?;
            output.copy_from_slice(&self.bytes[range]);
            Ok(())
        }

        fn erase_sector(&mut self, address: u32) -> Result<(), Self::Error> {
            if address as usize % SECTOR_SIZE != 0 {
                return Err(MockError::Unaligned);
            }
            let range = self.range(address, SECTOR_SIZE)?;
            self.bytes[range].fill(0xff);
            self.erased.push(address);
            Ok(())
        }

        fn program(&mut self, address: u32, data: &[u8]) -> Result<(), Self::Error> {
            let range = self.range(address, data.len())?;
            for (old, new) in self.bytes[range.clone()].iter().zip(data) {
                if old & new != *new {
                    return Err(MockError::NeedsErase);
                }
            }
            for (old, new) in self.bytes[range].iter_mut().zip(data) {
                *old &= *new;
            }
            Ok(())
        }
    }

    fn named_patch(name: &str, cutoff: f32) -> LayerPatch {
        let mut patch = LayerPatch::default();
        patch.name.push_str(name).unwrap();
        patch.filter.cutoff = cutoff;
        patch
    }

    #[test]
    fn erased_flash_formats_to_logical_default_programs() {
        let (mut store, status) = ProgramStore::open(MockFlash::erased()).unwrap();
        assert_eq!(status, InitStatus::Formatted);
        assert_eq!(store.last_load(), (0, 0));
        assert!(store.load(0, 0).unwrap().name.is_empty());
        let flash = store.into_flash();
        assert_eq!(
            flash.erased.len(),
            2 + SLOT_COUNT * SLOT_STRIDE / SECTOR_SIZE
        );
        assert!(flash.bytes[PROGRAMS_ADDRESS as usize..]
            .iter()
            .all(|byte| *byte == 0xff));
    }

    #[test]
    fn sector_rewrite_preserves_neighboring_slots() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let first = named_patch("first", 1111.0);
        let neighbor = named_patch("neighbor", 2222.0);
        store.save(0, 0, &first).unwrap();
        store.save(0, 1, &neighbor).unwrap();
        store.save(0, 0, &named_patch("updated", 3333.0)).unwrap();
        assert_eq!(store.load(0, 0).unwrap().name.as_str(), "updated");
        let loaded_neighbor = store.load(0, 1).unwrap();
        assert_eq!(loaded_neighbor.name.as_str(), "neighbor");
        assert!((loaded_neighbor.filter.cutoff - 2222.0).abs() < 2.0);
    }

    #[test]
    fn programs_and_last_load_survive_reopen() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        store.save(7, 127, &named_patch("last", 9876.0)).unwrap();
        store.persist_last_load(7, 127).unwrap();
        let (mut reopened, status) = ProgramStore::open(store.into_flash()).unwrap();
        assert_eq!(status, InitStatus::Opened);
        assert_eq!(reopened.last_load(), (7, 127));
        assert_eq!(reopened.load(7, 127).unwrap().name.as_str(), "last");
    }

    #[test]
    fn selection_journal_avoids_an_erase_per_load() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        for index in 1..=100 {
            store
                .persist_last_load((index % 8) as u8, (index % 128) as u8)
                .unwrap();
        }
        assert_eq!(
            store
                .flash
                .erased
                .iter()
                .filter(|address| **address == CATALOG_A_ADDRESS)
                .count(),
            1
        );
        let expected = store.last_load();
        let (reopened, status) = ProgramStore::open(store.into_flash()).unwrap();
        assert_eq!(status, InitStatus::Opened);
        assert_eq!(reopened.last_load(), expected);
    }

    #[test]
    fn full_or_interrupted_journal_compacts_on_next_selection() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let capacity = (SECTOR_SIZE - CATALOG_HEADER_LEN) / SELECTION_ENTRY_LEN;
        for index in 1..=capacity {
            store
                .persist_last_load((index % 8) as u8, (index % 128) as u8)
                .unwrap();
        }
        store.persist_last_load(7, 127).unwrap();
        assert_eq!(
            store
                .flash
                .erased
                .iter()
                .filter(|address| **address == CATALOG_A_ADDRESS)
                .count(),
            1
        );
        assert_eq!(
            store
                .flash
                .erased
                .iter()
                .filter(|address| **address == CATALOG_B_ADDRESS)
                .count(),
            2
        );

        let corrupt_offset = store.next_selection_offset;
        store.flash.bytes[CATALOG_B_ADDRESS as usize + corrupt_offset] = 0;
        let (mut reopened, _) = ProgramStore::open(store.into_flash()).unwrap();
        assert_eq!(reopened.next_selection_offset, SECTOR_SIZE);
        reopened.persist_last_load(6, 126).unwrap();
        assert_eq!(reopened.last_load(), (6, 126));
    }

    #[test]
    fn corrupt_catalog_reformats_entire_store() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        store.save(2, 3, &named_patch("discarded", 1234.0)).unwrap();
        let mut flash = store.into_flash();
        flash.bytes[CATALOG_A_ADDRESS as usize] ^= 1;
        flash.bytes[CATALOG_B_ADDRESS as usize] ^= 1;
        let (mut reopened, status) = ProgramStore::open(flash).unwrap();
        assert_eq!(status, InitStatus::Formatted);
        assert!(reopened.load(2, 3).unwrap().name.is_empty());
    }

    #[test]
    fn corrupt_slot_is_reported_without_reformatting() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        store.save(1, 9, &named_patch("damaged", 4444.0)).unwrap();
        let address = slot_address::<MockError>(1, 9).unwrap() as usize;
        store.flash.bytes[address + 30] ^= 1;
        assert!(matches!(
            store.load(1, 9),
            Err(ProgramStoreError::Record(
                LayerPatchRecordError::ChecksumMismatch
            ))
        ));
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        assert!(matches!(
            store.load(8, 0),
            Err(ProgramStoreError::InvalidAddress)
        ));
        assert!(matches!(
            store.save(0, 128, &LayerPatch::default()),
            Err(ProgramStoreError::InvalidAddress)
        ));
    }
}
