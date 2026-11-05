# Installation

Install the collector on Kubernetes with its Helm chart. By default, the chart runs three workloads: the OOB collector, NICo hardware health, and a telemetry gateway that forwards NICo's metrics and logs to Fleet Intelligence.

## Prerequisites

- A Kubernetes cluster you can reach with `kubectl`
- Helm 3
- Network access from the cluster to your BMCs and to Fleet Intelligence
- The IP and MAC address of each BMC, and its login credentials
- A Fleet Intelligence bootstrap token

## Choose a namespace

The commands on this page install into the namespace in `$NAMESPACE`. Set it before you start, and set it again if you continue in a new terminal. If it is empty, `kubectl` and `helm` use your current context's default namespace instead.

```bash
export NAMESPACE=fleet-intelligence
```

## Configure targets and Secrets

You give the chart two things: your target list, in a values file, and two Kubernetes Secrets holding the BMC credentials file and the bootstrap token. The chart never stores passwords; it only references the Secrets.

The steps below create the Secrets by hand. To sync them from Vault or OpenBao instead, follow [Credentials](credentials.md) and then continue at [Save your values file](#save-your-values-file).

### Create the Secrets

Copy [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json) to a private directory, for example `/secure/path/to/credentials.json`, set its mode to `0600`, and fill in your BMC usernames and passwords. [Configuration](configuration.md#bmc-credentials-file) explains the file format.

Get the bootstrap token from your Fleet Intelligence administrator. Save it to `/secure/path/to/agent-token` with nothing else in the file, not even a trailing newline, and set its mode to `0600`.

Create the Secrets from those files:

```bash
kubectl create namespace "$NAMESPACE" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace "$NAMESPACE" \
  --from-file=agent-token=/secure/path/to/agent-token

kubectl create secret generic fleet-intelligence-bmc-credentials \
  --namespace "$NAMESPACE" \
  --from-file=credentials.json=/secure/path/to/credentials.json
```

### Save your values file

Save your targets and the Secret names in a values file, for example `/path/to/collector-values.yaml`. List one entry under `targets` for each BMC; [Configuration](configuration.md#targets) describes every field.

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

helm upgrade --install fleet-intelligence-oob \
  fleet-intelligence-oob \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version "$OOB_CHART_VERSION" \
  --values /path/to/collector-values.yaml \
  --namespace "$NAMESPACE" \
  --create-namespace \
  --wait \
  --timeout 10m
```

For a deployment using private registry mirrors, configure `imagePullSecrets` and authenticate Helm to the chart registry as required by your organization.

Collector and NICo image tags default to the chart version. See [workload settings](configuration.md#workload-settings) for supported overrides.

## Verify

```bash
helm status fleet-intelligence-oob \
  --namespace "$NAMESPACE"

kubectl get pods \
  --namespace "$NAMESPACE" \
  -l app.kubernetes.io/instance=fleet-intelligence-oob

kubectl logs \
  --namespace "$NAMESPACE" \
  deployment/fleet-intelligence-oob

kubectl logs --namespace "$NAMESPACE" \
  deployment/nico-hardware-health --container generate-nico-config

kubectl logs --namespace "$NAMESPACE" \
  deployment/fleet-intelligence-oob-otel-gateway
```

Use your release's Deployment names, and skip any component you disabled.

Running pods don't prove everything works. Check that:

- NICo's `generate-nico-config` init container completed successfully.
- The collector logs show inventory uploads succeeding.
- The gateway logs show no enrollment or export errors.

If something fails:

| Problem | What to check |
| --- | --- |
| A BMC rejects the login | The target's `credentialsProfile`, any override for its MAC, and the account on the BMC itself |
| The collector or gateway can't enroll | That the bootstrap token is valid and `backend.url` is correct |
| The gateway can't export telemetry | That the backend is reachable from the cluster and its TLS certificate is trusted |

To change settings later, edit your values file and run the same `helm upgrade --install` command. See [Rotation and ownership](credentials.md#rotation-and-ownership) for which changes take effect on their own and which need a restart.

## Disconnect an individual machine

Before you delete a machine in Fleet Intelligence (the Delete Machine flow), remove it from every collector that monitors it. Otherwise the next collection cycle reports it again. Data that was already uploaded stays in Fleet Intelligence.

If your values file is generated by configuration management, make the change there too, so the machine doesn't come back on the next deploy. Substitute your own resource names in the commands below.

1. In your values file, find the target with the machine's BMC `mac` under `inventory.targets` and delete it. If it's the last target, [uninstall the collector](#uninstall-the-oob-collector) instead, because the target list can't be empty.
2. Run the same `helm upgrade --install` command with the updated file. The collector and NICo restart with the new target list.
3. Wait for both to finish restarting:

   ```bash
   kubectl rollout status deployment/fleet-intelligence-oob \
     --namespace "$NAMESPACE" --timeout=10m
   kubectl rollout status deployment/nico-hardware-health \
     --namespace "$NAMESPACE" --timeout=10m
   ```

Skip the NICo command if NICo is disabled. Then confirm that the release's ConfigMap no longer contains the machine's MAC, and check the [collector logs](#verify) to make sure the remaining targets still work. A few uploads that were already in progress may still arrive after the restart.

Keep the shared credentials Secret. If the machine had its own MAC override in the credentials file, you can remove it once nothing else uses it.

## Uninstall the OOB collector

Uninstalling stops collection for every target in the release. Data that was already uploaded stays in Fleet Intelligence. Substitute your own release name if it differs, and turn off any automation that would reinstall the release.

1. Remove the Helm release, including its OOB collector, NICo hardware health, and OpenTelemetry gateway:

   ```bash
   helm uninstall fleet-intelligence-oob \
     --namespace "$NAMESPACE" --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl get pods --namespace "$NAMESPACE" \
     -l app.kubernetes.io/instance=fleet-intelligence-oob
   ```

3. Delete unused credential Secrets:

   ```bash
   kubectl delete secret fleet-intelligence-oob-collector-agent-token \
     fleet-intelligence-bmc-credentials --namespace "$NAMESPACE"
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a Secret does not revoke its credentials; retire them through their issuing system. Keep shared resources and separately managed external telemetry collectors.
