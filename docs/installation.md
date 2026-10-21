# Installation

The supported deployment uses the OCI Helm chart. It installs the OOB
collector and, by default, NICo hardware health and an authenticated
OpenTelemetry gateway.

For the experimental bare-metal/VM DEB/RPM installer, see the
[host package prototype](../deployments/packages/README.md).

## Prerequisites

- Kubernetes cluster access through `kubectl`
- Helm 3 with OCI registry support
- Network access from the cluster to the BMCs and Fleet Intelligence backend
- Shared target Helm values (see [Configuration](configuration.md#helm-values))
- A Fleet Intelligence bootstrap agent token
- A trusted edge that injects customer identity before requests reach the Fleet
  Intelligence agent API

## Configure targets and Secrets

Helm creates the password-free ConfigMap from target values. Store the shared
JSON BMC credential file and the agent token in separate existing Secrets:

```bash
kubectl create namespace fleet-intelligence \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace fleet-intelligence \
  --from-file=agent-token=/secure/path/to/agent-token

kubectl create secret generic fleet-intelligence-bmc-credentials \
  --namespace fleet-intelligence \
  --from-file=credentials.json=/secure/path/to/credentials.json
```

The JSON file uses named `profiles` and per-MAC `overrides`, as in
[credentials.example.json](../credentials.example.json). Copy it outside the
repository and replace its placeholder values. The chart mounts this same file
into the collector, NICo init container, and NICo runtime at
`/run/secrets/bmc/credentials.json`; credential values never enter Helm release state.
The init container uses `bmc-collector render-nico-config` to generate NICo TOML
from the chart's collector TOML in a memory-backed pod volume.

Save `/path/to/collector-values.yaml` with target metadata and Secret references:

```yaml
inventory:
  computeZone: us-west-dc1
  nodeGroup: gb200-training
  targets:
    - kind: compute
      name: node-01
      address: 192.0.2.10
      mac: "00:11:22:33:44:55"
      credentialsProfile: bmc
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
```

If the images are private, create an image pull Secret using a token with
`read:packages` access:

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

```bash
export OOB_CHART_VERSION=0.1.0

helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/dsx-ai-factory/charts/fleet-intelligence-oob-collector \
  --version "$OOB_CHART_VERSION" \
  --values /path/to/collector-values.yaml \
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

The collector reloads its mounted TOML and JSON credentials each collection
cycle. NICo reloads the JSON file without restarting. Run `helm upgrade` after
changing targets or non-secret configuration; checksum annotations roll both
deployments automatically.

## Disconnect an individual machine

Remove the machine from each collector that monitors it before completing the
Delete Machine flow in Fleet Intelligence. This stops future collection;
previously uploaded data remains in Fleet Intelligence.

Use your deployment's resource names if they differ from the examples below.
Update any configuration management source too, so it does not restore the
machine.

1. Find the machine by its BMC `mac` in `inventory.targets` in your values file
   and remove that entry. For the last target,
   [uninstall the collector](#uninstall-the-oob-collector); an empty list is invalid.
2. Run the same `helm upgrade --install` command with the updated values file.
   Helm updates the collector ConfigMap and rolls the collector and NICo
   deployments. NICo's init container regenerates its configuration.
3. Wait for both enabled deployments to complete their rollouts:

   ```bash
   kubectl rollout status deployment/fleet-intelligence-oob-collector \
     --namespace fleet-intelligence --timeout=10m
   kubectl rollout status deployment/nico-hardware-health \
     --namespace fleet-intelligence --timeout=10m
   ```

Skip the NICo command when it is disabled. Confirm the release ConfigMap no
longer contains the machine's BMC MAC and check the [collector logs](#verify)
for failures affecting remaining targets. Data already in transit can still
arrive after the rollout. Keep the shared credential Secret; remove an unused
per-MAC override from its JSON file only when no other workload needs it.

## Uninstall the OOB Collector

This stops collection for all targets in the deployment. Previously uploaded
data remains in Fleet Intelligence. Substitute your release and namespace names
if they differ, and disable any automation that would reinstall the release.

1. Remove the Helm release, including its OOB collector, NICo hardware health,
   and OpenTelemetry gateway:

   ```bash
   helm uninstall fleet-intelligence-oob-collector \
     --namespace fleet-intelligence --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl get pods --namespace fleet-intelligence \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

3. Delete credential Secrets that are no longer used:

   ```bash
   kubectl delete secret fleet-intelligence-oob-collector-agent-token \
     fleet-intelligence-bmc-credentials --namespace fleet-intelligence
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a
Secret does not revoke its credentials; retire them through their issuing
system. Keep shared resources and separately managed external telemetry
collectors.
