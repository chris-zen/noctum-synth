#!/bin/sh
set -eu

script_dir=$(dirname -- "$0")
script_dir=$(cd -- "$script_dir" && pwd)
binary=$(mktemp /tmp/usb-audio-raw-capture.XXXXXX)
midi_device=${USB_MIDI_DEVICE:-Noctum USB MIDI (development)}
capture_pid=

cleanup() {
    sendmidi dev "$midi_device" ch 1 off 60 0 off 64 0 off 67 0 >/dev/null 2>&1 || true
    if [ -n "$capture_pid" ]; then
        kill -INT "$capture_pid" >/dev/null 2>&1 || true
    fi
    rm -f "$binary"
}
trap cleanup EXIT INT TERM

cc -std=c11 -Wall -Wextra -Werror \
    -I/opt/homebrew/include/libusb-1.0 \
    "$script_dir/usb-audio-raw-capture.c" \
    -L/opt/homebrew/lib -lusb-1.0 -o "$binary"

"$binary" "${USB_AUDIO_VID:-0xc0de}" "${USB_AUDIO_PID:-0xcafe}" &
capture_pid=$!

if [ "${USB_AUDIO_RAW_SEND_MIDI:-1}" -ne 0 ]; then
    sleep 1
    sendmidi dev "$midi_device" ch 1 on 60 110
    sleep 1
    sendmidi dev "$midi_device" ch 1 off 60 0 on 64 110
    sleep 1
    sendmidi dev "$midi_device" ch 1 off 64 0 on 67 110
    sleep 1
    sendmidi dev "$midi_device" ch 1 off 67 0
fi

wait "$capture_pid"
capture_pid=
