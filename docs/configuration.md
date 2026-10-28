# Configuration

The collector reads targets from a TOML configuration file and credentials from
a separate JSON secret file shared with NICo hardware health. Start from
`config.example.toml` and define one or more Redfish targets:

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

The JSON file uses NICo's `profiles` and per-MAC `overrides` schema; see
[credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json). Each field resolves from
the MAC override, then the explicitly selected `credentials_profile`. There is
no implicit default profile. Omitted or null fields inherit; an empty string is
an explicit value. A username field must resolve; passwords default to an empty
string when omitted from both sources. MAC matching normalizes letter case
and separator format, and equivalent override keys are rejected as duplicates.
Unknown fields, invalid MACs, malformed JSON, unknown profiles, or unresolved
usernames reject the configuration without including credential values in errors.

The collector reads the file on each collection cycle. During attestation, a BMC
`401 Unauthorized` also triggers one credential reload and request retry per
target, using its existing profile and MAC override. Target configuration stays
fixed until the next cycle. Replace the complete file atomically, including
when rotating a Kubernetes Secret. NICo reloads the
same file through its cached reader and refreshes consumers after authentication
failures. Use an absolute path for both processes; relative collector paths are
resolved from the directory containing its TOML file.

At least one `[[targets]]` entry is required. Unknown TOML fields are rejected.
Each target's optional `kind` is one of `unknown`, `compute`, `nvswitch`, or
`power_shelf`, and defaults to `unknown` when omitted.

## CLI commands

The executable is `bmc-collector`; from a source checkout, use
`cargo run --locked -- <command>`. Use `<command> --help` to inspect its options.

| Command | Purpose |
| --- | --- |
| `inventory` | Collect once and print a table; use `--format json` for JSON or `--detail` for detailed table output. |
| `run` | Enroll and continuously upload inventory and attestation. Requires a backend URL and bootstrap token. |
| `render-nico-config --output nico.toml` | Generate NICo TOML referencing the shared credentials JSON file. |

`inventory` and `run` share these options:

| Option | Default | Purpose |
| --- | --- | --- |
| `--config` | `config.toml` | Collector TOML path. |
| `--target-concurrency` | `64` | Concurrent target workflows. |
| `--bmc-max-in-flight` | `4` | In-flight Redfish requests per BMC. |
| `--bmc-requests-per-second` | `10` | Per-BMC request rate. |
| `--bmc-burst` | `10` | Per-BMC burst capacity. |
| `--timeout-seconds` | `30` | Redfish request timeout. |
| `--insecure-tls` | Off | Disable collector BMC certificate verification. |
| `--disable-latency-probe` | Off | Skip TCP latency measurements; also accepts `FLEETINT_DISABLE_LATENCY_PROBE`. |

For `run`, set `FLEETINT_BACKEND_URL` to the backend API origin and
`FLEETINT_AGENT_TOKEN` to the bootstrap token. `--backend-url` and `--agent-token`
are also accepted; prefer the token environment variable to keep it out of
process arguments. `--interval-seconds` defaults to `86400` and sets the delay
after a cycle finishes. `RUST_LOG` configures collector logging and defaults to
`info`. A zero CLI collection interval is treated as one second; Helm requires
a positive interval.

The renderer accepts `--otlp-endpoint` (default `http://localhost:4317`),
`--metrics-endpoint` (default `0.0.0.0:9009`), and `--metrics-prefix` (default
`fleetint`). These settings configure NICo's generated telemetry destinations;
they do not start a gateway.

## Collector TOML

| Field | Required | Description |
| --- | --- | --- |
| Top-level `compute_zone` | No | Default Fleet Intelligence compute zone inherited by targets and mirrored to telemetry as `compute_zone`. |
| Top-level `node_group` | No | Default Fleet Intelligence node group inherited by targets and mirrored to telemetry as `node_group`. |
| Top-level `labels` | No | Labels inherited by every target. Per-target labels with the same key override them. |
| `credentials.file` | Yes | Path to the JSON secret file shared with NICo hardware health. |
| `targets[].credentials_profile` | No | Named profile in the shared JSON file; a per-MAC override can supply credentials without a profile. |
| `targets[].kind` | No | Target type: `unknown`, `compute`, `nvswitch`, or `power_shelf`; defaults to `unknown`. |
| `targets[].name` | No | Human-readable name exported as inventory `name` and the NICo `machine_name` label. |
| `targets[].compute_zone` | No | Per-target compute-zone override. |
| `targets[].node_group` | No | Per-target node-group override. |
| `targets[].ip` | Yes | IPv4 or IPv6 address of the BMC. |
| `targets[].port` | No | Redfish port; the HTTPS default is used when omitted. |
| `targets[].mac` | Yes | Stable BMC identity: six-byte MAC in colon-separated, hyphen-separated, or unseparated hexadecimal form. Stored as bytes, formatted as uppercase colon-separated text, and unique in the file. The config file is not rewritten. |
| `targets[].labels` | No | Labels added to this target or overriding inherited top-level labels. |

Every target requires an explicit BMC MAC. The collector does not derive identity
from the IP address. The normalized MAC is used consistently in configuration, inventory, and NICo
telemetry; equivalent spellings are rejected as duplicate targets.

Label keys must match `[a-zA-Z_][a-zA-Z0-9_]*`. Each target supports at most
32 labels in total, including `oob="true"` and labels derived from `name`,
`compute_zone`, and `node_group` when present. Values for labels and these
fields are limited to 1,024 bytes. Names and placement values are trimmed and
must not be blank when supplied.
The derived labels `machine_name`, `compute_zone`, `node_group`, and `oob`
are reserved. NICo also reserves endpoint, hardware identity, and resource
labels, including `bmc_ip`, `endpoint_mac`, `gpu_uuid`, `machine_id`, and
`system_uuid`. See the complete `RESERVED_NICO_LABELS` list in the
[configuration implementation](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/src/config/targets.rs).

