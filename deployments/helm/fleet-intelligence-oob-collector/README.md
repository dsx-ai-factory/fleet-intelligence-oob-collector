# Fleet Intelligence OOB Collector

This chart deploys one long-running Fleet Intelligence out-of-band collector.
It can also deploy the NICo hardware-health service. Both workloads use a
`Recreate` deployment strategy so duplicate collectors do not run during an
update.

## Prerequisites

Create one Secret for the backend bootstrap token and another for the shared
TOML configuration:

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
name. Set `config.secretName` or `config.secretKey` when the shared configuration
is stored under a different Secret or key. The chart does
not accept credentials as values, preventing them from being stored in Helm
release state. Omit `imagePullSecrets` when the images are public.

The shared TOML must define at least one
`endpoint_sources.static_bmc_endpoints` entry. The BMC collector does not use
NICo API endpoint discovery and no longer accepts the legacy INI target format.

By default, the collector sends data to
`https://data.fleet-intelligence.nvidia.com`. Set `bmcCollector.backendUrl` to
use a different Fleet Intelligence endpoint.

See `values.yaml` for the small set of supported settings.
When `bmcCollector.image.tag` is empty, the chart uses its own version as the
image tag.

## NICo hardware health

NICo hardware health is enabled by default and mounts the same `config`
Secret as the BMC collector. It can be disabled when only inventory and
attestation are needed:

```bash
helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector \
  --version 0.1.0 \
  --set nicoHardwareHealth.enabled=false
```

The service image is `ghcr.io/nvidia/nico-hardware-health`; its tag defaults to
the chart version. The configuration can use NICo API discovery or static BMC
endpoints. When using NICo API mTLS, create a Secret containing `ca.crt`,
`tls.crt`, and `tls.key`, then set `nicoHardwareHealth.tlsSecretName`. The files
are mounted at `/var/run/secrets/spiffe.io`, matching NICo defaults.

The release workflow builds this image from the `infra-controller` `v0.10.7`
tag configured in `.github/workflows/release.yml`, publishes amd64 and arm64
images, then publishes the chart only after both collector images are available.
