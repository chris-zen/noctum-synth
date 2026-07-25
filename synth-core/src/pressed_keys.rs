use crate::patch::KeyMode;

#[derive(Clone)]
pub(crate) struct PressedKeys {
    order: [u16; 128],
    len: usize,
}

impl Default for PressedKeys {
    fn default() -> Self {
        Self {
            order: [0; 128],
            len: 0,
        }
    }
}

impl PressedKeys {
    pub(crate) fn press(&mut self, note: u8, velocity: f32) {
        if note >= 128 {
            return;
        }
        self.release(note);
        let velocity = (velocity.clamp(0.0, 1.0) * 127.0 + 0.5) as u16;
        self.order[self.len] = u16::from(note) << 8 | velocity;
        self.len += 1;
    }

    pub(crate) fn release(&mut self, note: u8) {
        let Some(index) = self.order[..self.len]
            .iter()
            .position(|held| (*held >> 8) as u8 == note)
        else {
            return;
        };
        self.order.copy_within(index + 1..self.len, index);
        self.len -= 1;
    }

    pub(crate) fn clear(&mut self) {
        self.len = 0;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn selected(&self, mode: KeyMode) -> Option<(u8, f32)> {
        let packed = match mode {
            KeyMode::Low | KeyMode::LowRetrigger => *self.order[..self.len]
                .iter()
                .min_by_key(|held| *held >> 8)?,
            KeyMode::High | KeyMode::HighRetrigger => *self.order[..self.len]
                .iter()
                .max_by_key(|held| *held >> 8)?,
            KeyMode::Last | KeyMode::LastRetrigger => *self.order[..self.len].last()?,
        };
        Some(((packed >> 8) as u8, f32::from(packed & 0x7f) / 127.0))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (u8, f32)> + '_ {
        self.order[..self.len]
            .iter()
            .copied()
            .map(|packed| ((packed >> 8) as u8, f32::from(packed & 0x7f) / 127.0))
    }
}
