.PHONY: test coverage coverage-html build

test:
	cargo test

build:
	cargo build --bin bmc-collector

coverage:
	@cargo llvm-cov --version >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov is required for coverage"; \
		echo "install with: cargo install cargo-llvm-cov"; \
		exit 1; \
	}
	cargo llvm-cov --lib --fail-under-lines 80

coverage-html:
	@cargo llvm-cov --version >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov is required for coverage"; \
		echo "install with: cargo install cargo-llvm-cov"; \
		exit 1; \
	}
	cargo llvm-cov --lib --html --open
