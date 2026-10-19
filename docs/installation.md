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

## Create ConfigMap and Secrets

Store the password-free BMC configuration in a ConfigMap. Store BMC credentials
and the agent token in separate Secrets:

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

kubectl create secret generic default-bmc-credentials \
  --namespace fleet-intelligence \
  --from-file=username=/secure/path/to/username \
  --from-file=password=/secure/path/to/password
```

The chart references existing Secrets so credential values do not enter Helm
release state. Save the mount mapping as `/path/to/credentials-values.yaml`; it
contains only the Secret name and target directory:

```yaml
credentials:
  secrets:
    - name: default-bmc-credentials
      target: default
```

Add another entry with a distinct `target` for each per-machine override. Each
Secret must contain `username` and `password` keys. The chart projects them to
`/run/secrets/bmc/<target>/username` and `password`.

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
  --values /path/to/credentials-values.yaml \
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
updating the ConfigMap so its init container regenerates NICo's configuration.
Skip this command when NICo is disabled:

```bash
kubectl rollout restart \
  --namespace fleet-intelligence \
  deployment/nico-hardware-health
```

## Disconnect an individual machine

Remove the machine from each collector that monitors it before completing the
Delete Machine flow in Fleet Intelligence. This stops future collection;
previously uploaded data remains in Fleet Intelligence.

Use your deployment's resource names if they differ from the examples below.
Update any configuration management source too, so it does not restore the
machine.

1. Find the machine by its BMC `mac` in `config.toml` and remove its entire
   `[[targets]]` entry. Keep the other targets and shared settings. For the last
   target, [uninstall the collector](#uninstall-the-oob-collector); an empty
   target list is invalid.
2. Apply the updated password-free configuration:

   ```bash
   kubectl create configmap fleet-intelligence-config \
     --namespace fleet-intelligence \
     --from-file=config.toml=/path/to/config.toml \
     --dry-run=client -o yaml | kubectl apply -f -
   ```

3. Restart the collector to apply the change immediately, then wait for its
   rollout:

   ```bash
   kubectl rollout restart deployment/fleet-intelligence-oob-collector \
     --namespace fleet-intelligence
   kubectl rollout status deployment/fleet-intelligence-oob-collector \
     --namespace fleet-intelligence --timeout=10m
   ```

4. If NICo hardware health is enabled, restart it to regenerate its target
   configuration, then wait for its rollout:

   ```bash
   kubectl rollout restart deployment/nico-hardware-health \
     --namespace fleet-intelligence
   kubectl rollout status deployment/nico-hardware-health \
     --namespace fleet-intelligence --timeout=10m
   ```

Confirm the ConfigMap no longer contains the machine's BMC MAC and check the
[collector logs](#verify) for failures affecting remaining targets. Data already
in transit can still arrive after the restart. Keep shared credential Secrets;
remove a dedicated Secret only after removing its `credentials.secrets` mapping
through a Helm upgrade and confirming no workloads use it.

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

3. Delete the separately created ConfigMap and Secrets only if no other
   workloads use them:

   ```bash
   kubectl delete configmap fleet-intelligence-config \
     --namespace fleet-intelligence
   kubectl delete secret fleet-intelligence-oob-collector-agent-token \
     default-bmc-credentials --namespace fleet-intelligence
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a
Secret does not revoke its credentials; retire them through their issuing
system. Keep shared resources and separately managed external telemetry
collectors.
