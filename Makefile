.PHONY: test coverage coverage-html build container-build nico-container-build chart-lint chart-template

test:
	cargo test --workspace

build:
	cargo build --bin bmc-collector

container-build:
	docker build --tag fleet-intelligence-oob-collector:dev .

# NICO_SOURCE points to a local checkout of NVIDIA/infra-controller.
nico-container-build:
	docker build --file Dockerfile.nico-hardware-health \
		--tag nico-hardware-health:dev "$${NICO_SOURCE:?set NICO_SOURCE to the infra-controller checkout}"

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

.PHONY: setup-git-hooks
setup-git-hooks: ## Configure the local commit-format hook
	git config core.hooksPath .git-hooks
	chmod +x .git-hooks/commit-msg

.PHONY: test-commit-conventions
test-commit-conventions: ## Validate commit subjects and PR titles
	sh scripts/test-commit-conventions.sh
