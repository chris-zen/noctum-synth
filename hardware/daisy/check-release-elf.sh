#!/bin/sh
set -eu

elf="${1:-target/thumbv7em-none-eabihf/release/analog-synth-daisy-firmware}"
prefix="${ARM_NONE_EABI_PREFIX:-arm-none-eabi-}"

test -f "$elf" || {
    echo "missing release ELF: $elf" >&2
    exit 1
}

attributes="$(${prefix}readelf -A "$elf")"
echo "$attributes" | grep -q 'Tag_CPU_name: "cortex-m7"'
echo "$attributes" | grep -q 'Tag_THUMB_ISA_use: Thumb-2'
echo "$attributes" | grep -q 'Tag_FP_arch: FPv5/FP-D16'
echo "$attributes" | grep -q 'Tag_ABI_VFP_args: VFP registers'

if ${prefix}nm -C --defined-only "$elf" | grep -Eq \
    'libm::math::| (expf|exp2f|powf|sqrtf|tanf|tanhf|expm1f)$'; then
    echo "release ELF contains a software libm implementation" >&2
    exit 1
fi

if ! ${prefix}objdump -d "$elf" | grep -Eiq \
    'vmla\.f32|vmls\.f32|vfma\.f32|vfms\.f32|vsqrt\.f32'; then
    echo "release ELF contains no expected FPv5 fused/square-root instructions" >&2
    exit 1
fi

echo "release ELF OK: Cortex-M7 Thumb-2, hard FPv5-D16, no linked libm"
