# fleet-intelligence-oob-collector Helm chart

This chart deploys the OOB inventory and attestation collector, NICo hardware health, and a telemetry gateway. Follow the [installation guide](../../../docs/installation.md) to install it.

The chart is published as `fleet-intelligence-oob-collector` at `https://helm.ngc.nvidia.com/nvidia/fleet-intelligence`.

## Minimal values

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
  enrollmentToken:
    existingSecret: fleet-intelligence-enrollment-token
    secretKey: enrollment-token
```

Each target needs a unique BMC MAC and an IP address in `address`. Set `port` separately if the BMC doesn't use the HTTPS default. `kind` is `compute`, `nvswitch`, `power_shelf`, or `unknown` (the default).

The two Secrets hold the [BMC credentials file](../../../credentials.example.json) and the enrollment token. Create them by hand, or sync them from Vault or OpenBao with the [examples](../examples/) and the [credentials guide](../../../docs/credentials.md).

## Choosing components

| To run | Set |
| --- | --- |
| Everything (default) | Nothing |
| Inventory and attestation only | `nicoHardwareHealth.enabled: false` and `otelGateway.enabled: false` |
| Health telemetry only | `bmcCollector.enabled: false` |
| NICo with your own OTLP/gRPC collector | `telemetry.endpoint` and `otelGateway.enabled: false` |

The gateway requires NICo. NICo's init container uses the collector image even when the collector is disabled.

## How it works

With release name `fleet-intelligence-oob-collector`, the workloads are named `fleet-intelligence-oob-collector`, `nico-hardware-health`, and `fleet-intelligence-oob-collector-otel-gateway`.

The chart turns `inventory` and optional NICo collection settings into a collector TOML file. NICo's init container runs `bmc-collector render-nico-config` on that file to generate NICo's configuration in a pod-local volume; every target kind becomes a generic Redfish endpoint. The collector, the init container, and NICo all mount the same credentials file at `/run/secrets/bmc/credentials.json`.

Set optional flat fields under `nicoHardwareHealth.config` to configure NICo collection. See [NICo collection controls](../../../docs/configuration.md#nico-collection-controls) for supported fields and defaults.

Every workload accepts `enabled`, `image`, `resources`, `nodeSelector`, `tolerations`, `affinity`, `podAnnotations`, and `logLevel`. Collector and NICo image tags default to the chart version, and `imagePullSecrets` sets registry credentials.

Helm checks value types, label key syntax, MAC uniqueness, and component combinations when you install. The collector checks the remaining rules, such as reserved label names, label counts, and value lengths, when it loads the configuration, including in NICo's init container.

After you change values, run `helm upgrade`. See [rotation and ownership](../../../docs/credentials.md#rotation-and-ownership) for what restarts automatically and what doesn't, and [values.yaml](values.yaml) and the [configuration reference](../../../docs/configuration.md) for every setting.
