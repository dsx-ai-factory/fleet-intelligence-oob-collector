# Fleet Intelligence OOB

Standalone Rust OOB inventory collector for Fleet Intelligence.

The current MVP collects BMC inventory through Redfish using NICo's
`bmc-explorer` abstraction pinned to the stable `infra-controller` `v0.9.0`
tag. The collector keeps a Fleet Intelligence owned inventory model between the
source and sink so NICo remains the Redfish compatibility layer, not the product
schema.

## Structure

```text
src/main.rs               CLI entry point
src/config.rs             target config loading
src/inventory/source.rs   Redfish/NICo source
src/inventory/mapper.rs   NICo report to FI inventory mapper
src/inventory/types.rs    FI inventory model
src/inventory/sink.rs     CLI table sink
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
```

The config format matches the Go POC:

```ini
[node-name]
address=10.0.0.10
username=root
password=secret
```
