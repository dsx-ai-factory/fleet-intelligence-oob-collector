# Installation

Install the collector on Kubernetes with its Helm chart. By default, the chart runs three workloads: the OOB collector, NICo hardware health, and a telemetry gateway that forwards NICo's metrics and logs to Fleet Intelligence.

For DEB/RPM installations, use the [Linux package guide](host-installation.md).

## Prerequisites

- A Kubernetes cluster you can reach with `kubectl`
- Helm 3
- Network access from the cluster to your BMCs and to Fleet Intelligence
- The IP and MAC address of each BMC, and its login credentials
- A Fleet Intelligence enrollment token

## Choose a Kubernetes context and namespace

List your kubeconfig contexts, then replace `your-cluster-context` with the one for this installation. Set the namespace to the one you want to use, or to the existing release's namespace when updating. Set these variables again if you open a new terminal.

```bash
kubectl config get-contexts
export KUBE_CONTEXT='your-cluster-context'
export NAMESPACE=fleet-intelligence
kubectl config get-contexts "$KUBE_CONTEXT"
```

The commands below pass the context and namespace explicitly. Check both before installing, updating, or uninstalling.

## Configure targets and Secrets

Put the target list and Secret references in your values file. Store the BMC credentials file and enrollment token in two Kubernetes Secrets.

