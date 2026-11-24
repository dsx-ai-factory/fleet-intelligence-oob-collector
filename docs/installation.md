# Installation

Install the collector on Kubernetes with its Helm chart. By default, the chart runs three workloads: the OOB collector, NICo hardware health, and a telemetry gateway that forwards NICo's metrics and logs to Fleet Intelligence.

For DEB/RPM installations, use the [Linux package guide](host-installation.md).

For a NICo-managed site, use [endpoint source configuration](configuration.md#endpoint-source) to configure the target source and mount its OAuth2 client credential files.

## Prerequisites

- A Kubernetes cluster you can reach with `kubectl`
- Helm 3 or 4
- Network access from the cluster to your BMCs and to Fleet Intelligence
- The IP and MAC address of each BMC, and its login credentials
- A [service key](credentials.md#get-a-service-key) for Fleet Intelligence with the **Enroll Machines** scope

## Choose a chart version

Choose a version from the [NGC chart listing](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/fleet-intelligence/helm-charts/fleet-intelligence-oob-collector) and open the matching version of the [published collector documentation](https://docs.nvidia.com/fleet-intel/oob-collector) before creating any Secrets. Continue with that version's guide so the Secret names and keys match your chart. For example, for chart `1.0.0`, read `docs/installation.md` at the `v1.0.0` Git tag.

Set the chart version in the shell where you'll run Helm. Replace `<chart-version>` with the version you chose, using the chart's spelling without a leading `v`.

```bash
export OOB_CHART_VERSION='<chart-version>'
```

## Set up your shell

List your Kubernetes contexts, then replace `your-cluster-context` with the one for this installation.

```bash
kubectl config get-contexts
kubectl config use-context 'your-cluster-context' && kubectl config current-context
```

Confirm that the last command printed the intended context before creating the namespace or Secrets. `kubectl` and `helm` use the current context, so keep it selected until you finish.

```bash
export NAMESPACE=fleet-intelligence
kubectl create namespace "$NAMESPACE"
```

If the namespace already exists, `kubectl` reports `AlreadyExists`. Then set restrictive permissions for new files and open the working directory. The `chmod` command also protects the directory if it already exists.

```bash
umask 077
mkdir -p ~/fleetint-oob && chmod 700 ~/fleetint-oob && cd ~/fleetint-oob
```

Confirm you are in `~/fleetint-oob` before saving files. The examples use filenames relative to this directory. If you open a new terminal, confirm the context and set `OOB_CHART_VERSION` and `NAMESPACE` again, then run the working directory commands.

## Create the Secrets

The collector reads BMC logins from a credentials file and authenticates with the service key. Both go into Kubernetes Secrets; the chart only references them. To sync the Secrets from Vault or OpenBao instead, follow [Credentials](credentials.md) and continue at [Save your values file](#save-your-values-file).

Save this as `credentials.json` and replace the example username and password:

```json
{
  "profiles": {
    "compute": {
      "username": "replace-with-bmc-username",
      "password": "replace-with-bmc-password"
    }
  }
}
```

This is a complete file for compute BMCs that share one login. Each target in your values file selects a profile by name, here `compute`. Add more only if you need them:

- **Another profile:** if other BMCs, such as NVSwitch trays, use a different login, add a profile such as `nvswitch` next to `compute` and select it on those targets. Profiles no target uses are allowed, but leave out ones you don't need.
- **A per-MAC override:** if a single BMC has its own username or password, add an [override for its MAC](configuration.md#how-credentials-are-resolved).

Create the credentials Secret, then paste the service key when prompted. The key isn't echoed or saved to disk.

```bash
kubectl -n "$NAMESPACE" create secret generic fleet-intelligence-bmc-credentials \
  --from-file=credentials.json

printf 'Service key: '; read -rs TOKEN; echo
printf '%s' "$TOKEN" | kubectl -n "$NAMESPACE" create secret generic \
  fleet-intelligence-enrollment-token --from-file=enrollment-token=/dev/stdin
unset TOKEN
```

If a Secret already exists, `kubectl` reports `AlreadyExists`. To replace its contents, follow [Update credentials](#update-credentials).

## Save your values file

Save this as `values.yaml`, replace the example values, and add one entry under `targets` for each BMC:

```yaml
inventory:
  computeZone: your-compute-zone  # Optional. Exact compute zone name in Fleet Intelligence.
  nodeGroup: your-node-group      # Optional. Exact node group name in Fleet Intelligence.
  targets:
    - kind: compute               # Optional. compute, nvswitch, power_shelf, or unknown (default).
      name: your-node-name        # Optional. Display name; can't be blank.
      address: 192.0.2.10         # Required. BMC IP address, without a scheme or port.
      mac: "00:11:22:33:44:55"    # Required. BMC MAC address, in quotes; unique per target.
      credentialsProfile: compute # Profile name in credentials.json.
```

Replace every `your-...` value and the example address and MAC. Delete an optional line to leave it unset.

The chart's defaults already point to the two Secrets you created and to the production Fleet Intelligence endpoint. Add `port` to a target only if its Redfish port isn't `443`. [Configuration](configuration.md#targets) lists every field, including per-target placement and labels.

## Install

```bash
helm upgrade --install fleet-intelligence-oob-collector \
  fleet-intelligence-oob-collector \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version "$OOB_CHART_VERSION" \
  --namespace "$NAMESPACE" \
  --values values.yaml
```

Helm returns once it has applied the release; the pods start in the next step. If you pull images through a private registry mirror, set `imagePullSecrets` and authenticate Helm to the chart registry as your organization requires. Collector and NICo image tags default to the chart version; see [workload settings](configuration.md#workload-settings).

## Verify

1. Watch the pods start. Image pulls can take a few minutes.

   ```bash
   kubectl -n "$NAMESPACE" get pods -w \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

   With the default components, three pods start. NICo first shows `Init:0/1` and `PodInitializing` while it generates its configuration. When all three look like this, press Ctrl+C:

   ```text
   NAME                                                            READY   STATUS    RESTARTS   AGE
   fleet-intelligence-oob-collector-6c9f7d8b5-k2x9p                1/1     Running   0          3m
   fleet-intelligence-oob-collector-otel-gateway-5d8f6c7b9-r4t7w   1/1     Running   0          3m
   nico-hardware-health-7f9b6d4c8-m3n5q                            1/1     Running   0          3m
   ```

   Pod name suffixes differ in your cluster. If you disabled a component, its pod doesn't appear. If a pod shows any other status or keeps restarting, see [Troubleshoot](#troubleshoot).

2. Follow the collector log to check enrollment and the first inventory upload. Skip this check if you disabled the collector.

   ```bash
   kubectl -n "$NAMESPACE" logs --follow deployment/fleet-intelligence-oob-collector
   ```

   Look for `backend enrollment succeeded` and `first inventory upload accepted in this cycle`. The first upload can take several minutes, depending on the number of BMCs, their response times, and whether collection needs a retry. If you see warnings or errors while you wait, use [Troubleshoot](#troubleshoot) to decide what to check. Press Ctrl+C once you've seen the first upload or identified a failure.

3. If you enabled the collector, find each BMC's MAC under Machines in Fleet Intelligence and check that its inventory shows the expected hardware.

4. If you enabled NICo, verify telemetry delivery. Look for recent BMC sensor readings for the configured machines and confirm that expected BMC log events arrive. Check in Fleet Intelligence when using the bundled gateway, or at your configured OTLP destination when using an external collector.

   If telemetry is missing, check NICo and the bundled gateway logs. Skip the second command if you disabled the gateway.

   ```bash
   kubectl -n "$NAMESPACE" logs deployment/nico-hardware-health
   kubectl -n "$NAMESPACE" logs deployment/fleet-intelligence-oob-collector-otel-gateway
   ```

   Empty logs can be normal for NICo's `generate-nico-config` init container and the gateway. Use recent telemetry at the destination to confirm delivery.

## Troubleshoot

Find the pod's status in the output of the watch command:

| Pod status | What to check |
| --- | --- |
| `Pending` | The pod's events, for insufficient resources or scheduling settings that match no node. See [workload settings](configuration.md#workload-settings). |
| `ErrImagePull` or `ImagePullBackOff` | That the nodes can reach `nvcr.io` or your registry mirror, and that `imagePullSecrets` is set for a mirror. |
| `CreateContainerConfigError`, or a `FailedMount` event | That the Secrets referenced by the chart exist in `$NAMESPACE` and contain the configured keys. The defaults are `credentials.json` and `enrollment-token`. |
| `Init:Error` or `Init:CrashLoopBackOff` on `nico-hardware-health` | The logs of its `generate-nico-config` init container. |
| `CrashLoopBackOff` | The logs of the previous, crashed container (`--previous`). |
| `Running` but not ready | The container's logs. |

These commands show a pod's events and logs. Set `POD` to the name from the watch output. Set `CONTAINER` to `collector`, `nico-hardware-health`, `otel-gateway`, or `generate-nico-config` for the NICo init container.

```bash
POD='your-pod-name'
CONTAINER='collector'
kubectl -n "$NAMESPACE" describe pod "$POD"
kubectl -n "$NAMESPACE" logs "$POD" --container "$CONTAINER"
kubectl -n "$NAMESPACE" logs "$POD" --container "$CONTAINER" --previous
```

If startup stalls, use the affected Deployment's name to check its conditions.

```bash
DEPLOYMENT='your-deployment-name'
kubectl -n "$NAMESPACE" describe deployment "$DEPLOYMENT"
```

If the Deployment reports `ProgressDeadlineExceeded`, Kubernetes detected that the rollout stopped making progress. Use the pod events and logs to identify the cause.

If the pods run but collection fails, check the collector log. Per-BMC problems are logged at `warn` with `bmc_mac`, `bmc_address`, and the reason. Set the collector log level to `debug` to also see healthy per-BMC results.

| Log message or symptom | What it means and what to check |
| --- | --- |
| Enrollment fails with HTTP `401` or `403` | Fleet Intelligence refused the enrollment request. The affected workload exits and Kubernetes retries startup. Check that the service key is valid, has the **Enroll Machines** scope, and belongs to the intended organization. Check `backend.url` as well. |
| `retrying failed inventory targets after a delay` | The collector will retry the affected BMCs once after about five minutes. Check earlier warnings for the affected MAC and the cause. If Redfish rejected the login, check the target's `credentialsProfile`, its MAC override, and the BMC account. |
| `backend JWT rejected; enrolling again before retry` | An inventory upload received HTTP `401`. The collector re-enrolls and retries the upload once. If that fails, check the service key and backend URL. |
| `backend rejected inventory node` or `inventory mini-batch export failed` | The backend rejected a node or the batch could not be uploaded. The collector continues with later batches when possible. Check the logged reason and MAC for a node rejection, or the error and backend connectivity for a batch failure. |
| The gateway reports repeated telemetry export errors | Inspect the HTTP status or connection error in the gateway log. Check backend connectivity and certificate trust from the cluster. For HTTP `401`, the gateway attempts to re-enroll and replay the request once when it can. If enrollment fails, check the service key and backend URL. |

If `helm upgrade --install` itself fails, render the chart locally to see download, values validation, or template errors:

```bash
helm template fleet-intelligence-oob-collector fleet-intelligence-oob-collector \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version "$OOB_CHART_VERSION" --namespace "$NAMESPACE" \
  --values values.yaml --debug
```

## Update an installation

Run these steps from `~/fleetint-oob` with the release's context and namespace selected, as in [Set up your shell](#set-up-your-shell).

### Update credentials

For Secrets synced from Vault or OpenBao, change the value at the source and let the operator sync it. For Secrets you manage by hand, use these commands to update the configured keys in place. Server-side apply with the field manager below avoids writing the updated values to a last-applied annotation.

To change BMC logins, edit `credentials.json`, then run:

```bash
kubectl -n "$NAMESPACE" create secret generic fleet-intelligence-bmc-credentials \
  --from-file=credentials.json --dry-run=client -o yaml |
  kubectl -n "$NAMESPACE" apply --server-side --force-conflicts \
    --field-manager=fleetint-oob-install -f -
```

To replace the service key:

```bash
printf 'Service key: '; read -rs TOKEN; echo
printf '%s' "$TOKEN" | kubectl -n "$NAMESPACE" create secret generic \
  fleet-intelligence-enrollment-token --from-file=enrollment-token=/dev/stdin \
  --dry-run=client -o yaml |
  kubectl -n "$NAMESPACE" apply --server-side --force-conflicts \
    --field-manager=fleetint-oob-install -f -
unset TOKEN
```

If you previously managed either Secret using client-side `kubectl apply`, its `last-applied-configuration` annotation can still contain credential data. After a successful update, remove that annotation from each affected manually managed Secret. Set `SECRET` to its name. This command reports the Secret name without displaying the annotation.

```bash
SECRET='fleet-intelligence-enrollment-token'
kubectl -n "$NAMESPACE" annotate secret "$SECRET" kubectl.kubernetes.io/last-applied-configuration-
```

The collector picks up new BMC logins without a restart; a new service key needs one. [Rotation and ownership](credentials.md#rotation-and-ownership) gives the timing and the restart commands.

### Add BMCs or change settings

Add the new entries under `inventory.targets` in `values.yaml`, keeping the BMCs you still collect. If a new BMC needs a new login, [update the credentials](#update-credentials) first. Run the [install command](#install) again with the [installed chart version](#upgrade-the-chart), then [verify the rollout](#verify-an-update). Target changes restart the collector and NICo.

### Upgrade the chart

Check the installed version:

```bash
helm list -n "$NAMESPACE" --filter '^fleet-intelligence-oob-collector$'
```

Read the [OOB Collector documentation](https://docs.nvidia.com/fleet-intel/oob-collector) for the new chart version and confirm that the chart points to the Secret containing your service key. See [Fleet Intelligence connection](configuration.md#fleet-intelligence-connection) for the default Secret name and key.

Set `OOB_CHART_VERSION` to the new chart version and run the [install command](#install) again. Then [verify the rollout](#verify-an-update).

### Verify an update

After Helm returns, follow the latest rollout of each Deployment in the release. This reports progress while Kubernetes starts the updated pods.

```bash
kubectl -n "$NAMESPACE" rollout status deployment \
  -l app.kubernetes.io/instance=fleet-intelligence-oob-collector --timeout=10m
```

When the rollouts complete, repeat [Verify](#verify) for the affected workloads and targets. If a rollout stalls, see [Troubleshoot](#troubleshoot).

## Disconnect an individual machine

Before you delete a machine in Fleet Intelligence (the Delete Machine flow), remove it from every collector that monitors it. Otherwise the next collection cycle reports it again. Data that was already uploaded stays in Fleet Intelligence.

If your values file is generated by configuration management, make the change there too, so the machine doesn't come back on the next deploy. Substitute your own resource names in the commands below.

1. In your values file, find the target with the machine's BMC `mac` under `inventory.targets` and delete it. If it's the last target, [uninstall the collector](#uninstall-the-oob-collector) instead, because the target list can't be empty.
2. Run the same `helm upgrade --install` command with the updated file. The collector and NICo restart with the new target list.
3. Wait for both to finish restarting:

   ```bash
   kubectl -n "$NAMESPACE" rollout status deployment/fleet-intelligence-oob-collector --timeout=10m
   kubectl -n "$NAMESPACE" rollout status deployment/nico-hardware-health --timeout=10m
   ```

Skip the NICo command if NICo is disabled. Then confirm that the release's ConfigMap no longer contains the machine's MAC, and check the [collector logs](#verify) to make sure the remaining targets still work. A few uploads that were already in progress may still arrive after the restart.

Keep the shared credentials Secret. If the machine had its own MAC override in the credentials file, you can remove it once nothing else uses it.

## Uninstall the OOB collector

Uninstalling stops collection for every target in the release. Data that was already uploaded stays in Fleet Intelligence. Substitute your own release name if it differs, and turn off any automation that would reinstall the release.

1. Remove the Helm release, including its OOB collector, NICo hardware health, and OpenTelemetry gateway:

   ```bash
   helm uninstall fleet-intelligence-oob-collector -n "$NAMESPACE" --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl -n "$NAMESPACE" get pods -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

3. Remove credentials dedicated to this release. Keep any credentials, Secrets, sync resources, and secret stores shared with other deployments.

   - **Vault Secrets Operator (VSO) or External Secrets Operator (ESO).** Remove the relevant `VaultStaticSecret` or `ExternalSecret` resources first. If deployment automation manages them, remove them from its configuration as well. Then remove the dedicated source values from Vault or OpenBao. As long as the sync resource exists, the operator can recreate its Kubernetes Secret. Check which Secrets remain and delete any unused ones left by the operator. See [Sync the Secrets into Kubernetes](credentials.md#4-sync-the-secrets-into-kubernetes) for the resource names and source paths used in the examples.
   - **Manually managed Secrets.** Delete each unused Secret directly. Replace the example name with the name of the Secret you want to remove.

   ```bash
   SECRET='fleet-intelligence-enrollment-token'
   kubectl -n "$NAMESPACE" delete secret "$SECRET"
   ```

4. Revoke unused credentials through their issuing system and remove the local credential files for this release. Keep any files used by another deployment.
