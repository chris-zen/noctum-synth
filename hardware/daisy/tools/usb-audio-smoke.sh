#!/bin/sh
set -eu

audio_device=${USB_AUDIO_DEVICE:-Analog Synth (development)}
midi_device=${USB_MIDI_DEVICE:-Analog Synth USB MIDI (development)}
output=${USB_AUDIO_OUTPUT:-/tmp/analog-synth-usb-audio-smoke.wav}
capture_seconds=${USB_AUDIO_CAPTURE_SECONDS:-6}
capture_pid=

cleanup() {
    sendmidi dev "$midi_device" ch 1 off 60 0 off 64 0 off 67 0 >/dev/null 2>&1 || true
    if [ -n "$capture_pid" ]; then
        kill -INT "$capture_pid" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT INT TERM

sox -V1 -t coreaudio "$audio_device" -r 48000 -c 2 -b 24 "$output" \
    trim 0 "$capture_seconds" &
capture_pid=$!

# Allow CoreAudio to select alternate setting 1 and let firmware prime.
sleep 1
sendmidi dev "$midi_device" ch 1 on 60 110
sleep 1
sendmidi dev "$midi_device" ch 1 off 60 0 on 64 110
sleep 1
sendmidi dev "$midi_device" ch 1 off 64 0 on 67 110
sleep 1
sendmidi dev "$midi_device" ch 1 off 67 0

wait "$capture_pid"
capture_pid=

sox "$output" -n stats
maximum_amplitude=$(sox "$output" -n stat 2>&1 | awk '/Maximum amplitude/ { print $3 }')
if ! awk -v peak="$maximum_amplitude" 'BEGIN { exit !(peak > 0.00001) }'; then
    privacy_probe=/tmp/analog-synth-coreaudio-privacy-probe.wav
    rm -f "$privacy_probe"
    if sox -V0 -d -r 48000 -c 1 -b 24 "$privacy_probe" trim 0 1; then
        privacy_peak=$(sox "$privacy_probe" -n stat 2>&1 | awk '/Maximum amplitude/ { print $3 }')
        rm -f "$privacy_probe"
        if ! awk -v peak="$privacy_peak" 'BEGIN { exit !(peak > 0.00001) }'; then
            echo "CoreAudio default input is also exact silence; macOS microphone privacy is likely blocking this process" >&2
        fi
    fi
    rm -f "$privacy_probe"
    echo "USB audio smoke test failed: captured signal is silent" >&2
    exit 1
fi

echo "USB audio smoke test passed: peak=$maximum_amplitude file=$output"
