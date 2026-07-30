.PHONY: build test check clippy fmt test-core-matrix factory-corpus
.PHONY: daisy-build-micro-4 daisy-check-micro-4
.PHONY: daisy-build-micro-1 daisy-check-micro-1

build:  ; cargo build --workspace
test:   ; cargo test --workspace
check:  ; cargo check --workspace
clippy: ; cargo clippy --workspace
fmt:    ; cargo fmt --all

test-core-matrix:
	$(MAKE) -C synth-core test-matrix

factory-corpus:
	cargo run --release -p synth-tools --bin factory_corpus_acceptance -- \
		Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx \
		target/factory-corpus.csv

daisy-%:
	$(MAKE) -C hardware/daisy $*
