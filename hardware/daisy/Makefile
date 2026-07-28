TARGET = thumbv7em-none-eabihf

# ── Models ──────────────────────────────────────────
# Micro 4 — 4 voices, half-rate DSP, Gain-Limited TPT
FEATURES_micro-4 = fast-math,wide-4,downsampling,filter-gain-limited

# Micro 1 — 1 voice, full-rate DSP, Huovilainen ladder
FEATURES_micro-1 = fast-math,wide-1,filter-huovilainen

# ── Targets ─────────────────────────────────────────
# Do not list pattern-stem targets (build-micro-4, …) in .PHONY: Make 4.4+
# treats those as empty explicit rules and skips the pattern recipes.

MODELS = micro-4 micro-1

define model_rules
.PHONY: build-$(1) check-$(1) run-$(1) bench-dsp-$(1) bench-factory-presets-$(1)

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

bench-factory-presets-$(1):
	cargo run --release -p noctum-micro \
		--no-default-features \
		--features "$(FEATURES_$(1)),audio-profiling" \
		--bin bench-factory-presets
endef

$(foreach m,$(MODELS),$(eval $(call model_rules,$(m))))
