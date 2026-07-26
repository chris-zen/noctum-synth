TARGET = thumbv7em-none-eabihf

# ── Models ──────────────────────────────────────────
# Micro 4 — 4 voices, with downsampling quality tradeoff
# Available tradeoffs: add fast-math for speed, remove downsampling for full rate
FEATURES_micro-4 = wide-4,downsampling

# Micro 1 — 1 voice, no quality tradeoff
FEATURES_micro-1 = wide-1

# ── Targets ─────────────────────────────────────────
.PHONY: build-micro-4 build-micro-1 check-micro-4 check-micro-1

build-% check-%:
	$(eval MODEL=$*)
	$(eval FEATURES=$(FEATURES_$(MODEL)))
	cargo $(word 1,$(subst -, ,$@)) -p synth-core \
		--no-default-features \
		--features "$(FEATURES)" \
		--target $(TARGET)