The steps below create or update Secrets you manage by hand. To sync them from Vault or OpenBao, follow [Credentials](credentials.md) using the same cluster and namespace, then continue at [Save your values file](#save-your-values-file).

### Prepare the files

Replace these paths with your local paths. Save the credential files in a private directory and use `umask 077` before creating them so only your user can read them.

```bash
umask 077
export BMC_CREDENTIALS_FILE='/secure/path/to/credentials.json'
export ENROLLMENT_TOKEN_FILE='/secure/path/to/enrollment-token'
```

For compute BMCs that share a login, save this JSON to `$BMC_CREDENTIALS_FILE` and replace both example values.

```json
{
  "profiles": {
    "bmc": {
      "username": "replace-with-bmc-username",
      "password": "replace-with-bmc-password"
    }
  }
}
```

Each target selects a named profile through `credentialsProfile`. If you add switches that use a separate login, add a profile such as `switch` and reference it on those targets. Omit unused profiles. Use [per-MAC overrides](configuration.md#how-credentials-are-resolved) when an individual BMC needs a different username or password.

Get the enrollment token from your Fleet Intelligence administrator and save it to `$ENROLLMENT_TOKEN_FILE` with no surrounding whitespace or trailing newline. Set both files to mode `0600`.

```bash
chmod 600 "$BMC_CREDENTIALS_FILE" "$ENROLLMENT_TOKEN_FILE"
```

### Create the Secrets

Use these commands for the two dedicated Secrets in this example. They create missing Secrets and update existing ones in place. Server-side apply avoids storing a second copy of the credentials in the last-applied annotation. The commands take ownership of the supplied fields if another field manager previously wrote them. For Secrets maintained by an operator or deployment automation, make changes at their source.

```bash
kubectl --context "$KUBE_CONTEXT" create namespace "$NAMESPACE" \
  --dry-run=client -o yaml | kubectl --context "$KUBE_CONTEXT" apply -f -

kubectl --context "$KUBE_CONTEXT" create secret generic fleet-intelligence-enrollment-token \
  --namespace "$NAMESPACE" \
  --from-file="enrollment-token=$ENROLLMENT_TOKEN_FILE" --dry-run=client -o yaml |
  kubectl --context "$KUBE_CONTEXT" --namespace "$NAMESPACE" apply \
    --server-side --field-manager=fleetint-oob-install --force-conflicts -f -

kubectl --context "$KUBE_CONTEXT" create secret generic fleet-intelligence-bmc-credentials \
  --namespace "$NAMESPACE" \
  --from-file="credentials.json=$BMC_CREDENTIALS_FILE" --dry-run=client -o yaml |
  kubectl --context "$KUBE_CONTEXT" --namespace "$NAMESPACE" apply \
    --server-side --field-manager=fleetint-oob-install --force-conflicts -f -
```

### Save your values file

Set `OOB_VALUES_FILE` to where you'll save your values file. Start with this example for one compute BMC and add one entry under `inventory.targets` per BMC.

```bash
export OOB_VALUES_FILE='/path/to/collector-values.yaml'
```

```yaml
inventory:
  computeZone: your-compute-zone
  nodeGroup: your-node-group
  targets:
    - kind: compute
      name: your-node-name
      address: 192.0.2.10
      mac: "00:11:22:33:44:55"
      credentialsProfile: bmc
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
backend:
  enrollmentToken:
    existingSecret: fleet-intelligence-enrollment-token
    secretKey: enrollment-token
```

- Replace `address` and `mac` with the BMC's IP and MAC. Both are required. Enter the IP without a scheme or port and quote each unique MAC. Set `port` only if Redfish uses a port other than the default `443`.
- Set `credentialsProfile` to the profile name in your JSON. You can omit it when a per-MAC override supplies the target's credentials.
- Set `kind` to `compute`, `nvswitch`, `power_shelf`, or `unknown` (the default). If you set a display `name`, include at least one non-whitespace character.
- Replace `computeZone` and `nodeGroup` with the exact placement names in your Fleet Intelligence account, or omit either field to leave it unset. Targets inherit the values under `inventory` and can override them individually.
- If you use different Secret names or keys, update the references to match. `backend.url` defaults to `https://data.fleet-intelligence.nvidia.com`. Set it only if you use a different Fleet Intelligence endpoint.

See [Configuration](configuration.md#targets) for the full field reference.

## Install

Choose a version from the [NGC chart listing](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/fleet-intelligence/helm-charts/fleet-intelligence-oob-collector) and follow the matching version of the [published collector documentation](https://docs.nvidia.com/fleet-intel/oob-collector). For example, `--version 1.0.0` installs chart `1.0.0`, which matches the `v1.0.0` Git tag. On GitHub, read the guides at the matching tag. Set `OOB_CHART_VERSION` to your chosen chart version.

```bash
export OOB_CHART_VERSION='<chart-version>'

helm upgrade --install fleet-intelligence-oob-collector \
  fleet-intelligence-oob-collector \
  --kube-context "$KUBE_CONTEXT" \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version "$OOB_CHART_VERSION" \
  --values "$OOB_VALUES_FILE" \
  --namespace "$NAMESPACE" \
  --create-namespace
```

For a deployment using private registry mirrors, configure `imagePullSecrets` and authenticate Helm to the chart registry as required by your organization.

Collector and NICo image tags default to the chart version. See [workload settings](configuration.md#workload-settings) for supported overrides.

## Verify

After Helm returns, watch the release's pods start in the same terminal. Image pulls and pod initialization can take several minutes.

```bash
kubectl --context "$KUBE_CONTEXT" get pods --namespace "$NAMESPACE" \
  -l app.kubernetes.io/instance=fleet-intelligence-oob-collector --watch
```

Wait for each enabled pod to show `Running` with all containers ready, such as `1/1`. Press Ctrl+C to stop watching. If any pod stays unready or keeps restarting, see [Troubleshoot installation](#troubleshoot-installation).

Check the release and workloads. Helm's `STATUS: deployed` indicates that Helm applied the release. Use Kubernetes readiness and the checks below to verify startup and collection. Use your release's Deployment names and skip any component you disabled.

```bash
helm status fleet-intelligence-oob-collector \
  --kube-context "$KUBE_CONTEXT" --namespace "$NAMESPACE"

kubectl --context "$KUBE_CONTEXT" get deployments,pods \
  --namespace "$NAMESPACE" \
  -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
```

With the default workloads and one replica each, expect output like this. Pod suffixes are assigned by Kubernetes.

```text
NAME                                                          READY   UP-TO-DATE   AVAILABLE
deployment.apps/fleet-intelligence-oob-collector               1/1     1            1
deployment.apps/fleet-intelligence-oob-collector-otel-gateway  1/1     1            1
deployment.apps/nico-hardware-health                          1/1     1            1

NAME                                                    READY   STATUS    RESTARTS
pod/fleet-intelligence-oob-collector-<suffix>            1/1     Running   0
pod/fleet-intelligence-oob-collector-otel-gateway-<suffix> 1/1     Running   0
pod/nico-hardware-health-<suffix>                        1/1     Running   0
```

Check the collector, NICo init container, and gateway logs.

```bash
kubectl --context "$KUBE_CONTEXT" logs --namespace "$NAMESPACE" \
  deployment/fleet-intelligence-oob-collector

kubectl --context "$KUBE_CONTEXT" logs --namespace "$NAMESPACE" \
  deployment/nico-hardware-health --container generate-nico-config

kubectl --context "$KUBE_CONTEXT" logs --namespace "$NAMESPACE" \
  deployment/fleet-intelligence-oob-collector-otel-gateway
```

At the default `info` level, look for `backend enrollment succeeded` and `first inventory upload accepted in this cycle` in the collector log. Per-BMC problems appear at `warn` with `bmc_mac`, `bmc_address`, and the failure reason before any retry delay. Set the collector log level to `debug` to see healthy per-BMC results.

Empty logs can be normal for the `generate-nico-config` init container and the gateway. In `kubectl describe pod` output for NICo, look for `Reason: Completed` and `Exit Code: 0` under that init container. For the gateway, check that the pod is ready and look for enrollment or export errors if the log has entries.

To verify inventory end to end, look up each configured BMC MAC under Machines in Fleet Intelligence after the first cycle. Open the matching machine and check that its inventory includes the expected hardware details.

## Troubleshoot installation

Use the Deployment and pod statuses from [Verify](#verify) to find the affected workload. Describe an unready or restarting pod, then use the affected container name from its description to retrieve logs.

```bash
POD='your-pod-name'
kubectl --context "$KUBE_CONTEXT" describe pod "$POD" --namespace "$NAMESPACE"

CONTAINER='collector' # Replace with the affected container name.
kubectl --context "$KUBE_CONTEXT" logs "$POD" \
  --namespace "$NAMESPACE" --container "$CONTAINER"
kubectl --context "$KUBE_CONTEXT" logs "$POD" \
  --namespace "$NAMESPACE" --container "$CONTAINER" --previous
```

The pod description includes scheduling, mount, and image pull events. For a pod in `CrashLoopBackOff`, use `--previous` to read the most recent crashed container's logs. For NICo startup failures, inspect `generate-nico-config` first.

If Helm fails before creating pods, render the same chart locally to diagnose chart downloads, values validation, or template errors.

```bash
helm template fleet-intelligence-oob-collector fleet-intelligence-oob-collector \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version "$OOB_CHART_VERSION" --values "$OOB_VALUES_FILE" \
  --namespace "$NAMESPACE" --debug
```

| Problem | What to check |
| --- | --- |
| The Deployment reports `ProgressDeadlineExceeded` | Find the unready pod and inspect its events and logs for the cause. |
| `FailedMount` or `CreateContainerConfigError` | Check that each referenced Secret exists in the selected namespace and contains the key specified in your values file. |
| A BMC rejects the login | The target's `credentialsProfile`, any override for its MAC, and the account on the BMC itself |
| The collector or gateway can't enroll | That the enrollment token is valid and `backend.url` is correct |
| The gateway can't export telemetry | That the backend is reachable from the cluster and its TLS certificate is trusted |

## Update an existing installation

1. Set the [context and namespace](#choose-a-kubernetes-context-and-namespace) for the existing release and confirm its installed chart version.

   ```bash
   helm list --kube-context "$KUBE_CONTEXT" --namespace "$NAMESPACE" \
     --filter '^fleet-intelligence-oob-collector$'
   ```

2. To rotate credentials, update the password at the BMC or get a new enrollment token from your Fleet Intelligence administrator. For manually managed Secrets, edit the corresponding local file and rerun its pipeline under [Create the Secrets](#create-the-secrets) with the Secret name and key from your existing values file. For operator-managed Secrets, update the secrets manager and wait for the sync. Follow [Rotation and ownership](credentials.md#rotation-and-ownership) for reload timing and the commands to restart the collector and gateway after an enrollment token change.
3. To add BMCs, append entries to `inventory.targets` in the existing values file and retain the entries for machines you still collect. If a new target needs new credentials, update the BMC credentials Secret first. Run the same [Helm upgrade command](#install) with your updated values. Keep the installed chart version for a values-only change, or select the new version and consult its matching documentation when upgrading the chart. Target changes restart the collector and NICo.
4. After a Helm upgrade, check that each Deployment has finished its latest rollout before checking inventory. The command below prints rollout progress.

   ```bash
   kubectl --context "$KUBE_CONTEXT" rollout status deployment --namespace "$NAMESPACE" \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector --timeout=10m
   ```

   Repeat the [verification checks](#verify) for the affected workloads and targets.

## Disconnect an individual machine

Before you delete a machine in Fleet Intelligence (the Delete Machine flow), remove it from every collector that monitors it. Otherwise the next collection cycle reports it again. Data that was already uploaded stays in Fleet Intelligence.

If your values file is generated by configuration management, make the change there too, so the machine doesn't come back on the next deploy. Substitute your own resource names in the commands below.

1. In your values file, find the target with the machine's BMC `mac` under `inventory.targets` and delete it. If it's the last target, [uninstall the collector](#uninstall-the-oob-collector) instead, because the target list can't be empty.
2. Run the same `helm upgrade --install` command with the updated file. The collector and NICo restart with the new target list.
3. Wait for both to finish restarting:

   ```bash
   kubectl --context "$KUBE_CONTEXT" rollout status deployment/fleet-intelligence-oob-collector \
     --namespace "$NAMESPACE" --timeout=10m
   kubectl --context "$KUBE_CONTEXT" rollout status deployment/nico-hardware-health \
     --namespace "$NAMESPACE" --timeout=10m
   ```

Skip the NICo command if NICo is disabled. Then confirm that the release's ConfigMap no longer contains the machine's MAC, and check the [collector logs](#verify) to make sure the remaining targets still work. A few uploads that were already in progress may still arrive after the restart.

Keep the shared credentials Secret. If the machine had its own MAC override in the credentials file, you can remove it once nothing else uses it.

## Uninstall the OOB collector

Uninstalling stops collection for every target in the release. Data that was already uploaded stays in Fleet Intelligence. Substitute your own release name if it differs, and turn off any automation that would reinstall the release.

1. Remove the Helm release, including its OOB collector, NICo hardware health, and OpenTelemetry gateway:

   ```bash
   helm uninstall fleet-intelligence-oob-collector \
     --kube-context "$KUBE_CONTEXT" --namespace "$NAMESPACE" --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl --context "$KUBE_CONTEXT" get pods --namespace "$NAMESPACE" \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

3. Delete unused credential Secrets:

   ```bash
   kubectl --context "$KUBE_CONTEXT" delete secret fleet-intelligence-enrollment-token \
     fleet-intelligence-bmc-credentials --namespace "$NAMESPACE"
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a Secret does not revoke its credentials; retire them through their issuing system. Keep shared resources and separately managed external telemetry collectors.
