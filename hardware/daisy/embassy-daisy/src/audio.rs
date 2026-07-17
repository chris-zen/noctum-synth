//! Audio transport for Daisy hardware.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_stm32::bind_interrupts;
use embassy_stm32::dma;
use embassy_stm32::i2c::{Config as I2cConfig, I2c};
use embassy_stm32::peripherals;
use embassy_stm32::sai::{
    self, BitOrder, ClockStrobe, DataSize, FifoThreshold, FrameSyncOffset, FrameSyncPolarity,
    MasterClockDivider, Mode, Sai, StereoMono, SyncInput, TxRx,
};

use crate::wm8731::Wm8731;

pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const BLOCK_LENGTH: usize = 32;
pub const CHANNELS: usize = 2;
pub const BLOCK_SAMPLES: usize = BLOCK_LENGTH * CHANNELS;

pub type Frame = (f32, f32);
pub type Block = [Frame; BLOCK_LENGTH];

/// Resources reserved for the onboard Seed 1.1 audio path.
pub struct AudioResources {
    pub(crate) sai: embassy_stm32::Peri<'static, peripherals::SAI1>,
    pub(crate) mclk: embassy_stm32::Peri<'static, peripherals::PE2>,
    pub(crate) sd_b: embassy_stm32::Peri<'static, peripherals::PE3>,
    pub(crate) fs: embassy_stm32::Peri<'static, peripherals::PE4>,
    pub(crate) sck: embassy_stm32::Peri<'static, peripherals::PE5>,
    pub(crate) sd_a: embassy_stm32::Peri<'static, peripherals::PE6>,
    pub(crate) codec_i2c: embassy_stm32::Peri<'static, peripherals::I2C2>,
    pub(crate) codec_scl: embassy_stm32::Peri<'static, peripherals::PH4>,
    pub(crate) codec_sda: embassy_stm32::Peri<'static, peripherals::PB11>,
    pub(crate) dma_a: embassy_stm32::Peri<'static, peripherals::DMA1_CH0>,
    pub(crate) dma_b: embassy_stm32::Peri<'static, peripherals::DMA1_CH1>,
}

#[derive(Debug)]
pub enum Error {
    BuffersAlreadyTaken,
    Codec(embassy_stm32::i2c::Error),
    SaiStart(sai::Error),
    SaiTransmit(sai::Error),
    SaiReceive(sai::Error),
}

impl Error {
    /// Stable diagnostic category that does not couple the BSP to a logging framework.
    pub const fn category(&self) -> &'static str {
        match self {
            Self::BuffersAlreadyTaken => "buffers-already-taken",
            Self::Codec(_) => "codec",
            Self::SaiStart(_) => "sai-start",
            Self::SaiTransmit(_) => "sai-transmit",
            Self::SaiReceive(_) => "sai-receive",
        }
    }
}

type SaiDriver = Sai<'static, peripherals::SAI1, u32>;

pub struct Audio<M: sealed::Mode> {
    tx: SaiDriver,
    rx: SaiDriver,
    _mode: PhantomData<M>,
}

impl Audio<sealed::Output> {
    pub fn output(resources: AudioResources) -> Result<Self, Error> {
        Self::open(resources)
    }

    /// Prime transmit DMA before enabling the master receive clock.
    ///
    /// SAI B is synchronous to SAI A, so its DMA can be armed safely while no
    /// frame clock is present. Starting the receiver afterwards prevents its
    /// ring buffer from filling while the first output block is prepared.
    pub async fn start(&mut self, initial_output: &Block) -> Result<(), Error> {
        let tx_words = AudioBuffer::from(initial_output);
        self.tx
            .write(tx_words.words())
            .await
            .map_err(Error::SaiTransmit)?;
        self.rx.start().map_err(Error::SaiStart)
    }

    /// Write one fixed-size stereo block to the codec.
    ///
    /// The master receive path is drained each block so its ring buffer cannot
    /// overrun, but incoming samples are not decoded.
    pub async fn transfer(&mut self, output: &Block) -> Result<(), Error> {
        let tx_words = AudioBuffer::from(output);
        let _rx_words = self.transfer_blocks(&tx_words).await?;
        Ok(())
    }
}

impl Audio<sealed::Input> {
    pub fn input(resources: AudioResources) -> Result<Self, Error> {
        Self::open(resources)
    }

    /// Prime transmit DMA with silence before enabling the master receive clock.
    pub async fn start(&mut self) -> Result<(), Error> {
        self.tx
            .write(SILENCE_WORDS.words())
            .await
            .map_err(Error::SaiTransmit)?;
        self.rx.start().map_err(Error::SaiStart)
    }

