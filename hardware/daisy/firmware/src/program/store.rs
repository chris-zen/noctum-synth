//! Power-loss-safe indexed program storage for Daisy QSPI NOR flash.

use synth_core::{PATCH_RECORD_SIZE, Patch, PatchRecord, PatchRecordError};

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
pub const SECTOR_SIZE: usize = 4096;
pub const PAGE_SIZE: usize = 256;
pub const PATCH_BLOCK_COUNT: usize = SLOT_COUNT + 1;
pub const CATALOG_A_ADDRESS: u32 = 0x000c_0000;
pub const CATALOG_B_ADDRESS: u32 = CATALOG_A_ADDRESS + SECTOR_SIZE as u32;
pub const PROGRAMS_ADDRESS: u32 = CATALOG_B_ADDRESS + SECTOR_SIZE as u32;
pub const SLOT_STRIDE: usize = SECTOR_SIZE;
pub const STORAGE_END: u32 = PROGRAMS_ADDRESS + (PATCH_BLOCK_COUNT * SECTOR_SIZE) as u32;

const QSPI_SIZE_BYTES: u32 = 8 * 1024 * 1024;
const FACTORY_BANK_SIZE: u32 = 512 * 2346;
const FACTORY_BANK_SECTORS: u32 = FACTORY_BANK_SIZE.div_ceil(SECTOR_SIZE as u32);
const FACTORY_BANK_ADDRESS: u32 = QSPI_SIZE_BYTES - FACTORY_BANK_SECTORS * SECTOR_SIZE as u32;
const INVALID_BLOCK: u16 = u16::MAX;

const _: () = {
    assert!(CATALOG_A_ADDRESS % SECTOR_SIZE as u32 == 0);
    assert!(CATALOG_B_ADDRESS % SECTOR_SIZE as u32 == 0);
    assert!(PROGRAMS_ADDRESS % SECTOR_SIZE as u32 == 0);
    assert!(PATCH_RECORD_SIZE + BLOCK_HEADER_LEN <= SECTOR_SIZE);
    assert!(PATCH_BLOCK_COUNT < INVALID_BLOCK as usize);
    assert!(STORAGE_END <= FACTORY_BANK_ADDRESS);
    assert!(STORAGE_END <= QSPI_SIZE_BYTES);
};

#[cfg(target_arch = "arm")]
const _: () = {
    assert!(CATALOG_A_ADDRESS == embassy_daisy::qspi::APPLICATION_RESERVED_END);
    assert!(SECTOR_SIZE as u32 == embassy_daisy::qspi::SECTOR_SIZE);
    assert!(PAGE_SIZE == embassy_daisy::qspi::PAGE_SIZE);
    assert!(QSPI_SIZE_BYTES == embassy_daisy::qspi::SIZE_BYTES);
};

const BLOCK_MAGIC: [u8; 4] = *b"NMBK";
const BLOCK_VERSION: u8 = 1;
const BLOCK_HEADER_LEN: usize = 32;
const BLOCK_COMMIT_OFFSET: usize = 24;
const COMMITTED: u8 = 0;

const INDEX_MAGIC: [u8; 4] = *b"NMIX";
const INDEX_VERSION: u8 = 1;
const INDEX_HEADER_LEN: usize = 32;
const INDEX_COMMIT_OFFSET: usize = 24;
const INDEX_TABLE_OFFSET: usize = INDEX_HEADER_LEN;
const INDEX_TABLE_LEN: usize = SLOT_COUNT * 2;
const INDEX_JOURNAL_OFFSET: usize = INDEX_TABLE_OFFSET + INDEX_TABLE_LEN;
const INDEX_ENTRY_LEN: usize = 12;
const INDEX_ENTRY_COMMIT_OFFSET: usize = 10;
const INDEX_ENTRY_POINTER: u8 = 1;
const INDEX_ENTRY_SELECTION: u8 = 2;

#[cfg(not(test))]
static mut SECTOR_BUFFER: [u8; SECTOR_SIZE] = [0u8; SECTOR_SIZE];

#[cfg(test)]
std::thread_local! {
    static TEST_SECTOR_BUFFER: core::cell::RefCell<[u8; SECTOR_SIZE]> =
        const { core::cell::RefCell::new([0; SECTOR_SIZE]) };
}

#[cfg(not(test))]
fn with_sector_buffer<R>(f: impl FnOnce(&mut [u8; SECTOR_SIZE]) -> R) -> R {
    #[allow(static_mut_refs)]
    f(unsafe { &mut SECTOR_BUFFER })
}

