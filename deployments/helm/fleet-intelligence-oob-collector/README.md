# Fleet Intelligence OOB Collector

This chart deploys one long-running Fleet Intelligence out-of-band collector.
It uses a `Recreate` deployment strategy so two collector instances do not
upload the same target data during an update.

## Prerequisites

Create one Secret for the backend bootstrap token and another for the INI-style
BMC target configuration:

```bash
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --from-literal=agent-token="$FLEETINT_AGENT_TOKEN"

kubectl create secret generic fleet-intelligence-oob-collector-bmc-config \
  --from-file=targets.config=/path/to/oob.config
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

Set `agentTokenSecretName` or `bmcConfigSecretName` when either Secret uses a
different name. Their key names are fixed as `agent-token` and `targets.config`.
The chart does not accept credentials as values, preventing them from being
stored in Helm release state. Omit `imagePullSecrets` when the image is public.

By default, the collector sends data to
`https://data.fleet-intelligence.nvidia.com`. Set `backendUrl` to use a
different Fleet Intelligence endpoint.

See `values.yaml` for the small set of supported settings.
When `image.tag` is empty, the chart uses its own version as the image tag.
