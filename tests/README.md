# Tests

Unit tests live next to the code they test, under `src/` and `crates/*/src/`. This directory holds workspace-level integration tests: tests that go through the public crate API, run the CLI, or walk fixture-backed Redfish trees.

All tests use local HTTP fixtures. They never contact real BMCs or Fleet Intelligence backends.

Run the package you changed first, then the whole workspace:

```bash
cargo test -p bmc-inventory
cargo test --workspace
```

Where to look:

| Area | Tests |
| --- | --- |
| Runtime cycle, shutdown, upload batching, re-enrollment | `tests/shutdown.rs`, which runs the collector as a child process against localhost fixtures |
| Attestation uploads and response matching | `crates/bmc-attestation/src/sink/backend_tests.rs` |
| BMC credential reload and retry | Attestation source unit tests in `crates/bmc-attestation/src/source/` |
| Backend URL validation, endpoint paths, enrollment, HTTP errors | `crates/backend-client/src/lib.rs` |

Run `make debian-test` for a real three-component Debian package installation,
non-root daemon startup, authenticated telemetry export to loopback fixtures,
and upgrade/removal coverage. These checks use shell scripts in Docker.

Run `make rpm-test` for the same daemon runtime and authenticated telemetry checks with a real RPM install, upgrade, configuration preservation, and removal in a Rocky Linux 9 container. These tests execute the packaged RPM scriptlets and stub only the systemd manager.

## Script responsibilities

| Area | Entry points and scope |
| --- | --- |
| Package builds | `Makefile` builds and inspects packages; `deployments/packages/Dockerfile` provides the Linux toolchains. |
| Signing | `scripts/package-signing.sh` shares the signing request implementation across packages, checksum manifests, and complete releases. |
| Verification | `scripts/verify-package-release.sh` checks the complete signed inventory; `scripts/verify-linux-package-signature.sh` verifies an individual DEB/RPM and is included in release assets. |
| GitHub releases | `scripts/package-release.cjs` reserves a draft and uploads immutable verified assets. |
| Installed services | `deployments/packages/run-service` loads collector/gateway tokens; `run-nico` renders private NICo configuration. Distro-specific hooks remain separate for package-manager lifecycle semantics. |
| Package tests | `hooks.sh` and `build.sh` use isolated mocks; `smoke.sh` and `rpm.sh` execute actual package transactions; shared `startup.sh` and `runtime.sh` cover the installed daemons. `lifecycle.sh` tests installed Debian hooks, and `linking.sh` validates the musl toolchain. |
| Release tests | The host-package workflow runs helper tests once, then installs/tests actual packages on each architecture. `make release-test` runs signing/tamper tests with ephemeral keys and a fake signing service, plus GitHub API fixture tests. It uses no production credentials or signing requests. |
| Docs and notices | `docs-fetch-versions.py` stages versioned docs; `check-docs-links.py` validates source/staged links; `generate-notice.py` is an offline-capable maintainer tool, outside package builds. Their Python tests cover distinct behaviors. |
