CARGO ?= cargo
RUSTC ?= rustc
ACTIONLINT ?= actionlint
HOST_TARGET := $(shell $(RUSTC) -vV | sed -n 's/^host: //p')

.PHONY: verify fmt lint rust-lint source-lint architecture-lint docs-language-lint workflow-lint check test coverage deny machete

verify: fmt lint check test coverage deny machete

lint: architecture-lint docs-language-lint workflow-lint rust-lint source-lint

source-lint:
	$(CARGO) run --manifest-path tools/source-lint/Cargo.toml --target $(HOST_TARGET) -- source src

architecture-lint:
	$(CARGO) run --manifest-path tools/source-lint/Cargo.toml --target $(HOST_TARGET) -- architecture src

docs-language-lint:
	$(CARGO) run --manifest-path tools/source-lint/Cargo.toml --target $(HOST_TARGET) -- docs-language

workflow-lint:
	$(ACTIONLINT) -color=false .github/workflows/ci.yml .github/workflows/release.yml

fmt:
	$(CARGO) fmt --all -- --check
	$(CARGO) fmt --manifest-path tools/source-lint/Cargo.toml -- --check

rust-lint:
	$(CARGO) clippy --target $(HOST_TARGET) --all-targets --all-features -- -D warnings
	$(CARGO) clippy --manifest-path tools/source-lint/Cargo.toml --target $(HOST_TARGET) --all-targets -- -D warnings

check:
	$(CARGO) check --target $(HOST_TARGET) --all-targets --all-features

test:
	$(CARGO) test --target $(HOST_TARGET) --all-targets --all-features
	$(CARGO) test --manifest-path tools/source-lint/Cargo.toml --target $(HOST_TARGET)

coverage:
	$(CARGO) llvm-cov --target $(HOST_TARGET) --all-targets --all-features --fail-under-lines 95

deny:
	$(CARGO) deny check
	$(CARGO) deny --manifest-path tools/source-lint/Cargo.toml check --config tools/source-lint/deny.toml

machete:
	$(CARGO) machete
