# Usage

The `bmc-collector` binary has three commands:

```text
inventory           Collect BMC inventory and print it locally
run                 Continuously collect and export to Fleet Intelligence
render-nico-config  Generate NICo static configuration from collector TOML
```

Run `bmc-collector <command> --help` for the authoritative option list for the
installed version.

## Inspect inventory locally

Use table output for a quick inspection:

```bash
target/debug/bmc-collector inventory --config config.toml
```

Add `--detail` for expanded table output, or request the complete API-shaped
payload as JSON:

```bash
target/debug/bmc-collector inventory \
  --config config.toml \
  --format json
```

Local inventory mode does not enroll with or write to the Fleet Intelligence
backend. Collection errors are represented in the output where possible; see
[Inventory collection notes](../crates/bmc-inventory/README.md#partial-collection).

## Run continuous backend sync

Pass the Fleet Intelligence API base URL without a `/v1/agent` suffix and a
bootstrap agent token:

```bash
target/debug/bmc-collector run \
  --config config.toml \
  --backend-url https://data.fleet-intelligence.nvidia.com \
  --agent-token "$FLEETINT_AGENT_TOKEN"
```

The backend settings may instead be provided through environment variables:

```bash
export FLEETINT_BACKEND_URL=https://data.fleet-intelligence.nvidia.com
export FLEETINT_AGENT_TOKEN='<bootstrap-token>'
target/debug/bmc-collector run --config config.toml
```

At startup, the collector enrolls at `POST /v1/agent/enroll` and uses the
returned JWT for subsequent calls. Each cycle then:

1. Collects inventory from configured BMCs.
2. Uploads the batch to `PUT /v1/agent/oob_nodes`.
3. Collects attestation for targets with a resolved node UUID, but only
   when the inventory upload succeeded.
4. Uploads the attestation batch.
5. Waits for the next interval.

The first cycle runs immediately. The default interval is 86,400 seconds (24
hours); change it with `--interval-seconds`. `Ctrl-C` stops the runtime while it
is waiting between cycles. A rejected JWT causes one re-enrollment and retry.

The collector does not send customer identity headers. A trusted edge or auth
proxy must inject customer context before requests reach the agent API.

## Collection controls

The `inventory` and `run` commands share these Redfish controls:

| Option | Default | Purpose |
| --- | ---: | --- |
| `--target-concurrency` | `64` | Maximum targets collected concurrently. Values below one are treated as one. |
| `--bmc-max-in-flight` | `4` | Maximum concurrent Redfish requests per BMC. Must be greater than zero. |
| `--bmc-requests-per-second` | `10` | Per-BMC sustained request rate. Must be greater than zero. |
| `--bmc-burst` | `10` | Per-BMC token-bucket capacity. Must be greater than zero. |
| `--timeout-seconds` | `30` | Timeout for Redfish requests. |
| `--insecure-tls` | Off | Disable BMC certificate verification. |

The dispatcher sets its global in-flight ceiling to the product of
`--target-concurrency` and `--bmc-max-in-flight`. Within that ceiling it
schedules BMCs round-robin, applies the concurrency and token-bucket limits
independently to each BMC, and fairly alternates queued inventory and
attestation requests for the same BMC. Each workflow has a bounded queue of 256
requests. Tune these values carefully for older BMCs and large fleets.

## Render NICo configuration

The Helm chart uses this command in an init container, but it is also useful
for inspecting generated NICo settings locally:

```bash
target/debug/bmc-collector render-nico-config \
  --config config.toml \
  --output nico-config.toml \
  --otlp-endpoint http://localhost:4317 \
  --metrics-endpoint 0.0.0.0:9009 \
  --metrics-prefix fleetint
```

The output includes BMC credentials and is written with mode `0640` on Unix.
Treat it as a secret and delete it when it is no longer needed. The generated
configuration enables Prometheus and OTLP sinks and enables BMC logs in `auto`
mode, which prefers Redfish SSE and falls back to LogService polling per
endpoint.

## Logging

Logs are written to standard error. Set `RUST_LOG` to a Rust tracing filter:

```bash
RUST_LOG=debug target/debug/bmc-collector inventory --config config.toml
```

More targeted filters reduce noise:

```bash
RUST_LOG='fleet_intelligence_oob=debug,bmc_attestation=trace' \
  target/debug/bmc-collector run --config config.toml
```

Proxy setup and TLS behavior are covered in
[Configuration](configuration.md#proxy-environment-variables).
