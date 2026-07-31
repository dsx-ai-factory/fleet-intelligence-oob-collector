# Architecture

The repository separates Redfish collection from Fleet Intelligence data
models and backend transport.

## Components

| Component | Responsibility |
| --- | --- |
| `src/` | Runtime orchestration, enrollment, and backend session lifecycle |
| `bmc-dispatch` | Shared per-BMC concurrency, rate limiting, and fair scheduling |
| `bmc-inventory` | Redfish inventory collection, mapping, and output |
| `bmc-attestation` | Nonce retrieval, GPU evidence collection, and upload |
| NICo hardware health | BMC metrics and logs |
| OpenTelemetry gateway | Authenticated telemetry export |

## Data flow

```text
collector Secret
    ├─► OOB collector ──► inventory and attestation APIs
    │       └─ shared dispatcher ──► BMC Redfish endpoints
    │
    └─► NICo config init container ──► NICo hardware health
                                             │
                                      metrics and logs
                                             ▼
                                      OTLP gateway
                                             ▼
                                       telemetry API
```

Inventory follows a source → mapper → model → sink pipeline. This keeps
vendor-specific Redfish structures out of the Fleet Intelligence schema.
Inventory and attestation use the same dispatcher so their requests share
per-BMC concurrency and rate limits.

The configured BMC MAC correlates targets across workflows. Inventory normally
uses `ComputerSystem.UUID` for node identity and provides deterministic
fallbacks when it is missing. Attestation runs only after a successful
inventory upload and only for records with a resolved node UUID.

## Deployment boundaries

The same Secret supplies BMC targets to the collector and NICo config renderer,
avoiding duplicated credentials. Generated NICo configuration is held in a
memory-backed pod volume. The collector and NICo use `Recreate` deployments to
avoid duplicate collection; the independently scalable gateway uses
`RollingUpdate`.

Workloads run as non-root, without service account tokens or Linux
capabilities, and with read-only root filesystems. The default NetworkPolicy
restricts gateway ingress to pods in the same Helm release when enforced by the
cluster CNI.
