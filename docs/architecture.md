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

Inventory follows a source → model → sink pipeline. The Redfish source builds
the Fleet Intelligence inventory model directly, keeping vendor-specific
Redfish structures inside the source.
Inventory and attestation use the same dispatcher so their requests share
per-BMC concurrency and rate limits.

After each target's Redfish collection, the source measures TCP connect time to
the BMC to record collector-to-BMC network latency. The probe round uses one
inventory dispatcher admission, and a once-per-run check against an unroutable
address detects local TCP interception. Measurements are uploaded with each
node's inventory; see the
[inventory collection notes](../crates/bmc-inventory/README.md#network-latency-measurement).

The required configured BMC MAC identifies observations and correlates targets
across workflows. Inventory node identity resolution is independent from
attestation. Attestation uses the normalized BMC MAC to request a nonce and key
uploaded evidence. Completed
inventories flow through a bounded channel to a backend worker, which flushes small batches by size, time, or the
end of the collection cycle. The current worker flushes when the batch reaches
the configured target concurrency or after 30 seconds, and the channel holds
at most twice that concurrency. Backend item results are matched to submitted
observations by position, so one rejected item does not hide accepted nodes in the same batch. Attestation
runs for configured BMC targets after the inventory phase, even if that phase
fails. The backend requires an active OOB source registered through inventory,
including sources registered in an earlier cycle; a resolved node UUID is not
required. Backend verification also uses inventory-derived model and firmware
metadata. After concurrent nonce and evidence collection completes, results are
uploaded in mini-batches bounded by the same target concurrency. Attestation
retains its collection barrier so nonce-driven JWT rotation and
whole-collection authentication retry remain deterministic. Upload responses
are matched by BMC MAC, independently of response order. Each target is reported
as accepted, rejected, or unconfirmed. Missing or duplicate results leave only
that target unconfirmed; an unreadable response leaves the mini-batch unconfirmed.
Unexpected response rows are reported separately. Other mini-batches still run
unless shutdown is requested or re-enrollment fails. Accepted items are not
retried because another item fails.

## Collection lifecycle

At startup, the collector exchanges its bootstrap token for a runtime JWT. It
then runs one collection cycle immediately and repeats every 24 hours by
default. Each cycle collects inventory concurrently while a bounded backend
worker uploads completed nodes in mini-batches. It then collects attestation for
configured BMCs by MAC, uploads bounded
attestation mini-batches, and waits for the configured interval. A backend `401 Unauthorized`
during an inventory upload, attestation nonce request, or attestation upload
causes one re-enrollment and retry. Re-enrollment cannot fix a `401` caused by
missing customer context; a trusted edge must inject that context before
requests reach the agent API.

The collector loads one resolved target and credential snapshot per cycle and
synchronizes the dispatcher's BMC endpoint set before starting collection.
Inventory and attestation use that same snapshot. Startup validation supplies
the first snapshot; subsequent cycles reload the TOML and JSON credential files.
Target updates during a cycle take effect on the next cycle. During attestation,
a BMC `401 Unauthorized` reloads credentials for that target and retries the
failed request once, keeping the same nonce and previously collected evidence.
Only one credential reload is allowed per target per attestation attempt. Invalid
configuration skips the cycle without collecting against a stale target set.

The runtime listens for SIGINT and SIGTERM throughout enrollment, collection,
and sleep. Shutdown cancels Redfish collection and closes the inventory
producer, drains its bounded queue of completed observations, and finishes
in-flight uploads. Attestation collection can be cancelled; shutdown does not
start another attestation upload batch.

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
Helm generates only the collector TOML from `inventory` values. ConfigMap
checksum annotations roll both deployments when that inventory changes, and
NICo's init container regenerates its TOML with `render-nico-config`.
Generated NICo configuration is held in a pod-local `emptyDir` volume. The
collector and NICo use `Recreate` deployments to avoid duplicate collection;
the independently scalable gateway uses `RollingUpdate`.

Workloads run as non-root, without service account tokens or Linux
capabilities, and with read-only root filesystems. The default NetworkPolicy
restricts gateway ingress to pods in the same Helm release when enforced by the
cluster CNI.

The experimental `fleetint-oob` host package runs the same three components as
`fleetint-oob-collector`, `fleetint-nico-health`, and `fleetint-otel-gateway` systemd
services, grouped by `fleetint-oob.target` for full-stack start and stop. NICo's
`ExecStartPre` renders NICo's private configuration under `/run` before the
service starts. These prototype services run as root; the gateway
listens only on loopback. Rust binaries are built statically with musl and the
gateway with Go's CGO disabled; package builds verify no ELF interpreter or
shared-library requirements remain. See the [host package prototype](../deployments/packages/README.md)
for installation, credentials, and validation limits.

### Redfish URI sanitization

The Redfish source removes URL authorities (including credentials) and queries
from `@odata.id` references while building product inventory models. Absolute
and scheme-relative references retain only their resource path and fragment;
malformed or pathless URL references are replaced with `[URL_REDACTED]`.
Relative resource paths remain relative, with query data removed.

Inventory errors, condition messages, message arguments, resolutions, and
attestation errors also sanitize embedded URLs, including scheme-relative URLs,
before output models reach the backend, JSON, or table sinks. Intentional inventory
addresses, hostnames, and MAC addresses remain available, while attestation evidence
and certificates are not rewritten. This is URL sanitization, not general secret
detection in arbitrary BMC-provided strings.

Product-model URI and error fields use transparent sanitized string types with
private inner values. All public construction paths therefore apply the same
sanitization, including callers that construct models without using the Redfish
source. Their serialized JSON representation remains a normal string.
