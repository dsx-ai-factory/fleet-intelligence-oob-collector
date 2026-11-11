# Integrate FI while preserving an existing NICo configuration

Use this procedure when NICo hardware health already runs in the customer's Kubernetes environment and its endpoint sources, custom plugins, collectors, or telemetry destinations must remain configured. Fleet Intelligence (FI) deploys the OOB collector and authenticated telemetry gateway, then adds its required settings to the existing NICo TOML instead of replacing that TOML.

This procedure requires the existing NICo Deployment and the FI Helm release to run in the same namespace.

## Assumptions

This integration assumes:

- GMI and FI run in the same namespace, and FI may patch and restart the NICo Deployment.
- NICo uses a volume-mounted TOML whose path appears exactly once in the selected container command or arguments.
- The FI-qualified image supports the existing TOML and includes every required GMI plugin.
- NICo endpoints provide stable BMC identities; FI OOB targets remain separately configured under `inventory.targets`.

## Consequences

- GMI keeps ownership, but FI patches and restarts NICo with the qualified image and generated config path.
- The original TOML remains unchanged; FI generates an effective TOML that preserves customer settings and overrides only FI-owned fields.
- Existing exporters remain active, and FI adds its OTLP destination.
- Configuration updates or GMI reconciliation may require a restart, Helm upgrade, or integration-command rerun.
- NICo and OOB target lists remain independent and can drift.

## How the configuration overlay works

On every pod start, Kubernetes runs the existing GMI init containers first. The FI init container then reads two inputs:

- The customer-owned NICo TOML mounted from the existing Deployment.
- The FI collector TOML rendered by Helm from FI values.

The `render-nico-config --base-config` command parses both files as structured TOML. It does not append text. It starts with the complete customer TOML, applies the FI-owned fields, validates table and array types, and writes `/etc/nico-health/config.toml` to the private generated-config volume. NICo then starts with that generated path.

FI overlays:

- Known collector controls represented by `nicoHardwareHealth.config`.
- `attributes.gpu_identity`.
- `processors.resource_status` and the firmware collection settings it requires.
- `sinks.otlp.enabled`.
- One FI OTLP target with diagnostics, alert details, batching, and flush settings.
- FI placement and identity labels on matching static endpoints, using normalized BMC MAC as the key.

FI preserves:

