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
in-flight backend upload. It also exercises a complete backend cycle with one
HTTP 401, re-enrollment, refreshed inventory timestamps, inventory acceptance,
and JWT rotation from the nonce response. It never contacts real BMCs or
customer backends.

`shutdown.rs` also checks that help output hides the bootstrap token's
environment value. Unit tests in `crates/backend-client/src/lib.rs` cover URL
validation, backend paths, enrollment parsing, typed HTTP errors, and credential
redaction. HTTP fixtures use localhost only.
