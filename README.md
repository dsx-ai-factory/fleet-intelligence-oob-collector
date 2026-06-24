# Fleet Intelligence Out Of Band Collector

Fleet Intelligence Out Of Band Collector is a Rust collector for gathering
Fleet Intelligence data from server BMC interfaces. It is intended to collect
inventory, attestation, and telemetry from out-of-band management paths without
depending on an in-band host agent.

The current implementation collects BMC inventory through Redfish using
`nv-redfish` and can either print inventory locally or send it to the Fleet
Intelligence backend. Attestation and telemetry crates are present as workspace
placeholders for the next collector domains.

The collector keeps Fleet Intelligence owned models between the BMC source and
the output/backend sinks so vendor protocol data does not become the product
schema.

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

## Run Inventory Collection

Print inventory locally:

```bash
cargo run -- inventory --config /path/to/.config --insecure-tls
target/debug/bmc-collector inventory --config /path/to/.config --insecure-tls
target/debug/bmc-collector inventory --config /path/to/.config --insecure-tls --detail
```

Run continuously and send inventory to the backend:

```bash
target/debug/bmc-collector run --config /path/to/.config --insecure-tls \
  --backend-url https://api.example.com \
  --agent-token "$FLEETINT_AGENT_TOKEN"
```

`inventory` writes local table or JSON output only. Backend submission happens
only in `run`.

`run` exchanges the bootstrap agent token with `POST /v1/agent/enroll`, then
uses the returned JWT for `PUT /v1/agent/oob_nodes`. The backend URL must be the
API base URL without an API path suffix. `--backend-url` and `--agent-token` can
also be provided via `FLEETINT_BACKEND_URL` and `FLEETINT_AGENT_TOKEN`.

The collector does not send customer identity headers. In deployed environments,
the trusted edge/auth proxy injects the customer context before the request
reaches the agent API.

`run` is the long-running agent mode. It performs one backend sync immediately,
then repeats every 24 hours by default. Override the cadence with
`--interval-seconds`.

The target config file uses INI-style sections:

```ini
[node-name]
address=10.0.0.10
username=root
password=secret
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, validation
commands, pull request expectations, and contribution sign-off guidance.

## Security

Do not report security vulnerabilities through public GitHub issues. See
[SECURITY.md](SECURITY.md) for NVIDIA private vulnerability reporting channels.

## License

This project is licensed under Apache-2.0. See [LICENSE](LICENSE).