- The complete `[endpoint_sources]` section, including SQL and custom plugin fields ([https://github.com/dsx-ai-factory/infra-controller/pull/8098](https://github.com/dsx-ai-factory/infra-controller/pull/8098)).
- Existing static endpoint IPs, ports, credential profiles, and customer labels.
- Unknown customer collector and plugin fields.
- Existing processors and attributes outside the FI-owned paths.
- Existing sinks and OTLP targets.
- Customer credential, database, and Secret-file references.
- Unrelated top-level fields.

The merge fails instead of silently replacing data when the base TOML is invalid, an FI-owned path has an incompatible type, the FI endpoint is duplicated, the base config mount is ambiguous, or an FI-reserved workload name collides with a different customer resource.

## Telemetry destination behavior

External integration does not remove existing NICo exporters. It preserves all customer sinks and OTLP targets and adds one target for the FI gateway.

- If NICo had no existing OTLP target, the FI gateway becomes its only OTLP destination.
- If NICo already exported elsewhere, it exports to both the existing destination and the FI gateway.
- If the existing config already contains the exact FI gateway endpoint, the merge updates that target with FI-required settings instead of adding a duplicate.
- If an existing customer collector also forwards the same NICo records to FI, GMI and FI must remove one route to prevent duplicate records.

Independent delivery and backpressure behavior for multiple customer targets must be tested with the exact GMI NICo image.

## Target ownership and GMI plugins

FI OOB inventory and attestation still require `inventory.targets` in the OOB values. External integration does not copy target rows from NICo into the OOB collector.

If GMI's plugin writes static endpoints into the mounted TOML before the FI init container runs, the merge can read and preserve those NICo endpoints. It can also add FI labels when a static endpoint MAC matches an OOB `inventory.targets` MAC. The OOB collector still uses its own target entry.

If GMI's plugin or SQL source discovers endpoints dynamically after NICo starts, the resolved target list is not present in the TOML. The current NICo change provides source configuration and a query, not a target-list API, so the OOB collector cannot read those resolved targets from NICo config.

A future integration could remove manual duplication only if GMI exposes a stable shared target source containing at least BMC IP, real BMC MAC, target kind, credential profile, machine name, and placement labels. FI would need a new, explicitly supported reader for that source, or both products would need to generate their configurations from the same authoritative inventory. That capability is not implemented by this option.

## Requirements

Before starting, provide:

- A shared Kubernetes namespace for the existing NICo Deployment and FI release.
- The existing NICo Deployment name, container name, and absolute TOML path used by that container.
- An existing TOML file backed by a Kubernetes volume mount. An inline environment-only configuration is not supported.
- The exact current NICo image and an FI-qualified target image that includes every required GMI plugin.
- Permission to read and patch the existing NICo Deployment and install the FI Helm release.
- Helm 3, `kubectl`, and Python 3 on the operator workstation.
- Network access from NICo to the BMCs and from the FI gateway to Fleet Intelligence.
- One FI inventory target for each BMC used for OOB inventory and attestation, including its real BMC MAC.
- A rollback source of truth for the original GMI Deployment.

Dynamic NICo endpoint sources must emit a valid BMC MAC and the required FI labels. For static endpoints, the merge command adds FI labels by matching normalized BMC MACs.

## 1. Discover the existing NICo inputs

**Purpose:** Identify the exact customer-owned Deployment, container, config path, source image, and qualified target image so the integration cannot patch the wrong workload or config.

**How:** Inspect the existing Deployment and record the values used by the Helm release and integration command.

Set the shared namespace and list the Deployment containers and images:

```bash
export NAMESPACE='<shared-namespace>'

kubectl get deployments \
  --namespace "$NAMESPACE" \
  -o custom-columns='DEPLOYMENT:.metadata.name,CONTAINERS:.spec.template.spec.containers[*].name,IMAGES:.spec.template.spec.containers[*].image'
```

Set the selected Deployment and container:

```bash
export FI_RELEASE='fleet-intelligence-oob-collector'
export NICO_DEPLOYMENT='<existing-nico-deployment>'
export NICO_CONTAINER='<nico-container-name>'
export FI_QUALIFIED_NICO_REVISION='844ae0766b6070ab69b3ca744cb87a72a3b0209a'
```

Display the selected container's command, arguments, and mounts:

```bash
kubectl get deployment "$NICO_DEPLOYMENT" \
  --namespace "$NAMESPACE" \
  -o jsonpath="{range .spec.template.spec.containers[?(@.name=='$NICO_CONTAINER')]}command={.command}{'\n'}args={.args}{'\n'}{range .volumeMounts[*]}mount={.name}:{.mountPath} subPath={.subPath}{'\n'}{end}{end}"
```

Find the absolute TOML path in the command or arguments and confirm that one listed volume mount covers that path:

```bash
export NICO_CONFIG_PATH='<absolute-existing-config-path>'
```

Record the exact current image:

```bash
export EXPECTED_SOURCE_NICO_IMAGE="$(
  kubectl get deployment "$NICO_DEPLOYMENT" \
    --namespace "$NAMESPACE" \
    -o jsonpath="{.spec.template.spec.containers[?(@.name=='$NICO_CONTAINER')].image}"
)"
test -n "$EXPECTED_SOURCE_NICO_IMAGE"
printf 'Expected source NICo image: %s\n' "$EXPECTED_SOURCE_NICO_IMAGE"
```

Record the FI-qualified target image. For a GMI plugin build, use the reviewed image containing both the plugin and the FI-qualified changes:

```bash
export FI_QUALIFIED_NICO_REPOSITORY='<qualified-image-repository>'
export FI_QUALIFIED_NICO_TAG='<explicit-qualified-tag>'
```

Do not use `latest` or an unreviewed mutable tag.

**Effect:** This step is read-only. It records the fail-closed image and config-path contract used by later steps.

## 2. Create the FI Secrets

**Purpose:** Provide FI enrollment and OOB BMC credentials without placing credential values in Helm values, ConfigMaps, generated TOML, or the repository.

**How:** Create or verify the two named Secrets in the shared namespace while preserving any separate GMI credential mounts used by NICo.

Create the bootstrap-token and FI BMC-credential Secrets in the shared namespace. Keep source files outside the repository:

```bash
kubectl create secret generic fleet-intelligence-oob-collector-agent-token \
  --namespace "$NAMESPACE" \
  --from-file=agent-token=/secure/path/to/agent-token

kubectl create secret generic fleet-intelligence-bmc-credentials \
  --namespace "$NAMESPACE" \
  --from-file=credentials.json=/secure/path/to/credentials.json
```

The FI credential Secret is used to validate the FI inventory configuration. Preserve the GMI credential mounts referenced by the existing NICo endpoint sources.

**Effect:** FI workloads can enroll and resolve OOB credentials. Existing NICo credential sources remain unchanged.

## 3. Configure external NICo

**Purpose:** Tell the chart to deploy FI inventory, attestation, and gateway components while integrating the existing NICo instead of creating another NICo workload.

**How:** Create a private values file that references Secrets, defines OOB targets, selects `deploymentMode: external`, verifies the current image, and selects the qualified target image.

Create a private Helm values file:

```yaml
inventory:
  computeZone: <compute-zone>
  nodeGroup: <node-group>
  targets:
    - kind: compute
      name: <machine-name>
      address: <bmc-ip>
      mac: "<bmc-mac>"
      credentialsProfile: bmc

credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json

backend:
  url: https://data.fleet-intelligence.nvidia.com
  agentToken:
    existingSecret: fleet-intelligence-oob-collector-agent-token
    secretKey: agent-token

nicoHardwareHealth:
  enabled: true
  deploymentMode: external
  external:
    expectedImage: <EXPECTED_SOURCE_NICO_IMAGE>
  image:
    repository: <FI_QUALIFIED_NICO_REPOSITORY>
    tag: <FI_QUALIFIED_NICO_TAG>
    pullPolicy: IfNotPresent

otelGateway:
  enabled: true
```

Replace `backend.url` with the assigned FI environment. Do not put BMC, database, plugin, or bootstrap credentials in Helm values.

**Effect:** The values define the FI integration contract. `inventory.targets` drives OOB inventory and attestation; NICo continues using its preserved endpoint sources.

## 4. Install FI without installing another NICo

**Purpose:** Install the FI collector, authenticated OTEL gateway, and external-NICo contract without creating a competing NICo Deployment or Service.

**How:** Install or upgrade the chart with the private values file, then list release-owned resources to verify the rendered topology.

Install or upgrade the FI release:

```bash
helm upgrade --install "$FI_RELEASE" \
  fleet-intelligence-oob-collector \
  --repo https://helm.ngc.nvidia.com/nvidia/fleet-intelligence \
  --version '<chart-version>' \
  --namespace "$NAMESPACE" \
  --values /secure/path/to/values.yaml
```

Verify that FI created the collector, gateway, and integration contract but did not create a second NICo Deployment:

```bash
kubectl get deployment,configmap \
  --namespace "$NAMESPACE" \
  -l app.kubernetes.io/instance="$FI_RELEASE"
```

**Effect:** FI resources and the merge contract exist, but the customer NICo Deployment is not changed until step 5.

## 5. Validate and apply the integration

**Purpose:** Validate the proposed pod-template mutation before changing NICo, then apply it and wait for a healthy rollout.

**How:** Run the integration command first with `--dry-run`, review any validation failure, and rerun without `--dry-run` only after validation succeeds.

From a checkout of the matching OOB collector release, ask the Kubernetes API server to validate the Deployment patch:

```bash
python3 scripts/integrate_external_nico.py \
  --namespace "$NAMESPACE" \
  --release "$FI_RELEASE" \
  --deployment "$NICO_DEPLOYMENT" \
  --container "$NICO_CONTAINER" \
  --existing-config-path "$NICO_CONFIG_PATH" \
  --confirm-qualified-revision "$FI_QUALIFIED_NICO_REVISION" \
  --dry-run
```

Apply the patch and wait for NICo:

```bash
python3 scripts/integrate_external_nico.py \
  --namespace "$NAMESPACE" \
  --release "$FI_RELEASE" \
  --deployment "$NICO_DEPLOYMENT" \
  --container "$NICO_CONTAINER" \
  --existing-config-path "$NICO_CONFIG_PATH" \
  --confirm-qualified-revision "$FI_QUALIFIED_NICO_REVISION"
```

The command requires the original path on every run. It records that path and refuses a different path on a later run.

**Effect:** The existing Deployment keeps its customer config source and receives the FI merge init container, generated-config volume, qualified image, gateway label, and validated config-path change. NICo restarts; the original customer TOML remains unchanged.

## 6. Verify configuration preservation and telemetry

**Purpose:** Prove that GMI configuration remains active and that FI inventory, attestation, metrics, and logs use the expected BMC identity.

**How:** Inspect init-container ordering and NICo arguments, review merge and startup logs, verify the existing GMI telemetry route, and check FI records and labels.

Confirm that customer init containers run before the FI merge and that the selected container uses the generated file:

```bash
kubectl get deployment "$NICO_DEPLOYMENT" \
  --namespace "$NAMESPACE" \
  -o jsonpath="{range .spec.template.spec.initContainers[*]}init={.name}{'\n'}{end}{range .spec.template.spec.containers[?(@.name=='$NICO_CONTAINER')]}args={.args}{'\n'}{end}"
```

Check the merge and NICo startup logs without printing the effective configuration:

```bash
kubectl logs deployment/"$NICO_DEPLOYMENT" \
  --namespace "$NAMESPACE" \
  --container fleet-intelligence-nico-config

kubectl logs deployment/"$NICO_DEPLOYMENT" \
  --namespace "$NAMESPACE" \
  --container "$NICO_CONTAINER"
```

Confirm that existing GMI telemetry continues to arrive. In Fleet Intelligence, verify inventory, attestation, metrics, and logs by configured BMC MAC and confirm `oob="true"`, `machine_name`, `compute_zone`, and `node_group`.

**Effect:** Successful checks establish that both customer-owned behavior and the FI telemetry path are active. Treat missing customer settings, duplicate records, identity mismatches, or export failures as a failed integration.

## Apply later changes

**Purpose:** Keep the effective NICo config synchronized with either the customer's source configuration or FI's integration contract.

**How:** Restart NICo after customer-config changes; after FI Helm-value changes, run `helm upgrade` and rerun the integration command.

Update the original GMI configuration source, then restart NICo so the init containers regenerate the effective TOML. After changing FI targets, collection controls, proxy settings, or the FI gateway endpoint, run `helm upgrade` and rerun the integration command.

Run the integration command again after GMI reapplies its original Deployment because that reconciliation can remove the FI patch.

**Effect:** The next pod start rebuilds the effective TOML from the latest customer source and FI overlay. GMI's source file remains authoritative for customer-owned fields.

## Roll back

**Purpose:** Remove FI's pod-template mutation and return NICo to the GMI-owned image, config path, mounts, and lifecycle.

**How:** Restore the Deployment from GMI's source of truth before disabling or removing the FI gateway.

Restore the existing NICo Deployment from GMI's Helm release, Operator, or manifest source of truth. This removes the FI init container, generated-config mount, image change, process argument, labels, and annotations while restoring the original GMI configuration path.

Then disable NICo integration and the FI gateway, or uninstall the FI release if OOB inventory and attestation are no longer required. Do not remove the gateway while the merged NICo configuration is still active.

**Effect:** NICo returns to GMI-only operation. Removing the gateway first would leave the active generated configuration pointing to a missing destination.