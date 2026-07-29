//! MIDI program storage, bank selection, and boot bring-up.

pub mod selection;
pub mod store;

#[cfg(target_arch = "arm")]
pub mod task;

#[cfg(target_arch = "arm")]
pub use task::run_task;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use synth_core::Patch;

pub use selection::ProgramSelection;

pub const STORAGE_QUEUE_CAPACITY: usize = 8;

#[derive(Clone, Debug)]
pub enum ProgramStorageRequest {
    Load { bank: u8, program: u8 },
    Save { bank: u8, program: u8, patch: Patch },
}

pub type ProgramStorageQueue =
    Channel<CriticalSectionRawMutex, ProgramStorageRequest, STORAGE_QUEUE_CAPACITY>;

#[cfg(target_arch = "arm")]
use embassy_daisy::qspi::QspiFlash;
#[cfg(target_arch = "arm")]
use store::{InitStatus, ProgramStore};

#[cfg(target_arch = "arm")]
pub fn init(
    resources: embassy_daisy::qspi::QspiFlashResources,
) -> (
    store::ProgramStore<embassy_daisy::qspi::QspiFlash>,
    synth_core::Patch,
    u8,
) {
    let mut qspi = QspiFlash::new(resources);
    let jedec_id = qspi.jedec_id();
    defmt::info!(
        "initialized QSPI JEDEC={:02x}:{:02x}:{:02x}",
        jedec_id[0],
        jedec_id[1],
        jedec_id[2]
    );
    if jedec_id != embassy_daisy::qspi::JEDEC_ID {
        crate::fatal("unexpected QSPI JEDEC identity");
    }
    let status = qspi.prepare_storage();
    defmt::debug!("QSPI status register {:02x}", status);
    let (mut program_store, init_status) = match ProgramStore::open(qspi) {
        Ok(store) => store,
        Err(store::ProgramStoreError::Flash(_)) => {
            crate::fatal("program storage flash operation failed");
        }
        Err(store::ProgramStoreError::VerifyFailed) => {
            crate::fatal("program storage verify failed");
        }
        Err(_) => crate::fatal("program storage initialization failed"),
    };
    match init_status {
        InitStatus::Opened => defmt::info!("opened existing MIDI program catalog"),
        InitStatus::Formatted => {
            defmt::warn!("formatted MIDI program catalog with default slots")
        }
    }
    let (last_bank, last_program) = program_store.last_load();
    let initial_patch = match program_store.load(last_bank, last_program) {
        Ok(patch) => patch,
        Err(_) => {
            defmt::error!(
                "stored boot program invalid bank={} program={}; using default",
                last_bank,
                last_program
            );
            synth_core::Patch::default()
        }
    };
    (program_store, initial_patch, last_bank)
}
