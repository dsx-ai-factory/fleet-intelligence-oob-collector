# Fleet Intelligence OOB Collector

This chart deploys one long-running Fleet Intelligence out-of-band collector.
When enabled, NICo hardware health runs as a sidecar in the same pod and reads
generated cluster inventory from a shared `emptyDir`. The Deployment uses a
`Recreate` strategy so duplicate collectors do not run during an update.

## Prerequisites

Create one Secret for the backend bootstrap token and another for the OOB
collector's `[[machines]]` TOML configuration:

```bash
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-config \
  --from-file=config.toml=/path/to/config.toml
```

When NICo hardware health is enabled, create a separate Secret for its service
configuration. Configure its cluster endpoint source to read the generated
inventory path:

```toml
# The OOB collector owns endpoint assignment. Do not shard this local file again.
shard = 0
shards_count = 1

[endpoint_sources.cluster]
inventory_path = "/var/lib/fleetint/nico-health-inventory.json"
```

```bash
kubectl create secret generic nico-hardware-health-config \
  --from-file=config.toml=/path/to/nico-health-config.toml
```

If the collector image is private, create a Docker registry Secret with a GHCR
token that has `read:packages` access:

```bash
kubectl create secret docker-registry ghcr-pull-secret \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN"
```

## Install

```bash
helm install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set 'imagePullSecrets[0].name=ghcr-pull-secret'
```

Set `bmcCollector.agentTokenSecretName` when the token Secret uses a different
name. Set `config.secretName` or `config.secretKey` when the OOB configuration
is stored under a different Secret or key. Set
`nicoHardwareHealth.config.secretName` or `secretKey` for the NICo service
configuration. The chart does not accept credentials as values, preventing
them from being stored in Helm release state. Omit `imagePullSecrets` when the
images are public.

The OOB TOML must define at least one `[[machines]]` entry. The BMC collector
does not use NICo API endpoint discovery.

## Proxy configuration

The collector honors `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY`
(including their lowercase forms). HTTP, HTTPS, and SOCKS proxy URLs are
supported. Configure a plain, cluster-reachable proxy directly in Helm values:

```yaml
bmcCollector:
  proxy:
    httpProxy: http://proxy.example.internal:3128
    httpsProxy: http://proxy.example.internal:3128
    allProxy: ""
    noProxy: localhost,127.0.0.1,.svc,.cluster.local
```

A Teleport SOCKS tunnel listening on a developer workstation is suitable for
local collector runs, but Kubernetes cannot use `127.0.0.1` on that workstation.
The cluster deployment requires a cluster-reachable proxy or a proxy sidecar.
Authenticated proxy URLs are not supported by the chart yet and must not be
committed to values files.

By default, the collector sends data to
`https://data.fleet-intelligence.nvidia.com`. Set `bmcCollector.backendUrl` to
use a different Fleet Intelligence endpoint.

The collector logs at `info` by default. Set `bmcCollector.logLevel` to a Rust
tracing filter such as `debug` or `fleet_intelligence_oob=debug,bmc_attestation=trace`.

See `values.yaml` for the small set of supported settings.
When `bmcCollector.image.tag` is empty, the chart uses its own version as the
image tag.

## NICo hardware health

NICo hardware health is enabled by default and runs as a sidecar. The OOB
collector writes `nicoHardwareHealth.inventoryOutputPath` into an ephemeral
volume mounted by both containers. NICo retries discovery until the first
inventory file is available. It can be disabled when only inventory and
attestation are needed:

```bash
helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set nicoHardwareHealth.enabled=false
```

The service image is `ghcr.io/nvidia/nico-hardware-health`; its tag defaults to
the chart version. The NICo service configuration should use cluster file
discovery and its `inventory_path` must equal
`nicoHardwareHealth.inventoryOutputPath`. When using NICo API mTLS, create a Secret containing `ca.crt`,
`tls.crt`, and `tls.key`, then set `nicoHardwareHealth.tlsSecretName`. The files
are mounted at `/var/run/secrets/spiffe.io`, matching NICo defaults.

The release workflow builds this image from the `infra-controller` `v2.0.0-rc.5`
tag configured in `.github/workflows/release.yml`, publishes amd64 and arm64
images, then publishes the chart only after both collector images are available.
