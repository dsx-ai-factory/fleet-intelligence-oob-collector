# External NICo integration options for GMI

## Status

This document is a design proposal for integrating Fleet Intelligence (FI) OOB collection into a GMI environment that already runs NICo hardware health. The existing GMI NICo version, configuration, ownership model, namespace, and telemetry route are unknown, so this document defines supported assumptions and ranks integration options by the amount of FI-side engineering required.

The preferred approach is to make the smallest FI change and place the environment-specific work with GMI. More flexible options remain possible, but each additional ownership, compatibility, namespace, or telemetry boundary adds implementation and validation work.

## Goals

- Deploy the FI OOB inventory and attestation collector without deploying a second NICo instance.
- Preserve one authoritative NICo instance for each BMC.
- Send the required NICo metrics and logs to Fleet Intelligence.
- Preserve BMC identity by using the configured BMC MAC throughout inventory, attestation, metrics, and logs.
- Keep BMC credentials in a Kubernetes Secret and out of Helm values, generated configuration, logs, and telemetry.
- Avoid taking Helm ownership of a GMI-managed Deployment, ConfigMap, Secret, Service, or service account.
- Define a separately testable contract for every supported integration option.

## Non-goals

- FI workloads will not mutate the GMI NICo Deployment through a hook, privileged Job, or Kubernetes API access. An operator-run integration command may patch the named Deployment with the operator's existing credentials.
- The FI chart will not install a second NICo instance beside the GMI instance.
- FI will not guarantee compatibility with an unknown NICo build or configuration schema.
- FI preserves existing GMI telemetry destinations and adds one FI destination.

## Current chart constraints

The chart currently treats `nicoHardwareHealth.enabled` as both workload ownership and health-telemetry enablement. Setting it to `false` prevents the NICo Deployment from being created, but the chart also rejects an enabled FI OTEL gateway and reports NICo as disabled.

The FI OTEL gateway NetworkPolicy currently accepts OTLP traffic only from pods in the same namespace whose `app.kubernetes.io/instance` label matches the FI Helm release. A GMI NICo pod in another namespace cannot use that gateway, and a same-namespace pod must carry the expected label.

The existing `bmc-collector render-nico-config` command generates the FI-required NICo configuration from the shared target and credential references. It configures static Redfish endpoints, required identity labels, GPU identity attributes, resource-status processing, metrics, logs, diagnostic details, alert details, and an OTLP/gRPC destination. Reusing this renderer minimizes configuration drift.

## Common FI requirements

Every option that claims full FI health integration must meet these requirements:

1. GMI starts with the supported official NICo release. For Option 1, that baseline is `v2.3.4`.
2. GMI remains the owner of its NICo workload and permits the FI integration command to replace the selected container image with the FI-qualified build and apply the agreed init container, volume, Secret, label, process argument, and restart changes.
3. NICo receives the FI target identity for every BMC, including the configured BMC MAC. IP address alone is not sufficient.
4. NICo adds `oob="true"` and the configured `machine_name`, `compute_zone`, `node_group`, and custom labels to telemetry.
5. NICo can read the shared BMC credentials file from a read-only Secret mount without copying credential values into its configuration.
6. NICo exports OTLP/gRPC metrics and logs. FI does not currently require a traces pipeline.
7. NICo enables the FI-required GPU identity attributes, resource-status logs, diagnostic details, and health-alert details.
8. GMI identifies any existing NICo OTLP destinations and removes any duplicate route that already forwards the same records to FI.
9. The combined FI collector and NICo BMC request load must remain within the customer's BMC capacity and account-lockout policy.
10. GMI provides a non-production environment or representative deployment in which the selected option can be validated before production rollout.

## Option 1: Same namespace, preserve GMI configuration, FI gateway

This is the recommended and lowest-change supported integration. FI merges and validates its required settings without taking ownership of GMI endpoint sources, plugins, or telemetry destinations.

### Assumptions imposed on GMI

- GMI deploys FI OOB in the same namespace as its existing NICo.
- The existing NICo process reads one TOML file from a Kubernetes volume mount, and GMI provides that absolute path to the integration command.
- GMI permits the integration command to mount the existing configuration into an FI init container, write a merged configuration to a private volume, and restart the named Deployment.
- The selected target image contains both the GMI custom plugin and the FI-qualified telemetry changes. Preserving configuration cannot preserve plugin code that is absent from the replacement image.
- GMI accepts the documented FI ownership boundary for required collection, processing, identity, and OTLP fields.

