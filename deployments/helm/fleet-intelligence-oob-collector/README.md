# Fleet Intelligence OOB Collector

This chart deploys one long-running Fleet Intelligence out-of-band collector.
It can also deploy the NICo hardware-health service. Both workloads use a
`Recreate` deployment strategy so duplicate collectors do not run during an
update.

For the concise end-to-end workflow, see the repository
[installation guide](../../../docs/installation.md). This file remains a
self-contained chart reference for packaged-chart users.

## Prerequisites

Create separate Secrets for the backend bootstrap token and generic collector
configuration:

```bash
kubectl create namespace fleet-intelligence

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-config \
  --namespace fleet-intelligence \
  --from-file=config.toml=/path/to/config.toml
```

If the collector image is private, create a Docker registry Secret with a GHCR
token that has `read:packages` access:

```bash
kubectl create secret docker-registry ghcr-pull-secret \
  --namespace fleet-intelligence \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN"
```

## Install

No values file is required when using the default Secret names, backend URL,
and published images. The collector and NICo image tags default to the chart
version; the OTel gateway uses its independently pinned image tag.

```bash
export OOB_CHART_VERSION=0.1.0

helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version "$OOB_CHART_VERSION" \
  --namespace fleet-intelligence \
  --create-namespace \
  --wait \
  --timeout 10m
```

If the images require registry authentication, append
`--set 'imagePullSecrets[0].name=ghcr-pull-secret'` to the Helm command.

Set `bmcCollector.agentTokenSecretName` when the token Secret uses a different
name. Set `config.secretName` or `config.secretKey` when the generic
configuration is stored under a different Secret or key. The chart does not
accept credentials as values, preventing them from being stored in Helm release
state. Override the collector or NICo image tag only for testing an image that
does not match the chart version.

The OOB TOML declares shared NICo labels once and defines one or more machines:

```toml
[labels]
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

When a machine's effective labels include `compute_zone` or `node_group`, the
collector also exports them as the backend `computeZone` and `nodeGroup`
placement fields during inventory upload.

An init container translates the generic config into NICo static endpoints and
marks every endpoint as a machine. NICo then emits the shared labels and the
discovered primary Redfish ComputerSystem UUID (`system.uuid`) on OTLP
resources. Per-machine labels can add to or override the shared labels. The
collector adds the reserved `oob="true"` label to every endpoint so it is
present on NICo metrics and logs.

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
written to a memory-backed pod-local `emptyDir`; no second user-managed Secret
is needed. It can be disabled when only inventory and attestation are needed:

```bash
helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set nicoHardwareHealth.enabled=false \
  --set otelGateway.enabled=false
```

After changing the external collector config Secret, restart both the collector
and NICo Deployments. The collector reads the Secret at startup, and the NICo
init container regenerates its pod-local configuration only when a pod starts.

The service image is `ghcr.io/nvidia/nico-hardware-health`; its tag defaults to
the chart version. By default, NICo sends OTLP/gRPC to a separate gateway
Deployment and ClusterIP Service running
`ghcr.io/nvidia/fleetint-otelcol:0.0.2-poc.otelgateway`. The gateway reuses
`bmcCollector.backendUrl` and the `agent-token` Secret to authenticate to the
Fleet Intelligence telemetry API. Its NetworkPolicy accepts OTLP only from
workloads in the same Helm release when supported by the cluster CNI.

Disable `otelGateway.enabled` and set
`nicoHardwareHealth.otlpEndpoint` to use an external collector instead.

## OpenTelemetry gateway

The gateway is a top-level chart component with an independent Deployment,
ClusterIP Service, configuration, rollout, and scaling lifecycle. It receives
OTLP/gRPC on port `4317` and OTLP/HTTP on port `4318`. NICo's generated
configuration automatically uses the gateway Service when
`otelGateway.enabled=true`.

By default, the gateway derives its enrollment URL, telemetry backend URL, and
Secret name from `bmcCollector.backendUrl` and
`bmcCollector.agentTokenSecretName`. Override them when the telemetry API uses
different routing or credentials:

```yaml
otelGateway:
  enroll:
    endpoint: https://data.example.com/api/v1/health/enroll
    existingSecret: fleet-intelligence-otel-gateway-token
    secretKey: agent-token
  backendEndpoint: https://data.example.com/api/v1/health
```

The chart supports gateway replica scaling, topology spreading, an optional
PodDisruptionBudget, resource configuration, and an ingress NetworkPolicy.

Set `nicoHardwareHealth.metricsPrefix` when a non-default metric prefix is
needed. When using NICo API mTLS, create a Secret containing `ca.crt`,
`tls.crt`, and `tls.key`, then set `nicoHardwareHealth.tlsSecretName`. The
files are mounted at `/var/run/secrets/spiffe.io`, matching NICo defaults.

The release workflow builds this image from the pinned `infra-controller`
commit configured by `NICO_SOURCE_REF` in `.github/workflows/release.yml`,
publishes amd64 and arm64 images, then publishes the chart only after both
collector images are available.
