//! Global MIDI bank-selection state shared by CC0/CC32 host conventions.

use crate::program::store::BANK_COUNT;

pub struct ProgramSelection {
    current_bank: u8,
    cc0_pending: Option<u8>,
    cc32_pending: Option<u8>,
}

impl ProgramSelection {
    pub fn new(current_bank: u8) -> Self {
        Self {
            current_bank: if current_bank < BANK_COUNT {
                current_bank
            } else {
                0
            },
            cc0_pending: None,
            cc32_pending: None,
        }
    }

    /// Observe CC0 (controller=0) or CC32 (controller=32).
    /// Returns false for an out-of-range bank.
    pub fn bank_select(&mut self, controller: u8, value: u8) -> bool {
        if value >= BANK_COUNT {
            return false;
        }
        match controller {
            0 => self.cc0_pending = Some(value),
            32 => self.cc32_pending = Some(value),
            _ => unreachable!(),
        }
        true
    }

    pub fn requested_bank(&self) -> u8 {
        resolve(self.cc0_pending, self.cc32_pending, self.current_bank)
    }

    /// Commit only after the load request enters the storage queue.
    pub fn commit(&mut self) {
        let bank = resolve(self.cc0_pending, self.cc32_pending, self.current_bank);
        self.current_bank = bank;
        self.cc0_pending = None;
        self.cc32_pending = None;
    }

    pub fn current_bank(&self) -> u8 {
        self.current_bank
    }
}

fn resolve(cc0: Option<u8>, cc32: Option<u8>, fallback: u8) -> u8 {
    match (cc0, cc32) {
        (None, None) => fallback,
        (Some(v), None) if v == 0 => 0,
        (None, Some(v)) if v == 0 => 0,
        (Some(0), Some(0)) => 0,
        (Some(v), _) if v != 0 => v,
        (_, Some(v)) if v != 0 => v,
        _ => fallback,
    }
}

impl Default for ProgramSelection {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_cc0_or_cc32_style_pairs() {
        let mut selection = ProgramSelection::default();
        assert!(selection.bank_select(0, 5));
        assert!(selection.bank_select(32, 0));
        assert_eq!(selection.requested_bank(), 5);
        selection.commit();
        assert_eq!(selection.current_bank(), 5);

        assert!(selection.bank_select(0, 0));
        assert!(selection.bank_select(32, 3));
        assert_eq!(selection.requested_bank(), 3);
        selection.commit();
        assert_eq!(selection.current_bank(), 3);
    }

    #[test]
    fn zero_selects_bank_zero_and_invalid_values_do_not_replace_it() {
        let mut selection = ProgramSelection::default();
        selection.bank_select(0, 7);
        selection.commit();
        assert!(selection.bank_select(0, 0));
        assert!(!selection.bank_select(0, 8));
        assert_eq!(selection.requested_bank(), 0);
        selection.commit();
        assert_eq!(selection.current_bank(), 0);
    }

    #[test]
    fn pending_bank_remains_until_commit() {
        let mut selection = ProgramSelection::default();
        selection.bank_select(0, 6);
        assert_eq!(selection.requested_bank(), 6);
        assert_eq!(selection.requested_bank(), 6);
        assert_eq!(selection.current_bank(), 0);
        selection.commit();
        assert_eq!(selection.current_bank(), 6);
    }

    #[test]
    fn starts_from_persisted_bank() {
        let selection = ProgramSelection::new(7);
        assert_eq!(selection.requested_bank(), 7);
        assert_eq!(selection.current_bank(), 7);
    }

    #[test]
    fn single_controller_zero_reselects_bank_zero() {
        let mut selection = ProgramSelection::default();
        selection.bank_select(0, 3);
        selection.commit();
        assert_eq!(selection.current_bank(), 3);

        selection.bank_select(0, 0);
        assert_eq!(selection.requested_bank(), 0);
        selection.commit();
        assert_eq!(selection.current_bank(), 0);
    }

    #[test]
    fn first_controller_zero_then_second_nonzero_uses_nonzero() {
        let mut selection = ProgramSelection::default();
        selection.bank_select(0, 0);
        selection.bank_select(32, 5);
        assert_eq!(selection.requested_bank(), 5);
        selection.commit();
        assert_eq!(selection.current_bank(), 5);
    }
}
