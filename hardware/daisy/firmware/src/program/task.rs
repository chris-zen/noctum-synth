//! Thread-mode owner for all runtime QSPI program operations.

use embassy_daisy::qspi::{FlashError, QspiFlash};

use crate::audio::PatchQueue;
use crate::diagnostics::{self, StorageFailureReason, StorageOperation};
use crate::program::{ProgramStorageQueue, ProgramStorageRequest};
use crate::program::store::{ProgramStore, ProgramStoreError};

#[embassy_executor::task]
pub async fn run_task(
    mut store: ProgramStore<QspiFlash>,
    requests: &'static ProgramStorageQueue,
    patches: &'static PatchQueue,
) -> ! {
    loop {
        let request = requests.receive().await;
        match &request {
            ProgramStorageRequest::Load { bank, program } => {
                let started = embassy_time::Instant::now().as_micros();
                let bank = *bank;
                let program = *program;
                match store.load(bank, program) {
                    Ok(patch) => {
                        if patches.try_send(patch).is_err() {
                            diagnostics::emit(diagnostics::Event::PatchQueueFull);
                        } else {
                            diagnostics::emit(diagnostics::Event::ProgramLoaded {
                                bank,
                                program,
                                elapsed_micros: embassy_time::Instant::now()
                                    .as_micros()
                                    .saturating_sub(started),
                            });
                            if let Err(error) = store.persist_last_load(bank, program) {
                                emit_failure(
                                    StorageOperation::PersistSelection,
                                    bank,
                                    program,
                                    error,
                                );
                            }
                        }
                    }
                    Err(error) => emit_failure(StorageOperation::Load, bank, program, error),
                }
            }
            ProgramStorageRequest::Save {
                bank,
                program,
                patch,
            } => {
                let bank = *bank;
                let program = *program;
                diagnostics::emit(diagnostics::Event::ProgramDataReceived { bank, program });
                match store.save(bank, program, patch) {
                    Ok(()) => diagnostics::emit(diagnostics::Event::ProgramSaved { bank, program }),
                    Err(error) => emit_failure(StorageOperation::Save, bank, program, error),
                }
            }
        }
    }
}

fn emit_failure(
    operation: StorageOperation,
    bank: u8,
    program: u8,
    error: ProgramStoreError<FlashError>,
) {
    let reason = match error {
        ProgramStoreError::Flash(_) => StorageFailureReason::Flash,
        ProgramStoreError::InvalidAddress => StorageFailureReason::InvalidAddress,
        ProgramStoreError::Record(_) => StorageFailureReason::InvalidRecord,
        ProgramStoreError::VerifyFailed => StorageFailureReason::VerifyFailed,
    };
    diagnostics::emit(diagnostics::Event::ProgramStorageFailed {
        operation,
        reason,
        bank,
        program,
    });
}
