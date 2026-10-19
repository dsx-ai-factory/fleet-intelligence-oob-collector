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

.PHONY: test coverage coverage-html build container-build nico-container-build chart-lint chart-template deb rpm package packages-check-deb packages-check-rpm packages-test-hooks

NFPM ?= nfpm
VERSION ?= 0.1.0~prototype1
ARCH ?= amd64
INPUTDIR ?= dist/package-input
OUTPUTDIR ?= dist/packages
export OOB_PACKAGE_VERSION = $(VERSION)
export OOB_PACKAGE_ARCH = $(ARCH)
export OOB_PACKAGE_INPUT = $(abspath $(INPUTDIR))

package: deb rpm

deb rpm:
	mkdir -p "$(OUTPUTDIR)"
	$(NFPM) package --config deployments/packages/nfpm/fleetint-oob.yaml \
		--packager $@ --target "$(OUTPUTDIR)/"

packages-test-hooks:
	sh deployments/packages/tests/hooks.sh

# Native package inspection; never installs packages or starts services.
packages-check-deb:
	@set -e; for package in "$(OUTPUTDIR)"/*.deb; do \
		test -f "$$package" || continue; \
		dpkg-deb --info "$$package" >/dev/null; \
		name=$$(dpkg-deb --field "$$package" Package); \
		test "$$name" = fleetint-oob; \
		for config in /etc/fleetint-oob/service.conf /etc/fleetint-oob/gateway.yaml; do \
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
	@set -e; for package in "$(OUTPUTDIR)"/*.rpm; do \
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
