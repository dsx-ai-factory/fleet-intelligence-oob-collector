# Configuration

The collector and NICo hardware health share one TOML file. Start from
`config.example.toml` and define one or more BMCs:

```toml
[labels]
compute_zone = "az51"
node_group = "dev3-dh1"

[[machines]]
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
username = "bmc-user"
password = "bmc-password"
labels = { site = "dev", environment = "development" }
```

Unknown fields, an empty machine list, blank credentials, and duplicate MACs
are rejected.

## Collector TOML

| Field | Required | Description |
| --- | --- | --- |
| Top-level `labels` | No | Labels applied to every machine. |
| `machines[].ip` | Yes | IPv4 or IPv6 address of the BMC. |
| `machines[].port` | No | Redfish port; the HTTPS default is used when omitted. |
| `machines[].mac` | No | Stable BMC identity. It must be unique in the file. |
| `machines[].username` | Yes | Redfish username. |
| `machines[].password` | Yes | Redfish password. |
| `machines[].labels` | No | Labels added to this machine or overriding shared labels. |

When `mac` is omitted, the collector derives a locally administered MAC from
`02:00` and the final four IP bytes. An explicit stable MAC is preferred.

Label keys must match `[a-zA-Z_][a-zA-Z0-9_]*`. Each machine supports 31
user-defined labels with values up to 1,024 bytes. NICo identity fields and the
collector-added `oob="true"` label are reserved. Effective `compute_zone` and
`node_group` labels are also exported as backend placement fields.

The file contains clear-text BMC credentials. Keep it out of source control and
store it in the Kubernetes Secret created during
[installation](installation.md#create-secrets).

## Helm values

The commonly configured chart values are:

| Value | Default | Description |
| --- | --- | --- |
| `config.secretName` | `fleet-intelligence-config` | Existing Secret containing the TOML file. |
| `config.secretKey` | `config.toml` | Key containing the TOML file. |
| `bmcCollector.backendUrl` | Fleet Intelligence production URL | API base URL without `/v1/agent`. |
| `bmcCollector.agentTokenSecretName` | `fleet-intelligence-oob-collector-agent-token` | Existing Secret containing `agent-token`. |
| `bmcCollector.intervalSeconds` | `86400` | Collection interval. |
| `bmcCollector.insecureTls` | `false` | Disable BMC certificate verification. |
| `bmcCollector.logLevel` | `info` | Rust tracing filter. |
| `nicoHardwareHealth.enabled` | `true` | Deploy NICo metrics and log collection. |
| `nicoHardwareHealth.metricsPrefix` | `fleetint` | NICo metric prefix. |
| `otelGateway.enabled` | `true` | Deploy the authenticated telemetry gateway. |

See `deployments/helm/fleet-intelligence-oob-collector/values.yaml` for the full
value reference. To use an external OTLP/gRPC collector, disable
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
