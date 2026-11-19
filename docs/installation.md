# Installation

Install the collector on Kubernetes with its Helm chart. By default, the chart runs three workloads: the OOB collector, NICo hardware health, and a telemetry gateway that forwards NICo's metrics and logs to Fleet Intelligence.

For DEB/RPM installations, use the [Linux package guide](host-installation.md).

## Prerequisites

- A Kubernetes cluster you can reach with `kubectl`
- Helm 3 or 4
- Network access from the cluster to your BMCs and to Fleet Intelligence
- The IP and MAC address of each BMC, and its login credentials
- A Fleet Intelligence enrollment token

## Interactive installation

Run the [installer](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/install.sh) directly from GitHub on a machine with Bash 3.2 or later, Helm 3 or 4, and `kubectl`. It uses standard system utilities and needs no Python or `jq`. You need an interactive terminal and access to your cluster and NGC; you can run it without checking out the repository or generating a script:

```bash
curl --fail --silent --show-error \
  https://raw.githubusercontent.com/dsx-ai-factory/fleet-intelligence-oob-collector/main/install.sh | bash
```

The installer opens a Unicode terminal UI by default with NVIDIA green accents, a five-step progress indicator, bordered input fields, choice menus, and a paged review of your settings. It draws a dark background, readable gray helper text, shaded input fields, and a green highlight across the selected menu row. The panel is centered in your terminal. Colors adapt to truecolor, 256-color, and basic ANSI terminals. Rounded borders and Unicode symbols are used regardless of your shell locale. Input stays inside the field as you type, with horizontal scrolling for long values. Use Left/Right, Home/End, Backspace, or Delete to edit. Your terminal controls the font. Use a window at least 80 columns wide and 24 rows high; smaller windows use plain prompts automatically, and `--tui` asks you to enlarge the window. The UI uses the standard `stty` utility to read the terminal size at startup and restore input settings when you leave. If `stty` is unavailable, use the plain prompts. Use arrow keys or `j`/`k` to move through menus, then press Enter to select; pressing an option's number selects it directly. Press Esc in a menu or Ctrl-C at any time to cancel. During review, use `n`/`p` to change pages, or Enter to advance through them. The installer restores your terminal before displaying installation stages and Helm output, or exiting.

To use the plain text prompts, pass `--plain`:

```bash
curl --fail --silent --show-error \
  https://raw.githubusercontent.com/dsx-ai-factory/fleet-intelligence-oob-collector/main/install.sh | bash -s -- --plain
```

With either supported Helm version, the installer uses client-side apply and waits up to 10 minutes for readiness. With Helm 4, it selects `--server-side=false` and `--wait=legacy`.

Use `--ascii` for basic borders and symbols while keeping the terminal UI. `--tui` explicitly requires the terminal UI, and `--unicode` explicitly selects the default Unicode symbols. `--plain` selects text prompts. The installer uses plain prompts with a notice when the terminal UI is unavailable. `NO_COLOR` disables colors without changing the selected symbols. Use `--help` to see the available options.

Answer the questions about your Kubernetes context, namespace, release name, exact published NGC chart version, backend, and BMC targets. Use the compute zone and node group names from your Fleet Intelligence account. For each target, choose compute node, NVSwitch, power shelf, or unspecified device first. The installer then asks for that device's BMC IP and MAC, Redfish port, display name, credentials profile, optional placement overrides, and telemetry labels such as `rack`. These questions fill the [target fields](configuration.md#targets) in your Helm values. Enter one label key and value at a time, then leave the next key blank to finish; reserved and duplicate keys are rejected. Leave the credentials profile blank only when your credentials file supplies a complete per-MAC override. The installer keeps all three workloads and the chart's collection and TLS defaults. It reads the selected chart version's token setting and keeps the Secret name and key you choose. For proxies, registry mirrors, or other advanced settings, follow the manual steps below with your own [configuration](configuration.md).

