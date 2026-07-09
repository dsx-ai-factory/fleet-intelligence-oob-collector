# Local OTLP observability stack

This Docker Compose stack provides a local destination for telemetry emitted by
the `forge-hw-health` binary from the sibling `infra-controller` repository. It
is intended for development only; it does not connect to the Fleet Intelligence
backend.

## Data flow

```text
forge-hw-health (host)
  └─ OTLP/gRPC :4317 → OpenTelemetry Collector
                           ├─ logs → Loki :3100 ───────┐
                           └─ metrics → :8889          │
                                          ↓           │
                                    Prometheus :9090  │
                                          └───────────┴→ Grafana :3000
```

## Prerequisites

- Docker with the Compose plugin
- Rust and Cargo if you want to run the health service locally

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
| Collector Prometheus exporter | <http://localhost:8889/metrics> |
| Loki API | <http://localhost:3100> |
| Prometheus | <http://localhost:9090> |

## Send health telemetry

Clone `infra-controller` next to this repository if it is not already available
locally:

```bash
cd ..
git clone git@github.com:NVIDIA/infra-controller.git
cd infra-controller
```

These commands assume that you have access to the NVIDIA repository. A different
clone location also works; run the remaining commands from the root of that
clone.

Start with one of the health service example configurations:

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

Run the mock and health service from the `infra-controller` repository in
separate terminals:

```bash
# Terminal 1: serve a mock BMC on localhost:1266.
cargo run -p bmc-mock

# Terminal 2: collect from the configured BMC and export to localhost:4317.
cargo run -p carbide-health --bin forge-hw-health -- \
  "${HOME}/.config/forge-hw-health/config.toml"
```

The Collector assigns the `nico-hardware-health` service name because the health
service's custom OTLP exporter does not currently set `service.name`. In Grafana
Explore, select Loki and start with:

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

The production Fleet Intelligence path uses OTLP/HTTP and enrollment-issued
authentication. This local OTLP/gRPC stack intentionally does not model that
authentication flow.