### Configuration ownership

GMI continues to own and may update:

- All endpoint sources, including static, NICo API, cluster, SQL, and custom plugin settings.
- Customer-specific collectors and unknown plugin fields.
- Existing telemetry sinks and OTLP targets.
- Existing BMC, database, and plugin credential references.
- Unrelated root-level NICo settings.

FI overlays and owns:

- Known collector settings represented by `nicoHardwareHealth.config`.
- GPU identity attributes.
- Resource-status processing and its required firmware collection settings.
- One OTLP target for the FI gateway, including diagnostics, alert details, batching, and flush behavior.
- FI labels added to matching static endpoints by normalized BMC MAC.

The merge preserves unknown tables and fields. When GMI and FI set the same FI-owned field, the FI value wins. The command rejects invalid table types, duplicate FI gateway targets, an inaccessible base configuration, and an unqualified image transition rather than silently dropping settings.

For static endpoints, FI merges labels into the existing row without replacing its IP, port, credential profile, or customer labels. Dynamic endpoint sources must emit the configured BMC MAC and FI-required labels because their resolved targets do not exist in the startup TOML.

The generated TOML preserves configuration values but not comments or source formatting. GMI's original ConfigMap, Secret, or generated file remains the editable source of truth.

### FI-side changes

1. Add an external deployment mode that always preserves the existing NICo configuration.
2. Extend `render-nico-config` to accept a customer-owned base TOML and perform a structured, fail-closed merge.
3. Discover the existing config volume from the selected NICo container and mount it read-only into the FI config init container.
4. Preserve GMI init containers so a customer config generator runs before the FI merge.
5. Keep the existing command, unrelated arguments, config volume, and runtime mounts while switching only the validated config-path reference to the generated merged file; reject collisions with FI-reserved resource names.
6. Make repeated integration runs reuse and validate the recorded base-config path.
7. Add unit, Helm, and live rollout tests proving preservation and idempotence.

### Validation

- Start a pre-existing NICo with custom root settings, endpoint-source fields, collector fields, plugin fields, and an existing OTLP target.
- Apply the FI integration and verify that every customer-owned value remains in the effective configuration.
- Verify that FI-owned conflicting values are changed to the required values and that the FI gateway target is added exactly once.
- Verify that matching static targets receive FI labels without changing their customer-owned connection or credential fields.
- Verify that customer config-generating init containers remain ordered before the FI merge init container.
- Change the customer ConfigMap, restart NICo, and verify that the next merge incorporates the change.
- Run the integration command twice and verify an identical Deployment contract and one FI telemetry stream.
- Verify rollback from GMI's original Deployment source of truth.

### Local POC result

The preserve-mode POC passed in namespace `fleet-intelligence-dev1-external-nico-poc`. A pre-existing NICo started from a customer ConfigMap with its own static endpoint, credential reference, `customer_owner` label, periodic log mode, tracing and Prometheus sinks, and `gmi_preserved` metrics prefix.

After integration, NICo retained the endpoint address, MAC, credential profile, customer label, sinks, environment variable, and metrics prefix. FI added its placement labels, changed FI-owned collection fields, enabled GPU identity and resource-status processing, and added one OTLP target. NICo discovered the BMC and connected both metrics and logs to the FI gateway. Gateway counters confirmed that accepted OTLP/gRPC log records and metric points were exported successfully to the dev1 FI backend.

A second integration run left the Deployment generation unchanged. Updating the customer ConfigMap and restarting the Deployment preserved the new `customer_owner` value and metrics prefix in the next merged configuration.

This POC did not validate the draft SQL source, a compiled GMI plugin, a customer config-generating init container, or multiple pre-existing OTLP targets. Unit tests cover preservation of unknown plugin tables, existing OTLP targets, customer init-container ordering, static label enrichment, malformed input rejection, and repeated application; the GMI-specific cases still require their image and representative configuration.

## Option 1A investigation: preserve GMI's SQL inventory source for NICo telemetry targets

This option investigates GMI's draft [SQL inventory and configurable endpoint identity change](https://github.com/dsx-ai-factory/infra-controller/pull/8098). It applies only to the targets used by NICo health telemetry. The FI OOB collector continues to use `inventory.targets` for inventory and attestation and does not read resolved targets from NICo.

### What the draft change provides

