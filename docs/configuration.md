# Configuration

You configure the collector through your Helm values file. This page covers each part of it, in the order you'd fill it in. [Installation](installation.md#save-your-values-file) has a complete minimal example, and [values.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/fleet-intelligence-oob/values.yaml) lists every setting with its default.

Passwords never go in the values file. They live in a separate [BMC credentials file](#bmc-credentials-file) that you store in a Kubernetes Secret, and the chart only references it.

## Targets

List one entry under `inventory.targets` for each BMC. The collector and NICo hardware health both read this list.

```yaml
inventory:
  computeZone: us-west-dc1
  nodeGroup: gb200-training
  labels:
    site: dc1
  targets:
    - kind: compute
      name: node-01
      address: 192.0.2.10
      mac: "00:11:22:33:44:55"
      credentialsProfile: bmc
      labels:
        rack: rack-12
    - kind: nvswitch
      name: switch-tray-01
      address: 192.0.2.11
      mac: "02:00:0a:00:00:0b"
      credentialsProfile: switch
    - kind: power_shelf
      name: power-shelf-01
      address: 192.0.2.12
      port: 8443
      mac: "02:00:0a:00:00:0c"
      credentialsProfile: bmc
```

You need at least one target. The chart rejects fields it doesn't recognize, so a typo fails the install instead of being ignored.

| Field | Required | Description |
| --- | --- | --- |
| `address` | Yes | IPv4 or IPv6 address of the BMC, with no scheme or port. |
| `mac` | Yes | MAC address of the BMC. See [target identity](#target-identity). |
| `port` | No | Redfish port. Defaults to the HTTPS port. |
| `kind` | No | `compute`, `nvswitch`, `power_shelf`, or `unknown` (the default). |
| `name` | No | Display name, shown as the inventory `name` and the NICo `machine_name` label. |
| `credentialsProfile` | No | Profile to use from the credentials file. Optional only when a MAC override supplies the credentials. |
| `computeZone` | No | Overrides `inventory.computeZone` for this target. See [placement](#placement). |
| `nodeGroup` | No | Overrides `inventory.nodeGroup` for this target. |
| `labels` | No | Extra telemetry labels for this target. See [labels](#labels). |

These settings apply to every target unless the target overrides them:

| Field | Description |
| --- | --- |
| `inventory.computeZone` | Default compute zone. |
| `inventory.nodeGroup` | Default node group. |
| `inventory.labels` | Labels for every target. A target label with the same key wins. |

### Target identity

Fleet Intelligence recognizes a machine by its BMC MAC address, not its IP address. That keeps inventory, attestation, and telemetry for the same machine together even if its IP changes, so every target needs its real BMC MAC.

Write the MAC with colons (`00:11:22:33:44:55`), hyphens, or no separators, in any letter case. Quote it in YAML. Fleet Intelligence shows it as uppercase with colons. Two targets with the same MAC, however it is spelled, are rejected as duplicates.

### Placement

`computeZone` and `nodeGroup` place each machine in a Fleet Intelligence compute zone and node group. Use the exact names configured in your Fleet Intelligence account; the values on this page are only examples. A compute zone is typically a location or failure domain, such as `us-west-dc1`. A node group is typically a pool of similar hardware or workloads, such as `gb200-training`.

Set them under `inventory` when every target shares the same placement, and override them on any target that differs:

```yaml
inventory:
  computeZone: us-west-dc1
  nodeGroup: gb200-training
  targets:
    - name: node-01
      address: 192.0.2.10
      mac: "02:00:0a:00:00:0a"
      credentialsProfile: bmc
    - name: node-02
      address: 192.0.2.11
      mac: "02:00:0a:00:00:0b"
      credentialsProfile: bmc
      computeZone: us-west-dc2
      nodeGroup: gb200-inference
```

Here `node-01` uses the shared values, while `node-02` is in `us-west-dc2` and `gb200-inference`. If neither place sets a value, the machine is sent without it.

The collector sends placement with the inventory and adds it to telemetry as the `compute_zone` and `node_group` labels. Custom labels such as `site` stay labels; they don't set placement.

### Labels

Labels are added to NICo telemetry for each target.

- Keys must match `[a-zA-Z_][a-zA-Z0-9_]*`.
- Each target can have up to 32 labels. That count includes the labels added automatically: `oob="true"`, plus `machine_name`, `compute_zone`, and `node_group` when `name`, `computeZone`, and `nodeGroup` are set.
- Label values, and the `name`, `computeZone`, and `nodeGroup` fields, can be up to 1,024 bytes. Those three fields are trimmed and can't be blank when set.
- You can't use reserved names: the four labels above, and the labels NICo sets itself, such as `bmc_ip`, `endpoint_mac`, `gpu_uuid`, `machine_id`, and `system_uuid`. The full list is `RESERVED_NICO_LABELS` in the [configuration source](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/src/config/targets.rs).

Helm checks key syntax when you install. The remaining rules are checked when the collector and NICo's init container load the configuration, so a violation shows up as a failed pod.

## BMC credentials file

The credentials file holds named `profiles` and per-BMC `overrides`, keyed by MAC address:

```json
{
  "profiles": {
    "bmc": { "username": "admin", "password": "replace-with-bmc-password" },
    "switch": { "username": "switch-admin", "password": "replace-with-switch-password" }
  },
  "overrides": {
    "02:00:0a:00:00:0b": { "password": "replace-with-target-password" }
  }
}
```

Store it in a Kubernetes Secret, either [by hand](installation.md#create-the-secrets) or [from Vault or OpenBao](credentials.md), and point the chart at it:

```yaml
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
```

The chart mounts the file into the collector and NICo at `/run/secrets/bmc/credentials.json`. A complete example is in [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json).

### How credentials are resolved

For each BMC, the collector and NICo look up each field (`username`, `password`) in this order:

1. The override for that BMC's MAC, if there is one.
2. The profile named in the target's `credentialsProfile`.

A field that is missing or `null` in the override comes from the profile. An empty string `""` is a real value: it is used as-is and doesn't fall back to the profile. There is no default profile.

Every target must end up with a username. If neither source sets a password, the password is empty.

Override keys follow the same MAC rules as targets: any separator or case works, and two keys for the same MAC are rejected.

The collector rejects a credentials file with unknown fields, invalid MACs, malformed JSON, unknown profile names, or a target without a username. Error messages never include credential values.

### When credential changes take effect

You can change passwords without restarting anything: the collector and NICo both re-read the file. See [rotation](credentials.md#rotation-and-ownership) for timing and for what does need a restart.

## Fleet Intelligence connection

```yaml
backend:
  url: https://data.fleet-intelligence.nvidia.com
  agentToken:
    existingSecret: fleet-intelligence-oob-collector-agent-token
    secretKey: agent-token
```

`backend.url` defaults to the production endpoint shown above. Change it only if you use a different Fleet Intelligence endpoint. Use the API's base address, without a path such as `/v1/agent`. The URL can't contain a username, password, query string, or fragment.

`backend.agentToken` points to the Secret that holds the bootstrap token. The collector and the bundled gateway both use it.

## Components and telemetry

By default, the chart runs the collector, NICo hardware health, and a gateway that forwards NICo's metrics and logs to Fleet Intelligence. Each has an `enabled` switch:

| To run | Set |
| --- | --- |
| Everything (default) | Nothing |
| Inventory and attestation only | `nicoHardwareHealth.enabled: false` and `otelGateway.enabled: false` |
| Health telemetry only | `bmcCollector.enabled: false` |
| NICo with your own OTLP/gRPC collector | `telemetry.endpoint` (an `http://` or `https://` URL) and `otelGateway.enabled: false` |

The gateway requires NICo.

NICo defaults to automatic log collection: it starts with Redfish SSE streaming, falls back to periodic polling when SSE is unavailable or keeps failing, and retries SSE after 30 minutes of polling. You can select fixed SSE streaming or periodic polling with `nicoHardwareHealth.config.logsMode`. It sends metrics and logs in batches of up to 512 records, at least every two seconds.

### NICo collection controls

Set these optional values under `nicoHardwareHealth.config`. Workload settings such as `image`, `resources`, and scheduling remain under `nicoHardwareHealth`. Omitted or `null` settings use the defaults below, including on native installations without a `[nico_health]` section. Timing settings alone preserve each collector's enabled state: metrics, telemetry reports, firmware, and the iteration limiter require their corresponding `*Enabled: true` setting to start. TCP reachability checks are enabled by default; set `reachabilityEnabled: false` to disable them. Logs default to auto mode with SSE recovery enabled.

| Setting under `nicoHardwareHealth.config` | Default | Description |
| --- | --- | --- |
| `bmcRequestConcurrency` | `2` | Shared maximum concurrent Redfish operations per BMC. |
| `endpointDiscoveryIntervalSeconds` | `300` | Interval between endpoint reconciliation iterations. |
| `discoveryRefreshIntervalSeconds` | `3600` | Interval between Redfish entity-tree discovery iterations. |
| `discoveryConcurrency` | `4` | Concurrent endpoint identity resolutions. |
| `sensorFetchIntervalSeconds` | `60` | Interval between sensor fetch iterations. |
| `sensorsEnabled` | `true` | Collect sensor readings. |
| `includeSensorThresholds` | `true` | Include thresholds in sensor attributes. |
| `leakDetectorEnabled` | `true` | Query thermal-subsystem leak detectors. |
| `leakDetectorPollIntervalSeconds` | `60` | Interval between leak-detector polls. |
| `leakDetectorStateRefreshIntervalSeconds` | `1800` | Interval between leak-detector discovery refreshes. |
| `logsEnabled` | `true` | Collect BMC logs. |
| `logsMode` | `auto` | `sse`, `periodic`, or `auto` (SSE with per-endpoint polling fallback). |
| `logsSseInitialBackoffSeconds` | `1` | Initial reconnect backoff, before jitter. |
| `logsSseMaxBackoffSeconds` | `30` | Maximum reconnect backoff, before jitter. |
| `logsCollectionIntervalSeconds` | `300` | Interval between log polls in periodic mode or auto fallback. |
| `logsStateRefreshIntervalSeconds` | `1800` | Interval between log-service discovery refreshes. |
| `logsExcludeServices` | `["Journal"]` | Skip LogServices whose resource IDs contain any listed substring; `[]` includes every service. |
| `logsSkipInitialHistory` | `false` | Skip retained history on the first successful baseline without a checkpoint. |
| `logsAutoSseNotAvailableThreshold` | `1` | Unsupported-SSE attempts before falling back to polling. |
| `logsAutoConnectFailureWindowSeconds` | `300` | Window for counting SSE connection failures. |
| `logsAutoConnectFailureThreshold` | `5` | Connection failures within that window before falling back. |
| `logsAutoRetrySseAfterDowngrade` | `true` | In auto mode, retry SSE after 30 minutes of polling fallback. Set `false` to remain in polling mode. |
| `metricsEnabled` | `false` | Collect Redfish entity metrics. |
| `metricsFetchIntervalSeconds` | `120` | Interval between entity-metric iterations. |
| `telemetryEnabled` | `false` | Collect Redfish TelemetryService reports. |
| `telemetryFetchIntervalSeconds` | `60` | Interval between report-fetch iterations. |
| `firmwareEnabled` | `false` | Collect firmware inventory in NICo. |
| `firmwareRefreshIntervalSeconds` | `7200` | Interval between firmware iterations. |
| `managerEnabled` | `true` | Poll Manager status for NICo power-shelf endpoints. |
| `managerPollIntervalSeconds` | `300` | Interval between eligible Manager polls. |
| `reachabilityEnabled` | `true` | Probe eligible endpoint TCP ports. |
| `reachabilityIntervalSeconds` | `60` | Interval between TCP probe cycles. |
| `reachabilityTimeoutSeconds` | `3` | Timeout for one TCP connection attempt. |
| `reachabilityLogMode` | `unreachable` | Log failed probes (`unreachable`) or every probe (`all`). |
| `rateLimitEnabled` | `false` | Enable NICo's global collector-iteration token bucket. |
| `rateLimitBucketBurst` | `100` | Collector iterations admitted in a burst. |
| `rateLimitBucketReplenishMilliseconds` | `30` | Interval between token replenishments. |
| `rateLimitMaxJitterMilliseconds` | `50` | Maximum token-bucket jitter; `0` disables this jitter. |

For example:

```yaml
nicoHardwareHealth:
  config:
    sensorFetchIntervalSeconds: 120
    bmcRequestConcurrency: 2
    leakDetectorPollIntervalSeconds: 120
    logsSseMaxBackoffSeconds: 60
```

Integer controls accept `1` through `4294967295`; `rateLimitMaxJitterMilliseconds` also accepts `0`. SSE maximum backoff must be at least its initial backoff. Explicit `false`, zero jitter, and empty exclusion arrays are preserved.

Controls apply to all NICo targets, independently of inventory and attestation. Polling intervals are waits between iterations. Each collector runs independently, so disabling sensors leaves discovery for GPU identity and other enabled collectors active. Established SSE streams release their concurrency slot after the handshake. The global limiter counts collector iterations, not HTTP requests.

Auto fallback uses the configured log-polling fields. Mode changes can replay or miss records when SSE and LogService histories differ; temporary checkpoints can be lost on pod replacement or native directory cleanup. Manager polling requires power-shelf metadata and stays inactive for the renderer's generic endpoints.

Apply with `helm upgrade`, or use flat snake_case fields under `[nico_health]` and restart `fleetint-nico-health.service` on native installations.

## Collection settings

| Setting | Default | Description |
| --- | --- | --- |
| `bmcCollector.intervalSeconds` | `86400` | Wait time after one collection cycle ends before the next starts (24 hours). |
| `bmcCollector.targetConcurrency` | `64` | Number of BMCs collected at the same time. |
| `bmcCollector.redfish.maxInFlight` | `2` | Maximum simultaneous requests to one BMC. |
| `bmcCollector.redfish.requestsPerSecond` | `10` | Maximum request rate to one BMC. Short bursts up to the same number are allowed. |
| `bmcCollector.redfish.timeoutSeconds` | `30` | Redfish request timeout. |
| `bmcCollector.redfish.insecureTls` | `false` | Skip BMC certificate verification. See [TLS](#proxy-and-tls). |
| `bmcCollector.latencyProbe.enabled` | `true` | Measure network latency to each BMC. |

These limits apply to the collector only. NICo makes its own requests to the BMCs on its own schedule. The two concurrency limits are independent, so inventory and NICo can each issue two requests to the same BMC at once with the defaults. See [request limits](architecture.md#request-limits) for how they work together.

## Workload settings

Every workload (`bmcCollector`, `nicoHardwareHealth`, and `otelGateway`) accepts `enabled`, `image`, `resources`, `nodeSelector`, `tolerations`, `affinity`, `podAnnotations`, and `logLevel`. `otelGateway.replicas` sets the number of gateway replicas.

NICo application logging defaults to `warn` to suppress routine per-reading messages. Set `nicoHardwareHealth.logLevel` to `info` or `debug` when investigating collection. This setting controls application log verbosity; BMC event log collection and telemetry export continue independently.

Collector and NICo image tags default to the chart version. If you pull images through a private registry mirror, set `imagePullSecrets`.

## Reported collector behavior

With each inventory upload, the collector reports its version and the settings it is running with, such as the collection interval, request limits, and which telemetry components are enabled. The report never includes secrets, credential references, or URLs. It describes configuration, not whether those components are healthy.

`collector.config.nicoHealth` reports explicitly configured controls in camelCase from each cycle's `[nico_health]` snapshot. Omitted controls stay omitted. These are configured values; checking what NICo has loaded requires checking that service after a restart.

NICo reports also include `enabled`, `logLevel`, and `telemetryDestination`. Pod settings such as image, resources, and scheduling stay in Helm. Component reports require explicit deployment metadata, supplied by Helm or native `service.conf`; a manually started collector without it omits those reports.

## Proxy and TLS

To send traffic through an HTTP proxy, enable `proxy` and set the proxy URLs. They must be reachable from the cluster and can't contain credentials.

```yaml
proxy:
  enabled: true
  httpsProxy: http://proxy.example.com:3128
  noProxy: localhost,127.0.0.1,.svc,.cluster.local
```

`proxy` also accepts `httpProxy` and `allProxy`. The default `noProxy` keeps local and in-cluster traffic off the proxy; if you change it, keep those entries.

`bmcCollector.redfish.insecureTls` turns off BMC certificate checks for the collector. Use it only for BMCs with self-signed certificates on a network you trust, because it allows traffic to be intercepted. It doesn't change how the collector verifies the Fleet Intelligence backend. NICo doesn't verify BMC certificates, whatever this setting is.
