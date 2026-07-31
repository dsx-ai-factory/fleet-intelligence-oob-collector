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
- A trusted edge that injects customer identity before requests reach the Fleet
  Intelligence agent API

## Create Secrets

Store the BMC configuration and agent token in separate Secrets:

```bash
: "${FLEETINT_AGENT_TOKEN:?Set FLEETINT_AGENT_TOKEN before continuing}"

kubectl create namespace fleet-intelligence \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-config \
  --namespace fleet-intelligence \
  --from-file=config.toml=/path/to/config.toml \
  --dry-run=client -o yaml | kubectl apply -f -
```

The chart references existing Secrets so credentials do not enter Helm release
state.

If the images are private, create an image pull Secret using a token with
`read:packages` access:

```bash
: "${GITHUB_USER:?Set GITHUB_USER before continuing}"
: "${GITHUB_TOKEN:?Set GITHUB_TOKEN before continuing}"

kubectl create secret docker-registry ghcr-pull-secret \
  --namespace fleet-intelligence \
  --docker-server=ghcr.io \
  --docker-username="$GITHUB_USER" \
  --docker-password="$GITHUB_TOKEN" \
  --dry-run=client -o yaml | kubectl apply -f -
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

The collector reloads the mounted TOML at the start of each collection cycle.
When `nicoHardwareHealth.enabled=true`, restart the NICo Deployment after
updating the Secret so its init container regenerates NICo's configuration.
Skip this command when NICo is disabled:

```bash
kubectl rollout restart \
  --namespace fleet-intelligence \
  deployment/nico-hardware-health
```
