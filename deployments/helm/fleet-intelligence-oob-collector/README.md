# Fleet Intelligence OOB Collector

This chart deploys one long-running Fleet Intelligence out-of-band collector.
It can also deploy
[NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health).
Both workloads use a `Recreate` deployment strategy so duplicate collectors do
not run during an update.

## Prerequisites

Create a ConfigMap for the password-free collector configuration and separate
Secrets for the backend bootstrap token and BMC credentials.

The ConfigMap contains targets and a reference to the shared JSON credentials
file. Store credential values in the JSON file's profiles and per-MAC overrides.

```bash
kubectl create namespace fleet-intelligence \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-file=agent-token=/secure/path/to/agent-token

kubectl create configmap fleet-intelligence-config \
  --namespace fleet-intelligence \
  --from-file=config.toml=/path/to/config.toml \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-bmc-credentials \
  --namespace fleet-intelligence \
  --from-file=credentials.json=/secure/path/to/credentials.json
```

If the collector image is private, create a Docker registry Secret with a GHCR
token that has `read:packages` access:

```bash
: "${GITHUB_USER:?Set GITHUB_USER before continuing}"
: "${GITHUB_TOKEN:?Set GITHUB_TOKEN before continuing}"

kubectl create secret docker-registry ghcr-pull-secret \
  --namespace fleet-intelligence \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN"
```

## Install

If the OCI chart package is private, authenticate Helm separately from the
Kubernetes image pull Secret:

```bash
: "${GITHUB_USER:?Set GITHUB_USER before continuing}"
: "${GITHUB_TOKEN:?Set GITHUB_TOKEN before continuing}"
printf '%s' "$GITHUB_TOKEN" | helm registry login ghcr.io \
  --username "$GITHUB_USER" \
  --password-stdin
```

This login lets Helm pull the chart. The `ghcr-pull-secret` instead lets
Kubernetes nodes pull the deployed images.

Save the credential Secret mapping as `/path/to/credentials-values.yaml`. The
file contains only the Secret name and key, not credential values:

```yaml
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
```

The collector and NICo image tags default to the chart version; the
OpenTelemetry gateway uses its independently pinned image tag.

```bash
export OOB_CHART_VERSION=0.1.0

helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/dsx-ai-factory/charts/fleet-intelligence-oob-collector \
  --version "$OOB_CHART_VERSION" \
  --values /path/to/credentials-values.yaml \
  --namespace fleet-intelligence \
  --create-namespace \
  --wait \
  --timeout 10m
```

If the images require registry authentication, append
`--set 'imagePullSecrets[0].name=ghcr-pull-secret'` to the Helm command.

Set `bmcCollector.agentTokenSecretName` when the token Secret uses a different
name. Set `config.configMapName` or `config.configKey` when the generic
configuration is stored under a different ConfigMap or key. The chart does not
accept credentials as values, preventing them from being stored in Helm release
state. Override the collector or NICo image tag only for testing an image that
does not match the chart version.

The OOB TOML declares shared NICo labels once and defines one or more targets:

```toml
# Fleet Intelligence placement defaults. Targets can override these fields.
compute_zone = "us-west-dc1"
node_group = "gb200-training"

[credentials]
file = "/run/secrets/bmc/credentials.json"

[[targets]]
kind = "compute"
credentials_profile = "bmc"
name = "node-01"
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
labels = { site = "dev", environment = "development" }

[[targets]]
kind = "nvswitch"
credentials_profile = "switch"
name = "switch-tray-01"
ip = "10.0.0.11"
mac = "02:00:0a:00:00:0b"
```

Credential fields resolve from a per-MAC override, then the selected profile
in the shared JSON file. See [credentials.example.json](../../../credentials.example.json).
The generated NICo config references this file without copying its values.

Every target requires an explicit BMC MAC. Colon-separated, hyphen-separated,
and unseparated hexadecimal forms are accepted in either case. MACs are stored
as six bytes and formatted as uppercase colon-separated text. Duplicate MACs are
rejected across all targets. The config file is not rewritten.

The target `name` is optional. When present, the collector includes it as the
top-level `name` in inventory uploads and adds it to NICo metrics and logs as
the reserved `machine_name` label.

Top-level `compute_zone` and `node_group` values are inherited by every target.
A target can override either field directly. The collector exports the values
as inventory `computeZone` and `nodeGroup` and mirrors them to NICo telemetry as
`compute_zone` and `node_group`. Custom labels remain telemetry-only.

An init container runs `bmc-collector render-nico-config` to translate the shared
config into NICo static endpoints. All target kinds use generic Redfish endpoints
without NICo machine, switch, or power-shelf descriptors.
NICo then emits the shared labels on metrics and logs. Per-target labels can add to
or override the shared labels. The collector adds the reserved
`oob="true"` label to every endpoint, plus the
reserved `machine_name` label for named targets, so both are present on NICo
metrics and logs.

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

Inventory and attestation share the collector's BMC request controls. Configure
the maximum concurrent target workflows with `bmcCollector.targetConcurrency`,
the per-BMC in-flight limit with `bmcCollector.bmcMaxInFlight`, and per-BMC rate
limiting with `bmcCollector.bmcRequestsPerSecond` and `bmcCollector.bmcBurst`.
The global in-flight ceiling is `targetConcurrency * bmcMaxInFlight`. Set
`bmcCollector.timeoutSeconds` to control the timeout for each Redfish request.

After each BMC's inventory collection, the collector measures TCP connect time
to the BMC's Redfish port to record collector-to-BMC network latency. Each
round opens five connections without sending TLS or HTTP data and shares the
BMC's request limits. Set `bmcCollector.latencyProbe: false` to
disable it. If a service mesh sidecar intercepts the collector's outbound TCP,
the measurement is reported as `intercepted`; exclude the collector pod from
sidecar injection, or exclude BMC address ranges from outbound capture, to get
usable measurements.

See `values.yaml` for the small set of supported settings.
When `bmcCollector.image.tag` is empty, the chart uses its own version as the
image tag.

## NICo hardware health

NICo hardware health is enabled by default. Its generated configuration is
written to a memory-backed pod-local `emptyDir`. It can be disabled when only
inventory and attestation are needed:

```bash
helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/dsx-ai-factory/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set nicoHardwareHealth.enabled=false \
  --set otelGateway.enabled=false
```

The collector reloads the TOML and JSON files at the start of each collection
cycle. NICo reloads the JSON credentials without restarting. After changing
targets in the ConfigMap, restart the NICo Deployment so its init container
regenerates the pod-local configuration. Skip the restart when NICo is disabled.

The service image is `ghcr.io/dsx-ai-factory/nico-hardware-health`; its tag defaults to
the chart version. By default, NICo sends OTLP/gRPC to a separate gateway
Deployment and ClusterIP Service running
`ghcr.io/nvidia/fleetint-otelcol:1.4.0`. The gateway reuses
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

Set `nicoHardwareHealth.metricsPrefix` to configure the metric prefix.

The release workflow builds this image from the pinned `infra-controller`
commit configured by `NICO_SOURCE_REF` in `.github/workflows/release.yml`,
publishes amd64 and arm64 images, then publishes the chart only after both
collector images are available.
