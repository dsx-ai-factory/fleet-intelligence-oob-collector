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

## Container Image

Build the image locally:

```bash
make container-build
docker run --rm fleet-intelligence-oob-collector:dev --help
```

Release tags publish Linux `amd64` and `arm64` images to
`ghcr.io/nvidia/fleet-intelligence-oob-collector`. For example:

```bash
docker pull ghcr.io/nvidia/fleet-intelligence-oob-collector:0.1.0
```

The runtime image uses Distroless `static-debian12:nonroot` with a statically
linked `bmc-collector` binary. It runs as a numeric non-root user and contains no
shell or package manager while retaining minimal runtime files such as CA
certificates.

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

## Helm Chart

The chart is published as an OCI artifact at
`oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector`. Create separate
Secrets for the backend bootstrap token and BMC target config before installing
it:

```bash
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-oob-collector-bmc-config \
  --from-file=targets.config=/path/to/oob.config

kubectl create secret docker-registry ghcr-pull-secret \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN"

helm install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set 'imagePullSecrets[0].name=ghcr-pull-secret'
```

The chart intentionally references existing Secrets instead of accepting
credentials as values, which keeps credentials out of Helm release state. See
`deployments/helm/fleet-intelligence-oob-collector/values.yaml` for the supported image
and collector settings. The image pull Secret is only required for a private
collector image. Validate local chart changes with `make chart-lint`.
By default, the collector sends data to
`https://data.fleet-intelligence.nvidia.com`. Set `backendUrl` to use a
different Fleet Intelligence endpoint.
By default, the chart version is also used as the collector image tag.

## Releases

Pushing a semantic version tag such as `v0.1.0` runs tests, publishes the
multi-architecture container image, and publishes a chart with the matching
version. Pre-release tags such as `v0.2.0-rc.1` are supported and do not update
the image's `latest` tag.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup, validation
commands, pull request expectations, and contribution sign-off guidance.

## Security

Do not report security vulnerabilities through public GitHub issues. See
[SECURITY.md](SECURITY.md) for NVIDIA private vulnerability reporting channels.

## License

This project is licensed under Apache-2.0. See [LICENSE](LICENSE).