    /// Read one fixed-size stereo block from the codec.
    ///
    /// Silence is retransmitted each block so the synchronous DAC path cannot
    /// underrun. The encoded silence is constant, so only the DMA write is
    /// repeated; it is not re-encoded from `f32` every block.
    pub async fn transfer(&mut self, input: &mut Block) -> Result<(), Error> {
        let rx_words = self.transfer_blocks(&SILENCE_WORDS).await?;
        rx_words.decode_into(input);
        Ok(())
    }
}

impl Audio<sealed::Duplex> {
    pub fn duplex(resources: AudioResources) -> Result<Self, Error> {
        Self::open(resources)
    }

    /// Prime transmit DMA before enabling the master receive clock.
    ///
    /// SAI B is synchronous to SAI A, so its DMA can be armed safely while no
    /// frame clock is present. Starting the receiver afterwards prevents its
    /// ring buffer from filling while the first output block is prepared.
    pub async fn start(&mut self, initial_output: &Block) -> Result<(), Error> {
        let tx_words = AudioBuffer::from(initial_output);
        self.tx
            .write(tx_words.words())
            .await
            .map_err(Error::SaiTransmit)?;
        self.rx.start().map_err(Error::SaiStart)
    }

    /// Exchange one fixed-size stereo block with the codec.
    ///
    /// Output is written first so the SAI master begins generating clocks before
    /// the synchronous receive operation is awaited.
    pub async fn transfer(&mut self, output: &Block, input: &mut Block) -> Result<(), Error> {
        let tx_words = AudioBuffer::from(output);
        let rx_words = self.transfer_blocks(&tx_words).await?;
        rx_words.decode_into(input);
        Ok(())
    }
}

impl<M: sealed::Mode> Audio<M> {
    fn open(resources: AudioResources) -> Result<Self, Error> {
        let i2c = I2c::new_blocking(
            resources.codec_i2c,
            resources.codec_scl,
            resources.codec_sda,
            I2cConfig::default(),
        );
        let mut codec = Wm8731::new(i2c);

        let (sub_a, sub_b) = sai::split_subblocks(resources.sai);

        let mut master_rx = sai_config();
        master_rx.mode = Mode::Master;
        master_rx.tx_rx = TxRx::Receiver;
        master_rx.sync_output = true;
        master_rx.clock_strobe = ClockStrobe::Rising;

        let mut slave_tx = master_rx;
        slave_tx.mode = Mode::Slave;
        slave_tx.tx_rx = TxRx::Transmitter;
        slave_tx.sync_input = SyncInput::Internal;
        slave_tx.sync_output = false;

        let (rx_buffer, tx_buffer) = take_dma_buffers().ok_or(Error::BuffersAlreadyTaken)?;

        let rx = Sai::new_asynchronous_with_mclk(
            sub_a,
            resources.sck,
            resources.sd_a,
            resources.fs,
            resources.mclk,
            resources.dma_a,
            rx_buffer,
            Irqs,
            master_rx,
        );
        let tx = Sai::new_synchronous(
            sub_b,
            resources.sd_b,
            resources.dma_b,
            tx_buffer,
            Irqs,
            slave_tx,
        );

        // Configure SAI before activating the codec, matching the established
        // Daisy bring-up sequence: clocks and data framing exist when ACTIVE
        // is asserted in the WM8731.
        codec.start().map_err(Error::Codec)?;

        Ok(Self {
            tx,
            rx,
            _mode: PhantomData,
        })
    }

    async fn transfer_blocks(
        &mut self,
        tx_words: &FilledAudioBuffer,
    ) -> Result<FilledAudioBuffer, Error> {
        let transmit = self.tx.write(tx_words.words()).await;
        // Always service RX even if TX just underruns. Both Embassy rings
        // resynchronize themselves after an error; returning before this read
        // would let the still-running receiver overrun during TX recovery.
        let mut rx_words = EmptyAudioBuffer::empty();
        let receive = self.rx.read(rx_words.as_mut_slice()).await;

        transmit.map_err(Error::SaiTransmit)?;
        receive.map_err(Error::SaiReceive)?;
        Ok(rx_words.assume_filled())
    }
}

type FilledAudioBuffer = AudioBuffer<sealed::Filled>;
type EmptyAudioBuffer = AudioBuffer<sealed::Empty>;

