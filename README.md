# Fleet Intelligence Out-of-Band Collector

The Fleet Intelligence out-of-band (OOB) collector gathers hardware inventory
and GPU attestation evidence from server BMCs over Redfish. Its Helm deployment
includes NICo hardware health for BMC metrics and logs, and an authenticated
OpenTelemetry gateway for telemetry export. Collection runs independently of
the host operating system, without an in-band host agent.

## What it provides

- Inventory collection for systems, managers, chassis, processors, PCIe
  devices, memory modules, storage/drives, power supplies, firmware, fabrics,
  switches, and secure boot state.
- GPU attestation using backend-issued nonces and Redfish evidence collection.
- Per-BMC concurrency and rate limiting shared by inventory and attestation.
- [NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health)
  from the `infra-controller` health crate for metrics and BMC log collection.
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

The OOB collector enrolls with Fleet Intelligence and uploads inventory from
configured BMCs in batches. It preserves successfully collected data when an
individual Redfish domain fails. GPU attestation uses backend-issued nonces
and configured BMC MACs. Attestation needs the node's inventory to have been
uploaded at least once. NICo reads the shared target configuration and BMC
credentials to export metrics and logs through the gateway.

See [Architecture](docs/architecture.md) for collection, authentication, and
delivery details.

## Getting started

For Kubernetes deployment, follow [Installation](docs/installation.md) to
configure targets, provision credentials, and install the Helm chart.

To build from source and collect inventory locally, use the steps below.

### Prerequisites

- Git
- [Rustup](https://rustup.rs/), which installs the toolchain pinned in
  `rust-toolchain.toml`
- A C linker for the host platform (for example, the toolchain provided by
  `build-essential` on Debian or Ubuntu)

Clone the repository, build the collector, and inspect its commands:

```bash
git clone https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector.git
cd fleet-intelligence-oob-collector
cargo run --locked -- --help
```

The final command prints the available `inventory`, `run`, and
`render-nico-config` commands without requiring access to a BMC or Fleet
Intelligence backend.

Collecting inventory requires a reachable Redfish BMC and its credentials.
Copy the example configuration, replace the example target and credential
paths, and run a local collection:

```bash
umask 077
cp config.example.toml config.toml
cp credentials.example.json credentials.json
chmod 600 config.toml credentials.json
```

Before running inventory, edit `config.toml` with your BMC targets and set
`credentials.file = "credentials.json"`. Edit `credentials.json` with your
BMC credentials and matching profiles or MAC overrides. The relative credential
path resolves beside `config.toml`. Keep both files private and outside commits.
Then collect inventory:

```bash
cargo run --locked -- inventory
```

See [Configuration](docs/configuration.md) for the complete configuration
contract and [Installation](docs/installation.md) for the supported Helm
deployment.

## Documentation

Read the [collector documentation](https://docs.nvidia.com/fleet-intel/oob-collector)
and the broader [Fleet Intelligence documentation](https://docs.nvidia.com/fleet-intel).
The guides are also available in this repository:

- [Installation](docs/installation.md): Kubernetes setup, verification, and removal.
- [Configuration](docs/configuration.md): CLI commands, targets, and Helm values.
- [Credentials](docs/credentials.md): shared JSON credentials, secrets managers, and rotation.
- [Architecture](docs/architecture.md): components, collection lifecycle, and data flow.

Component references cover [inventory collection](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/README.md),
the [Helm chart](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/fleet-intelligence-oob-collector/README.md),
and the experimental [host package](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/packages/README.md).

## Contributing

See [CONTRIBUTING.md](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/CONTRIBUTING.md) for development setup and guidelines.

## Security

Do not report vulnerabilities in public issues. Follow the
[security policy](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/SECURITY.md) for private reporting instructions.

## License

This project is licensed under the Apache License 2.0. See the
[LICENSE](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/LICENSE) file for details.