The draft adds `[endpoint_sources.sql]` to NICo hardware health. NICo runs a configured PostgreSQL query on each endpoint-discovery cycle and creates its Redfish endpoints from the result. The NICo config contains the database URL file, query, credential profile, timeout, default port, and identity preference; it does not contain the materialized target rows.

The query requires a text `bmc_ip` column. It can also return `hostname`, `bmc_mac`, `bmc_port`, `rack_id`, and `serial`; every other text column becomes a telemetry label. A shared credentials file supplies one profile for the SQL source, with per-MAC overrides when a row includes `bmc_mac`.

The draft models every SQL row as a machine endpoint. It does not provide SQL columns for NICo's switch-host or power-shelf endpoint metadata, so those endpoint roles require separate design work if GMI needs them.

The draft also adds `endpoint_sources.identity`, with `hostname`, `mac`, and `ip` in preference order. The default `["mac", "ip"]` preserves NICo's current MAC-first endpoint identity, but the SQL source permits rows without a MAC.

The change is still a draft, has not been deployed, and is not part of an official NICo release. The investigation must pin the reviewed commit or later merged revision rather than assume the current draft interface is stable.

### Preliminary feasibility

NICo can use the SQL source instead of the static endpoints generated from the FI OOB collector file. This removes the need to duplicate GMI's telemetry target inventory under `inventory.targets` solely for NICo.

The OOB collector cannot obtain its inventory and attestation targets from this NICo config. The config contains a query, not its results, and the draft does not add a target-list API or export. OOB `inventory.targets` therefore remains an independent required configuration.

Option 1 provides the required preservation and overlay mechanism; Option 1A adds SQL-specific identity, label, and reconciliation requirements.

### FI-owned overlay

The integration should preserve GMI's SQL source, database URL Secret mount, query, credential profile, endpoint identity order, and any custom plugin settings. FI should own only the settings required to send compatible telemetry:

- The FI OTLP/gRPC gateway target, batching, diagnostics, and alert details.
- GPU identity attributes and resource-status processing.
- Agreed collector controls that are necessary for the FI telemetry contract.
- The gateway NetworkPolicy source label and rollout checksum.

The overlay must not enable static endpoints or the NICo API beside the SQL source unless duplicate-source behavior has been explicitly tested.

### Hard requirements

- The SQL query must return a valid, unique `bmc_mac` for every target sent to FI, even though the draft makes `bmc_mac` optional. FI correlates telemetry with OOB inventory by BMC MAC.
- `endpoint_sources.identity` must remain MAC-first for FI targets. Hostname-first identity may be evaluated separately, but it does not remove the required `endpoint_mac` telemetry attribute.
- The query must return the FI correlation labels, including `oob`, `machine_name`, `compute_zone`, and `node_group` when applicable, or the integration must add them without replacing GMI labels.
- SQL label names and values must satisfy NICo and FI reserved-name, count, and length rules.
- The database URL must remain in a mounted Secret file. It must not appear in Helm values, a ConfigMap, logs, or the FI integration contract.
- The SQL credential profile and per-MAC overrides must resolve through the shared BMC credentials file without copying credentials into NICo TOML.
- The set of SQL MACs used for telemetry must be compared with OOB `inventory.targets`; missing, duplicate, and unexpected MACs must fail validation or produce an explicit reconciliation report.
- FI support must pin a merged and qualified NICo revision. A draft GMI commit is suitable for a POC only.

### Investigation and validation

1. Build a POC NICo image from the draft commit `b01f82ca0c7d3fc633dfa994bf5a2f4c94d7e7c8` or a newer GMI-provided revision.
2. Obtain a redacted SQL query and schema fixture that returns representative GMI compute BMC targets.
3. Start NICo with only the SQL endpoint source and verify that additions, removals, IP changes, MAC changes, invalid rows, duplicate addresses, and database outages reconcile predictably.
4. Verify credential profile lookup, per-MAC overrides, credential rotation, and database URL rotation without exposing either credential source.
5. Implement a configuration overlay that preserves unknown GMI/plugin fields and changes only the agreed FI telemetry sections.
6. Compare the SQL-discovered MAC set with OOB `inventory.targets` and verify that both paths produce the same FI machine identity.
7. Send metrics, BMC logs, resource-status logs, GPU identity, diagnostics, and alert details through the FI gateway and run the same telemetry conformance checks as Option 1.
8. Confirm that the SQL source is the only active NICo endpoint source and that one database row produces one telemetry stream.