#[cfg(test)]
fn with_sector_buffer<R>(f: impl FnOnce(&mut [u8; SECTOR_SIZE]) -> R) -> R {
    TEST_SECTOR_BUFFER.with(|buffer| f(&mut buffer.borrow_mut()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitStatus {
    Opened,
    Recovered,
    Formatted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgramStoreError<E> {
    Flash(E),
    InvalidAddress,
    Record(PatchRecordError),
    VerifyFailed,
    NoSpareBlock,
    CorruptIndex,
}

impl<E> From<PatchRecordError> for ProgramStoreError<E> {
    fn from(error: PatchRecordError) -> Self {
        Self::Record(error)
    }
}

pub struct ProgramStore<F> {
    flash: F,
    index: [u16; SLOT_COUNT],
    spare_block: u16,
    last_bank: u8,
    last_program: u8,
    epoch: u32,
    next_journal_offset: usize,
    active_catalog_address: u32,
}

impl<F: ProgramFlash> ProgramStore<F> {
    pub fn open(mut flash: F) -> Result<(Self, InitStatus), ProgramStoreError<F::Error>> {
        let first = read_index(&mut flash, CATALOG_A_ADDRESS)?;
        let second = read_index(&mut flash, CATALOG_B_ADDRESS)?;
        let selected = match (first, second) {
            (Some(a), Some(b)) => {
                if generation_newer(b.epoch, a.epoch) {
                    Some(b)
                } else {
                    Some(a)
                }
            }
            (Some(index), None) | (None, Some(index)) => Some(index),
            (None, None) => None,
        };

        if let Some(snapshot) = selected {
            if let Some(spare_block) = validate_index(&mut flash, &snapshot.index)? {
                ensure_erased(&mut flash, block_address(spare_block))?;
                return Ok((
                    Self {
                        flash,
                        index: snapshot.index,
                        spare_block,
                        last_bank: snapshot.bank,
                        last_program: snapshot.program,
                        epoch: snapshot.epoch,
                        next_journal_offset: snapshot.next_journal_offset,
                        active_catalog_address: snapshot.address,
                    },
                    InitStatus::Opened,
                ));
            }
        }

        recover_blocks(flash)
    }

    pub fn last_load(&self) -> (u8, u8) {
        (self.last_bank, self.last_program)
    }

    pub fn load(&mut self, bank: u8, program: u8) -> Result<Patch, ProgramStoreError<F::Error>> {
        let logical = logical_slot(bank, program)?;
        let block = self.index[logical];
        read_patch(&mut self.flash, block, logical as u16)
    }

    pub fn save(
        &mut self,
        bank: u8,
        program: u8,
        patch: &Patch,
    ) -> Result<(), ProgramStoreError<F::Error>> {
        let logical = logical_slot(bank, program)?;
        let old_block = self.index[logical];
        let generation = read_block_header(&mut self.flash, old_block)?
            .filter(|header| header.logical_slot == logical as u16)
            .map_or(1, |header| header.generation.wrapping_add(1));
        let new_block = self.spare_block;

        with_sector_buffer(|sector| {
            sector.fill(0xff);
            let record: &mut [u8; PATCH_RECORD_SIZE] = (&mut sector
                [BLOCK_HEADER_LEN..BLOCK_HEADER_LEN + PATCH_RECORD_SIZE])
                .try_into()
                .unwrap();
            PatchRecord::encode(patch, record)?;
            let header = BlockHeader {
                logical_slot: logical as u16,
                generation,
                payload_len: PATCH_RECORD_SIZE as u16,
                payload_crc: synth_core::patch_storage::crc32(record),
            };
            sector[..BLOCK_HEADER_LEN].copy_from_slice(&header.encode_uncommitted());

            self.flash
                .erase_sector(block_address(new_block))
                .map_err(ProgramStoreError::Flash)?;
            program_pages(
                &mut self.flash,
                block_address(new_block),
                &sector[..BLOCK_HEADER_LEN + PATCH_RECORD_SIZE],
            )?;
            verify_bytes(
                &mut self.flash,
                block_address(new_block),
                &sector[..BLOCK_HEADER_LEN + PATCH_RECORD_SIZE],
            )?;
            commit_byte(
                &mut self.flash,
                block_address(new_block) + BLOCK_COMMIT_OFFSET as u32,
            )?;
            Ok::<(), ProgramStoreError<F::Error>>(())
        })?;

        self.ensure_journal_space()?;
        let entry = IndexEntry::pointer(logical as u16, new_block);
        self.append_index_entry(entry)?;
        self.index[logical] = new_block;

        self.flash
            .erase_sector(block_address(old_block))
            .map_err(ProgramStoreError::Flash)?;
        self.spare_block = old_block;
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
        self.ensure_journal_space()?;
        self.append_index_entry(IndexEntry::selection(bank, program))?;
        self.last_bank = bank;
        self.last_program = program;
        Ok(())
    }

    fn ensure_journal_space(&mut self) -> Result<(), ProgramStoreError<F::Error>> {
        if self.next_journal_offset + INDEX_ENTRY_LEN <= SECTOR_SIZE {
            return Ok(());
        }
        let alternate = other_catalog(self.active_catalog_address);
        let epoch = self.epoch.wrapping_add(1);
        write_index(
            &mut self.flash,
            alternate,
            epoch,
            self.last_bank,
            self.last_program,
            &self.index,
        )?;
        self.active_catalog_address = alternate;
        self.epoch = epoch;
        self.next_journal_offset = INDEX_JOURNAL_OFFSET;
        self.flash
            .erase_sector(other_catalog(alternate))
            .map_err(ProgramStoreError::Flash)?;
        Ok(())
    }

    fn append_index_entry(&mut self, entry: IndexEntry) -> Result<(), ProgramStoreError<F::Error>> {
        let address = self.active_catalog_address + self.next_journal_offset as u32;
        let encoded = entry.encode_uncommitted();
        program_pages(&mut self.flash, address, &encoded)?;
        verify_bytes(&mut self.flash, address, &encoded)?;
        commit_byte(&mut self.flash, address + INDEX_ENTRY_COMMIT_OFFSET as u32)?;
        self.next_journal_offset += INDEX_ENTRY_LEN;
        Ok(())
    }

    #[cfg(test)]
    fn into_flash(self) -> F {
        self.flash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlockHeader {
    logical_slot: u16,
    generation: u32,
    payload_len: u16,
    payload_crc: u32,
}

impl BlockHeader {
    fn encode_uncommitted(self) -> [u8; BLOCK_HEADER_LEN] {
        let mut bytes = [0xff; BLOCK_HEADER_LEN];
        bytes[..4].copy_from_slice(&BLOCK_MAGIC);
        bytes[4] = BLOCK_VERSION;
        bytes[6..8].copy_from_slice(&self.logical_slot.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.generation.to_le_bytes());
        bytes[12..14].copy_from_slice(&self.payload_len.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.payload_crc.to_le_bytes());
        let checksum = synth_core::patch_storage::crc32(&bytes[..20]);
        bytes[20..24].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8; BLOCK_HEADER_LEN]) -> Option<Self> {
        if bytes[..4] != BLOCK_MAGIC
            || bytes[4] != BLOCK_VERSION
            || bytes[BLOCK_COMMIT_OFFSET] != COMMITTED
            || synth_core::patch_storage::crc32(&bytes[..20])
                != u32::from_le_bytes(bytes[20..24].try_into().ok()?)
        {
            return None;
        }
        let payload_len = u16::from_le_bytes(bytes[12..14].try_into().ok()?);
        if payload_len != 0 && usize::from(payload_len) != PATCH_RECORD_SIZE {
            return None;
        }
        Some(Self {
            logical_slot: u16::from_le_bytes(bytes[6..8].try_into().ok()?),
            generation: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
            payload_len,
            payload_crc: u32::from_le_bytes(bytes[16..20].try_into().ok()?),
        })
    }
}

struct IndexSnapshot {
    address: u32,
    epoch: u32,
    bank: u8,
    program: u8,
    index: [u16; SLOT_COUNT],
    next_journal_offset: usize,
}

#[derive(Clone, Copy)]
struct IndexEntry {
    kind: u8,
    first: u8,
    second: u16,
    third: u16,
}

impl IndexEntry {
    const fn pointer(logical: u16, block: u16) -> Self {
        Self {
            kind: INDEX_ENTRY_POINTER,
            first: 0,
            second: logical,
            third: block,
        }
    }

    const fn selection(bank: u8, program: u8) -> Self {
        Self {
            kind: INDEX_ENTRY_SELECTION,
            first: bank,
            second: program as u16,
            third: INVALID_BLOCK,
        }
    }

    fn encode_uncommitted(self) -> [u8; INDEX_ENTRY_LEN] {
        let mut bytes = [0xff; INDEX_ENTRY_LEN];
        bytes[0] = self.kind;
        bytes[1] = self.first;
        bytes[2..4].copy_from_slice(&self.second.to_le_bytes());
        bytes[4..6].copy_from_slice(&self.third.to_le_bytes());
        let checksum = synth_core::patch_storage::crc32(&bytes[..6]);
        bytes[6..10].copy_from_slice(&checksum.to_le_bytes());
        bytes
    }

    fn decode(bytes: &[u8; INDEX_ENTRY_LEN]) -> Option<Self> {
        if bytes[INDEX_ENTRY_COMMIT_OFFSET] != COMMITTED
            || synth_core::patch_storage::crc32(&bytes[..6])
                != u32::from_le_bytes(bytes[6..10].try_into().ok()?)
        {
            return None;
        }
        let entry = Self {
            kind: bytes[0],
            first: bytes[1],
            second: u16::from_le_bytes(bytes[2..4].try_into().ok()?),
            third: u16::from_le_bytes(bytes[4..6].try_into().ok()?),
        };
        match entry.kind {
            INDEX_ENTRY_POINTER
                if usize::from(entry.second) < SLOT_COUNT
                    && usize::from(entry.third) < PATCH_BLOCK_COUNT =>
            {
                Some(entry)
            }
            INDEX_ENTRY_SELECTION
                if entry.first < BANK_COUNT && entry.second < u16::from(PROGRAMS_PER_BANK) =>
            {
                Some(entry)
            }
            _ => None,
        }
    }
}

fn read_index<F: ProgramFlash>(
    flash: &mut F,
    address: u32,
) -> Result<Option<IndexSnapshot>, ProgramStoreError<F::Error>> {
    with_sector_buffer(|sector| {
        flash
            .read(address, sector)
            .map_err(ProgramStoreError::Flash)?;
        if sector[..4] != INDEX_MAGIC
            || sector[4] != INDEX_VERSION
            || sector[INDEX_COMMIT_OFFSET] != COMMITTED
            || u16::from_le_bytes([sector[12], sector[13]]) != SLOT_COUNT as u16
            || u16::from_le_bytes([sector[14], sector[15]]) != PATCH_BLOCK_COUNT as u16
            || synth_core::patch_storage::crc32(&sector[..20])
                != u32::from_le_bytes(sector[20..24].try_into().unwrap())
            || synth_core::patch_storage::crc32(
                &sector[INDEX_TABLE_OFFSET..INDEX_TABLE_OFFSET + INDEX_TABLE_LEN],
            ) != u32::from_le_bytes(sector[16..20].try_into().unwrap())
        {
            return Ok(None);
        }
        let bank = sector[5];
        let program = sector[6];
        if bank >= BANK_COUNT || program >= PROGRAMS_PER_BANK {
            return Ok(None);
        }
        let mut index = [INVALID_BLOCK; SLOT_COUNT];
        for (logical, block) in index.iter_mut().enumerate() {
            let offset = INDEX_TABLE_OFFSET + logical * 2;
            *block = u16::from_le_bytes([sector[offset], sector[offset + 1]]);
        }
        let mut snapshot = IndexSnapshot {
            address,
            epoch: u32::from_le_bytes(sector[8..12].try_into().unwrap()),
            bank,
            program,
            index,
            next_journal_offset: INDEX_JOURNAL_OFFSET,
        };
        replay_journal(&mut snapshot, sector);
        Ok(Some(snapshot))
    })
}

fn replay_journal(snapshot: &mut IndexSnapshot, sector: &[u8; SECTOR_SIZE]) {
    let mut offset = INDEX_JOURNAL_OFFSET;
    while offset + INDEX_ENTRY_LEN <= SECTOR_SIZE {
        let bytes: &[u8; INDEX_ENTRY_LEN] =
            sector[offset..offset + INDEX_ENTRY_LEN].try_into().unwrap();
        if bytes.iter().all(|byte| *byte == 0xff) {
            snapshot.next_journal_offset = offset;
            return;
        }
        let Some(entry) = IndexEntry::decode(bytes) else {
            snapshot.next_journal_offset = SECTOR_SIZE;
            return;
        };
        if entry.kind == INDEX_ENTRY_POINTER {
            snapshot.index[usize::from(entry.second)] = entry.third;
        } else {
            snapshot.bank = entry.first;
            snapshot.program = entry.second as u8;
        }
        offset += INDEX_ENTRY_LEN;
    }
    snapshot.next_journal_offset = SECTOR_SIZE;
}

fn write_index<F: ProgramFlash>(
    flash: &mut F,
    address: u32,
    epoch: u32,
    bank: u8,
    program: u8,
    index: &[u16; SLOT_COUNT],
) -> Result<(), ProgramStoreError<F::Error>> {
    with_sector_buffer(|sector| {
        sector.fill(0xff);
        sector[..4].copy_from_slice(&INDEX_MAGIC);
        sector[4] = INDEX_VERSION;
        sector[5] = bank;
        sector[6] = program;
        sector[8..12].copy_from_slice(&epoch.to_le_bytes());
        sector[12..14].copy_from_slice(&(SLOT_COUNT as u16).to_le_bytes());
        sector[14..16].copy_from_slice(&(PATCH_BLOCK_COUNT as u16).to_le_bytes());
        for (logical, block) in index.iter().copied().enumerate() {
            let offset = INDEX_TABLE_OFFSET + logical * 2;
            sector[offset..offset + 2].copy_from_slice(&block.to_le_bytes());
        }
        let table_crc = synth_core::patch_storage::crc32(
            &sector[INDEX_TABLE_OFFSET..INDEX_TABLE_OFFSET + INDEX_TABLE_LEN],
        );
        sector[16..20].copy_from_slice(&table_crc.to_le_bytes());
        let header_crc = synth_core::patch_storage::crc32(&sector[..20]);
        sector[20..24].copy_from_slice(&header_crc.to_le_bytes());

        flash
            .erase_sector(address)
            .map_err(ProgramStoreError::Flash)?;
        program_pages(flash, address, &sector[..INDEX_JOURNAL_OFFSET])?;
        verify_bytes(flash, address, &sector[..INDEX_JOURNAL_OFFSET])?;
        commit_byte(flash, address + INDEX_COMMIT_OFFSET as u32)
    })
}

fn validate_index<F: ProgramFlash>(
    flash: &mut F,
    index: &[u16; SLOT_COUNT],
) -> Result<Option<u16>, ProgramStoreError<F::Error>> {
    let mut referenced = [false; PATCH_BLOCK_COUNT];
    for (logical, block) in index.iter().copied().enumerate() {
        let physical = usize::from(block);
        if physical >= PATCH_BLOCK_COUNT || referenced[physical] {
            return Ok(None);
        }
        let Some(header) = read_valid_block(flash, block)? else {
            return Ok(None);
        };
        if header.logical_slot != logical as u16 {
            return Ok(None);
        }
        referenced[physical] = true;
    }
    let mut spare = None;
    for (block, used) in referenced.into_iter().enumerate() {
        if !used {
            if spare.is_some() {
                return Ok(None);
            }
            spare = Some(block as u16);
        }
    }
    Ok(spare)
}

fn recover_blocks<F: ProgramFlash>(
    mut flash: F,
) -> Result<(ProgramStore<F>, InitStatus), ProgramStoreError<F::Error>> {
    let mut index = [INVALID_BLOCK; SLOT_COUNT];
    let mut generations = [0_u32; SLOT_COUNT];
    let mut found_valid = false;
    for physical in 0..PATCH_BLOCK_COUNT {
        if let Some(header) = read_valid_block(&mut flash, physical as u16)? {
            let logical = usize::from(header.logical_slot);
            if logical >= SLOT_COUNT {
                continue;
            }
            found_valid = true;
            if index[logical] == INVALID_BLOCK
                || generation_newer(header.generation, generations[logical])
            {
                index[logical] = physical as u16;
                generations[logical] = header.generation;
            }
        }
    }

    let mut selected = [false; PATCH_BLOCK_COUNT];
    for block in index
        .iter()
        .copied()
        .filter(|block| *block != INVALID_BLOCK)
    {
        selected[usize::from(block)] = true;
    }
    let mut candidate = 0;
    for logical in 0..SLOT_COUNT {
        if index[logical] != INVALID_BLOCK {
            continue;
        }
        while candidate < PATCH_BLOCK_COUNT && selected[candidate] {
            candidate += 1;
        }
        if candidate == PATCH_BLOCK_COUNT {
            return Err(ProgramStoreError::NoSpareBlock);
        }
        write_default_block(&mut flash, candidate as u16, logical as u16, 1)?;
        index[logical] = candidate as u16;
        selected[candidate] = true;
    }
    let spare = selected
        .iter()
        .position(|selected| !selected)
        .ok_or(ProgramStoreError::NoSpareBlock)? as u16;
    ensure_erased(&mut flash, block_address(spare))?;

    write_index(&mut flash, CATALOG_A_ADDRESS, 1, 0, 0, &index)?;
    flash
        .erase_sector(CATALOG_B_ADDRESS)
        .map_err(ProgramStoreError::Flash)?;
    Ok((
        ProgramStore {
            flash,
            index,
            spare_block: spare,
            last_bank: 0,
            last_program: 0,
            epoch: 1,
            next_journal_offset: INDEX_JOURNAL_OFFSET,
            active_catalog_address: CATALOG_A_ADDRESS,
        },
        if found_valid {
            InitStatus::Recovered
        } else {
            InitStatus::Formatted
        },
    ))
}

fn write_default_block<F: ProgramFlash>(
    flash: &mut F,
    block: u16,
    logical: u16,
    generation: u32,
) -> Result<(), ProgramStoreError<F::Error>> {
    let header = BlockHeader {
        logical_slot: logical,
        generation,
        payload_len: 0,
        payload_crc: synth_core::patch_storage::crc32(&[]),
    }
    .encode_uncommitted();
    let address = block_address(block);
    flash
        .erase_sector(address)
        .map_err(ProgramStoreError::Flash)?;
    flash
        .program(address, &header)
        .map_err(ProgramStoreError::Flash)?;
    verify_bytes(flash, address, &header)?;
    commit_byte(flash, address + BLOCK_COMMIT_OFFSET as u32)
}

fn read_patch<F: ProgramFlash>(
    flash: &mut F,
    block: u16,
    logical: u16,
) -> Result<Patch, ProgramStoreError<F::Error>> {
    let header = read_block_header(flash, block)?
        .filter(|header| header.logical_slot == logical)
        .ok_or(ProgramStoreError::CorruptIndex)?;
    if header.payload_len == 0 {
        return Ok(Patch::default());
    }
    with_sector_buffer(|sector| {
        let record = &mut sector[..PATCH_RECORD_SIZE];
        flash
            .read(block_address(block) + BLOCK_HEADER_LEN as u32, record)
            .map_err(ProgramStoreError::Flash)?;
        if synth_core::patch_storage::crc32(record) != header.payload_crc {
            return Err(ProgramStoreError::Record(
                PatchRecordError::ChecksumMismatch,
            ));
        }
        let record: &[u8; PATCH_RECORD_SIZE] = (&*record).try_into().unwrap();
        PatchRecord::decode(record).map_err(ProgramStoreError::Record)
    })
}

fn read_valid_block<F: ProgramFlash>(
    flash: &mut F,
    block: u16,
) -> Result<Option<BlockHeader>, ProgramStoreError<F::Error>> {
    let Some(header) = read_block_header(flash, block)? else {
        return Ok(None);
    };
    if usize::from(header.logical_slot) >= SLOT_COUNT {
        return Ok(None);
    }
    if header.payload_len == 0 {
        return Ok((header.payload_crc == synth_core::patch_storage::crc32(&[])).then_some(header));
    }
    let valid = with_sector_buffer(|sector| {
        let record = &mut sector[..PATCH_RECORD_SIZE];
        flash
            .read(block_address(block) + BLOCK_HEADER_LEN as u32, record)
            .map_err(ProgramStoreError::Flash)?;
        if synth_core::patch_storage::crc32(record) != header.payload_crc {
            return Ok(false);
        }
        let record: &[u8; PATCH_RECORD_SIZE] = (&*record).try_into().unwrap();
        Ok::<bool, ProgramStoreError<F::Error>>(PatchRecord::decode(record).is_ok())
    })?;
    Ok(valid.then_some(header))
}

fn read_block_header<F: ProgramFlash>(
    flash: &mut F,
    block: u16,
) -> Result<Option<BlockHeader>, ProgramStoreError<F::Error>> {
    if usize::from(block) >= PATCH_BLOCK_COUNT {
        return Ok(None);
    }
    let mut bytes = [0; BLOCK_HEADER_LEN];
    flash
        .read(block_address(block), &mut bytes)
        .map_err(ProgramStoreError::Flash)?;
    Ok(BlockHeader::decode(&bytes))
}

fn program_pages<F: ProgramFlash>(
    flash: &mut F,
    mut address: u32,
    mut data: &[u8],
) -> Result<(), ProgramStoreError<F::Error>> {
    while !data.is_empty() {
        let remaining = PAGE_SIZE - address as usize % PAGE_SIZE;
        let count = remaining.min(data.len());
        flash
            .program(address, &data[..count])
            .map_err(ProgramStoreError::Flash)?;
        address += count as u32;
        data = &data[count..];
    }
    Ok(())
}

fn verify_bytes<F: ProgramFlash>(
    flash: &mut F,
    address: u32,
    expected: &[u8],
) -> Result<(), ProgramStoreError<F::Error>> {
    let mut offset = 0;
    let mut buffer = [0; PAGE_SIZE];
    while offset < expected.len() {
        let count = (expected.len() - offset).min(PAGE_SIZE);
        flash
            .read(address + offset as u32, &mut buffer[..count])
            .map_err(ProgramStoreError::Flash)?;
        if buffer[..count] != expected[offset..offset + count] {
            return Err(ProgramStoreError::VerifyFailed);
        }
        offset += count;
    }
    Ok(())
}

fn commit_byte<F: ProgramFlash>(
    flash: &mut F,
    address: u32,
) -> Result<(), ProgramStoreError<F::Error>> {
    flash
        .program(address, &[COMMITTED])
        .map_err(ProgramStoreError::Flash)?;
    let mut verify = [0xff];
    flash
        .read(address, &mut verify)
        .map_err(ProgramStoreError::Flash)?;
    if verify[0] != COMMITTED {
        return Err(ProgramStoreError::VerifyFailed);
    }
    Ok(())
}

fn ensure_erased<F: ProgramFlash>(
    flash: &mut F,
    address: u32,
) -> Result<(), ProgramStoreError<F::Error>> {
    let erased = with_sector_buffer(|sector| {
        flash
            .read(address, sector)
            .map_err(ProgramStoreError::Flash)?;
        Ok::<bool, ProgramStoreError<F::Error>>(sector.iter().all(|byte| *byte == 0xff))
    })?;
    if !erased {
        flash
            .erase_sector(address)
            .map_err(ProgramStoreError::Flash)?;
    }
    Ok(())
}

const fn block_address(block: u16) -> u32 {
    PROGRAMS_ADDRESS + block as u32 * SECTOR_SIZE as u32
}

fn logical_slot<E>(bank: u8, program: u8) -> Result<usize, ProgramStoreError<E>> {
    validate_address(bank, program)?;
    Ok(usize::from(bank) * usize::from(PROGRAMS_PER_BANK) + usize::from(program))
}

fn validate_address<E>(bank: u8, program: u8) -> Result<(), ProgramStoreError<E>> {
    if bank >= BANK_COUNT || program >= PROGRAMS_PER_BANK {
        Err(ProgramStoreError::InvalidAddress)
    } else {
        Ok(())
    }
}

const fn other_catalog(address: u32) -> u32 {
    if address == CATALOG_A_ADDRESS {
        CATALOG_B_ADDRESS
    } else {
        CATALOG_A_ADDRESS
    }
}

const fn generation_newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::vec;
    use std::vec::Vec;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum MockError {
        OutOfBounds,
        Unaligned,
        CrossPage,
        NeedsErase,
        Injected,
    }

    #[derive(Clone)]
    struct MockFlash {
        state: Rc<RefCell<MockFlashState>>,
    }

    #[derive(Clone)]
    struct MockFlashState {
        bytes: Vec<u8>,
        erased: Vec<u32>,
        mutations: usize,
        fail_at: Option<usize>,
    }

    impl MockFlash {
        fn erased() -> Self {
            Self {
                state: Rc::new(RefCell::new(MockFlashState {
                    bytes: vec![0xff; STORAGE_END as usize],
                    erased: Vec::new(),
                    mutations: 0,
                    fail_at: None,
                })),
            }
        }

        fn deep_clone(&self) -> Self {
            Self {
                state: Rc::new(RefCell::new(self.state.borrow().clone())),
            }
        }

        fn inject_next(&self, offset: usize) {
            let mut state = self.state.borrow_mut();
            state.fail_at = Some(state.mutations + offset);
        }

        fn clear_failure(&self) {
            self.state.borrow_mut().fail_at = None;
        }

        fn range(
            state: &MockFlashState,
            address: u32,
            len: usize,
        ) -> Result<core::ops::Range<usize>, MockError> {
            let start = address as usize;
            let end = start.checked_add(len).ok_or(MockError::OutOfBounds)?;
            if end > state.bytes.len() {
                Err(MockError::OutOfBounds)
            } else {
                Ok(start..end)
            }
        }

        fn mutated(state: &mut MockFlashState) -> Result<(), MockError> {
            let operation = state.mutations;
            state.mutations += 1;
            if state.fail_at == Some(operation) {
                Err(MockError::Injected)
            } else {
                Ok(())
            }
        }
    }

    impl ProgramFlash for MockFlash {
        type Error = MockError;

        fn read(&mut self, address: u32, output: &mut [u8]) -> Result<(), Self::Error> {
            let state = self.state.borrow();
            let range = Self::range(&state, address, output.len())?;
            output.copy_from_slice(&state.bytes[range]);
            Ok(())
        }

        fn erase_sector(&mut self, address: u32) -> Result<(), Self::Error> {
            if address as usize % SECTOR_SIZE != 0 {
                return Err(MockError::Unaligned);
            }
            let mut state = self.state.borrow_mut();
            let range = Self::range(&state, address, SECTOR_SIZE)?;
            state.bytes[range].fill(0xff);
            state.erased.push(address);
            Self::mutated(&mut state)
        }

        fn program(&mut self, address: u32, data: &[u8]) -> Result<(), Self::Error> {
            if address as usize / PAGE_SIZE != (address as usize + data.len() - 1) / PAGE_SIZE {
                return Err(MockError::CrossPage);
            }
            let mut state = self.state.borrow_mut();
            let range = Self::range(&state, address, data.len())?;
            for (old, new) in state.bytes[range.clone()].iter().zip(data) {
                if old & new != *new {
                    return Err(MockError::NeedsErase);
                }
            }
            for (old, new) in state.bytes[range].iter_mut().zip(data) {
                *old &= *new;
            }
            Self::mutated(&mut state)
        }
    }

    fn named_patch(name: &str, cutoff: f32) -> Patch {
        let mut patch = Patch::default();
        patch.layer_a.name.push_str(name).unwrap();
        patch.layer_a.filter.cutoff = cutoff;
        patch.layer_a.sequence.sequencer_type = synth_core::SequencerType::Polyphonic;
        patch.layer_a.sequence.poly.steps[63].lanes[5] = synth_core::PolyLaneStep {
            note: synth_core::PolyNote::Tie,
            velocity: synth_core::PolyVelocity::Velocity(127),
        };
        patch
    }

    #[test]
    fn format_creates_1024_defaults_and_exactly_one_spare() {
        let (mut store, status) = ProgramStore::open(MockFlash::erased()).unwrap();
        assert_eq!(status, InitStatus::Formatted);
        assert_eq!(store.last_load(), (0, 0));
        assert_eq!(store.load(7, 127).unwrap().layer_a.name.as_str(), "");
        assert_eq!(usize::from(store.spare_block), SLOT_COUNT);
        assert_eq!(STORAGE_END, PROGRAMS_ADDRESS + 1025 * 4096);
    }

    #[test]
    fn every_interrupted_initialization_can_resume_safely() {
        let probe = MockFlash::erased();
        let before = probe.state.borrow().mutations;
        ProgramStore::open(probe.clone()).unwrap();
        let mutation_count = probe.state.borrow().mutations - before;

        for failure in 0..mutation_count {
            let flash = MockFlash::erased();
            flash.inject_next(failure);
            assert!(matches!(
                ProgramStore::open(flash.clone()),
                Err(ProgramStoreError::Flash(MockError::Injected))
            ));
            flash.clear_failure();
            let (mut reopened, status) = ProgramStore::open(flash).unwrap();
            assert!(matches!(
                status,
                InitStatus::Opened | InitStatus::Recovered | InitStatus::Formatted
            ));
            assert_eq!(reopened.load(0, 0).unwrap().layer_a.name.as_str(), "");
            assert_eq!(reopened.load(7, 127).unwrap().layer_a.name.as_str(), "");
        }
    }

    #[test]
    fn save_rotates_the_single_spare_and_preserves_other_programs() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let initial_spare = store.spare_block;
        store.save(0, 0, &named_patch("first", 1111.0)).unwrap();
        assert_eq!(store.spare_block, 0);
        store.save(0, 1, &named_patch("second", 2222.0)).unwrap();
        assert_eq!(store.spare_block, 1);
        store.save(0, 0, &named_patch("updated", 3333.0)).unwrap();
        assert_eq!(store.spare_block, initial_spare);
        assert_eq!(store.load(0, 0).unwrap().layer_a.name.as_str(), "updated");
        assert_eq!(store.load(0, 1).unwrap().layer_a.name.as_str(), "second");
        assert_eq!(
            store.load(0, 0).unwrap().layer_a.sequence.poly.steps[63].lanes[5].note,
            synth_core::PolyNote::Tie
        );
    }

    #[test]
    fn ordinary_save_has_a_fixed_two_erase_sixteen_program_budget() {
        let flash = MockFlash::erased();
        let (mut store, _) = ProgramStore::open(flash.clone()).unwrap();
        let before_mutations = flash.state.borrow().mutations;
        let before_erases = flash.state.borrow().erased.len();
        store.save(0, 0, &named_patch("budget", 1234.0)).unwrap();
        let state = flash.state.borrow();
        assert_eq!(state.mutations - before_mutations, 18);
        assert_eq!(state.erased.len() - before_erases, 2);
        assert_eq!(18 - 2, 16, "page/commit program operations");
    }

    #[test]
    fn repeated_saves_ping_pong_between_exactly_two_blocks() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let original = store.index[0];
        let original_spare = store.spare_block;
        for revision in 0..6 {
            store
                .save(0, 0, &named_patch("hot", 1000.0 + revision as f32))
                .unwrap();
            assert!(
                matches!(store.index[0], block if block == original || block == original_spare)
            );
            assert!(
                matches!(store.spare_block, block if block == original || block == original_spare)
            );
            assert_ne!(store.index[0], store.spare_block);
        }
    }

    #[test]
    fn every_interrupted_save_reopens_to_old_or_new_patch() {
        let (mut baseline_store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        baseline_store
            .save(0, 0, &named_patch("old", 1000.0))
            .unwrap();
        let baseline = baseline_store.into_flash();

        let probe = baseline.deep_clone();
        let before = probe.state.borrow().mutations;
        let (mut store, _) = ProgramStore::open(probe.clone()).unwrap();
        store.save(0, 0, &named_patch("new", 2000.0)).unwrap();
        let mutation_count = probe.state.borrow().mutations - before;

        for failure in 0..mutation_count {
            let flash = baseline.deep_clone();
            flash.inject_next(failure);
            let (mut store, _) = ProgramStore::open(flash.clone()).unwrap();
            assert!(store.save(0, 0, &named_patch("new", 2000.0)).is_err());
            flash.clear_failure();
            let (mut reopened, _) = ProgramStore::open(flash).unwrap();
            let name = reopened.load(0, 0).unwrap().layer_a.name;
            assert!(
                matches!(name.as_str(), "old" | "new"),
                "failure {failure}: {name}"
            );
            assert!(reopened.load(0, 1).unwrap().layer_a.name.is_empty());
        }
    }

    #[test]
    fn both_lost_indexes_rebuild_from_committed_block_headers() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        store.save(3, 9, &named_patch("recover", 4321.0)).unwrap();
        let flash = store.into_flash();
        {
            let mut state = flash.state.borrow_mut();
            state.bytes[CATALOG_A_ADDRESS as usize..CATALOG_A_ADDRESS as usize + SECTOR_SIZE]
                .fill(0);
            state.bytes[CATALOG_B_ADDRESS as usize..CATALOG_B_ADDRESS as usize + SECTOR_SIZE]
                .fill(0);
        }
        let (mut reopened, status) = ProgramStore::open(flash).unwrap();
        assert_eq!(status, InitStatus::Recovered);
        assert_eq!(
            reopened.load(3, 9).unwrap().layer_a.name.as_str(),
            "recover"
        );
        assert_eq!(reopened.load(7, 127).unwrap().layer_a.name.as_str(), "");
    }

    #[test]
    fn torn_index_entry_is_ignored_and_compacted_on_next_write() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let address = store.active_catalog_address + store.next_journal_offset as u32;
        let torn = IndexEntry::selection(7, 127).encode_uncommitted();
        store.flash.program(address, &torn).unwrap();
        let (mut reopened, status) = ProgramStore::open(store.into_flash()).unwrap();
        assert_eq!(status, InitStatus::Opened);
        assert_eq!(reopened.last_load(), (0, 0));
        assert_eq!(reopened.next_journal_offset, SECTOR_SIZE);
        reopened.persist_last_load(6, 126).unwrap();
        assert_eq!(reopened.last_load(), (6, 126));
    }

    #[test]
    fn selection_and_index_compaction_survive_reopen() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let capacity = (SECTOR_SIZE - INDEX_JOURNAL_OFFSET) / INDEX_ENTRY_LEN;
        for index in 0..=capacity {
            store
                .persist_last_load((index % 8) as u8, ((index + 1) % 128) as u8)
                .unwrap();
        }
        let expected = store.last_load();
        assert_eq!(store.active_catalog_address, CATALOG_B_ADDRESS);
        let (reopened, status) = ProgramStore::open(store.into_flash()).unwrap();
        assert_eq!(status, InitStatus::Opened);
        assert_eq!(reopened.last_load(), expected);
    }

    #[test]
    fn every_interrupted_index_compaction_reopens_to_prior_or_new_selection() {
        let (mut baseline_store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        let capacity = (SECTOR_SIZE - INDEX_JOURNAL_OFFSET) / INDEX_ENTRY_LEN;
        for index in 0..capacity {
            baseline_store
                .persist_last_load((index % 8) as u8, ((index + 1) % 128) as u8)
                .unwrap();
        }
        assert_eq!(
            baseline_store.next_journal_offset + INDEX_ENTRY_LEN > SECTOR_SIZE,
            true
        );
        let prior = baseline_store.last_load();
        let baseline = baseline_store.into_flash();

        let probe = baseline.deep_clone();
        let before = probe.state.borrow().mutations;
        let (mut store, _) = ProgramStore::open(probe.clone()).unwrap();
        store.persist_last_load(7, 127).unwrap();
        let mutation_count = probe.state.borrow().mutations - before;

        for failure in 0..mutation_count {
            let flash = baseline.deep_clone();
            flash.inject_next(failure);
            let (mut store, _) = ProgramStore::open(flash.clone()).unwrap();
            assert!(store.persist_last_load(7, 127).is_err());
            flash.clear_failure();
            let (reopened, _) = ProgramStore::open(flash).unwrap();
            assert!(matches!(reopened.last_load(), value if value == prior || value == (7, 127)));
        }
    }

    #[test]
    fn corrupt_payload_is_recovered_from_older_committed_duplicate() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        store.save(0, 0, &named_patch("old", 1000.0)).unwrap();
        let old_block = store.index[0];
        let spare = store.spare_block;
        // Manually retain an older duplicate before writing a corrupt newer block.
        let old_sector = {
            let state = store.flash.state.borrow();
            state.bytes
                [block_address(old_block) as usize..block_address(old_block) as usize + SECTOR_SIZE]
                .to_vec()
        };
        store.flash.erase_sector(block_address(spare)).unwrap();
        program_pages(&mut store.flash, block_address(spare), &old_sector).unwrap();
        store.flash.state.borrow_mut().bytes
            [block_address(store.index[0]) as usize + BLOCK_HEADER_LEN + 30] ^= 1;
        let flash = store.into_flash();
        flash.state.borrow_mut().bytes[CATALOG_A_ADDRESS as usize..CATALOG_A_ADDRESS as usize + 4]
            .fill(0);
        let (mut reopened, status) = ProgramStore::open(flash).unwrap();
        assert_eq!(status, InitStatus::Recovered);
        assert_eq!(reopened.load(0, 0).unwrap().layer_a.name.as_str(), "old");
    }

    #[test]
    fn invalid_addresses_are_rejected() {
        let (mut store, _) = ProgramStore::open(MockFlash::erased()).unwrap();
        assert!(matches!(
            store.load(8, 0),
            Err(ProgramStoreError::InvalidAddress)
        ));
        assert!(matches!(
            store.save(0, 128, &Patch::default()),
            Err(ProgramStoreError::InvalidAddress)
        ));
    }

    #[test]
    fn generation_comparison_is_wrap_safe() {
        assert!(generation_newer(0, u32::MAX));
        assert!(!generation_newer(u32::MAX, 0));
        assert!(!generation_newer(7, 7));
    }
}
