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
one target snapshot, that unresolved inventory does not gate attestation, and
that inventory-phase errors do not skip attestation attempts. Mixed and
malformed upload responses retain per-target counts and allow later mini-batches
to run, including after backend token renewal. Nonce failures skip the affected
targets without uploading failed records or empty batches; skipped targets are
attempted again next cycle. Nonce HTTP 401 still triggers re-enrollment.
It never contacts real BMCs or customer backends.

`crates/bmc-attestation/src/sink/backend_tests.rs` covers per-MAC response
validation and exercises HTTP uploads against a localhost backend fixture,
including partial rejection, malformed responses, and typed JWT rejection.

Attestation source unit tests use localhost HTTP fixtures to verify one BMC
credential reload and retry, unchanged measurement nonces, rejection of repeated
unauthorized requests, and safe reload errors. Public collection requires HTTPS.

The runtime suite also covers shared-transport JWT recovery, refreshed inventory
timestamps, and token environment help. Unit tests in
`crates/backend-client/src/lib.rs` cover URL validation, endpoint paths,
enrollment parsing, and typed HTTP errors.
