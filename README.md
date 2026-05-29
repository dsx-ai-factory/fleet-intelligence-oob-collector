# Fleet Intelligence OOB

Standalone Rust OOB inventory collector for Fleet Intelligence.

The current MVP collects BMC inventory through Redfish using `nv-redfish`
directly. NICo's `bmc-explorer` remains useful as a reference implementation,
but it is not linked into this collector. The collector keeps a Fleet
Intelligence owned inventory model between the source and sink so Redfish data
does not become the product schema.

## Structure

```text
src/main.rs                    CLI entry point
src/lib.rs                     library orchestration
src/config.rs                  target config loading
src/inventory/types.rs         FI inventory model
src/inventory/source/mod.rs    source trait
src/inventory/source/redfish.rs nv-redfish source
src/inventory/mapper/mod.rs    mapper trait
src/inventory/mapper/redfish.rs Redfish report to FI inventory mapper
src/inventory/sink/mod.rs      sink trait
src/inventory/sink/table.rs    CLI table sink
```

Pipeline:

```text
config targets
  -> RedfishSource
  -> RedfishMapper
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
