# Architecture

This page explains what runs, what one collection cycle does, and how the collector keeps BMCs and credentials safe.

## What runs

| Component | What it does |
| --- | --- |
| OOB collector | Collects hardware inventory and GPU attestation evidence from BMCs over Redfish and uploads them to Fleet Intelligence |
| [NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health) | Collects BMC metrics and logs |
| Telemetry gateway | Signs in to Fleet Intelligence and forwards NICo's metrics and logs |

The collector and NICo read the same target list and the same credentials file, so you configure each BMC once:

```text
target list + credentials file
    ├─► OOB collector ──► Fleet Intelligence inventory and attestation APIs
    │       └─ request scheduler ──► BMC Redfish endpoints
    │
    └─► NICo config generator ──► NICo hardware health
                                             │
                                      metrics and logs
                                             ▼
                                      telemetry gateway
                                             ▼
                                   Fleet Intelligence telemetry API
```

Each machine is identified by its BMC MAC address, from configuration through inventory, attestation, and telemetry.

## A collection cycle

When the collector starts, it exchanges its bootstrap token for a Fleet Intelligence session, then runs its first cycle right away. Each cycle does the following:

1. **Reload configuration.** The collector re-reads the target list and credentials file, so edits take effect at the next cycle. If the new configuration is invalid, it skips the cycle rather than collect from an outdated target list.
2. **Collect inventory.** The collector reads many BMCs in parallel and uploads finished nodes in small batches while collection continues. A batch is sent when it has as many nodes as `--target-concurrency`, after 30 seconds, or when inventory collection ends.
3. **Measure latency.** After reading each BMC, the collector measures how long a TCP connection to it takes and includes the result in that node's inventory. See [network latency measurement](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/README.md#network-latency-measurement).
4. **Collect attestation.** For each BMC, the collector asks Fleet Intelligence for a one-time nonce, collects signed GPU evidence from the BMC, and uploads it. This step runs even if some inventory failed. Fleet Intelligence verifies the evidence against the model and firmware details from inventory, so a machine's inventory must have been uploaded at least once.
5. **Wait.** The collector waits for `--interval-seconds` (24 hours by default), then starts the next cycle.

On `SIGINT` or `SIGTERM`, the collector stops collecting, uploads the inventory it has already finished, completes uploads that are in progress, and exits. It doesn't start new attestation uploads.

## When things fail

The collector keeps whatever it could collect, and reports failures node by node.

- **Part of a BMC fails.** If one area, such as memory or firmware, can't be read, the rest of the node's inventory is still uploaded, and the failure is listed in `domainErrors`. Fleet Intelligence keeps its previously stored data for that area. See the [domain error contract](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/README.md#domain-error-contract).
- **A whole BMC fails.** If the BMC can't be read at all, for example because its Redfish service is down, the node is uploaded with a `targetError` and no inventory.
- **An upload is partly rejected.** The collector logs whether Fleet Intelligence accepted or rejected each node, or didn't confirm it either way. One bad node doesn't hold back the rest of its batch, and later batches still run unless the collector is shutting down or can't enroll again.
- **A nonce request fails.** That BMC's attestation is skipped until the next cycle. Skipped BMCs are logged and counted.
- **The Fleet Intelligence session expires.** If an upload or nonce request is rejected with `401 Unauthorized`, the collector enrolls again with the bootstrap token and retries once.
- **A BMC password changes.** If a BMC rejects the login during attestation, the collector reloads the credentials file and retries once, using the same nonce and evidence.

## Request limits

The collector protects BMCs from overload. By default, it reads 64 BMCs at a time, sends at most 4 simultaneous requests and 10 requests per second to any one BMC, and allows short bursts of up to 10 requests. At most 64 × 4 = 256 requests are in flight in total.

Inventory and attestation share these limits. The scheduler takes turns between BMCs and between inventory and attestation, so one slow BMC or one busy workflow can't hold up the others. NICo schedules its own requests separately, and these limits don't cover it.

See [CLI commands](configuration.md#cli-commands) and [Helm values](configuration.md#helm-values) to change the limits.

## Security

**Credentials.** Passwords live only in the credentials file. The collector's TOML, the Helm values, and NICo's generated configuration refer to the file by path and never copy passwords into it. With Helm, the file is mounted from a Kubernetes Secret.

**Kubernetes workloads.** All workloads run as non-root, with read-only root filesystems, no Linux capabilities, and no Kubernetes API token. The collector and NICo restart with `Recreate`, so two copies never collect at the same time; the gateway can be scaled out and uses rolling updates. A NetworkPolicy allows traffic to the gateway's OTLP ports (4317 and 4318) only from pods in the same release and namespace, if your cluster's network plugin enforces NetworkPolicies.

**URLs in BMC data.** BMCs sometimes include full URLs in their responses. Before inventory or attestation data leaves the collector, whether as an upload, JSON, or a table, the collector strips the host, credentials, and query string from Redfish resource links and from URLs inside error and status messages. Links that can't be parsed are replaced with `[URL_REDACTED]`. Inventory fields that are meant to hold addresses, hostnames, and MACs are kept, and attestation evidence and certificates are not changed. This removes URLs only; it doesn't detect other kinds of secrets in BMC text.

**Host package.** The experimental host package runs the same three components as systemd services: `fleetint-oob-collector`, `fleetint-nico-health`, and `fleetint-otel-gateway`, grouped under `fleetint-oob.target`. These services run as root, and the gateway listens only on loopback. All three binaries are statically linked. See the [host package guide](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/packages/README.md).
