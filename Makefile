export TMPDIR ?= /tmp

CARGO = cargo
LOCKED = --locked
COVERAGE_MIN = 70

.PHONY: fmt fmt-check lint doc deny test cover coverage-check check

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all --check

lint:
	$(CARGO) clippy --all-targets $(LOCKED) -- -D warnings

doc:
	RUSTDOCFLAGS="-D warnings" $(CARGO) doc --no-deps $(LOCKED)

deny:
	$(CARGO) deny check

test:
	$(CARGO) test --all-targets $(LOCKED)
	$(CARGO) test --doc $(LOCKED)

cover:
	$(CARGO) llvm-cov --all-features --lcov --output-path lcov.info

coverage-check:
	$(CARGO) llvm-cov --all-features --fail-under-lines $(COVERAGE_MIN)

check: fmt-check lint doc deny test coverage-check
