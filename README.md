# Fleet Intelligence Out-of-Band Collector

The Fleet Intelligence out-of-band (OOB) collector gathers hardware inventory and GPU attestation evidence from server BMCs over Redfish. Its Helm deployment includes NICo hardware health for BMC metrics and logs, and an authenticated OpenTelemetry gateway for telemetry export. Collection runs independently of the host operating system, without an in-band host agent.

## What it provides

- Inventory collection for systems, managers, chassis, processors, PCIe devices, memory modules, storage/drives, power supplies, firmware, fabrics, switches, and secure boot state.
- GPU attestation using backend-issued nonces and Redfish evidence collection.
- Per-BMC concurrency and rate limiting shared by inventory and attestation.
- [NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health) from the `infra-controller` health crate for metrics and BMC log collection.
- An authenticated OpenTelemetry gateway for telemetry export.
- An OCI Helm chart for Kubernetes deployment.

## How it works

The Helm chart deploys three components by default:

```text
OOB collector ──────────────► inventory and attestation APIs
      │
      └─ shared BMC config ─► NICo hardware health
                                      │
                               metrics and logs
                                      ▼
                               OTLP gateway ─► telemetry API
```

The OOB collector enrolls with Fleet Intelligence and uploads inventory from configured BMCs in batches. It preserves successfully collected data when an individual Redfish domain fails. GPU attestation uses backend-issued nonces and configured BMC MACs. Attestation needs the node's inventory to have been uploaded at least once. NICo reads the shared target configuration and BMC credentials to export metrics and logs through the gateway.

See [Architecture](docs/architecture.md) for collection, authentication, and delivery details.

## Getting started

- Follow [Installation](docs/installation.md) to deploy the Helm chart and verify collection.
- See [Configuration](docs/configuration.md) for targets, credentials, CLI options, and Helm values.
- See [Contributing](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/CONTRIBUTING.md#development-setup) to build and run from source.

## Documentation

Read the [collector documentation](https://docs.nvidia.com/fleet-intel/oob-collector) and the broader [Fleet Intelligence documentation](https://docs.nvidia.com/fleet-intel). The guides are also available in this repository:

- [Installation](docs/installation.md): Kubernetes setup, verification, and removal.
- [Configuration](docs/configuration.md): CLI commands, targets, and Helm values.
- [Credentials](docs/credentials.md): shared JSON credentials, secrets managers, and rotation.
- [Architecture](docs/architecture.md): components, collection lifecycle, and data flow.

Component references cover [inventory collection](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/README.md) and the [Helm chart](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/fleet-intelligence-oob-collector/README.md).

## Contributing

See [CONTRIBUTING.md](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/CONTRIBUTING.md) for development setup and guidelines.

## Security

Do not report vulnerabilities in public issues. Follow the [security policy](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/SECURITY.md) for private reporting instructions.

## License

This project is licensed under the Apache License 2.0. See the [LICENSE](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/LICENSE) file for details.
