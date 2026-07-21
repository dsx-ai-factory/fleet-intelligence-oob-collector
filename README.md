# Fleet Intelligence Out Of Band Collector

Fleet Intelligence Out Of Band Collector is a Rust collector for gathering
Fleet Intelligence data from server BMC interfaces. It is intended to collect
inventory, attestation, and telemetry from out-of-band management paths without
depending on an in-band host agent.

The current implementation collects BMC inventory and attestation through
Redfish using `nv-redfish` and can either print inventory locally or send it to
the Fleet Intelligence backend. NICo hardware health provides the telemetry
collection service deployed by the Helm chart.

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

The collector reads a TOML configuration. BMC targets come from
`endpoint_sources.static_bmc_endpoints`:

```toml
[[endpoint_sources.static_bmc_endpoints]]
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
username = "root"
password = "secret"
rack = "rack-01"
labels = { compute_zone = "az51", node_group = "dev3-dh1" }
```

The OOB collector reads the static endpoint fields it needs and ignores
service-only collector, processor, sink, and metrics settings. The optional
`rack` and `labels` fields are copied to generated NICo health cluster
inventory. The endpoint MAC address identifies the BMC and is included in the
collected inventory. Static endpoints must include a password for OOB inventory
and attestation. API discovery alone is not sufficient for the OOB collector;
the file must contain at least one static endpoint.

Backend API paths are derived from `--backend-url`. Connection settings
(`--insecure-tls`, `--concurrency`, `--timeout-seconds`) are shared between
inventory and attestation collection.

### Proxy support

The collector honors the standard proxy environment variables for Redfish and
Fleet Intelligence backend requests:

- `HTTP_PROXY` / `http_proxy` for HTTP destinations.
- `HTTPS_PROXY` / `https_proxy` for HTTPS destinations.
- `ALL_PROXY` / `all_proxy` as the fallback for either scheme.
- `NO_PROXY` / `no_proxy` for destinations that must connect directly.

HTTP, HTTPS, SOCKS4, and SOCKS5 proxy URLs are supported. Use `socks5h://` when
DNS resolution must happen through the proxy. For example, a local inventory
run through a Teleport SOCKS tunnel can use:

```bash
HTTPS_PROXY=socks5h://127.0.0.1:8888 \
  target/debug/bmc-collector inventory \
  --config /path/to/config.toml \
  --insecure-tls
```

Proxy credentials should be supplied through environment variables or a
Kubernetes Secret and must not be stored in the collector TOML.

## Run Inventory Collection

Print inventory locally (for debugging/testing):

```bash
cargo run -- inventory --config /path/to/config.toml --insecure-tls
target/debug/bmc-collector inventory --config /path/to/config.toml --insecure-tls
target/debug/bmc-collector inventory --config /path/to/config.toml --insecure-tls --detail
```

Generate an atomic, permission-restricted cluster inventory file for NICo
hardware health:

```bash
target/debug/bmc-collector inventory \
  --config /path/to/config.toml \
  --insecure-tls \
  --format nico-health \
  --output /var/lib/fleetint/nico-health-inventory.json
```

The generated file includes the discovered node UUID and hostname, configured
BMC IP/MAC, optional rack and labels, and the credentials NICo health needs to
connect. The first endpoint's credentials become `default_credentials`; nodes
with different credentials receive a per-node override. The output file is
written atomically with mode `0600` so NICo health never observes a partial
inventory and other users cannot read BMC credentials. Every configured target
must complete inventory discovery with a node UUID; otherwise the command fails
and leaves any previously generated NICo inventory file unchanged.

NICo health's cluster source config owns the BMC port, so all generated nodes
must use the same port configured in `endpoint_sources.cluster.port`. Inventory
label names are passed through to telemetry unchanged and should use
Prometheus-compatible names such as `compute_zone` or `node_group`.

## Run Agent (Inventory + Attestation)

Run continuously and send both inventory and attestation to the backend:

```bash
target/debug/bmc-collector run --config /path/to/config.toml --insecure-tls \
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

See `config.example.toml` for an OOB collector configuration. When using the
NICo health sink, point NICo hardware health at the generated cluster inventory
file rather than the collector's source TOML.

## Helm Chart

The chart is published as an OCI artifact at
`oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector`. Create separate
Secrets for the backend bootstrap token and shared hardware configuration
before installing it:

```bash
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-config \
  --from-file=config.toml=/path/to/config.toml

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
`https://data.fleet-intelligence.nvidia.com`. Set `bmcCollector.backendUrl` to
use a different Fleet Intelligence endpoint.
Set `bmcCollector.logLevel` to change the Rust tracing filter from its `info`
default, for example `--set-string bmcCollector.logLevel=debug`.
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