struct AudioBuffer<S: sealed::AudioBufferState> {
    words: MaybeUninit<[u32; BLOCK_SAMPLES]>,
    _state: PhantomData<S>,
}

impl AudioBuffer<sealed::Filled> {
    fn from(block: &Block) -> Self {
        let mut buffer = AudioBuffer::<sealed::Empty>::empty();
        for (frame, pair) in block.iter().zip(buffer.as_mut_slice().chunks_exact_mut(2)) {
            // The Seed SAI wiring presents the right channel first in memory.
            let right = (frame.1.clamp(-1.0, 1.0) * 8_388_607.0) as i32 as u32;
            let left = (frame.0.clamp(-1.0, 1.0) * 8_388_607.0) as i32 as u32;
            pair[0] = right;
            pair[1] = left;
        }
        buffer.assume_filled()
    }

    const fn silence() -> Self {
        Self {
            words: MaybeUninit::new([0; BLOCK_SAMPLES]),
            _state: PhantomData,
        }
    }

    fn words(&self) -> &[u32; BLOCK_SAMPLES] {
        // SAFETY: `Filled` is only constructed from complete encodes or reads.
        unsafe { self.words.assume_init_ref() }
    }

    fn decode_into(&self, block: &mut Block) {
        for (pair, frame) in self.words().chunks_exact(2).zip(block.iter_mut()) {
            let left = ((pair[1] << 8) as i32) >> 8;
            let right = ((pair[0] << 8) as i32) >> 8;
            *frame = (left as f32 / 8_388_608.0, right as f32 / 8_388_608.0);
        }
    }
}

impl AudioBuffer<sealed::Empty> {
    const fn empty() -> Self {
        Self {
            words: MaybeUninit::uninit(),
            _state: PhantomData,
        }
    }

    fn as_mut_slice(&mut self) -> &mut [u32; BLOCK_SAMPLES] {
        // SAFETY: the slice is fully written before `assume_filled` is called.
        unsafe { &mut *self.words.as_mut_ptr() }
    }

    fn assume_filled(self) -> AudioBuffer<sealed::Filled> {
        AudioBuffer {
            words: self.words,
            _state: PhantomData,
        }
    }
}

mod sealed {
    pub trait Mode {}

    pub struct Output;
    pub struct Input;
    pub struct Duplex;

    impl Mode for Output {}
    impl Mode for Input {}
    impl Mode for Duplex {}

    pub trait AudioBufferState {}

    pub struct Empty;
    pub struct Filled;

    impl AudioBufferState for Empty {}
    impl AudioBufferState for Filled {}
}

const DMA_BUFFER_SAMPLES: usize = BLOCK_SAMPLES * 2;

static SILENCE_WORDS: FilledAudioBuffer = AudioBuffer::silence();

bind_interrupts!(struct Irqs {
    DMA1_STREAM0 => dma::InterruptHandler<peripherals::DMA1_CH0>;
    DMA1_STREAM1 => dma::InterruptHandler<peripherals::DMA1_CH1>;
});

#[repr(C, align(32))]
struct DmaBuffer([u32; DMA_BUFFER_SAMPLES]);

#[repr(transparent)]
struct DmaStorage(UnsafeCell<DmaBuffer>);

// SAFETY: access is granted exactly once by `take_dma_buffers`; afterwards the
// two buffers are owned exclusively by the SAI drivers.
unsafe impl Sync for DmaStorage {}

// DMA1 can access D2 SRAM. Only the sample arrays belong in this non-cacheable
// region: atomic ownership metadata must remain in ordinary internal RAM,
// because Cortex-M exclusive atomic accesses are not supported by every
// STM32H7 bus path to D2 SRAM.
#[unsafe(link_section = ".sram1_bss")]
static TX_BUFFER: DmaStorage = DmaStorage(UnsafeCell::new(DmaBuffer([0; DMA_BUFFER_SAMPLES])));
#[unsafe(link_section = ".sram1_bss")]
static RX_BUFFER: DmaStorage = DmaStorage(UnsafeCell::new(DmaBuffer([0; DMA_BUFFER_SAMPLES])));
static DMA_BUFFERS_TAKEN: AtomicBool = AtomicBool::new(false);

fn take_dma_buffers() -> Option<(&'static mut [u32], &'static mut [u32])> {
    DMA_BUFFERS_TAKEN
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .ok()?;

    // SAFETY: the successful atomic transition is unique for the lifetime of
    // the firmware. RX_BUFFER and TX_BUFFER are distinct statics, the BSP
    // initialized/zeroed their D2 section, and the returned references are
    // transferred immediately to the two SAI drivers.
    let rx = unsafe { &mut (*RX_BUFFER.0.get()).0 };
    let tx = unsafe { &mut (*TX_BUFFER.0.get()).0 };
    Some((rx, tx))
}