### Decision

This option is viable in principle for NICo's telemetry targets, and it is likely the best fit if GMI's custom plugin is the SQL source proposed by the draft. It does not eliminate the OOB target file and cannot be treated as supported until the draft is merged, its configuration contract is stable, and the FI overlay and MAC reconciliation tests pass.

## Option 2: Different namespaces, FI gateway, FI-qualified NICo

This option preserves the FI-owned telemetry gateway but allows GMI to keep NICo in its existing namespace.

### Assumptions imposed on GMI

- GMI upgrades or pins NICo to the FI-qualified image or commit.
- GMI permits traffic from its NICo namespace to the FI namespace on OTLP/gRPC port 4317.
- GMI assigns stable namespace and pod labels that the FI gateway NetworkPolicy can select.
- GMI configures NICo with the fully qualified gateway address, such as `http://<service>.<fi-namespace>.svc.cluster.local:4317`.
- GMI provides NICo access to the FI target configuration and BMC credentials without copying secrets into Helm values.

### Additional FI-side changes

In addition to Option 1:

1. Add explicit gateway ingress values for a namespace selector and pod selector.
2. Render a NetworkPolicy rule that requires both selectors and does not expose the gateway to the entire cluster.
3. Document the fully qualified cross-namespace Service endpoint.
4. Validate failure messages for missing selectors and unsafe broad selectors.

### Validation

Run all Option 1 tests with NICo and FI in separate namespaces, then verify DNS resolution, NetworkPolicy enforcement, any cluster egress policy, and behavior when either namespace or pod labels change.

## Option 3: Same namespace, FI gateway, existing GMI NICo version

This option avoids a GMI NICo upgrade but transfers compatibility work to FI. It should only be selected when GMI cannot run the FI-qualified NICo build.

### Assumptions imposed on GMI

- GMI deploys FI in the same namespace as NICo.
- GMI provides the exact NICo image digest or source commit and a redacted copy of the active configuration.
- GMI permits changes to its NICo configuration, Secret mounts, pod labels, and restart procedure.
- The existing NICo version supports OTLP/gRPC metrics and logs and exposes enough endpoint identity to satisfy FI correlation requirements.

### Additional FI-side changes

In addition to Option 1:

1. Analyze the existing NICo configuration schema and telemetry output.
2. Add a version-specific configuration adapter or documented patch if the current renderer is incompatible.
3. Determine whether the FI settings can be merged without deleting GMI collectors, endpoint sources, or telemetry targets.
4. Add compatibility fixtures and tests for every GMI NICo version that FI agrees to support.

FI should reject this option if the existing version cannot emit the required identity, logs, diagnostics, or alert details. Silent degradation is not acceptable.

### Validation

Run the Option 1 test suite against the exact GMI image digest. Compare its merged configuration and OTLP output with the FI-qualified NICo baseline, and explicitly record any unsupported telemetry.

## Option 4: Different namespaces, FI gateway, existing GMI NICo version

This option combines the compatibility work of Option 3 with the cross-namespace work of Option 2 and is therefore more expensive than either independently.

### Assumptions imposed on GMI

GMI must satisfy all assumptions from Options 2 and 3.

### FI-side changes

FI must implement all changes from Options 1, 2, and 3.

### Validation

Run the full Option 2 cross-namespace suite against the exact GMI NICo version and configuration, including compatibility comparisons with the FI-qualified baseline.

## Option 5: GMI NICo dual-exports to its existing destination and the FI gateway

This option is useful when GMI must preserve its existing telemetry path while adding FI. It can be combined with either same-namespace or cross-namespace placement, but it adds telemetry duplication and backpressure considerations.

### Assumptions imposed on GMI

- The selected NICo version supports multiple OTLP targets with independent failure handling.
- GMI accepts the additional network, memory, CPU, and telemetry volume.
- GMI defines which destination is authoritative for operational monitoring.
- Failure or backpressure on the GMI destination must not block FI delivery, and failure on the FI destination must not block the GMI destination.

### Additional FI-side changes

1. Extend or adapt configuration generation so the FI OTLP target can be added without replacing GMI targets.
2. Preserve unknown customer settings rather than regenerating the entire customer configuration.
3. Document duplicate-record behavior if the GMI telemetry path also forwards to FI.

### Validation

