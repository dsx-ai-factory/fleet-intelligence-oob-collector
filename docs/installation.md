# Installation

The supported deployment uses the OCI Helm chart. It installs the OOB collector and, by default, NICo hardware health and an authenticated OpenTelemetry gateway.

For the experimental bare-metal/VM DEB/RPM installer, see the [host package prototype](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/packages/README.md).

## Prerequisites

- Kubernetes cluster access through `kubectl`
- Helm 3 with OCI registry support
- Network access from the cluster to the BMCs and Fleet Intelligence backend
- Shared target Helm values (see [Configuration](configuration.md#helm-values))
- A Fleet Intelligence bootstrap agent token

## Configure targets and Secrets

Helm creates the password-free ConfigMap from target values. Store the shared JSON BMC credential file and the agent token in separate existing Secrets.

Create them directly below, or synchronize them from Vault/OpenBao using the [credentials guide](credentials.md).

Copy [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json) to `/secure/path/to/credentials.json` in an existing private directory, set its mode to `0600`, and replace its placeholder values. Obtain the bootstrap token from your Fleet Intelligence administrator, authorized for both collector and gateway enrollment. Save only its value in `/secure/path/to/agent-token`, with no surrounding whitespace or trailing newline. For example, in Bash, read it without echoing it or putting it in shell history:

```bash
umask 077
read -r -s -p 'Fleet Intelligence bootstrap token: ' FLEETINT_BOOTSTRAP_TOKEN
printf '\n'
: "${FLEETINT_BOOTSTRAP_TOKEN:?Token must not be empty}"
printf '%s' "$FLEETINT_BOOTSTRAP_TOKEN" > /secure/path/to/agent-token
unset FLEETINT_BOOTSTRAP_TOKEN
```

Create the Secrets from those files:

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

The JSON file uses named `profiles` and per-MAC `overrides`. The chart mounts this file into the collector, NICo init container, and NICo runtime at `/run/secrets/bmc/credentials.json`; credential values never enter Helm release state. The init container uses `bmc-collector render-nico-config` to generate NICo TOML from the chart's collector TOML in a pod-local `emptyDir` volume.

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
backend:
  agentToken:
    existingSecret: fleet-intelligence-oob-collector-agent-token
    secretKey: agent-token
```

`backend.url` defaults to the production endpoint, `https://data.fleet-intelligence.nvidia.com`. Set it in your values file when using a different Fleet Intelligence endpoint.

## Install

Choose a published chart version from the repository [releases](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/releases) and replace `<chart-version>` below:

```bash
export OOB_CHART_VERSION='<chart-version>'

helm upgrade --install fleet-intelligence-oob-collector \
  oci://ghcr.io/dsx-ai-factory/charts/fleet-intelligence-oob-collector \
  --version "$OOB_CHART_VERSION" \
  --values /path/to/collector-values.yaml \
  --namespace fleet-intelligence \
  --create-namespace \
  --wait \
  --timeout 10m
```

For a deployment using private registry mirrors, configure `imagePullSecrets` and authenticate Helm to the chart registry as required by your organization.

Collector and NICo image tags default to the chart version. See [Configuration](configuration.md#helm-values) for supported overrides.

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

kubectl logs --namespace fleet-intelligence \
  deployment/nico-hardware-health --container generate-nico-config

kubectl logs --namespace fleet-intelligence \
  deployment/fleet-intelligence-oob-collector-otel-gateway
```

Use your release's Deployment names and omit disabled components. The NICo init container must complete successfully. Check collector logs for successful inventory uploads and the gateway logs for enrollment or export errors; running pods alone do not confirm access. For BMC authentication failures, check the target's `credentialsProfile`, matching MAC overrides, and actual BMC account. For Fleet Intelligence enrollment failures, check the bootstrap token's validity, authorization for both flows, and backend URL. For gateway export failures, also check backend reachability and TLS trust.

The collector reloads its mounted TOML and JSON credentials each collection cycle. NICo reloads the JSON file without restarting. Run `helm upgrade` after changing values. Changes to rendered target TOML roll the collector and NICo; changes to rendered gateway configuration roll the gateway. Environment, image, and other pod-template changes roll the affected workloads. Bootstrap token value changes require an explicit restart of the collector and enabled gateway; see [credential rotation](credentials.md#rotation-and-ownership).

## Disconnect an individual machine

Remove the machine from each collector that monitors it before completing the Delete Machine flow in Fleet Intelligence. This stops future collection; uploaded data remains in Fleet Intelligence.

Use your deployment's resource names if they differ from the examples below. Update any configuration management source too, so it does not restore the machine.

1. Find the machine by its BMC `mac` in `inventory.targets` in your values file and remove that entry. For the last target, [uninstall the collector](#uninstall-the-oob-collector); an empty list is invalid.
2. Run the same `helm upgrade --install` command with the updated values file. Helm updates the collector ConfigMap and rolls the collector and NICo deployments. NICo's init container regenerates its configuration.
3. Wait for both enabled deployments to complete their rollouts:

   ```bash
   kubectl rollout status deployment/fleet-intelligence-oob-collector \
     --namespace fleet-intelligence --timeout=10m
   kubectl rollout status deployment/nico-hardware-health \
     --namespace fleet-intelligence --timeout=10m
   ```

Skip the NICo command when it is disabled. Confirm the release ConfigMap omits the machine's BMC MAC and check the [collector logs](#verify) for failures affecting remaining targets. Data in transit can arrive after the rollout. Keep the shared credential Secret; remove an unused per-MAC override from its JSON file only when no other workload needs it.

## Uninstall the OOB collector

This stops collection for all targets in the deployment. Uploaded data remains in Fleet Intelligence. Substitute your release and namespace names if they differ, and disable any automation that would reinstall the release.

1. Remove the Helm release, including its OOB collector, NICo hardware health, and OpenTelemetry gateway:

   ```bash
   helm uninstall fleet-intelligence-oob-collector \
     --namespace fleet-intelligence --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl get pods --namespace fleet-intelligence \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

3. Delete unused credential Secrets:

   ```bash
   kubectl delete secret fleet-intelligence-oob-collector-agent-token \
     fleet-intelligence-bmc-credentials --namespace fleet-intelligence
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a Secret does not revoke its credentials; retire them through their issuing system. Keep shared resources and separately managed external telemetry collectors.
