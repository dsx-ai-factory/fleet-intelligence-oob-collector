.PHONY: test coverage coverage-html build container-build chart-lint chart-template

test:
	cargo test --workspace

build:
	cargo build --bin bmc-collector

container-build:
	docker build --tag fleet-intelligence-oob-collector:dev .

chart-lint:
	helm lint deployments/helm/fleet-intelligence-oob-collector \
		--values deployments/helm/fleet-intelligence-oob-collector/ci/test-values.yaml

chart-template:
	helm template fleet-intelligence-oob-collector deployments/helm/fleet-intelligence-oob-collector \
		--values deployments/helm/fleet-intelligence-oob-collector/ci/test-values.yaml

coverage:
	@cargo llvm-cov --version >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov is required for coverage"; \
		echo "install with: cargo install cargo-llvm-cov"; \
		exit 1; \
	}
	cargo llvm-cov --workspace --fail-under-lines 80

coverage-html:
	@cargo llvm-cov --version >/dev/null 2>&1 || { \
		echo "cargo-llvm-cov is required for coverage"; \
		echo "install with: cargo install cargo-llvm-cov"; \
		exit 1; \
	}
	cargo llvm-cov --workspace --html --open
