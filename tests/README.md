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
