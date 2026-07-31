# Kubernetes deployment

The OCI Helm chart deploys the OOB collector and, by default, NICo hardware
health plus an authenticated OpenTelemetry gateway.

## Prerequisites

- A Kubernetes cluster and `kubectl` access
- Helm 3 with OCI registry support
- Network access from the cluster to each BMC and Fleet Intelligence endpoint
- A Fleet Intelligence bootstrap agent token
- A collector TOML file described in [Configuration](configuration.md)
- An image pull credential if the GHCR packages are private

## Create Secrets

Create separate Secrets for backend enrollment and BMC configuration:

```bash
kubectl create namespace fleet-intelligence

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-config \
  --namespace fleet-intelligence \
  --from-file=config.toml=/path/to/config.toml
```

If necessary, create an image pull Secret using a token with `read:packages`:

```bash
kubectl create secret docker-registry ghcr-pull-secret \
  --namespace fleet-intelligence \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN"
```

The chart references existing Secrets and does not accept BMC or agent
credentials as values, preventing those credentials from entering Helm release
state.

## Install or upgrade

Published collector and NICo image tags default to the chart version:

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

For private images, append:

```text
--set 'imagePullSecrets[0].name=ghcr-pull-secret'
```

The default backend is `https://data.fleet-intelligence.nvidia.com`. Use a
values file for durable overrides:

```yaml
bmcCollector:
  backendUrl: https://data.example.com
  intervalSeconds: 86400
  logLevel: info
```

Apply it with `--values values-production.yaml`. The backend URL must be the API
base URL without `/v1/agent`.

## Deployed components

### BMC collector

One `Recreate` Deployment runs continuous inventory and attestation sync. It
mounts the configured Secret read-only and reads the bootstrap token from the
separate agent-token Secret.

Change these values when Secret names or keys differ:

```yaml
config:
  secretName: fleet-intelligence-config
  secretKey: config.toml

bmcCollector:
  agentTokenSecretName: fleet-intelligence-oob-collector-agent-token
```

Set `bmcCollector.logLevel` to a Rust tracing filter and
`bmcCollector.insecureTls` only when BMC certificate verification cannot be
used.

### NICo hardware health

NICo is enabled by default. An init container converts the collector TOML to
NICo static endpoints, adds `oob="true"`, enables log collection in `auto`
mode, and writes the generated credential-bearing file to a memory-backed
`emptyDir`.

NICo sends metrics and logs to the chart's gateway by default. Its Prometheus
metrics service listens on port `9009`. To expose its API with mTLS, create a
Secret containing `ca.crt`, `tls.crt`, and `tls.key`, then configure:

```yaml
nicoHardwareHealth:
  tlsSecretName: nico-hardware-health-tls
```

The files are mounted at `/var/run/secrets/spiffe.io`.

### OpenTelemetry gateway

The gateway receives OTLP/gRPC on `4317` and OTLP/HTTP on `4318`, enrolls with
the Fleet Intelligence health API, and exports metrics and logs over OTLP/HTTP.
By default, its endpoints and Secret derive from BMC collector settings.

Override them when telemetry uses different routing or credentials:

```yaml
otelGateway:
  enroll:
    endpoint: https://data.example.com/api/v1/health/enroll
    existingSecret: fleet-intelligence-otel-gateway-token
    secretKey: agent-token
  backendEndpoint: https://data.example.com/api/v1/health
```

The gateway supports replicas, resource requests and limits, topology
spreading, an optional PodDisruptionBudget, and an ingress NetworkPolicy.

## Optional component layouts

Disable both telemetry components when only inventory and attestation are
required:

```yaml
nicoHardwareHealth:
  enabled: false
otelGateway:
  enabled: false
```

To send NICo telemetry to an external OTLP/gRPC collector, disable the bundled
gateway and supply an endpoint:

```yaml
nicoHardwareHealth:
  enabled: true
  otlpEndpoint: http://external-otel-collector.observability.svc:4317
otelGateway:
  enabled: false
```

## Proxy configuration

Configure a cluster-reachable proxy with Helm values:

```yaml
bmcCollector:
  proxy:
    httpProxy: http://proxy.example.internal:3128
    httpsProxy: http://proxy.example.internal:3128
    allProxy: ""
    noProxy: localhost,127.0.0.1,.svc,.cluster.local
```

These settings are passed to the BMC collector and OTLP gateway. A SOCKS tunnel
on a developer workstation at `127.0.0.1` is not reachable from Kubernetes;
use a cluster-reachable proxy or sidecar. The chart does not currently provide
a Secret-backed proxy credential value, so do not put authenticated proxy URLs
in committed values files.

## Operate and troubleshoot

Inspect the release and workload logs:

```bash
helm status fleet-intelligence-oob-collector \
  --namespace fleet-intelligence

kubectl get pods \
  --namespace fleet-intelligence \
  -l app.kubernetes.io/instance=fleet-intelligence-oob-collector

kubectl logs \
  --namespace fleet-intelligence \
  deployment/fleet-intelligence-oob-collector
```

After changing the external collector config Secret, restart both workloads
that consume it. The collector reads it at process startup, while the NICo init
container regenerates its local configuration only when a pod starts:

```bash
kubectl rollout restart \
  --namespace fleet-intelligence \
  deployment/fleet-intelligence-oob-collector

kubectl rollout restart \
  --namespace fleet-intelligence \
  deployment/nico-hardware-health
```

Validate chart changes from a source checkout:

```bash
make chart-lint
make chart-template
```

The complete value reference remains in
`deployments/helm/fleet-intelligence-oob-collector/values.yaml`, with additional
chart-specific notes in the adjacent
[Helm README](../deployments/helm/fleet-intelligence-oob-collector/README.md).
