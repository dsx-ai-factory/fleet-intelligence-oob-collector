# Installation

Install the collector on Kubernetes with its Helm chart. By default, the chart runs three workloads: the OOB collector, NICo hardware health, and a telemetry gateway that forwards NICo's metrics and logs to Fleet Intelligence.

For DEB/RPM installations, use the [Linux package guide](host-installation.md).

## Prerequisites

- A Kubernetes context with permission to deploy the chart and read the two Secrets in your chosen namespace. The installer applies the namespace and can create Secrets from files. Its verification checks need access to Deployments, pods, and pod logs. Read access to ReplicaSets and events helps diagnose failures. If your administrator provisions the namespace and Secrets for you, the manual Helm path can use them directly.
- Helm 3 or 4 and `kubectl`. The interactive installer also needs Bash 3.2 or later, `curl`, and an interactive terminal.
- Network access from the pods to each BMC's Redfish HTTPS port and to Fleet Intelligence over HTTPS. Your install machine needs access to the NGC chart repository, and the cluster needs access to the workload image registry.
- One BMC to verify first. Have its hardware type, IP address, physical BMC MAC address, Redfish port, and login ready. Add the rest once inventory and health telemetry are visible.
- A private BMC credentials JSON file and a Fleet Intelligence enrollment token from your administrator, or provisioned Kubernetes Secrets containing those values. See [Prepare credentials and Secrets](#create-the-secrets).

## Plan your installation

For a new deployment, use the `fleet-intelligence` namespace, the `fleet-intelligence-oob-collector` release name, and the Fleet Intelligence production endpoint at `https://data.fleet-intelligence.nvidia.com`. Use Redfish HTTPS port `443` unless your BMC listens on a different port. Verify your cluster with `kubectl config current-context`.

Use the [interactive installer](#interactive-installation) to set up inventory and health together on BMCs without existing NICo coverage. If NICo already monitors those BMCs, see [Keep an existing NICo deployment](#keep-an-existing-nico-deployment). To change an installed release, use its [saved Helm values](#change-settings-after-installation).

## Configure NICo hardware health

For your first installation, use the bundled NICo and telemetry gateway with the chart defaults. The OOB collector uploads inventory and attestation. NICo uses the same BMC targets and credentials to collect sensor readings and BMC events. The gateway forwards health metrics and logs to Fleet Intelligence. You supply one target list, credentials file, and enrollment token for the deployment.

The defaults give you sensor readings, leak detection where supported, automatic BMC event streaming with polling fallback, and reachability and resource status. NICo and the OOB collector each make their own BMC requests. After verifying the first BMC, tune collection if your hardware or operational requirements call for it. Set overrides under `nicoHardwareHealth.config` in your Helm values. NICo generates its configuration when its pod starts. See [NICo collection controls](configuration.md#nico-collection-controls).

### Keep an existing NICo deployment

Check whether your existing NICo monitors the BMC MACs you plan to add and already sends the expected health data to Fleet Intelligence. A second NICo instance for the same BMCs adds Redfish traffic and can send duplicate health telemetry.

For a new OOB collector release that will use your existing NICo for health, follow the [manual Helm steps](#choose-a-namespace) and add this block to your values file before installing:

```yaml
nicoHardwareHealth:
  enabled: false
otelGateway:
  enabled: false
```

The chart runs inventory and attestation in this mode. Keep managing health targets, credentials, and export in your existing NICo deployment. Verify the new inventory upload and confirm that health telemetry continues through your existing path. When upgrading a release that already owns NICo and its gateway, start from that release's saved values to retain its component settings.

#### Roll back a separately managed NICo

1. Before editing, identify the configuration your existing NICo deployment uses and save a private copy of the working file. If Helm or another deployment tool manages that file, save the working values or configuration revision instead. Record the current telemetry destination so you can verify it after restoring.
2. If the change causes problems, restore that copy or revision through the same deployment tool. For a NICo service that reads a local file, restore it at its original path with its original ownership and permissions, then restart that service.
3. Check the NICo startup logs and confirm that fresh readings for the BMC MAC arrive at the original destination.

## Interactive installation

Run the installer from a terminal:

```bash
curl --fail --silent --show-error \
  https://raw.githubusercontent.com/dsx-ai-factory/fleet-intelligence-oob-collector/main/install.sh | bash
```

Use a terminal at least 80 columns wide and 24 rows high for the Unicode UI. Use arrows and Enter to select an option, or press its number. Enter accepts a displayed default. Esc cancels from menus and Ctrl-C cancels from any prompt. The installer uses plain prompts when the terminal UI is unavailable. Pass `--ascii` for basic borders, `--plain` for text prompts, or `--tui` to require the terminal UI.

1. **Cluster.** Confirm the Kubernetes context. The chart version defaults to the latest stable release in NGC unless you set it in the environment. Press Enter to accept the default or type an exact version. If the lookup fails, enter your version from the [NGC chart listing](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/fleet-intelligence/helm-charts/fleet-intelligence-oob-collector). Keep the suggested namespace, release name, and backend unless your deployment requires different values. Use the compute zone and node group from your Fleet Intelligence account if you have them.
2. **Health.** If these BMCs are new to NICo, press Enter to use the bundled setup. If NICo already monitors them, choose Yes to get the manual Helm path for keeping it.
3. **Targets.** Choose the hardware type and enter the real BMC IP address and MAC. Keep port `443` for standard Redfish HTTPS. For a first install, start with one target. Match the credentials profile name from your JSON file, for example `bmc` for the sample compute node. Leave it blank only if a per-MAC override supplies both login fields. You can leave optional placement overrides and labels blank. See [Target fields](configuration.md#targets).
4. **Secrets.** For a new namespace, choose `file` and enter the absolute path to your private credentials JSON. For a provisioned Secret, including a Vault or OpenBao-managed one, choose `existing` and enter its name and key.
5. **Access.** Choose the source of the enrollment token the same way. Leave the image-pull Secret blank unless your registry requires one.
6. **Review.** Check the cluster, version, targets, and Secret sources before choosing Yes. Enter advances through review pages. The final confirmation defaults to No.

The installer uses the exact chart version shown on the review screen for validation and installation. After confirmation, it checks cluster access and renders the chart, waits for the namespace to become active, and checks the Secret references. It creates missing Secrets from the selected files and reuses existing Secrets.

Once Helm returns, the installer checks the collector, NICo, and gateway Deployments. It then waits up to six minutes for the current collector's logs to confirm that Fleet Intelligence accepted an inventory upload. An unchanged collector can already have this evidence in its retained logs. The check confirms at least one accepted upload. Follow [Verify](#verify) to confirm fresh inventory and health telemetry for each BMC.

Both the terminal UI and plain mode show progress and recent workload warnings. In the terminal UI, the final panel opens on **Checks** and marks the checks that passed. Press **2** for **Logs**. Use Left and Right to switch between Kubernetes, collector, NICo, and gateway output, and Up and Down to change pages. Press **1** to return to the checks or **Enter** to close the panel and return to the terminal output. Plain mode prints `[OK]` beside completed checks and exits after printing the result.

If Helm fails, a Deployment does not finish rolling out, or the installer cannot confirm an inventory upload, it shows completed checks alongside the failure and available diagnostics. Those diagnostics include pod status, Kubernetes warning events, and recent workload logs for the release. Resources remain in the cluster for diagnosis.

To pass an option or run from a reviewed commit, use `bash -s -- --plain` (or another supported option) in place of `bash`, or replace `main` in the URL with the full commit SHA.

### Set installer defaults from environment variables

Export variables in the terminal before running the installer. When a prompt loads a default, it displays the value and identifies its source, for example `From environment: FLEETINT_INSTALL_NAMESPACE`. Press Enter to accept it, or enter or select a different value. The installer still asks you to review and confirm before applying changes.

```bash
export FLEETINT_INSTALL_KUBE_CONTEXT="$(kubectl config current-context)"
export FLEETINT_INSTALL_NAMESPACE=fleet-intelligence
export FLEETINT_INSTALL_BMC_KIND=compute
export FLEETINT_INSTALL_BMC_IP=192.0.2.10
export FLEETINT_INSTALL_BMC_MAC=02:00:00:00:00:01
export FLEETINT_INSTALL_BMC_CREDENTIALS_PROFILE=bmc
export FLEETINT_INSTALL_BMC_CREDENTIALS_FILE=/secure/path/to/credentials.json
export FLEETINT_INSTALL_ENROLLMENT_TOKEN_FILE=/secure/path/to/enrollment-token

curl --fail --silent --show-error \
  https://raw.githubusercontent.com/dsx-ai-factory/fleet-intelligence-oob-collector/main/install.sh | bash
```

Replace the sample BMC IP, MAC, profile, and file paths with your own. With `curl | bash`, export the variables before the pipeline so the Bash process receives them.

| Deployment variable | Prompt default it sets |
| --- | --- |
| `FLEETINT_INSTALL_KUBE_CONTEXT` | Kubernetes context name |
| `FLEETINT_INSTALL_NAMESPACE` | Kubernetes namespace |
| `FLEETINT_INSTALL_RELEASE` | Helm release name |
| `FLEETINT_INSTALL_CHART_VERSION` | Exact chart version, such as `1.2.3`. When unset, the installer looks up the latest stable release. |
| `FLEETINT_INSTALL_BACKEND_URL` | Fleet Intelligence HTTPS origin, without a path |
| `FLEETINT_INSTALL_COMPUTE_ZONE` | Default compute zone for all targets |
| `FLEETINT_INSTALL_NODE_GROUP` | Default node group for all targets |

| First BMC variable | Prompt default it sets |
| --- | --- |
| `FLEETINT_INSTALL_BMC_KIND` | `compute`, `nvswitch`, `power_shelf`, or `unknown` |
| `FLEETINT_INSTALL_BMC_IP` | BMC IPv4 or IPv6 address without a scheme or port |
| `FLEETINT_INSTALL_BMC_MAC` | Physical BMC MAC address |
| `FLEETINT_INSTALL_BMC_PORT` | Redfish HTTPS port |
| `FLEETINT_INSTALL_BMC_NAME` | Display name |
| `FLEETINT_INSTALL_BMC_CREDENTIALS_PROFILE` | Profile name in the credentials JSON file |
| `FLEETINT_INSTALL_BMC_COMPUTE_ZONE` | Compute zone override for this BMC |
| `FLEETINT_INSTALL_BMC_NODE_GROUP` | Node group override for this BMC |

Enter target labels and any additional BMCs at the prompts.

| Secret variable | Prompt default it sets |
| --- | --- |
| `FLEETINT_INSTALL_BMC_CREDENTIALS_FILE` | Absolute path to the private credentials JSON file. Selects the file source by default. |
| `FLEETINT_INSTALL_BMC_CREDENTIALS_SECRET_NAME` | BMC credentials Secret name |
| `FLEETINT_INSTALL_BMC_CREDENTIALS_SECRET_KEY` | Key in the BMC credentials Secret |
| `FLEETINT_INSTALL_ENROLLMENT_TOKEN_FILE` | Absolute path to the private enrollment token file. Selects the file source by default. |
| `FLEETINT_INSTALL_ENROLLMENT_TOKEN_SECRET_NAME` | Enrollment token Secret name |
| `FLEETINT_INSTALL_ENROLLMENT_TOKEN_SECRET_KEY` | Key in the enrollment token Secret |
| `FLEETINT_INSTALL_IMAGE_PULL_SECRET` | Name of an existing image-pull Secret in the chosen namespace |

Keep passwords and enrollment tokens in private files or Kubernetes Secrets. To reuse an existing Secret, leave its file variable unset and set its name and key if they differ from the defaults.

Unset or empty variables use the normal prompts and defaults. To omit a value supplied by the environment, unset its variable before running the installer. If an environment value fails prompt validation, the installer names the variable to correct or unset before retrying.

## Choose a namespace

Set the Kubernetes context, namespace, and release name once so all manual commands point to the same deployment. Substitute the values you chose if you used the installer. Check the printed context before continuing:

```bash
export KUBE_CONTEXT="$(kubectl config current-context)"
export NAMESPACE=fleet-intelligence
export RELEASE=fleet-intelligence-oob-collector
printf '%s\n' "$KUBE_CONTEXT"
```

## Configure targets and Secrets

You give the chart two things: your target list, in a values file, and two Kubernetes Secrets holding the BMC credentials file and the enrollment token. The chart never stores passwords; it only references the Secrets.

For the interactive installer, prepare the private local files below and let the installer create the Secrets. For manual Helm installation, run the Kubernetes commands after preparing the files. If the Secrets already exist in the selected namespace, use their names and keys in your values. To provision them from Vault or OpenBao, follow [Credentials](credentials.md).

### Create the Secrets

Copy [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json) to a private directory, for example `/secure/path/to/credentials.json`, set its mode to `0600`, and fill in your BMC usernames and passwords. [Configuration](configuration.md#bmc-credentials-file) explains the file format.

Get the enrollment token from your Fleet Intelligence administrator. Save it to `/secure/path/to/enrollment-token` with nothing else in the file, not even a trailing newline, and set its mode to `0600`.

For a manual install with local files, create the namespace if needed, then create the Secrets:

```bash
kubectl --context "$KUBE_CONTEXT" create namespace "$NAMESPACE" \
  --dry-run=client -o yaml | kubectl --context "$KUBE_CONTEXT" apply -f -

kubectl --context "$KUBE_CONTEXT" create secret generic fleet-intelligence-enrollment-token \
  --namespace "$NAMESPACE" \
  --from-file=enrollment-token=/secure/path/to/enrollment-token

kubectl --context "$KUBE_CONTEXT" create secret generic fleet-intelligence-bmc-credentials \
  --namespace "$NAMESPACE" \
  --from-file=credentials.json=/secure/path/to/credentials.json
```

### Save your values file

Save your targets and Secret references in a values file, for example `/path/to/collector-values.yaml`. Replace the example IP, MAC, and placement with your values. The name `bmc` must match a profile in your credentials JSON. Omit the placement fields if you have no values for them. See [Target fields](configuration.md#targets).

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
  enrollmentToken:
    existingSecret: fleet-intelligence-enrollment-token
    secretKey: enrollment-token
```

`backend.url` defaults to the production endpoint, `https://data.fleet-intelligence.nvidia.com`. Set it in your values file when using a different Fleet Intelligence endpoint.

## Install

Choose a published chart version from the [NGC chart listing](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/fleet-intelligence/helm-charts/fleet-intelligence-oob-collector) and replace `<chart-version>` below:

```bash
export OOB_CHART_VERSION='<chart-version>'

helm upgrade --install "$RELEASE" \
  fleet-intelligence-oob-collector \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version "$OOB_CHART_VERSION" \
  --values /path/to/collector-values.yaml \
  --kube-context "$KUBE_CONTEXT" \
  --namespace "$NAMESPACE" \
  --wait \
  --timeout 10m
```

For Helm 4, replace `--wait` with `--wait=legacy --server-side=false` to match the interactive installer. For a deployment using private registry mirrors, configure `imagePullSecrets` and authenticate Helm to the chart registry as required by your organization.

Collector and NICo image tags default to the chart version. See [workload settings](configuration.md#workload-settings) for supported overrides.

## Verify

1. Check the release and workloads. Use the cluster, namespace, and release you selected during installation.

   ```bash
   helm status "$RELEASE" --kube-context "$KUBE_CONTEXT" --namespace "$NAMESPACE"
   kubectl --context "$KUBE_CONTEXT" --namespace "$NAMESPACE" get pods \
     -l "app.kubernetes.io/instance=$RELEASE"
   ```

2. Check the collector logs for `backend enrollment succeeded` and `first inventory upload accepted in this cycle`. If you have multiple targets, an accepted upload confirms at least one target. Check the others by BMC MAC in Fleet Intelligence.

   ```bash
   kubectl --context "$KUBE_CONTEXT" --namespace "$NAMESPACE" logs \
     -l "app.kubernetes.io/instance=$RELEASE,app.kubernetes.io/name=fleet-intelligence-oob-collector" \
     --tail=100
   ```

3. For the bundled health setup, check that NICo's configuration generator finished and there are no gateway enrollment or export errors.

   ```bash
   kubectl --context "$KUBE_CONTEXT" --namespace "$NAMESPACE" logs \
     -l "app.kubernetes.io/instance=$RELEASE,app.kubernetes.io/component=nico-hardware-health" \
     --container generate-nico-config --tail=100
   kubectl --context "$KUBE_CONTEXT" --namespace "$NAMESPACE" logs \
     -l "app.kubernetes.io/instance=$RELEASE,app.kubernetes.io/component=otel-gateway" \
     --tail=100
   ```

4. Find the target in Fleet Intelligence using its BMC MAC. Confirm that inventory is fresh and expected health readings arrive from the bundled or existing NICo deployment. Allow for the selected polling interval. BMC events can be absent if the device has no events to report. Once the first BMC is verified, add the remaining targets.

| Problem | What to check |
| --- | --- |
| Pod fails to pull an image | Registry access, the image tag, and any required image-pull Secret |
| NICo cannot start | Logs from `generate-nico-config`, the target fields, and the mounted credentials Secret |
| A BMC rejects the login | Its credential profile, any per-MAC override, and the account on the BMC. Collector warnings include `bmc_mac` and `bmc_address`. |
| The collector or gateway cannot enroll | Enrollment token Secret and key, backend URL, and network access |
| Inventory arrives but health readings are missing | NICo application logs, access to the BMC, and gateway export logs. For an existing NICo deployment, check its configured export destination. |

## Change settings after installation

Keep one values file for the deployment, including targets, Secret references, and NICo controls. If you used the interactive installer, retrieve its saved Helm values before editing. Use the context, namespace, and release name you selected:

```bash
helm get values "$RELEASE" \
  --namespace "$NAMESPACE" --kube-context "$KUBE_CONTEXT" \
  --output yaml > /path/to/collector-values.yaml
```

Edit that file and repeat the [Install](#install) command with the same release, namespace, chart version, and Kubernetes context. Before adjusting NICo settings, [save the working values for rollback](#roll-back-bundled-nico-health-settings).

Changes to the shared targets or `nicoHardwareHealth.config` restart the collector and bundled NICo. Helm waits for the workloads to become ready, and NICo's init container generates its configuration before it starts. Repeat [verification](#verify) to confirm the updated configuration delivers fresh data. For credential and token changes, follow [Rotation and ownership](credentials.md#rotation-and-ownership).

### Roll back bundled NICo health settings

1. Before editing NICo settings, find the working release revision and its chart version with `helm history`. Set `NICO_GOOD_REVISION` to that revision number and save its values from a private directory. You can also do this after a failed change if the working revision is still in Helm history.

   ```bash
   helm history "$RELEASE" --namespace "$NAMESPACE" --kube-context "$KUBE_CONTEXT"

   NICO_GOOD_REVISION='<working-revision>'
   NICO_VALUES_BACKUP="$(mktemp ./collector-values.before-nico.XXXXXX)" &&
     helm get values "$RELEASE" --revision "$NICO_GOOD_REVISION" --namespace "$NAMESPACE" \
       --kube-context "$KUBE_CONTEXT" --output yaml > "$NICO_VALUES_BACKUP" &&
     printf 'Saved working values in %s\n' "$NICO_VALUES_BACKUP"
   ```

2. If the change causes problems, restore the previous `nicoHardwareHealth` settings in your deployment's values file. Remove any NICo keys you added that were absent in the backup. If you changed only NICo and made no other edits, you can use the backup as the entire `--values` file.
3. Repeat the [Install](#install) command with the restored values and the same chart version, release, namespace, and context. NICo generates its runtime configuration when its pod starts. If you manage Helm values in source control, restore those settings there as well so a later deployment keeps the rollback.
4. Follow [Verify](#verify) to check NICo startup and confirm that fresh health telemetry reaches Fleet Intelligence. Keep the existing BMC credentials and enrollment token Secrets in place.

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

1. Remove the Helm release and its workloads:

   ```bash
   helm uninstall "$RELEASE" \
     --kube-context "$KUBE_CONTEXT" --namespace "$NAMESPACE" --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl --context "$KUBE_CONTEXT" get pods --namespace "$NAMESPACE" \
     -l "app.kubernetes.io/instance=$RELEASE"
   ```

3. Delete unused credential Secrets:

   ```bash
   kubectl --context "$KUBE_CONTEXT" delete secret fleet-intelligence-enrollment-token \
     fleet-intelligence-bmc-credentials --namespace "$NAMESPACE"
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a Secret does not revoke its credentials; retire them through their issuing system. Keep shared resources and separately managed external telemetry collectors.
