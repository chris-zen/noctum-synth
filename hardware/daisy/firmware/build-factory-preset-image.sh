#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
DAISY_DIR=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
REPO_DIR=$(CDPATH= cd -- "$DAISY_DIR/../.." && pwd)
BANK=${1:-"$REPO_DIR/Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx"}
OUTPUT=${2:-"$DAISY_DIR/target/factory-preset-benchmark-with-bank.bin"}
ELF="$DAISY_DIR/target/thumbv7em-none-eabihf/release/factory-preset-benchmark"
PADDED="$DAISY_DIR/target/factory-preset-benchmark-padded.bin"

EXPECTED_BANK_SIZE=1201152
BANK_FILE_OFFSET_BLOCKS=128

if [ ! -f "$BANK" ]; then
    echo "factory bank not found: $BANK" >&2
    exit 1
fi

ACTUAL_BANK_SIZE=$(wc -c < "$BANK" | tr -d ' ')
if [ "$ACTUAL_BANK_SIZE" -ne "$EXPECTED_BANK_SIZE" ]; then
    echo "unexpected factory bank size: $ACTUAL_BANK_SIZE" >&2
    exit 1
fi

cd "$DAISY_DIR"
cargo build --release -p analog-synth-daisy-firmware \
    --features audio-profiling --bin factory-preset-benchmark

# The Daisy bootloader stores the file at QSPI 0x90040000 and copies the first
# 480 KiB into AXI SRAM. Pad the application storage to 512 KiB, then place the
# bank at file offset 0x80000, which maps to QSPI 0x900c0000.
arm-none-eabi-objcopy -O binary -S --gap-fill=0xff --pad-to=0x24080000 \
    "$ELF" "$PADDED"
cp "$PADDED" "$OUTPUT"
dd if="$BANK" of="$OUTPUT" bs=4096 seek="$BANK_FILE_OFFSET_BLOCKS" conv=notrunc

echo "created $OUTPUT"
echo "flash at 0x90040000 using the normal Daisy bootloader upload"
