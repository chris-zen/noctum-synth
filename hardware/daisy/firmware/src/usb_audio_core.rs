//! Host-testable buffering and PCM encoding for the firmware USB audio mirror.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub type Frame = (f32, f32);

pub const SAMPLE_RATE_HZ: usize = 48_000;
pub const CHANNELS: usize = 2;
pub const BYTES_PER_SAMPLE: usize = 3;
pub const NOMINAL_PACKET_FRAMES: usize = SAMPLE_RATE_HZ / 1_000;
pub const MIN_PACKET_FRAMES: usize = NOMINAL_PACKET_FRAMES - 1;
pub const MAX_PACKET_FRAMES: usize = NOMINAL_PACKET_FRAMES + 1;
pub const MAX_PACKET_BYTES: usize = MAX_PACKET_FRAMES * CHANNELS * BYTES_PER_SAMPLE;

const RING_FRAMES: usize = 256;
pub const PRIME_FRAMES: usize = 96;
const LOW_WATERMARK: usize = 80;
const HIGH_WATERMARK: usize = 112;
pub const STARTUP_FADE_FRAMES: usize = SAMPLE_RATE_HZ * 5 / 1_000;

/// Single-producer/single-consumer frame ring shared by audio and USB tasks.
pub struct UsbAudioBuffer {
    frames: [UnsafeCell<Frame>; RING_FRAMES],
    read: AtomicUsize,
    write: AtomicUsize,
    stream_active: AtomicBool,
}

// SAFETY: each slot is independently interior-mutable. The audio executor is
// the sole producer, the USB task is the sole consumer, and the release/acquire
// cursors ensure a slot is never read while it is being written.
unsafe impl Sync for UsbAudioBuffer {}

impl Default for UsbAudioBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbAudioBuffer {
    pub const fn new() -> Self {
        Self {
            frames: [const { UnsafeCell::new((0.0, 0.0)) }; RING_FRAMES],
            read: AtomicUsize::new(0),
            write: AtomicUsize::new(0),
            stream_active: AtomicBool::new(false),
        }
    }

    /// Copy rendered frames without ever waiting for the USB consumer.
    #[inline]
    pub fn push_block(&self, block: &[Frame]) {
        if !self.stream_active.load(Ordering::Acquire) {
            return;
        }

        let read = self.read.load(Ordering::Acquire);
        let write = self.write.load(Ordering::Relaxed);
        let free = (read + RING_FRAMES - write - 1) % RING_FRAMES;
        let count = block.len().min(free);

        for (offset, frame) in block.iter().copied().take(count).enumerate() {
            let index = (write + offset) % RING_FRAMES;
            // SAFETY: the producer exclusively owns every slot from `write`
            // up to the acquired consumer cursor. No reference to the slot is
            // materialized across this write.
            unsafe { self.frames[index].get().write(frame) };
        }
        self.write
            .store((write + count) % RING_FRAMES, Ordering::Release);
    }

    pub(crate) fn pop_into(&self, output: &mut [Frame]) -> usize {
        let write = self.write.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Relaxed);
        let available = (write + RING_FRAMES - read) % RING_FRAMES;
        let count = output.len().min(available);

        for (offset, target) in output.iter_mut().take(count).enumerate() {
            let index = (read + offset) % RING_FRAMES;
            // SAFETY: the acquired producer cursor publishes this slot, and
            // only the consumer reads it until the release store below.
            *target = unsafe { self.frames[index].get().read() };
        }
        self.read
            .store((read + count) % RING_FRAMES, Ordering::Release);
        count
    }

    pub(crate) fn occupancy(&self) -> usize {
        let write = self.write.load(Ordering::Acquire);
        let read = self.read.load(Ordering::Acquire);
        (write + RING_FRAMES - read) % RING_FRAMES
    }

    pub(crate) fn activate(&self) {
        self.stream_active.store(false, Ordering::Release);
        self.discard_queued();
        self.stream_active.store(true, Ordering::Release);
    }

    pub(crate) fn deactivate(&self) {
        self.stream_active.store(false, Ordering::Release);
        self.discard_queued();
    }

    fn discard_queued(&self) {
        let write = self.write.load(Ordering::Acquire);
        self.read.store(write, Ordering::Release);
    }
}

pub const fn packet_frames(primed: bool, occupancy: usize) -> usize {
    if !primed {
        NOMINAL_PACKET_FRAMES
    } else if occupancy < LOW_WATERMARK {
        MIN_PACKET_FRAMES
    } else if occupancy >= HIGH_WATERMARK {
        MAX_PACKET_FRAMES
    } else {
        NOMINAL_PACKET_FRAMES
    }
}