For each Secret, choose either an existing Kubernetes Secret or a private input file. Existing Secrets work with manually provisioned Secrets and Secrets synced by Vault or OpenBao. For file inputs, prepare the [credentials JSON and bootstrap token files](#create-the-secrets) and enter their absolute paths on the machine running the installer. The installer creates missing Secrets from those files and preserves existing Secrets. It checks that each referenced key is nonempty. See [Rotation and ownership](credentials.md#rotation-and-ownership) to update credentials or tokens later.

Review the settings displayed by the installer and answer `yes` to install. It checks cluster access and renders the pinned chart before creating resources, then installs and waits up to 10 minutes for workloads to become ready. Answers are read from your terminal independently of the downloaded script. Passwords and tokens remain in your files or Kubernetes Secrets. Temporary values are removed when the installer exits; use `helm get values` with your release, namespace, and context to retrieve them for future updates.

A failed run leaves created cluster resources in place for diagnosis; after resolving the error, you can run the installer again. Concurrent Helm operations on the same release can fail; wait for the active operation to finish before retrying. To run an installer from a reviewed revision, replace `main` in the download URL with its full commit SHA.

Finish with [Verify](#verify) to check collection and telemetry. Running pods alone do not establish that uploads work. To install by hand instead, follow the steps below.

## Choose a namespace

The commands on this page install into the namespace in `$NAMESPACE`. Set it before you start, and set it again if you continue in a new terminal. If it is empty, `kubectl` and `helm` use your current context's default namespace instead.

```bash
export NAMESPACE=fleet-intelligence
```

## Configure targets and Secrets

You give the chart two things: your target list, in a values file, and two Kubernetes Secrets holding the BMC credentials file and the enrollment token. The chart never stores passwords; it only references the Secrets.

The steps below create the Secrets by hand. To sync them from Vault or OpenBao instead, follow [Credentials](credentials.md) and then continue at [Save your values file](#save-your-values-file).

### Create the Secrets

Copy [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json) to a private directory, for example `/secure/path/to/credentials.json`, set its mode to `0600`, and fill in your BMC usernames and passwords. [Configuration](configuration.md#bmc-credentials-file) explains the file format.

Get the enrollment token from your Fleet Intelligence administrator. Save it to `/secure/path/to/enrollment-token` with nothing else in the file, not even a trailing newline, and set its mode to `0600`.

Create the Secrets from those files:

```bash
kubectl create namespace "$NAMESPACE" \
  --dry-run=client -o yaml | kubectl apply -f -

kubectl create secret generic fleet-intelligence-enrollment-token \
  --namespace "$NAMESPACE" \
  --from-file=enrollment-token=/secure/path/to/enrollment-token

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
  enrollmentToken:
    existingSecret: fleet-intelligence-enrollment-token
    secretKey: enrollment-token
```

`backend.url` defaults to the production endpoint, `https://data.fleet-intelligence.nvidia.com`. Set it in your values file when using a different Fleet Intelligence endpoint.

## Install

Choose a published chart version from the [NGC chart listing](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/fleet-intelligence/helm-charts/fleet-intelligence-oob-collector) and replace `<chart-version>` below:

```bash
export OOB_CHART_VERSION='<chart-version>'

helm upgrade --install fleet-intelligence-oob-collector \
  fleet-intelligence-oob-collector \
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
helm status fleet-intelligence-oob-collector \
  --namespace "$NAMESPACE"

kubectl get pods \
  --namespace "$NAMESPACE" \
  -l app.kubernetes.io/instance=fleet-intelligence-oob-collector

kubectl logs \
  --namespace "$NAMESPACE" \
  deployment/fleet-intelligence-oob-collector

kubectl logs --namespace "$NAMESPACE" \
  deployment/nico-hardware-health --container generate-nico-config

kubectl logs --namespace "$NAMESPACE" \
  deployment/fleet-intelligence-oob-collector-otel-gateway
```

Use your release's Deployment names, and skip any component you disabled.

Running pods don't prove everything works. Check that:

- NICo's `generate-nico-config` init container completed successfully.
- The collector logs show inventory uploads succeeding.
- The gateway logs show no enrollment or export errors.

At the default `info` level, look for `backend enrollment succeeded` and `first inventory upload accepted in this cycle`. Per-BMC problems are logged at `warn` with `bmc_mac`, `bmc_address`, and the failure reason, before any retry delay. Set the collector log level to `debug` to see healthy per-BMC results.

If something fails:

| Problem | What to check |
| --- | --- |
| A BMC rejects the login | The target's `credentialsProfile`, any override for its MAC, and the account on the BMC itself |
| The collector or gateway can't enroll | That the enrollment token is valid and `backend.url` is correct |
| The gateway can't export telemetry | That the backend is reachable from the cluster and its TLS certificate is trusted |

To change settings later, edit your values file and run the same `helm upgrade --install` command. See [Rotation and ownership](credentials.md#rotation-and-ownership) for which changes take effect on their own and which need a restart.

## Disconnect an individual machine

Before you delete a machine in Fleet Intelligence (the Delete Machine flow), remove it from every collector that monitors it. Otherwise the next collection cycle reports it again. Data that was already uploaded stays in Fleet Intelligence.

If your values file is generated by configuration management, make the change there too, so the machine doesn't come back on the next deploy. Substitute your own resource names in the commands below.

1. In your values file, find the target with the machine's BMC `mac` under `inventory.targets` and delete it. If it's the last target, [uninstall the collector](#uninstall-the-oob-collector) instead, because the target list can't be empty.
2. Run the same `helm upgrade --install` command with the updated file. The collector and NICo restart with the new target list.
3. Wait for both to finish restarting:

   ```bash
   kubectl rollout status deployment/fleet-intelligence-oob-collector \
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
   helm uninstall fleet-intelligence-oob-collector \
     --namespace "$NAMESPACE" --wait --timeout 10m
   ```

2. Verify that no pods remain for the release:

   ```bash
   kubectl get pods --namespace "$NAMESPACE" \
     -l app.kubernetes.io/instance=fleet-intelligence-oob-collector
   ```

3. Delete unused credential Secrets:

   ```bash
   kubectl delete secret fleet-intelligence-enrollment-token \
     fleet-intelligence-bmc-credentials --namespace "$NAMESPACE"
   ```

Also remove any other unused Secrets dedicated to this deployment. Deleting a Secret does not revoke its credentials; retire them through their issuing system. Keep shared resources and separately managed external telemetry collectors.
