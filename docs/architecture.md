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
| [NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health) | BMC metrics and logs |
| OpenTelemetry gateway | Authenticated telemetry export |

## Data flow

```text
target config + projected credential files
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
fallbacks when it is missing. Completed inventories flow through a bounded
channel to a backend worker, which flushes small batches by size, time, or the
end of the collection cycle. The current worker flushes when the batch reaches
the configured target concurrency or after one second, and the channel holds at
most twice that concurrency. The inventory backend contract is status-only, so
any 2xx response accepts the complete mini-batch and response bodies are
ignored. Attestation runs only for records from successful mini-batches with a
resolved node UUID. After
concurrent nonce and evidence collection completes, attestation results are
uploaded in mini-batches bounded by the same target concurrency. Attestation
retains its collection barrier so nonce-driven JWT rotation and
whole-collection authentication retry remain deterministic.

## Collection lifecycle

At startup, the collector exchanges its bootstrap token for a runtime JWT. It
then runs one collection cycle immediately and repeats every 24 hours by
default. Each cycle collects inventory concurrently while a bounded backend
worker uploads completed nodes in mini-batches. It then collects attestation for
successfully exported nodes with resolved identities, uploads bounded
attestation mini-batches, and waits for the configured interval. A backend `401 Unauthorized`
during an inventory upload, attestation nonce request, or attestation upload
causes one re-enrollment and retry. Re-enrollment cannot fix a `401` caused by
missing customer context; a trusted edge must inject that context before
requests reach the agent API.

The collector reloads the TOML and synchronizes the dispatcher's BMC endpoint
set at the start of each cycle. This allows target changes from a projected
Kubernetes Secret to take effect without restarting the collector.

Inventory preserves partial results. Recoverable domain failures are returned
in `domainErrors`; failures that prevent meaningful target collection are
returned in `targetError`. Detailed identity, mapping, and failure rules are in
the [inventory collection notes](../crates/bmc-inventory/README.md).

## Request admission

The default limits are 64 concurrent target workflows, four in-flight requests
per BMC, 10 requests per second per BMC, and a burst capacity of 10. The global
in-flight ceiling is the target concurrency multiplied by the per-BMC in-flight
limit: `64 × 4 = 256` requests by default. The per-BMC rate and burst limits are
applied separately.

The dispatcher schedules BMCs round-robin, independently rate-limits each BMC,
and fairly alternates inventory and attestation requests. This prevents a busy
endpoint or workflow from monopolizing collection.

## Deployment boundaries

The collector and NICo config renderer share the password-free target
configuration and projected credential files, avoiding duplicated credentials.
Generated NICo configuration is held in a memory-backed pod volume. The
collector and NICo use `Recreate` deployments to avoid duplicate collection;
the independently scalable gateway uses `RollingUpdate`.

Workloads run as non-root, without service account tokens or Linux
capabilities, and with read-only root filesystems. The default NetworkPolicy
restricts gateway ingress to pods in the same Helm release when enforced by the
cluster CNI.