### Fleet Intelligence placement fields

`compute_zone` and `node_group` are first-class configuration fields rather than
custom labels. They assign the node to its Fleet Intelligence compute zone and
node group, and the collector mirrors them to telemetry:

| TOML field | NICo telemetry label | Backend inventory field |
| --- | --- | --- |
| `compute_zone` | `compute_zone` | `computeZone` |
| `node_group` | `node_group` | `nodeGroup` |

The example value `us-west-dc1` illustrates a location or failure domain, while
`gb200-training` illustrates a logical hardware/workload pool. These are only
examples: deployments must use the exact compute-zone and node-group names
configured in their Fleet Intelligence account.

Set them at the top level when every configured target belongs to the same
Fleet Intelligence placement. Override them directly on a target when its
placement differs:

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

In this example, `node-01` inherits the `us-west-dc1` compute zone and the
`gb200-training` node group. `node-02` belongs to the `us-west-dc2` compute zone
and the `gb200-inference` node group. If either field is omitted from both
scopes, its corresponding Fleet Intelligence inventory placement field and
telemetry label are omitted. Custom labels are not promoted to Fleet
Intelligence placement fields.

The JSON secret file is required for `inventory`, `run`, and
`render-nico-config`. Store credential values
in its profiles or per-MAC overrides and select a profile with
`credentials_profile` where needed.
Helm renders the password-free TOML from `inventory` values and references an
existing Secret for the JSON file. Credential values never enter Helm release
state. See [installation](installation.md#configure-targets-and-secrets).

`bmc-collector render-nico-config` reads the same TOML and JSON files as the
collector. It writes the JSON file's absolute path and each selected profile
into NICo TOML, without copying credential values. Kubernetes invokes this
command in its init container; the host package uses
systemd `ExecStartPre`. All target kinds render as generic static Redfish
endpoints without NICo machine, switch, or power-shelf descriptors.

## Helm values

The collector and NICo use `inventory.targets`. Each target's `address` is an
IPv4 or IPv6 literal without a scheme or port; set its optional `port` separately.
Target placement and labels override shared defaults. Helm uses `computeZone`,
`nodeGroup`, and `credentialsProfile` for the TOML fields `compute_zone`,
`node_group`, and `credentials_profile`. All target kinds render as generic
static Redfish endpoints through the Rust renderer.

| Setting | Purpose |
| --- | --- |
| `inventory` | Targets, placement, and labels. |
| `credentials` | Shared BMC JSON Secret reference. |
| `backend` | Fleet Intelligence URL and bootstrap token reference. |
| `telemetry.endpoint` | External OTLP/gRPC destination; disable `otelGateway`. |
| `proxy` | Proxy environment variables for the Helm workloads, enabled explicitly; URLs cannot contain credentials. |
| `bmcCollector.redfish` | Per-BMC request limits, timeout, and TLS verification. |
| `bmcCollector.intervalSeconds` | Delay between collection cycles; defaults to `86400`. |
| `bmcCollector.targetConcurrency` | Concurrent target workflows; defaults to `64`. |
| `bmcCollector.latencyProbe.enabled` | Enable TCP latency measurements; defaults to `true`. |
| `nicoHardwareHealth` | Hardware metrics and logs workload controls. |
| `otelGateway` | Gateway deployment controls and replica count. |

All workloads share image, resource, scheduling, annotation, and logging fields.
Optional features use `enabled`; application Secret references use
`existingSecret` and `secretKey`. See [values.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/fleet-intelligence-oob-collector/values.yaml)
for defaults and [installation](installation.md#configure-targets-and-secrets)
for an example.

The chart sets the collector's burst capacity to
`bmcCollector.redfish.requestsPerSecond`; the CLI exposes burst capacity
separately through `--bmc-burst`. These dispatcher limits apply to the
collector's inventory and attestation requests. NICo runs its own collection
requests independently.

The renderer enables NICo's SSE log collection, GPU identity attributes,
tracing and Prometheus sinks, and OTLP export with diagnostics and alert
details. Its OTLP batches hold up to 512 records and flush every two seconds.
SSE log collection requires a BMC EventService; the rendered configuration does
not enable a polling fallback. These NICo settings are fixed by the renderer
and are not Helm values.

Provision the shared JSON Secret and `backend.agentToken` directly or through
an external secrets operator. See [Credentials](credentials.md) for Vault and
OpenBao setup and credential reload behavior.

## Reported collector behavior

The collector reports its version and effective settings with each inventory
batch, including the collection interval, request limits, and enabled telemetry
components. Reports exclude secrets, credential references, and URLs. These
settings describe configuration, not service health.

## Proxy and TLS

Use an absolute API origin, such as `https://api.example.com`, for
`FLEETINT_BACKEND_URL` or `--backend-url`, without a `/v1/agent` endpoint suffix.
Backend URLs cannot contain embedded credentials, query parameters, or fragments.
Supply the bootstrap token through `FLEETINT_AGENT_TOKEN`; authentication is
independent of the URL.

Enable `proxy` with a cluster-reachable URL without credentials.
`bmcCollector.redfish.insecureTls` disables BMC certificate verification and
permits interception. It does not affect backend TLS or NICo; the pinned NICo
client accepts invalid Redfish certificates.
