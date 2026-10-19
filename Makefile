# Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# SPDX-License-Identifier: Apache-2.0

.PHONY: test coverage coverage-html build container-build nico-container-build chart-lint chart-template deb rpm package packages-check-deb packages-check-rpm packages-test-hooks package-binaries package-collector package-nico package-gateway package-native-check

NFPM ?= nfpm
CARGO ?= cargo
GO ?= go
OCB_VERSION ?= v0.156.0
NICO_REF := $(shell sed -n 's/^  NICO_SOURCE_REF: //p' .github/workflows/release.yml)
AGENT_REF ?= 6eace17c3c19adf1524ba71c9cbd0450266a61f1
NICO_SOURCE ?= dist/package-sources/infra-controller
AGENT_SOURCE ?= dist/package-sources/fleet-intelligence-agent
PACKAGE_BUILDDIR ?= dist/package-build
VERSION ?= 0.1.0~prototype1
ARCH ?= amd64
INPUTDIR ?= dist/package-input
OUTPUTDIR ?= dist/packages
export OOB_PACKAGE_VERSION = $(VERSION)
export OOB_PACKAGE_ARCH = $(ARCH)
export OOB_PACKAGE_INPUT = $(abspath $(INPUTDIR))

package: deb rpm

deb rpm: package-binaries
	mkdir -p "$(OUTPUTDIR)"
	$(NFPM) package --config deployments/packages/nfpm/fleetint-oob.yaml \
		--packager $@ --target "$(OUTPUTDIR)/"

# Native builds; each toolchain reuses its normal incremental build cache.
package-binaries: package-collector package-nico package-gateway

package-native-check:
	@test "$$(uname -s)" = Linux || { echo 'Host packages must be built on Linux' >&2; exit 1; }
	@test "$(ARCH)" = "$$(uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/')" || { echo 'Build natively for ARCH=$(ARCH); cross-compilation is not configured' >&2; exit 1; }

package-collector: package-native-check
	mkdir -p "$(INPUTDIR)"
	FLEETINT_BUILD_VERSION="$(VERSION)" $(CARGO) build --release --locked --bin bmc-collector
	install -m 0755 target/release/bmc-collector "$(INPUTDIR)/bmc-collector"

$(NICO_SOURCE)/Cargo.toml:
	mkdir -p "$(dir $(NICO_SOURCE))"
	test -d "$(NICO_SOURCE)/.git" || git clone --no-checkout --depth 1 https://github.com/NVIDIA/infra-controller.git "$(NICO_SOURCE)"
	git -C "$(NICO_SOURCE)" fetch --depth 1 origin "$(NICO_REF)"
	git -C "$(NICO_SOURCE)" checkout --detach FETCH_HEAD

package-nico: package-native-check $(NICO_SOURCE)/Cargo.toml
	@command -v protoc >/dev/null || { echo 'NICo requires protobuf-compiler and libprotobuf-dev (sudo apt-get install protobuf-compiler libprotobuf-dev)' >&2; exit 1; }
	mkdir -p "$(INPUTDIR)" "$(PACKAGE_BUILDDIR)/nico"
	cd "$(NICO_SOURCE)" && VERSION="$(VERSION)" CI_COMMIT_SHORT_SHA="$$(git rev-parse HEAD)" \
		CARBIDE_VERSION_AVOID_REBUILD=1 FORGE_VERSION_AVOID_REBUILD=1 \
		CARGO_TARGET_DIR="$(abspath $(PACKAGE_BUILDDIR))/nico" \
		$(CARGO) build --release --locked -p carbide-health --bin forge-hw-health
	install -m 0755 "$(PACKAGE_BUILDDIR)/nico/release/forge-hw-health" "$(INPUTDIR)/nico-hardware-health"
	cat "$(NICO_SOURCE)/LICENSE" "$(NICO_SOURCE)/THIRD-PARTY-LICENSES" > "$(INPUTDIR)/nico-NOTICES"

$(AGENT_SOURCE)/otelcol/otelcol-builder.yaml:
	mkdir -p "$(dir $(AGENT_SOURCE))"
	test -d "$(AGENT_SOURCE)/.git" || git clone --no-checkout --depth 1 https://github.com/dsx-ai-factory/fleet-intelligence-agent.git "$(AGENT_SOURCE)"
	git -C "$(AGENT_SOURCE)" fetch --depth 1 origin "$(AGENT_REF)"
	git -C "$(AGENT_SOURCE)" checkout --detach FETCH_HEAD

package-gateway: package-native-check $(AGENT_SOURCE)/otelcol/otelcol-builder.yaml
	mkdir -p "$(INPUTDIR)" "$(PACKAGE_BUILDDIR)/gateway"
	cd "$(AGENT_SOURCE)/otelcol" && CGO_ENABLED=0 GOOS=linux GOARCH="$(ARCH)" \
		$(GO) run go.opentelemetry.io/collector/cmd/builder@$(OCB_VERSION) \
		--config=otelcol-builder.yaml --output-path="$(abspath $(PACKAGE_BUILDDIR))/gateway"
	install -m 0755 "$(PACKAGE_BUILDDIR)/gateway/fleetint-otelcol" "$(INPUTDIR)/otel-gateway"
	cat "$(AGENT_SOURCE)/LICENSE" "$(AGENT_SOURCE)/NOTICE" > "$(INPUTDIR)/gateway-NOTICES"

packages-test-hooks:
	sh deployments/packages/tests/hooks.sh

# Native package inspection; never installs packages or starts services.
packages-check-deb:
	@set -e; for package in "$(OUTPUTDIR)"/fleetint-oob_*.deb; do \
		test -f "$$package" || continue; \
		dpkg-deb --info "$$package" >/dev/null; \
		name=$$(dpkg-deb --field "$$package" Package); \
		test "$$name" = fleetint-oob; \
		for config in /etc/fleetint-oob/config.toml /etc/fleetint-oob/service.conf /etc/fleetint-oob/gateway.yaml; do \
			dpkg-deb --ctrl-tarfile "$$package" | tar --wildcards -xOf - '*conffiles' | grep -Fx "$$config"; \
		done; \
		if dpkg-deb --fsys-tarfile "$$package" | tar -tf - | grep -E '(^|/)(agent-token|username|password)$$|/sysusers[.]d/'; then \
			echo 'Credential file or account definition found in package' >&2; exit 1; \
		fi; \
	done
	@set -e; for script in deployments/packages/scripts/* deployments/packages/scripts/*/*; do \
		test -f "$$script" || continue; sh -n "$$script"; \
	done

packages-check-rpm:
	@set -e; for package in "$(OUTPUTDIR)"/fleetint-oob-*.rpm; do \
		test -f "$$package" || continue; \
		rpm --checksig --nosignature "$$package"; \
		if rpm -qpl "$$package" | grep -E '(^|/)(agent-token|username|password)$$|/sysusers[.]d/'; then \
			echo 'Credential file or account definition found in package' >&2; exit 1; \
		fi; \
	done

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
