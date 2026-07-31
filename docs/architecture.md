# Architecture

The repository separates collection domains from orchestration and keeps
source-protocol models away from Fleet Intelligence backend contracts.

## Components

| Path | Responsibility |
| --- | --- |
| `src/` | CLI, runtime orchestration, enrollment, and backend session lifecycle |
| `crates/bmc-dispatch/` | Shared per-BMC concurrency and rate limiting for Redfish workflows |
| `crates/bmc-inventory/` | Inventory source, mapping, product models, and output sinks |
| `crates/bmc-attestation/` | Backend nonce retrieval, GPU evidence collection, mapping, and upload |
| `deployments/helm/` | Kubernetes packaging for the collector, NICo, and the OTLP gateway |
| `dev/local-observability/` | Development-only OTLP, Loki, Prometheus, and Grafana stack |

`nv-redfish` supplies Redfish clients and traversal. Fleet Intelligence-owned
models form an explicit boundary between those source structures and backend
payloads.

## Inventory flow

```text
collector TOML
    │
    ├─ machine identity, credentials, labels
    ▼
shared Redfish dispatcher
    ▼
RedfishSource ──► RedfishReport ──► RedfishMapper ──► NodeInventory
                                                        │
                         ┌──────────────────────────────┼─────────────┐
                         ▼                              ▼             ▼
                    TableSink                       JsonSink    BackendSink
```

The source collects domain-oriented Redfish data. The mapper selects primary
node identity, performs best-effort reverse DNS, normalizes optional fields,
and creates the backend-facing inventory model. Output sinks do not own source
heuristics.

Detailed selection, partial-collection, and vendor rules are documented in
[Inventory collection notes](../crates/bmc-inventory/README.md).

## Agent cycle

The long-running `run` command has one authenticated backend session and one
shared Redfish dispatcher:

```text
bootstrap token
    ▼
POST /v1/agent/enroll ──► JWT
    │
    ├─► collect inventory ──► PUT /v1/agent/oob_nodes
    │                              │
    │                              └─ successful upload
    │                                      ▼
    └────────────────────────────── collect attestation
                                           │
                         backend nonce ◄───┤
                         GPU evidence ◄────┤
                                           ▼
                                  attestation upload
                                           │
                                           ▼
                                   wait for interval
```

Attestation targets are derived by matching configured BMC MACs to inventory
records that have a resolved node UUID. If inventory collection or upload
fails for the cycle, attestation is skipped. A `401 Unauthorized` from an
inventory or attestation backend call triggers re-enrollment and one retry;
other failures are reported without assuming the JWT is invalid.

## Concurrency and BMC protection

Inventory and attestation share a dispatcher keyed by BMC MAC. It applies:

- A global in-flight ceiling derived as target concurrency multiplied by the
  per-BMC in-flight limit.
- Round-robin scheduling across BMC endpoints.
- A maximum number of in-flight requests per BMC.
- A per-BMC token bucket for sustained request rate and burst allowance.
- Separate, round-robin inventory and attestation queues for each BMC.
- A bound of 256 pending requests per workflow queue.

This prevents inventory and attestation from independently overwhelming the
same management controller while preventing either workflow or a busy endpoint
from monopolizing request admission. Configuration is synchronized between
agent cycles so changes to the target set are reflected without starting a
second dispatcher. The CLI exposes the primary controls; see [Collection
controls](usage.md#collection-controls).

## Identity and failure boundaries

The configured BMC MAC is the target correlation key. The primary node UUID is
normally selected from Redfish `ComputerSystem.UUID`; deterministic fallbacks
handle absent or malformed UUIDs. The BMC service UUID remains source metadata
and is not substituted directly as node identity.

Inventory preserves useful data when optional domains fail. Recoverable
failures appear in `domainErrors`; a failure that prevents meaningful target
collection appears in `targetError`. Backend ingestion can therefore avoid
clearing a domain merely because the BMC temporarily failed to return it.

Attestation is also reported per node and can preserve component-level errors
alongside successful evidence from other components.

## Kubernetes telemetry flow

```text
collector Secret ──► bmc-collector ──► inventory + attestation APIs
        │
        └─► render-nico-config init container
                        │
                        ▼
                 NICo hardware health
                   │ metrics + logs
                   ▼
              OTLP gateway ──► telemetry API
```

The same Secret drives both collector targets and generated NICo endpoints, so
operators do not duplicate BMC credentials. Generated NICo TOML is held in a
memory-backed pod-local volume. The collector and NICo Deployments use
`Recreate` to avoid duplicate collection during rollout; the independently
scalable OTLP gateway uses `RollingUpdate`.

See [Kubernetes deployment](kubernetes.md) for installation and operational
details.

## Security boundaries

- BMC credentials remain in the local TOML or a Kubernetes Secret.
- The backend bootstrap token is separate from the BMC configuration.
- Enrollment exchanges the bootstrap token for a runtime JWT.
- A trusted edge injects customer identity; the collector does not assert it.
- Kubernetes workloads run without a service account token, as non-root users,
  with dropped capabilities and read-only root filesystems.
- The default NetworkPolicy restricts OTLP gateway ingress to pods in the same
  Helm release, subject to CNI enforcement.
