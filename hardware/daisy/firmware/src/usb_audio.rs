//! Non-blocking mirror from the SAI-paced synth output to USB Audio Class 1.

use embassy_daisy::usb_audio::Stream;
use embassy_time::{Duration, with_timeout};
use embassy_usb::driver::{Driver, EndpointError};

use crate::diagnostics;
use crate::usb_audio_core::{
    MAX_PACKET_FRAMES, PRIME_FRAMES, STARTUP_FADE_FRAMES, encode_frames, packet_frames,
    pad_to_silence,
};

pub use crate::usb_audio_core::{MAX_PACKET_BYTES, SAMPLE_RATE_HZ, UsbAudioBuffer};

/// Serve the UAC1 IN endpoint forever. Endpoint closure only stops the USB
/// mirror; the SAI-paced DAC producer continues independently.
pub async fn run<'d, D: Driver<'d>>(
    stream: &mut Stream<'d, D>,
    buffer: &'static UsbAudioBuffer,
) -> ! {
    let mut packet = [0u8; MAX_PACKET_BYTES];
    let mut frames = [(0.0, 0.0); MAX_PACKET_FRAMES];
    #[cfg(feature = "usb-audio-test-tone")]
    let mut test_tone_phase = 0usize;

    loop {
        stream.wait_connection().await;
        stream.wait_resumed().await;
        let endpoint = stream.endpoint_index();
        embassy_daisy::usb::set_isochronous_in_recovery(endpoint, true);
        buffer.activate();
        diagnostics::emit(diagnostics::Event::UsbAudioStarted);

        let mut primed = false;
        let mut fade_position = 0usize;
        let suspension_epoch = stream.suspension_epoch();

        loop {
            if stream.is_suspended() {
                stop_for_suspend(stream, buffer).await;
                break;
            }

            let count = packet_frames(primed, buffer.occupancy());

            if !primed && buffer.occupancy() >= PRIME_FRAMES {
                primed = true;
                diagnostics::emit(diagnostics::Event::UsbAudioPrimed);
            }

            let received = if primed {
                buffer.pop_into(&mut frames[..count])
            } else {
                0
            };
            if received < count {
                pad_to_silence(&mut frames[..count], received);
            }

            #[cfg(feature = "usb-audio-test-tone")]
            if primed {
                // USB-only 1 kHz, -18 dBFS square wave for automated transport
                // testing. This feature is intentionally absent from normal
                // firmware builds and does not alter the DAC signal.
                for frame in &mut frames[..count] {
                    let sample = if test_tone_phase < SAMPLE_RATE_HZ / 2_000 {
                        0.125
                    } else {
                        -0.125
                    };
                    *frame = (sample, sample);
                    test_tone_phase = (test_tone_phase + 1) % (SAMPLE_RATE_HZ / 1_000);
                }
            }

            if primed {
                for frame in &mut frames[..count] {
                    if fade_position >= STARTUP_FADE_FRAMES {
                        continue;
                    }
                    fade_position += 1;
                    let gain = fade_position as f32 / STARTUP_FADE_FRAMES as f32;
                    frame.0 *= gain;
                    frame.1 *= gain;
                }
            }

            let bytes = encode_frames(&frames[..count], &mut packet);
            // Missed frames are resynchronized by the OTG interrupt handler.
            // This timeout is only a last-resort watchdog for an unexpected
            // peripheral or driver state that did not produce an interrupt.
            let write = with_timeout(Duration::from_millis(100), stream.write_packet(bytes)).await;
            let result = match write {
                Ok(result) => result,
                Err(_) => {
                    embassy_daisy::usb::recover_disabled_isochronous_in(endpoint);
                    continue;
                }
            };
            match result {
                Ok(()) => {
                    let current_epoch = stream.suspension_epoch();
                    if current_epoch != suspension_epoch {
                        stop_for_suspend(stream, buffer).await;
                        break;
                    }
                }
                Err(EndpointError::Disabled) => {
                    finish_stream(endpoint, buffer);
                    break;
                }
                Err(EndpointError::BufferOverflow) => {}
            }
        }
    }
}

fn finish_stream(endpoint: usize, buffer: &UsbAudioBuffer) {
    embassy_daisy::usb::set_isochronous_in_recovery(endpoint, false);
    buffer.deactivate();
    diagnostics::emit(diagnostics::Event::UsbAudioStopped);
}

async fn stop_for_suspend<'d, D: Driver<'d>>(
    stream: &Stream<'d, D>,
    buffer: &UsbAudioBuffer,
) {
    finish_stream(stream.endpoint_index(), buffer);
    stream.wait_resumed().await;
}