fn sai_config() -> sai::Config {
    let mut config = sai::Config::default();
    config.master_clock_divider = MasterClockDivider::DIV1;
    config.stereo_mono = StereoMono::Stereo;
    config.data_size = DataSize::Data24;
    config.bit_order = BitOrder::MsbFirst;
    config.frame_sync_polarity = FrameSyncPolarity::ActiveHigh;
    config.frame_sync_offset = FrameSyncOffset::OnFirstBit;
    config.frame_length = 64;
    config.frame_sync_active_level_length = sai::word::U7(32);
    config.fifo_threshold = FifoThreshold::Quarter;
    config
}

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use super::{AudioBuffer, BLOCK_LENGTH, Block, EmptyAudioBuffer, Error};
    use embassy_stm32::i2c;
    use embassy_stm32::sai;

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.000_001);
    }

    #[test]
    fn round_trips_extremes() {
        let mut source: Block = [(0.0, 0.0); BLOCK_LENGTH];
        source[0] = (-1.0, 1.0);
        source[1] = (0.25, -0.5);

        let encoded = AudioBuffer::from(&source);
        let mut decoded: Block = [(0.0, 0.0); BLOCK_LENGTH];
        encoded.decode_into(&mut decoded);

        assert_close(decoded[0].0, -1.0);
        assert_close(decoded[0].1, 1.0);
        assert_close(decoded[1].0, 0.25);
        assert_close(decoded[1].1, -0.5);
    }

    #[test]
    fn round_trips_full_block() {
        let mut source: Block = [(0.0, 0.0); BLOCK_LENGTH];
        for (index, frame) in source.iter_mut().enumerate() {
            let value = index as f32 / BLOCK_LENGTH as f32;
            *frame = (value - 0.5, 0.5 - value);
        }

        let encoded = AudioBuffer::from(&source);
        let mut decoded: Block = [(0.0, 0.0); BLOCK_LENGTH];
        encoded.decode_into(&mut decoded);

        for (expected, actual) in source.iter().zip(decoded.iter()) {
            assert_close(actual.0, expected.0);
            assert_close(actual.1, expected.1);
        }
    }

    #[test]
    fn silence_is_all_zeros() {
        assert!(AudioBuffer::silence().words().iter().all(|&word| word == 0));
    }

    #[test]
    fn channel_order_is_right_first() {
        let source = [(-1.0, 1.0); BLOCK_LENGTH];
        let encoded = AudioBuffer::from(&source);
        let words = encoded.words();

        assert_eq!(words[0], 0x007f_ffff);
        assert_eq!(words[1], 0xff80_0001);
    }

    #[test]
    fn clamps_out_of_range() {
        let out_of_range = [(2.0, -2.0); BLOCK_LENGTH];
        let clamped = [(-1.0, 1.0); BLOCK_LENGTH];

        assert_eq!(
            AudioBuffer::from(&out_of_range).words(),
            AudioBuffer::from(&clamped).words()
        );
    }

    #[test]
    fn empty_fill_decode_round_trip() {
        let mut source: Block = [(0.0, 0.0); BLOCK_LENGTH];
        source[0] = (-0.75, 0.125);

        let encoded = AudioBuffer::from(&source);
        let mut empty = EmptyAudioBuffer::empty();
        empty.as_mut_slice().copy_from_slice(encoded.words());
        let filled = empty.assume_filled();

        let mut decoded: Block = [(0.0, 0.0); BLOCK_LENGTH];
        filled.decode_into(&mut decoded);

        assert_close(decoded[0].0, -0.75);
        assert_close(decoded[0].1, 0.125);
    }

    #[test]
    fn error_categories() {
        assert_eq!(
            Error::BuffersAlreadyTaken.category(),
            "buffers-already-taken"
        );
        assert_eq!(Error::Codec(i2c::Error::Timeout).category(), "codec");
        assert_eq!(Error::SaiStart(sai::Error::Overrun).category(), "sai-start");
        assert_eq!(
            Error::SaiTransmit(sai::Error::Overrun).category(),
            "sai-transmit"
        );
        assert_eq!(
            Error::SaiReceive(sai::Error::Overrun).category(),
            "sai-receive"
        );
    }
}