pub(crate) fn pad_to_silence(frames: &mut [Frame], valid: usize) {
    if valid >= frames.len() {
        return;
    }
    let (left, right) = valid
        .checked_sub(1)
        .and_then(|index| frames.get(index).copied())
        .unwrap_or((0.0, 0.0));
    let missing = frames.len() - valid;
    for (offset, frame) in frames[valid..].iter_mut().enumerate() {
        let gain = 1.0 - (offset + 1) as f32 / missing as f32;
        *frame = (left * gain, right * gain);
    }
}

pub(crate) fn encode_frames<'a>(
    frames: &[Frame],
    packet: &'a mut [u8; MAX_PACKET_BYTES],
) -> &'a [u8] {
    let length = frames.len() * CHANNELS * BYTES_PER_SAMPLE;
    for (frame, bytes) in frames.iter().zip(packet[..length].chunks_exact_mut(6)) {
        encode_sample(frame.0, &mut bytes[..3]);
        encode_sample(frame.1, &mut bytes[3..]);
    }
    &packet[..length]
}

fn encode_sample(sample: f32, bytes: &mut [u8]) {
    let value = (sample.clamp(-1.0, 1.0) * 8_388_607.0) as i32;
    bytes[0] = value as u8;
    bytes[1] = (value >> 8) as u8;
    bytes[2] = (value >> 16) as u8;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use super::*;

    #[test]
    fn ring_is_inactive_until_usb_stream_opens() {
        let ring = UsbAudioBuffer::new();
        ring.push_block(&[(0.5, -0.5); 32]);
        assert_eq!(ring.occupancy(), 0);

        ring.activate();
        ring.push_block(&[(0.5, -0.5); 32]);
        assert_eq!(ring.occupancy(), 32);
    }

    #[test]
    fn ring_wraps_without_reordering_frames() {
        let ring = UsbAudioBuffer::new();
        ring.activate();
        let mut next = 0.0;
        for _ in 0..20 {
            let mut block = [(0.0, 0.0); 32];
            for frame in &mut block {
                *frame = (next, -next);
                next += 1.0;
            }
            ring.push_block(&block);
            let mut output = [(0.0, 0.0); 32];
            assert_eq!(ring.pop_into(&mut output), output.len());
            let first = next - output.len() as f32;
            for (index, frame) in output.iter().copied().enumerate() {
                let expected = first + index as f32;
                assert_eq!(frame, (expected, -expected));
            }
        }
        assert_eq!(ring.occupancy(), 0);
    }

    #[test]
    fn concurrent_producer_and_consumer_preserve_order() {
        const FRAME_COUNT: usize = 100_000;
        let ring = Arc::new(UsbAudioBuffer::new());
        ring.activate();
        let producer_ring = Arc::clone(&ring);
        let producer = thread::spawn(move || {
            for value in 0..FRAME_COUNT {
                while producer_ring.occupancy() == RING_FRAMES - 1 {
                    thread::yield_now();
                }
                let value = value as f32;
                producer_ring.push_block(&[(value, -value)]);
            }
        });

        let mut frame = [(0.0, 0.0); 1];
        for expected in 0..FRAME_COUNT {
            while ring.pop_into(&mut frame) == 0 {
                thread::yield_now();
            }
            let expected = expected as f32;
            assert_eq!(frame[0], (expected, -expected));
        }
        producer.join().unwrap();
    }

    #[test]
    fn packet_policy_corrects_both_sides_of_target() {
        assert_eq!(packet_frames(false, HIGH_WATERMARK + 1), 48);
        assert_eq!(packet_frames(true, LOW_WATERMARK - 1), 47);
        assert_eq!(packet_frames(true, LOW_WATERMARK), 48);
        assert_eq!(packet_frames(true, HIGH_WATERMARK), 49);
    }

    #[test]
    fn encodes_packed_24_bit_stereo_little_endian() {
        let mut packet = [0u8; MAX_PACKET_BYTES];
        let encoded = encode_frames(&[(-1.0, 1.0), (0.0, 0.5)], &mut packet);
        assert_eq!(
            encoded,
            &[
                0x01, 0x00, 0x80, 0xff, 0xff, 0x7f, 0, 0, 0, 0xff, 0xff, 0x3f
            ]
        );
    }

    #[test]
    fn underflow_padding_reaches_silence() {
        let mut frames = [(1.0, -1.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)];
        pad_to_silence(&mut frames, 1);
        assert_eq!(frames[3], (0.0, -0.0));
        assert!(frames[1].0 > frames[2].0);
    }
}
