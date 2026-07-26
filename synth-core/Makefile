export RUST_MIN_STACK ?= 16777216

FEATURES_wide4 = wide-4,filter-all,serde
FEATURES_wide4-fast = wide-4,filter-all,serde,fast-math
FEATURES_wide1 = wide-1,filter-all,serde
FEATURES_wide1-fast = wide-1,filter-all,serde,fast-math
FEATURES_wide8 = wide-8,filter-all,serde
FEATURES_wide8-fast = wide-8,filter-all,serde,fast-math
FEATURES_daisy-micro-4 = wide-4,downsampling
FEATURES_daisy-micro-1 = wide-1
FEATURES_polyblep = wide-4,filter-all,serde,oscillator-polyblep

MATRIX = wide4 wide4-fast wide1 wide1-fast wide8 wide8-fast daisy-micro-4 daisy-micro-1 polyblep

.PHONY: test test-matrix $(MATRIX)

test:
	cargo test -p synth-core

test-matrix: $(MATRIX)

$(MATRIX):
	cargo test -p synth-core --lib --tests --no-default-features --features "$(FEATURES_$@)"
