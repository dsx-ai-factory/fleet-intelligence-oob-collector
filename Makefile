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
