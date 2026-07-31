# Usage and behavior

## Quick start

Create the configuration and install the chart:

```bash
cp config.example.toml config.toml
# Replace the example BMC address and credentials.

kubectl create namespace fleet-intelligence
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"
kubectl create secret generic fleet-intelligence-config \
  --namespace fleet-intelligence \
  --from-file=config.toml=config.toml

helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --namespace fleet-intelligence \
  --wait
```

See [Installation](installation.md) for prerequisites and private image access,
and [Configuration](configuration.md) for all supported inputs.

## Collection behavior

The collector runs one cycle immediately and repeats every 24 hours by default.
Each cycle:

1. Collects inventory from all configured BMCs.
2. Uploads inventory to `PUT /v1/agent/oob_nodes`.
3. Collects attestation for inventory records with a resolved node UUID when
   the inventory upload succeeds.
4. Uploads the attestation batch.
5. Waits for the configured interval.

At startup, the collector exchanges the bootstrap agent token at
`POST /v1/agent/enroll` for a runtime JWT. A `401 Unauthorized` during upload
causes one re-enrollment and retry. The collector does not send customer
identity headers; a trusted edge must inject that context.

Inventory preserves partial results. Recoverable domain failures are returned
in `domainErrors`, while a failure that prevents meaningful target collection
is returned in `targetError`. See the
[inventory collection notes](../crates/bmc-inventory/README.md) for identity and
mapping rules.

## Request concurrency

Defaults are 64 concurrent target workflows, four in-flight requests per BMC,
10 requests per second per BMC, and a burst capacity of 10. The global request
ceiling is the target concurrency multiplied by the per-BMC limit.

The shared dispatcher schedules BMCs round-robin, independently rate-limits
each BMC, and fairly alternates inventory and attestation requests. This keeps a
busy endpoint or workflow from monopolizing collection.

## Telemetry behavior

NICo hardware health is enabled by default. An init container converts the
collector TOML into NICo endpoints, and NICo sends metrics and logs over
OTLP/gRPC to the bundled gateway. The gateway enrolls separately and forwards
telemetry over authenticated OTLP/HTTP.

NICo log collection uses `auto` mode: it prefers Redfish SSE and falls back to
LogService polling for endpoints without EventService support.
