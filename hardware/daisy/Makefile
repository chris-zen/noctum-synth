TARGET = thumbv7em-none-eabihf
HOST_TARGET := $(shell rustc -vV | sed -n 's/^host: //p')

REPO_ROOT := $(abspath ../..)
FACTORY_SYX := $(REPO_ROOT)/Prophet-Rev2-Factory-Programs/Rev2_Programs_v1.0.syx
FACTORY_ZLIB := target/factory-bank.zlib

# ── Models ──────────────────────────────────────────
# Micro 4 — 4 voices, half-rate DSP, Gain-Limited TPT
FEATURES_micro-4 = fast-math,wide-4,downsampling,filter-gain-limited

# Micro 1 — 1 voice, full-rate DSP, Huovilainen ladder
FEATURES_micro-1 = fast-math,wide-1,filter-huovilainen

# ── Targets ─────────────────────────────────────────
# Do not list pattern-stem targets (build-micro-4, …) in .PHONY: Make 4.4+
# treats those as empty explicit rules and skips the pattern recipes.
.PHONY: factory-banks-flash clean

MODELS = micro-4 micro-1

define model_rules
.PHONY: build-$(1) check-$(1) run-$(1) bench-dsp-$(1) bench-factory-banks-$(1)

build-$(1):
	cargo build --release -p noctum-micro \
		--no-default-features \
		--features "$(FEATURES_$(1))" \
		--target $(TARGET)

check-$(1):
	cargo check --release -p noctum-micro \
		--no-default-features \
		--features "$(FEATURES_$(1))" \
		--target $(TARGET)

run-$(1):
	cargo run --release -p noctum-micro \
		--no-default-features \
		--features "$(FEATURES_$(1))"

bench-dsp-$(1):
	cargo run --release -p noctum-micro \
		--no-default-features \
		--features "$(FEATURES_$(1)),audio-profiling" \
		--bin bench-dsp

bench-factory-banks-$(1):
	cargo run --release -p noctum-micro \
		--no-default-features \
		--features "$(FEATURES_$(1)),audio-profiling,bench-factory-banks" \
		--bin bench-factory-banks
endef

$(foreach m,$(MODELS),$(eval $(call model_rules,$(m))))

$(FACTORY_ZLIB): $(FACTORY_SYX)
	mkdir -p $(dir $@)
	cargo run --release -p tools-host --target $(HOST_TARGET) \
		--bin factory-banks-compress -- $(FACTORY_SYX) $@

factory-banks-flash: $(FACTORY_ZLIB)
	cargo run --release -p tools-micro \
		--features flash \
		--bin factory-banks-flash

clean:
	rm -f $(FACTORY_ZLIB)
