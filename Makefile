COVERAGE_MIN = 70

# Temporary and build folders never go under $HOME.
export TMPDIR ?= /tmp
export CARGO_TARGET_DIR ?= /var/tmp/dayzmake-target

CARGO = cargo
LOCKED = --locked
FUZZ_TIME ?= 10
# cargo-fuzz binaries from prebuilt releases default to a musl target, where sanitizers do not work.
FUZZ_HOST = $(shell rustc +nightly -vV | sed -n "s/^host: //p")

.PHONY: fmt fmt-check lint doc deny test fuzz cover coverage-check miri features docsrs package-check check

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

lint:
	$(CARGO) clippy --workspace --all-targets $(LOCKED) -- -D warnings

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --no-deps $(LOCKED)

deny:
	$(CARGO) deny check

test:
	$(CARGO) test --workspace --all-targets $(LOCKED)
	$(CARGO) test --workspace --doc $(LOCKED)

# Each target runs FUZZ_TIME seconds (10 by default; `make fuzz FUZZ_TIME=60` for longer).
fuzz:
	@if [ -d fuzz ] && $(CARGO) +nightly fuzz --help >/dev/null 2>&1; then \
	  cd fuzz && for t in $$($(CARGO) +nightly fuzz list 2>/dev/null); do \
	    mkdir -p $(TMPDIR)/dayzmake-fuzz/$$t; \
	    $(CARGO) +nightly fuzz run --target $(FUZZ_HOST) $$t $(TMPDIR)/dayzmake-fuzz/$$t corpus/$$t -- -max_total_time=$(FUZZ_TIME) || exit 1; \
	  done; \
	else \
	  echo "skipping fuzz: cargo-fuzz or nightly missing"; \
	fi

cover:
	$(CARGO) llvm-cov --workspace --all-features --lcov --output-path lcov.info

coverage-check:
	$(CARGO) llvm-cov --workspace --all-features --fail-under-lines $(COVERAGE_MIN)

miri:
	$(CARGO) +nightly miri test --workspace $(LOCKED)

# The library must build with each feature alone and with none.
features:
	$(CARGO) clippy -p dayzmake-core --no-default-features --lib $(LOCKED) -- -D warnings
	$(CARGO) clippy -p dayzmake-core --no-default-features --features sign --lib $(LOCKED) -- -D warnings
	$(CARGO) clippy -p dayzmake-core --no-default-features --features image --lib $(LOCKED) -- -D warnings
	$(CARGO) test -p dayzmake-core --no-default-features $(LOCKED)

# docs.rs builds with nightly, `--cfg docsrs` and all features.
docsrs:
	@if $(CARGO) +nightly --version >/dev/null 2>&1; then \
	  RUSTDOCFLAGS="--cfg docsrs -D warnings" $(CARGO) +nightly doc -p dayzmake-core --all-features --no-deps $(LOCKED); \
	else \
	  echo "skipping docsrs: nightly missing"; \
	fi

# Packages both crates (the CLI against the packaged core). CI runs it without --allow-dirty.
package-check:
	$(CARGO) package --workspace $(LOCKED) --allow-dirty

check: fmt-check lint doc deny test features docsrs package-check fuzz coverage-check
