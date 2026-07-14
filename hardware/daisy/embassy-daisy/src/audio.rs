//! Full-duplex audio transport for Daisy hardware.

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};
use embassy_stm32::bind_interrupts;
use embassy_stm32::dma;
use embassy_stm32::i2c::{Config as I2cConfig, I2c};
use embassy_stm32::peripherals;
use embassy_stm32::sai::{
    self, BitOrder, ClockStrobe, DataSize, FifoThreshold, FrameSyncOffset, FrameSyncPolarity,
    MasterClockDivider, Mode, Sai, StereoMono, SyncInput, TxRx,
};

use crate::format::{BLOCK_SAMPLES, decode_block, encode_block};
use crate::wm8731::Wm8731;

pub use crate::format::{BLOCK_LENGTH, Block, CHANNELS, Frame, SAMPLE_RATE_HZ};
const DMA_BUFFER_SAMPLES: usize = BLOCK_SAMPLES * 2;

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

pub struct Audio {
    tx: Sai<'static, peripherals::SAI1, u32>,
    rx: Sai<'static, peripherals::SAI1, u32>,
}

impl Audio {
    pub fn new(resources: AudioResources) -> Result<Self, Error> {
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

        Ok(Self { tx, rx })
    }

    /// Prime transmit DMA before enabling the master receive clock.
    ///
    /// SAI B is synchronous to SAI A, so its DMA can be armed safely while no
    /// frame clock is present. Starting the receiver afterwards prevents its
    /// ring buffer from filling while the first output block is prepared.
    pub async fn start(&mut self, initial_output: &Block) -> Result<(), Error> {
        let mut tx_words = [0u32; BLOCK_SAMPLES];
        encode_block(initial_output, &mut tx_words);
        self.tx.write(&tx_words).await.map_err(Error::SaiTransmit)?;
        self.rx.start().map_err(Error::SaiStart)
    }

    /// Exchange one fixed-size stereo block with the codec.
    ///
    /// Output is written first so the SAI master begins generating clocks before
    /// the synchronous receive operation is awaited.
    pub async fn transfer(&mut self, output: &Block, input: &mut Block) -> Result<(), Error> {
        let mut tx_words = [0u32; BLOCK_SAMPLES];
        let mut rx_words = [0u32; BLOCK_SAMPLES];
        encode_block(output, &mut tx_words);
        self.tx.write(&tx_words).await.map_err(Error::SaiTransmit)?;
        self.rx
            .read(&mut rx_words)
            .await
            .map_err(Error::SaiReceive)?;
        decode_block(&rx_words, input);
        Ok(())
    }
}

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
