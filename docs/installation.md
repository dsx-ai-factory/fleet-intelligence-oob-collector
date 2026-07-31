# Installation

The supported deployment uses the OCI Helm chart. It installs the OOB
collector and, by default, NICo hardware health and an authenticated
OpenTelemetry gateway.

## Prerequisites

- Kubernetes cluster access through `kubectl`
- Helm 3 with OCI registry support
- Network access from the cluster to the BMCs and Fleet Intelligence backend
- A [collector configuration](configuration.md)
- A Fleet Intelligence bootstrap agent token

## Create Secrets

Store the BMC configuration and agent token in separate Secrets:

```bash
kubectl create namespace fleet-intelligence

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-config \
  --namespace fleet-intelligence \
  --from-file=config.toml=/path/to/config.toml
```

The chart references existing Secrets so credentials do not enter Helm release
state.

If the images are private, create an image pull Secret using a token with
`read:packages` access:

```bash
kubectl create secret docker-registry ghcr-pull-secret \
  --namespace fleet-intelligence \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN"
```

## Install

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

Collector and NICo image tags default to the chart version. See
[Configuration](configuration.md#helm-values) for supported overrides.

## Verify

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

After updating the collector Secret, restart the collector and NICo
Deployments. Both read or generate their configuration only when their pods
start.
