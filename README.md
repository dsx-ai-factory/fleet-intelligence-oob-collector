# Fleet Intelligence OOB

Standalone Rust OOB inventory collector for Fleet Intelligence.

The current MVP collects BMC inventory through Redfish using NICo's
`bmc-explorer` abstraction pinned to the stable `infra-controller` `v0.9.0`
tag. The collector keeps a Fleet Intelligence owned inventory model between the
source and sink so NICo remains the Redfish compatibility layer, not the product
schema.

## Structure

```text
src/main.rs                    CLI entry point
src/lib.rs                     library orchestration
src/config.rs                  target config loading
src/inventory/types.rs         FI inventory model
src/inventory/source/mod.rs    source trait
src/inventory/source/redfish.rs Redfish/NICo source
src/inventory/mapper/mod.rs    mapper trait
src/inventory/mapper/nico.rs   NICo report to FI inventory mapper
src/inventory/sink/mod.rs      sink trait
src/inventory/sink/table.rs    CLI table sink
```

Pipeline:

```text
config targets
  -> RedfishSource
  -> NicoMapper
  -> NodeInventory
  -> TableSink
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
