# Local OTLP observability stack

This Docker Compose stack provides a local destination for telemetry from
[NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health).
This stack is for development only: it does not run the OOB collector, perform
backend enrollment, or connect to Fleet Intelligence services.

## Data flow

```text
NICo hardware health (host)
  └─ OTLP/gRPC :4317 → OpenTelemetry Collector
                           ├─ logs → Loki :3100 ───────┐
                           └─ metrics → :8889          │
                                          ↓           │
                                    Prometheus :9090  │
                                          └───────────┴→ Grafana :3000
```

## Prerequisites

- Docker with the Compose plugin
- Rust and Cargo if you want to run NICo hardware health locally

## Start the stack

From this directory:

```bash
docker compose up -d
docker compose ps
```

All host-facing ports bind to `127.0.0.1`. Open
[Grafana](http://localhost:3000) to explore the data; anonymous administrator
access is enabled for this local stack.

The other local endpoints are:

| Service | URL or endpoint |
| --- | --- |
| OTLP/gRPC receiver | `http://localhost:4317` |
| OpenTelemetry Collector Prometheus exporter | <http://localhost:8889/metrics> |
| Loki API | <http://localhost:3100> |
| Prometheus | <http://localhost:9090> |

## Send NICo telemetry

Clone `infra-controller` into a separate working directory and run the
remaining commands from its repository root:

```bash
git clone https://github.com/NVIDIA/infra-controller.git
cd infra-controller
```

Start with one of NICo's example configurations:

- `crates/health/example/config.bmc-mock.toml` for the local BMC mock
- `crates/health/example/config.example.toml` as the starting point for real
  hardware

Copy the selected example outside the Git checkout. For a zero-hardware test:

```bash
mkdir -p "${HOME}/.config/forge-hw-health"
cp crates/health/example/config.bmc-mock.toml \
  "${HOME}/.config/forge-hw-health/config.toml"
chmod 600 "${HOME}/.config/forge-hw-health/config.toml"
```

In `~/.config/forge-hw-health/config.toml`, enable the OTLP sink and target this
stack:

```toml
[sinks.otlp]
enabled = true
endpoint = "http://localhost:4317"
```

Keep real BMC addresses and credentials outside both repositories and do not
commit them.

Run the mock and NICo's `forge-hw-health` binary from the `infra-controller`
repository in separate terminals:

```bash
# Terminal 1: serve a mock BMC on localhost:1266.
cargo run -p bmc-mock

# Terminal 2: collect from the configured BMC and export to localhost:4317.
cargo run -p carbide-health --bin forge-hw-health -- \
  "${HOME}/.config/forge-hw-health/config.toml"
```

The OpenTelemetry Collector assigns the `nico-hardware-health` service name
because NICo's custom OTLP exporter does not currently set `service.name`. In
Grafana Explore, select Loki and start with:

```logql
{service_name="nico-hardware-health"}
```

Select Prometheus to inspect the collector-exported hardware samples.

## Stop and troubleshoot

```bash
docker compose logs -f otel-collector
docker compose down
```

Use `docker compose down --volumes` if you also want to discard all locally
stored Grafana, Loki, and Prometheus data.

The production Fleet Intelligence path sends NICo OTLP/gRPC to the chart's
gateway, which then uses OTLP/HTTP and enrollment-issued authentication for the
backend. This local stack intentionally does not model that authentication
flow. See the repository [architecture guide](../../docs/architecture.md) for
the production data flow.
