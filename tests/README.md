# Tests

This directory is reserved for workspace-level integration tests and Redfish
fixtures.

Unit tests live next to the modules they exercise under `src/`. Use this
directory for tests that should go through the public crate API, CLI behavior,
or fixture-backed Redfish traversal.

Run an affected package first, then the complete workspace:

```bash
cargo test -p bmc-inventory
cargo test --workspace
```

Planned layout:

```text
tests/
  fixtures/
    redfish/
  inventory_cli.rs
  redfish_source.rs
```

`shutdown.rs` runs collector child processes against localhost fixtures to
verify SIGINT/SIGTERM handling during enrollment, Redfish collection, and an
in-flight backend upload. It also checks that inventory and attestation share
one target/credential snapshot, that unresolved inventory does not gate
attestation, and that inventory-phase errors do not skip attestation attempts.
It never contacts real BMCs or customer backends.

`crates/bmc-attestation/src/sink/backend_tests.rs` covers per-MAC response
validation and exercises HTTP uploads against a localhost backend fixture,
including partial rejection, malformed responses, and typed JWT rejection.
