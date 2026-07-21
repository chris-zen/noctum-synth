use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Lock-free fallback for releases that arrive while the performance FIFO is full.
pub struct PendingReleases {
    words: [AtomicU32; 4],
    all_notes_off: AtomicBool,
}

impl PendingReleases {
    pub const fn new() -> Self {
        Self {
            words: [const { AtomicU32::new(0) }; 4],
            all_notes_off: AtomicBool::new(false),
        }
    }

    pub fn note_off(&self, note: u8) {
        if note < 128 {
            self.words[usize::from(note / 32)].fetch_or(1u32 << (note % 32), Ordering::Release);
        }
    }

    pub fn all_notes_off(&self) {
        self.all_notes_off.store(true, Ordering::Release);
    }

    pub fn take_all_notes_off(&self) -> bool {
        self.all_notes_off.swap(false, Ordering::AcqRel)
    }

    pub fn take(&self) -> [u32; 4] {
        core::array::from_fn(|index| self.words[index].swap(0, Ordering::AcqRel))
    }
}
