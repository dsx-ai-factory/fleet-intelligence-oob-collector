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

Remove a machine from this collector's configuration to stop its inventory,
attestation, metrics, and log collection. Do this before completing the Delete
Machine flow in Fleet Intelligence. Removing the configuration does not delete
the machine or its previously uploaded data from Fleet Intelligence.

The commands below use the release, namespace, ConfigMap, and Deployment names
from this guide. Substitute your deployment's names if they differ. If a
configuration management system manages these resources, update its source so
it does not restore the removed machine.

1. Find the machine in the source `config.toml` by its BMC `mac`. Remove its
   entire `[[targets]]` entry, including any labels and credential file paths
   belonging to that entry. Keep the other targets and shared settings.
   If this is the last target, follow
   [Uninstall the OOB Collector](#uninstall-the-oob-collector); an empty target
   list is invalid.
2. Apply the updated password-free configuration:

   ```bash
   kubectl create configmap fleet-intelligence-config \
     --namespace fleet-intelligence \
     --from-file=config.toml=/path/to/config.toml \
     --dry-run=client -o yaml | kubectl apply -f -
   ```

3. Restart the collector to stop the current cycle and load the new target
   list immediately, then wait for the rollout:

   ```bash
   kubectl rollout restart \
     --namespace fleet-intelligence \
     deployment/fleet-intelligence-oob-collector

   kubectl rollout status \
     --namespace fleet-intelligence \
     deployment/fleet-intelligence-oob-collector \
     --timeout=10m
   ```

   Without a restart, the collector reads the mounted TOML at the next cycle,
   after Kubernetes propagates the ConfigMap update. The default interval is
   24 hours, and work from the current cycle can still be uploaded.
4. If NICo hardware health is enabled, restart it as well so its init container
   regenerates the configuration without the removed machine:

   ```bash
   kubectl rollout restart \
     --namespace fleet-intelligence \
     deployment/nico-hardware-health

   kubectl rollout status \
     --namespace fleet-intelligence \
     deployment/nico-hardware-health \
     --timeout=10m
   ```

5. Confirm the ConfigMap no longer contains the machine's BMC MAC and both
   enabled collectors have completed their rollouts. Check the collector logs
   using the [verification commands](#verify) for configuration or collection
   failures affecting the remaining targets. Data already in transit can still
   arrive after the restart. If the machine is configured in other collectors,
   remove it there too before deleting it in Fleet Intelligence.

Keep shared credential Secrets while remaining targets use them. If a Secret
was dedicated to the removed machine, remove its `credentials.secrets` mount
mapping through a Helm upgrade using your existing chart version and complete
values, then delete that Secret only after no workloads reference it.

For a collector run directly outside Kubernetes, edit the TOML passed to
`--config` and restart the running collector process. Update and restart any
separately managed NICo process too.

## Uninstall the OOB Collector

Use this workflow to stop collection for every target in a deployment. To keep
collecting from other machines, use
[Disconnect an individual machine](#disconnect-an-individual-machine) instead.
Uninstalling removes the deployed collectors; it does not delete machines or
previously uploaded data from Fleet Intelligence.

1. Disable any automation that would reinstall the release, such as a GitOps
   reconciliation, before removing it. Use your release and namespace names
   if they differ from this guide.
2. Uninstall the Helm release and wait for its resources to be removed:

   ```bash
   helm uninstall fleet-intelligence-oob-collector \
     --namespace fleet-intelligence \
     --wait \
     --timeout 10m
   ```

   This removes the OOB collector, NICo hardware health, and OpenTelemetry
   gateway resources installed by this release, including their Deployments,
   Services, and chart-managed ConfigMaps and policies. An external telemetry
   collector configured through `nicoHardwareHealth.otlpEndpoint` is managed
   separately.
3. Verify that no pods remain for the release:

   ```bash
   kubectl get pods \
     --namespace fleet-intelligence \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

   The command should report no resources. Investigate any remaining pods
   before considering collection stopped.
4. Helm does not remove the existing ConfigMap and Secrets created in
   [Create ConfigMap and Secrets](#create-configmap-and-secrets). Delete them
   only if they are dedicated to this deployment and no other workloads use
   them:

   ```bash
   kubectl delete configmap fleet-intelligence-config \
     --namespace fleet-intelligence

   kubectl delete secret \
     fleet-intelligence-oob-collector-agent-token \
     default-bmc-credentials \
     --namespace fleet-intelligence
   ```

   Also remove any unused per-machine credential, image pull, NICo TLS, or
   separate gateway token Secrets created for this deployment. Deleting a
   Kubernetes Secret removes the stored copy; it does not revoke its token or
   BMC credentials. Retire dedicated credentials through their issuing system
   when they are no longer needed. Keep the namespace if it contains other
   workloads or shared resources.

For a collector run directly outside Kubernetes, stop the collector process
and disable its service or scheduled job so it cannot restart. Stop any
separately managed NICo and gateway processes dedicated to that deployment,
then remove unused local configuration and credential files according to your
organization's credential handling policy.
