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
