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
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
labels = { site = "dev", environment = "development" }

[[targets]]
kind = "nvswitch"
credentials_profile = "switch"
name = "switch-tray-01"
ip = "10.0.0.11"
mac = "02:00:0a:00:00:0b"

[[targets]]
kind = "power_shelf"
credentials_profile = "bmc"
name = "power-shelf-01"
ip = "10.0.0.12"
mac = "02:00:0a:00:00:0c"
```

The JSON file uses NICo's `profiles` and per-MAC `overrides` schema; see
[credentials.example.json](../credentials.example.json). Each field resolves from
the MAC override, then the explicitly selected `credentials_profile`. There is
no implicit default profile. Omitted or null fields inherit; an empty string is
an explicit value. A username must be present; passwords may be omitted. MAC
matching ignores letter case. Unknown fields, invalid MACs, malformed JSON,
unknown profiles, or unresolved usernames reject the configuration without
including credential values in errors.

The collector reads the file on each collection cycle. Replace the complete
file atomically, including when rotating a Kubernetes Secret. NICo reloads the
same file through its cached reader and refreshes consumers after authentication
failures. Use an absolute path for both processes; relative collector paths are
resolved from the directory containing its TOML file.

Define each target with `[[targets]]`. Its optional `kind` is one of `unknown`,
`compute`, `nvswitch`, or `power_shelf`, and defaults to `unknown` when omitted.

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
| `targets[].mac` | Yes | Stable BMC identity: Six-byte MAC in colon-separated, hyphen-separated, or unseparated hexadecimal form. Stored as bytes, formatted as uppercase colon-separated text, and unique in the file. The config file is not rewritten. |
| `targets[].labels` | No | Labels added to this target or overriding inherited top-level labels. |

Every target requires an explicit BMC MAC. The collector does not derive identity
from the IP address. The normalized MAC is used consistently in configuration, inventory, and NICo
telemetry; equivalent spellings are rejected as duplicate targets.

Label keys must match `[a-zA-Z_][a-zA-Z0-9_]*`. The collector reserves capacity
for labels derived from `name`, `compute_zone`, and `node_group`, plus
`oob="true"`. Values for labels and these fields are limited to 1,024 bytes.
NICo identity fields and all four derived label names are reserved.

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
ip = "10.0.0.10"
mac = "02:00:0a:00:00:0a"
credentials_profile = "bmc"

[[targets]]
kind = "compute"
name = "node-02"
ip = "10.0.0.11"
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

The JSON secret file is required for all collector runs. Store credential values
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

Both services use `inventory.targets`; addresses are IP literals with optional
ports. Target placement and labels override shared defaults. All target kinds
render as generic static Redfish endpoints through the Rust renderer.

| Setting | Purpose |
| --- | --- |
| `inventory` | Targets, placement, and labels. |
| `credentials` | Shared BMC JSON Secret reference. |
| `backend` | Fleet Intelligence URL and bootstrap token reference. |
| `telemetry.endpoint` | External OTLP/gRPC destination; disable `otelGateway`. |
| `proxy` | Shared proxy, enabled explicitly; URLs cannot contain credentials. |
| `bmcCollector.redfish` | Per-BMC request limits, timeout, and TLS verification. |
| `otelGateway` | Gateway deployment controls and replica count. |

All workloads share image, resource, scheduling, annotation, and logging fields.
Optional features use `enabled`; application Secret references use
`existingSecret` and `secretKey`. See [values.yaml](../deployments/helm/fleet-intelligence-oob-collector/values.yaml)
for defaults and [installation](installation.md#configure-targets-and-secrets)
for an example.

NICo uses the renderer defaults for sensor collection, SSE logs, GPU identity,
CPER diagnostics, and alert details.

Provision the shared JSON Secret and `backend.agentToken` directly or through
an external secrets operator. See [Credentials](credentials.md) for Vault and
OpenBao setup and credential reload behavior.

## Reported collector behavior

Every backend inventory batch includes `collector.version` and a typed
`collector.config` shared by its observations. `runtime` reports the effective
collection interval, concurrency, per-BMC limits, timeout, TLS and latency-probe
settings, and logging filter (including the `info` fallback).

Helm also passes the following reporting metadata to the collector:

| Helm value | Reported field | Environment variable |
| --- | --- | --- |
| `proxy.enabled` | `runtime.proxyEnabled` | `FLEETINT_PROXY_ENABLED` |
| `nicoHardwareHealth.enabled` | `nicoHealth.enabled` | `FLEETINT_NICO_ENABLED` |
| `nicoHardwareHealth.logLevel` | `nicoHealth.logLevel` | `FLEETINT_NICO_LOG_LEVEL` |
| NICo/gateway enablement | `nicoHealth.telemetryDestination` | `FLEETINT_NICO_TELEMETRY_DESTINATION` |
| `otelGateway.enabled` | `otelGateway.enabled` | `FLEETINT_OTEL_GATEWAY_ENABLED` |
| `otelGateway.logLevel` | `otelGateway.logLevel` | `FLEETINT_OTEL_GATEWAY_LOG_LEVEL` |

The telemetry destination is `disabled` when NICo is disabled, `bundledGateway`
when both services are enabled, and `external` when NICo uses `telemetry.endpoint`.
These values describe configured behavior, not service health. Gateway disablement
does not mean telemetry export is disabled. Proxy reporting describes Helm's
proxy setting; it does not inspect inherited proxy environment variables.

For non-Helm installations, unset reporting variables omit their corresponding
fields; an unset service enablement flag omits that service object. Explicit
`false` remains false. Boolean variables accept only lowercase `true` and `false`.
NICo log levels accept `trace`, `debug`, `info`, `warn`, `error`, and `off`;
gateway log levels accept `debug`, `info`, `warn`, and `error`. Reporting a
service's log level or telemetry destination requires its enablement flag.
Invalid or incomplete metadata stops `run` before enrollment without echoing
invalid values. These variables report settings; they do not configure the
separate NICo or gateway process. Restart the collector to refresh metadata.

The report excludes credentials, secret references, backend/proxy/telemetry
URLs, hardware target lists (already represented by inventory observations),
and deployment settings such as images, resources, replicas, scheduling, and
annotations. The backend stores the behavior configuration with each OOB
observation, including observations with collection errors. Deploy the matching
backend support before rolling out collectors that use the `collector` envelope.

## Proxy and TLS

Enable `proxy` with a cluster-reachable URL without credentials.
`bmcCollector.redfish.insecureTls` disables BMC certificate verification and
permits interception. It does not affect backend TLS or NICo; the pinned NICo
client accepts invalid Redfish certificates.
