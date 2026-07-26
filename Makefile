.PHONY: build test check clippy fmt test-core-matrix
.PHONY: daisy-build-micro-4 daisy-check-micro-4
.PHONY: daisy-build-micro-1 daisy-check-micro-1

build:  ; cargo build --workspace
test:   ; cargo test --workspace
check:  ; cargo check --workspace
clippy: ; cargo clippy --workspace
fmt:    ; cargo fmt --all

test-core-matrix:
	$(MAKE) -C synth-core test-matrix

daisy-%:
	$(MAKE) -C hardware/daisy $*
