# Fleet Intelligence OOB

Standalone Rust OOB collector for Fleet Intelligence.

The current MVP collects BMC inventory through Redfish using `nv-redfish`
directly. NICo's `bmc-explorer` remains useful as a reference implementation,
but it is not linked into this collector. The collector keeps a Fleet
Intelligence owned inventory model between the source and sink so Redfish data
does not become the product schema.

## Workspace Structure

```text
src/main.rs                         CLI entry point
src/lib.rs                          CLI orchestration
crates/bmc-inventory/README.md      inventory assumptions and platform quirks
crates/bmc-inventory/src/config.rs  target config loading
crates/bmc-inventory/src/models/    FI inventory models
crates/bmc-inventory/src/source/    source trait and nv-redfish source
crates/bmc-inventory/src/mapper/    Redfish report to FI inventory mapper
crates/bmc-inventory/src/sink/      table, JSON, and backend sinks
crates/bmc-attestation/             attestation crate placeholder
crates/bmc-telemetry/               telemetry crate placeholder
```

Pipeline:

```text
config targets
  -> RedfishSource
  -> RedfishMapper
  -> NodeInventory
  -> TableSink / JsonSink / BackendSink
```

## Build

```bash
cargo build
```

This creates:

```text
target/debug/bmc-collector
```

For an optimized binary:

```bash
cargo build --release
```

## Inventory

```bash
cargo run -- inventory --config /path/to/.config --insecure-tls
target/debug/bmc-collector inventory --config /path/to/.config --insecure-tls
target/debug/bmc-collector inventory --config /path/to/.config --insecure-tls --detail
```

The config format matches the Go POC:

```ini
[node-name]
address=10.0.0.10
username=root
password=secret
```
