# Fleet Intelligence OOB Collector

Deploys the inventory/attestation collector, NICo hardware health, and an OTLP
gateway. See the [installation guide](../../../docs/installation.md) for setup.

## Configuration

```yaml
inventory:
  targets:
    - kind: compute
      address: 192.0.2.10
      mac: "00:11:22:33:44:55"
      credentialsProfile: bmc
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
backend:
  url: https://data.fleet-intelligence.nvidia.com
  agentToken:
    existingSecret: fleet-intelligence-oob-collector-agent-token
    secretKey: agent-token
```

The collector and NICo use the same inventory and
[JSON credential file](../../../credentials.example.json). Credential fields
resolve from the MAC override, then the selected profile.
Provision these Secrets directly or synchronize them from Vault/OpenBao using
the separately applied [examples](../examples/). Follow the
[credentials guide](../../../docs/credentials.md) for setup.

Targets require a unique MAC and an IP address in `address`; an optional
`port` is a separate integer field. Kinds are `compute`, `nvswitch`,
`power_shelf`, and `unknown` (default). Helm renders `collector.toml` from this
inventory. The NICo init container runs `bmc-collector render-nico-config` against
that file; all target kinds become generic static Redfish endpoints. The
collector, init container, and NICo mount the same JSON credentials file at
`/run/secrets/bmc/credentials.json`. NICo TOML stays in a pod-local `emptyDir`
volume.

Each workload supports `enabled`, `image`, `resources`, `nodeSelector`,
`tolerations`, `affinity`, `podAnnotations`, and `logLevel`. Collector and NICo
image tags default to the chart version. Registry auth uses `imagePullSecrets`.

The bundled gateway uses `backend.url` and `backend.agentToken`. For an external
OTLP/gRPC destination, set `telemetry.endpoint` and disable `otelGateway`.
Disable NICo and the gateway for inventory and attestation only, or disable the
collector for health telemetry only. NICo uses the collector image for its
configuration init container even when `bmcCollector.enabled` is `false`. The
gateway requires NICo to be enabled.

After changing values, run `helm upgrade`. Target TOML changes roll the collector
and NICo; gateway configuration changes roll the gateway. Pod-template changes
roll the affected workloads. BMC credential file values are read on each
collector cycle and reloaded by NICo without restarting. Bootstrap token value
changes require restarting the enabled collector and gateway.

See [values.yaml](values.yaml) for defaults and the
[configuration reference](../../../docs/configuration.md#helm-values) for
collection limits, telemetry destinations, and TLS behavior. Helm validates
value types, label-key syntax, MAC uniqueness, and workload combinations. The
collector validates reserved label names,
counts, and value lengths when loading configuration, including in the NICo
init container.
