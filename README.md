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
crates/bmc-attestation/             attestation crate
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

## Configuration

The config file lists BMC targets in INI-style sections:

```ini
[node-name]
address = 10.0.0.10
username = root
password = secret
```

Each `[section]` is a BMC target with `address`, `username`, and `password`.

Backend API paths are derived from `--backend-url`. Connection settings
(`--insecure-tls`, `--concurrency`, `--timeout-seconds`) are shared between
inventory and attestation collection.

## Run Inventory Collection

Print inventory locally (for debugging/testing):

```bash
cargo run -- inventory --config /path/to/oob.config --insecure-tls
target/debug/bmc-collector inventory --config /path/to/oob.config --insecure-tls
target/debug/bmc-collector inventory --config /path/to/oob.config --insecure-tls --detail
```

## Run Agent (Inventory + Attestation)

Run continuously and send both inventory and attestation to the backend:

```bash
target/debug/bmc-collector run --config /path/to/oob.config --insecure-tls \
  --backend-url http://127.0.0.1:18002 \
  --agent-token "$FLEETINT_AGENT_TOKEN"
```

`--backend-url` and `--agent-token` can also be provided via `FLEETINT_BACKEND_URL`
and `FLEETINT_AGENT_TOKEN` environment variables.

Each `run` cycle:

1. Collects inventory from all BMC targets concurrently
2. Uploads the inventory batch to `PUT /v1/agent/oob_nodes`
3. For targets where inventory discovered a system UUID, collects attestation
4. Uploads the attestation batch to the backend attestation endpoint
5. Sleeps for the configured interval (default 24 hours, override with
   `--interval-seconds`)

The agent enrolls with `POST /v1/agent/enroll` using the bootstrap agent token
and uses the returned JWT for all subsequent backend requests.

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
