# Fleet Intelligence OOB Collector

This chart deploys one long-running Fleet Intelligence out-of-band collector.
It can also deploy the NICo hardware-health service. Both workloads use a
`Recreate` deployment strategy so duplicate collectors do not run during an
update.

## Prerequisites

Create separate Secrets for the backend bootstrap token and generic collector
configuration:

```bash
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-config \
  --from-file=config.toml=/path/to/config.toml

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
name. Set `config.secretName` or `config.secretKey` when the generic
configuration is stored under a different Secret or key. The chart does not
accept credentials as values, preventing them from being stored in Helm release
state. Omit `imagePullSecrets` when the images are public.

The OOB TOML declares placement once and defines one or more machines:

```toml
compute_zone = "az51"
node_group = "dev3-dh1"

[[machines]]
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
username = "root"
password = "secret"
labels = { site = "dev", environment = "development" }
```

The MAC is optional. If omitted, the collector and generated NICo config use a
locally administered MAC formed as `02:00:<last four IP bytes>`.

The backend uses the OOB placement to find or create the compute zone and node
group and assign each node. An init container translates the same generic config
into NICo static endpoints, copies placement into NICo labels, and marks every
endpoint as a machine. NICo then emits those labels and the discovered primary
Redfish ComputerSystem UUID (`system.uuid`) on OTLP resources. Optional
per-machine labels are copied as well.

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

NICo hardware health is enabled by default. Its generated configuration is
written to a pod-local `emptyDir`; no second user-managed Secret is needed. It
can be disabled when only inventory and attestation are needed:

```bash
helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set nicoHardwareHealth.enabled=false
```

After changing the external collector config Secret, restart the NICo
Deployment so its init container regenerates the pod-local configuration.

The service image is `ghcr.io/nvidia/nico-hardware-health`; its tag defaults to
the chart version. Set `nicoHardwareHealth.otlpEndpoint` for the telemetry
destination and `nicoHardwareHealth.metricsPrefix` when a non-default metric
prefix is needed. When using NICo API mTLS, create a Secret containing
`ca.crt`, `tls.crt`, and `tls.key`, then set
`nicoHardwareHealth.tlsSecretName`. The files are mounted at
`/var/run/secrets/spiffe.io`, matching NICo defaults.

The release workflow builds this image from the pinned `infra-controller`
commit configured by `NICO_SOURCE_REF` in `.github/workflows/release.yml`,
publishes amd64 and arm64 images, then publishes the chart only after both
collector images are available.
