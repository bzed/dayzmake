export TMPDIR ?= /tmp

CARGO = cargo
LOCKED = --locked

.PHONY: fmt fmt-check lint doc deny test check

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

check: fmt-check lint doc deny test
