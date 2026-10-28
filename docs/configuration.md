# Configuration

The collector uses two files:

- A **TOML file** that lists the BMCs to collect from. It contains no passwords.
- A **JSON credentials file** with BMC usernames and passwords. NICo hardware health reads the same file, so you keep credentials in one place.

With Helm, you describe targets in your values file and the chart generates the TOML for you; see [Helm values](#helm-values). For a local run, copy `config.example.toml` and `credentials.example.json` and edit them.

## Collector TOML

Add one `[[targets]]` entry for each BMC:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"

[credentials]
file = "/run/secrets/bmc/credentials.json"

[[targets]]
kind = "compute"
credentials_profile = "bmc"
name = "node-01"
ip = "192.0.2.10"
port = 443
mac = "00:11:22:33:44:55"
labels = { site = "dc1", rack = "rack-12" }

[[targets]]
kind = "nvswitch"
credentials_profile = "switch"
name = "switch-tray-01"
ip = "192.0.2.11"
mac = "02:00:0a:00:00:0b"

[[targets]]
kind = "power_shelf"
credentials_profile = "bmc"
name = "power-shelf-01"
ip = "192.0.2.12"
mac = "02:00:0a:00:00:0c"
```

You need at least one target. The collector rejects fields it doesn't recognize, so a typo fails loudly instead of being ignored.

| Field | Required | Description |
| --- | --- | --- |
| `credentials.file` | Yes | Path to the JSON credentials file. Use an absolute path; a relative path is resolved from the directory containing the TOML file. |
| Top-level `compute_zone` | No | Default compute zone for every target. See [placement fields](#fleet-intelligence-placement-fields). |
| Top-level `node_group` | No | Default node group for every target. |
| Top-level `labels` | No | Labels applied to every target. A target label with the same key wins. |
| `targets[].ip` | Yes | IPv4 or IPv6 address of the BMC. |
| `targets[].mac` | Yes | MAC address of the BMC. See [target identity](#target-identity). |
| `targets[].port` | No | Redfish port. Defaults to the HTTPS port. |
| `targets[].kind` | No | `compute`, `nvswitch`, `power_shelf`, or `unknown` (the default). |
| `targets[].name` | No | Display name, shown as the inventory `name` and the NICo `machine_name` label. |
| `targets[].credentials_profile` | No | Profile to use from the credentials file. Optional only when a MAC override supplies the credentials. |
| `targets[].compute_zone` | No | Overrides the top-level compute zone. |
| `targets[].node_group` | No | Overrides the top-level node group. |
| `targets[].labels` | No | Extra labels for this target. |

### Target identity

Fleet Intelligence recognizes a machine by its BMC MAC address, not its IP address. That keeps inventory, attestation, and telemetry for the same machine together even if its IP changes, so every target needs its real BMC MAC.

You can write the MAC with colons (`00:11:22:33:44:55`), hyphens, or no separators, in any letter case. The collector reports it as uppercase with colons, and it doesn't rewrite your file. Two targets with the same MAC, however it is spelled, are rejected as duplicates.

### Labels

Labels are added to NICo telemetry for each target.

- Keys must match `[a-zA-Z_][a-zA-Z0-9_]*`.
- Each target can have up to 32 labels. That count includes the labels the collector adds itself: `oob="true"`, plus `machine_name`, `compute_zone`, and `node_group` when those fields are set.
- Label values, and the `name`, `compute_zone`, and `node_group` fields, can be up to 1,024 bytes. Those three fields are trimmed and can't be blank when set.
- You can't use reserved names: the four labels above, and the labels NICo sets itself, such as `bmc_ip`, `endpoint_mac`, `gpu_uuid`, `machine_id`, and `system_uuid`. The full list is `RESERVED_NICO_LABELS` in the [configuration source](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/src/config/targets.rs).

### Fleet Intelligence placement fields

`compute_zone` and `node_group` place each machine in a Fleet Intelligence compute zone and node group. Use the exact names configured in your Fleet Intelligence account; the values on this page are only examples. A compute zone is typically a location or failure domain, such as `us-west-dc1`. A node group is typically a pool of similar hardware or workloads, such as `gb200-training`.

Set them at the top level when every target shares the same placement, and override them on any target that differs:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"

[credentials]
file = "/run/secrets/bmc/credentials.json"

[[targets]]
kind = "compute"
name = "node-01"
ip = "192.0.2.10"
mac = "02:00:0a:00:00:0a"
credentials_profile = "bmc"

[[targets]]
kind = "compute"
name = "node-02"
ip = "192.0.2.11"
mac = "02:00:0a:00:00:0b"
credentials_profile = "bmc"
compute_zone = "us-west-dc2"
node_group = "gb200-inference"
```

Here `node-01` uses the top-level values, while `node-02` is in `us-west-dc2` and `gb200-inference`.

The collector sends these values with the inventory and adds them as telemetry labels:

| TOML field | Telemetry label | Inventory field |
| --- | --- | --- |
| `compute_zone` | `compute_zone` | `computeZone` |
| `node_group` | `node_group` | `nodeGroup` |

If a field is set in neither place, the machine is sent without it. Custom labels such as `site` stay labels; they don't set placement.

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

The `inventory`, `run`, and `render-nico-config` commands all require this file. A complete example is in [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json).

### How credentials are resolved

For each BMC, the collector looks up each field (`username`, `password`) in this order:

1. The override for that BMC's MAC, if there is one.
2. The profile named in the target's `credentials_profile`.

A field that is missing or `null` in the override comes from the profile. An empty string `""` is a real value: it is used as-is and doesn't fall back to the profile. There is no default profile.

Every target must end up with a username. If neither source sets a password, the password is empty.

Override keys follow the same MAC rules as targets: any separator or case works, and two keys for the same MAC are rejected.

The collector rejects a credentials file with unknown fields, invalid MACs, malformed JSON, unknown profile names, or a target without a username. Error messages never include credential values.

### When credential changes take effect

You can change passwords without restarting anything: the collector and NICo both re-read the file. Replace the whole file at once rather than editing it in place, so neither process reads a half-written file. Kubernetes Secret updates already work this way. See [rotation](credentials.md#rotation-and-ownership) for timing and for what does need a restart.

## CLI commands

The executable is `bmc-collector`. From a source checkout, run it with `cargo run --locked -- <command>`, and add `--help` to any command to list its options.

| Command | What it does |
| --- | --- |
| `inventory` | Collects once and prints a table. Add `--format json` for JSON or `--detail` for more columns. |
| `run` | Enrolls with Fleet Intelligence, then collects and uploads inventory and attestation on a schedule. |
| `render-nico-config --output nico.toml` | Generates NICo configuration from your TOML and credentials files. |

`inventory` and `run` share these options:

| Option | Default | Description |
| --- | --- | --- |
| `--config` | `config.toml` | Path to the collector TOML file. |
| `--target-concurrency` | `64` | Number of BMCs collected at the same time. |
| `--bmc-max-in-flight` | `4` | Maximum simultaneous Redfish requests to one BMC. |
| `--bmc-requests-per-second` | `10` | Maximum request rate to one BMC. |
| `--bmc-burst` | `10` | Requests allowed in a short burst above that rate. |
| `--timeout-seconds` | `30` | Redfish request timeout. |
| `--insecure-tls` | Off | Skip BMC certificate verification. See [Proxy and TLS](#proxy-and-tls). |
| `--disable-latency-probe` | Off | Skip network latency measurements. Also set by `FLEETINT_DISABLE_LATENCY_PROBE`. |

`run` also needs:

| Setting | Description |
| --- | --- |
| `FLEETINT_BACKEND_URL` or `--backend-url` | Fleet Intelligence API address. See [Proxy and TLS](#proxy-and-tls). |
| `FLEETINT_AGENT_TOKEN` or `--agent-token` | Bootstrap token. Prefer the environment variable so the token doesn't appear in the process list. |
| `--interval-seconds` | Wait time after one collection cycle ends before the next starts. Defaults to `86400` (24 hours). `0` is treated as one second. |

Set `RUST_LOG` to change the log level; it defaults to `info`.

`render-nico-config` writes NICo configuration that points at your credentials file; it doesn't copy passwords into it. It accepts:

| Option | Default | Description |
| --- | --- | --- |
| `--otlp-endpoint` | `http://localhost:4317` | Where NICo sends metrics and logs. |
| `--metrics-endpoint` | `0.0.0.0:9009` | Where NICo serves Prometheus metrics. |
| `--metrics-prefix` | `fleetint` | Prefix for NICo metric names. |

These options only configure NICo; they don't start a gateway. The Helm chart runs this command in NICo's init container, and the host package runs it before NICo starts.

## Helm values

In Helm, you list targets under `inventory.targets`, and the chart generates the collector TOML. Field names are camelCase: `computeZone`, `nodeGroup`, and `credentialsProfile`. Each target's `address` is a bare IP address, with no scheme or port; set `port` separately if needed. See [Installation](installation.md#configure-targets-and-secrets) for a complete example.

| Setting | Description |
| --- | --- |
| `inventory` | Targets, placement, and labels. |
| `credentials` | The existing Secret that holds the credentials file. |
| `backend` | Fleet Intelligence URL and the existing Secret that holds the bootstrap token. |
| `telemetry.endpoint` | Send NICo telemetry to your own OTLP/gRPC collector instead of the bundled gateway. Requires `otelGateway.enabled: false`. |
| `proxy` | HTTP proxy for all workloads. Off unless enabled. The proxy URL can't contain credentials. |
| `bmcCollector.redfish` | Per-BMC request limits, timeout, and TLS verification for the collector. |
| `bmcCollector.intervalSeconds` | Wait time between collection cycles. Defaults to `86400`; must be positive. |
| `bmcCollector.targetConcurrency` | Number of BMCs collected at the same time. Defaults to `64`. |
| `bmcCollector.latencyProbe.enabled` | Network latency measurements. Defaults to `true`. |
| `nicoHardwareHealth` | NICo hardware health workload. |
| `otelGateway` | Bundled telemetry gateway, including its replica count. |

Every workload accepts the usual `enabled`, `image`, `resources`, scheduling, annotation, and `logLevel` settings. Secret references use `existingSecret` and `secretKey`. See [values.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/fleet-intelligence-oob-collector/values.yaml) for every setting and its default.

A few things differ from the CLI:

- The chart sets the burst size equal to `bmcCollector.redfish.requestsPerSecond`; there is no separate burst value.
- The request limits apply to the collector only. NICo makes its own requests to the BMCs on its own schedule.
- NICo's telemetry settings are fixed by the chart, not exposed as values. NICo collects BMC logs from the Redfish event stream, so BMCs need EventService support for log collection; there is no polling fallback. It sends metrics and logs in batches of up to 512 records, at least every two seconds.

## Reported collector behavior

With each inventory upload, the collector reports its version and the settings it is running with, such as the collection interval, request limits, and which telemetry components are enabled. The report never includes secrets, credential references, or URLs. It describes configuration, not whether those components are healthy.

## Proxy and TLS

Set the backend URL to the API's base address, such as `https://api.example.com`, without a path like `/v1/agent`. The URL can't contain a username, password, query string, or fragment; supply the token through `FLEETINT_AGENT_TOKEN` instead.

To use a proxy with Helm, enable `proxy` and give a URL the cluster can reach, without credentials.

`bmcCollector.redfish.insecureTls` (or `--insecure-tls`) turns off BMC certificate checks for the collector. Use it only for BMCs with self-signed certificates on a network you trust, because it allows traffic to be intercepted. It doesn't change how the collector verifies the Fleet Intelligence backend. NICo doesn't verify BMC certificates, whatever this setting is.
