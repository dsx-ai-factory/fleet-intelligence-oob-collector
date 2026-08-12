# Configuration

The collector and NICo hardware health share one TOML file. Start from
`config.example.toml` and define one or more BMCs:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"

[credentials]
username_file = "/run/secrets/bmc/default/username"
password_file = "/run/secrets/bmc/default/password"

[[machines]]
name = "node-01"
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
labels = { site = "dev", environment = "development" }

[[machines]]
name = "node-02"
ip = "10.0.0.11"
username_file = "/run/secrets/bmc/node-02/username"
password_file = "/run/secrets/bmc/node-02/password"
```

The first machine inherits the top-level credential files. The second machine
overrides both files. Unknown fields, an empty machine list, incomplete or mixed
credential pairs, blank credentials, and duplicate MACs are rejected.

## Collector TOML

| Field | Required | Description |
| --- | --- | --- |
| Top-level `compute_zone` | No | Default Fleet Intelligence compute zone inherited by machines and mirrored to telemetry as `compute_zone`. |
| Top-level `node_group` | No | Default Fleet Intelligence node group inherited by machines and mirrored to telemetry as `node_group`. |
| Top-level `labels` | No | Labels inherited by every machine. Per-machine labels with the same key override them. |
| `credentials.username_file` | Conditional | Default username file inherited by machines; must be paired with `password_file`. |
| `credentials.password_file` | Conditional | Default password file inherited by machines; must be paired with `username_file`. |
| `machines[].name` | No | Human-readable name exported as inventory `name` and the NICo `machine_name` label. |
| `machines[].compute_zone` | No | Per-machine compute-zone override. |
| `machines[].node_group` | No | Per-machine node-group override. |
| `machines[].ip` | Yes | IPv4 or IPv6 address of the BMC. |
| `machines[].port` | No | Redfish port; the HTTPS default is used when omitted. |
| `machines[].mac` | No | Stable BMC identity. It must be unique in the file. |
| `machines[].username_file` | Conditional | Per-machine username file override; must be paired with `password_file`. |
| `machines[].password_file` | Conditional | Per-machine password file override; must be paired with `username_file`. |
| `machines[].username` | Conditional | Inline Redfish username; must be paired with inline `password`. |
| `machines[].password` | Conditional | Inline Redfish password; must be paired with inline `username`. |
| `machines[].labels` | No | Labels added to this machine or overriding inherited top-level labels. |

When `mac` is omitted, the collector derives a locally administered MAC from
`02:00` and the final four IP bytes. Because different IPv6 addresses can share
the same final four bytes, configure an explicit, unique MAC for IPv6 targets.
An explicit stable MAC is preferred for IPv4 targets as well.

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

Set them at the top level when every configured machine belongs to the same
Fleet Intelligence placement. Override them directly on a machine when its
placement differs:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"

[[machines]]
name = "node-01"
ip = "10.0.0.10"

[[machines]]
name = "node-02"
ip = "10.0.0.11"
compute_zone = "us-west-dc2"
node_group = "gb200-inference"
```

In this example, `node-01` inherits the `us-west-dc1` compute zone and the
`gb200-training` node group. `node-02` belongs to the `us-west-dc2` compute zone
and the `gb200-inference` node group. If either field is omitted from both
scopes, its corresponding Fleet Intelligence inventory placement field and
telemetry label are omitted. Custom labels are not promoted to Fleet
Intelligence placement fields.

Relative credential paths are resolved from the directory containing the
collector configuration. A machine must provide a complete file pair, a
complete inline pair, or inherit a complete top-level file pair.
Per-machine credentials override the top-level pair as a unit; sources cannot
be mixed.

Inline credentials are supported for direct collector use, but they must not be
used in the configuration referenced by the Helm chart. That configuration is
stored in a ConfigMap, which is not intended for sensitive data. Helm
deployments must use credential paths and store the credential values in
Kubernetes Secrets. See
[installation](installation.md#create-configmap-and-secrets).

## Helm values

The commonly configured chart values are:

| Value | Default | Description |
| --- | --- | --- |
| `config.configMapName` | `fleet-intelligence-config` | Existing ConfigMap containing the password-free TOML file. |
| `config.configKey` | `config.toml` | Key containing the TOML file. |
| `credentials.secrets` | `[]` | Existing BMC credential Secrets projected under `/run/secrets/bmc/<target>`. |
| `bmcCollector.backendUrl` | `https://data.fleet-intelligence.nvidia.com` | API base URL without `/v1/agent`. |
| `bmcCollector.agentTokenSecretName` | `fleet-intelligence-oob-collector-agent-token` | Existing Secret containing `agent-token`. |
| `bmcCollector.intervalSeconds` | `86400` | Collection interval. |
| `bmcCollector.insecureTls` | `false` | Disable BMC certificate verification. |
| `bmcCollector.logLevel` | `info` | Rust tracing filter. |
| `bmcCollector.targetConcurrency` | `64` | Maximum concurrent inventory or attestation target workflows. |
| `bmcCollector.bmcMaxInFlight` | `4` | Maximum concurrent Redfish requests to each BMC. |
| `bmcCollector.bmcRequestsPerSecond` | `10` | Sustained Redfish request rate for each BMC. |
| `bmcCollector.bmcBurst` | `10` | Per-BMC Redfish request burst capacity. |
| `bmcCollector.timeoutSeconds` | `30` | Timeout for each Redfish request. |
| `nicoHardwareHealth.enabled` | `true` | Deploy NICo metrics and log collection. |
| `nicoHardwareHealth.metricsPrefix` | `fleetint` | NICo metric prefix. |
| `otelGateway.enabled` | `true` | Deploy the authenticated telemetry gateway. |

See the chart's
[values.yaml](../deployments/helm/fleet-intelligence-oob-collector/values.yaml)
for the full value reference. To use an external OTLP/gRPC collector, disable
`otelGateway.enabled` and set `nicoHardwareHealth.otlpEndpoint`. To run only
inventory and attestation, disable both NICo and the gateway.

## Proxy and TLS

Configure a cluster-reachable proxy with:

```yaml
bmcCollector:
  proxy:
    httpProxy: http://proxy.example.internal:3128
    httpsProxy: http://proxy.example.internal:3128
    allProxy: ""
    noProxy: localhost,127.0.0.1,.svc,.cluster.local
```

These values are passed to the collector and gateway. Do not commit
authenticated proxy URLs. Set `bmcCollector.insecureTls=true` only when BMC
certificates cannot be validated; it does not disable backend TLS validation.