In addition to the applicable namespace and compatibility tests, independently interrupt each destination, verify that the other continues receiving data, measure queue growth and recovery, and prove that FI does not receive duplicate records through both paths.

## Option 6: GMI NICo to GMI OTEL collector to FI gateway

This option keeps NICo pointed at the GMI collector while using the FI gateway as the authenticated final hop to Fleet Intelligence.

```text
GMI NICo -> GMI OTEL collector -> FI OTEL gateway -> Fleet Intelligence
```

### Assumptions imposed on GMI

- The GMI collector preserves all FI-required resource attributes, record attributes, log bodies, diagnostic payloads, and metric names.
- The GMI collector can export OTLP/gRPC to the FI gateway without unsupported transformations.
- GMI configures routing so the required NICo metrics and logs reach the FI gateway exactly once.
- GMI exposes stable namespace and pod labels for NetworkPolicy selection.

### Additional FI-side changes

1. Generalize gateway ingress configuration from an external NICo source to an explicitly selected external OTLP producer.
2. Document required attribute-preservation and filtering behavior.
3. Add representative GMI collector configuration and telemetry fixtures.

### Validation

Compare NICo records before and after the GMI collector, verify lossless handling of diagnostic and alert payloads, test batching and backpressure across both collectors, and verify that filters or processors do not remove the BMC MAC or FI labels.

## Option 7: GMI NICo to GMI OTEL collector directly to Fleet Intelligence

This option removes the FI gateway and has the largest authentication and support surface.

```text
GMI NICo -> GMI OTEL collector -> Fleet Intelligence
```

### Assumptions imposed on GMI

- GMI can install and operate the FI enrollment and authentication extension or another FI-approved authentication mechanism.
- GMI accepts responsibility for token protection, enrollment, renewal, backend endpoint configuration, retries, queueing, and proxy behavior.
- The GMI collector supports separate FI metrics and logs endpoints and preserves the complete FI telemetry contract.
- FI approves distributing and supporting the required gateway extension outside the FI-managed image.

### FI-side changes

1. Define and support the external collector authentication contract.
2. Package or document the FI authentication extension for the GMI collector's exact distribution and version.
3. Define ownership for enrollment failures, token renewal, backend API changes, retries, and queue durability.
4. Add conformance tests for the GMI collector and every supported upgrade.
5. Disable the FI gateway without reporting NICo telemetry as disabled.

This option should not be committed until FI security and backend owners approve external ownership of enrollment credentials and runtime tokens.

### Validation

Run authentication lifecycle, token-expiry, backend rejection, retry, queue, proxy, TLS, metrics, logs, diagnostic-payload, and upgrade tests against the exact GMI collector distribution.

## Recommended support boundary

Option 1 is the default supported integration for an existing customer configuration or custom plugin.

Option 1A should remain an investigation path until the SQL-source change is merged into NICo, FI qualifies the resulting image, and the configuration-overlay and MAC-reconciliation tests pass.

Option 2 should be supported when GMI cannot colocate the deployments but can provide stable namespace and pod labels.

Options 3 and 4 should require an explicitly staffed compatibility investigation because an unknown NICo version cannot be treated as configuration-only work.

Option 5 should be supported only after proving independent multi-target delivery and absence of duplicate forwarding.

Option 6 should be conditional on a lossless telemetry conformance test for the GMI collector.

Option 7 should remain unsupported until authentication ownership and security review are complete.

## Proposed implementation sequence

1. Introduce an explicit external-NICo deployment state while preserving the current chart-managed behavior as the default.
2. Allow the FI gateway to run when NICo is externally managed.
3. Add the preserve-and-overlay contract, merge tests, and live POC for Option 1.
4. Run the Option 1A SQL-source POC after GMI's target-source change is available.
5. Add selector-based cross-namespace gateway ingress and tests for Option 2.
6. Build a NICo compatibility test harness that can run the same assertions against an image digest supplied by GMI.
7. Evaluate Options 3 through 7 only when GMI rejects the corresponding lower-cost requirement.

## Decision criteria

Select the first option in this document whose assumptions GMI accepts. Do not select a later option merely to avoid asking GMI for namespace placement, a NICo upgrade, workload labels, or configuration ownership; those concessions are cheaper and safer than adding permanent FI compatibility and authentication surfaces.

The final deployment decision must record the NICo image digest, namespace topology, configuration owner, credentials owner, OTLP route, existing telemetry destinations, NetworkPolicy selectors, restart procedure, and completed validation suite.
