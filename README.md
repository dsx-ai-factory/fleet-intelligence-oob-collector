# Fleet Intelligence Out-of-Band Collector

The Fleet Intelligence out-of-band (OOB) collector gathers inventory,
attestation, metrics, and logs from server BMCs over Redfish without requiring
an in-band host agent. It is designed for environments where hardware
information must be collected independently of the host operating system.

## What it provides

- Inventory collection for systems, managers, chassis, processors, PCIe
  devices, memory modules, storage/drives, power supplies, firmware, and secure boot state.
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

The OOB collector enrolls with Fleet Intelligence and collects inventory from
configured BMCs concurrently. Completed node inventories enter a bounded
delivery queue and are uploaded in small batches, without waiting for every BMC
to finish. Attestation then runs for successfully exported nodes with resolved
identities and uploads results in batches bounded by the target concurrency.
NICo uses the same BMC configuration to emit hardware metrics and logs through
the gateway. Partial inventory failures are preserved in the output so
unavailable Redfish domains do not erase otherwise valid data.

## Getting started

### Prerequisites

- Git
- [Rustup](https://rustup.rs/), which installs the toolchain pinned in
  `rust-toolchain.toml`

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
cp config.example.toml .config
chmod 600 .config
cargo run --locked -- inventory --config .config
```

See [Configuration](docs/configuration.md) for the complete configuration
contract and [Installation](docs/installation.md) for the supported Helm
deployment.

## Documentation

Full documentation is available at **[docs.nvidia.com/fleet-intel/oob-collector](https://docs.nvidia.com/fleet-intel/oob-collector)**, including installation guides, configuration reference, and architecture overview.

For the broader Fleet Intelligence platform documentation, see [docs.nvidia.com/fleet-intel](https://docs.nvidia.com/fleet-intel).

Component-specific details are available in the
[inventory collection notes](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/crates/bmc-inventory/README.md),
and [Helm chart reference](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/fleet-intelligence-oob-collector/README.md),

## Contributing

See [CONTRIBUTING.md](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/CONTRIBUTING.md) for development setup and guidelines.

## Security

Do not report vulnerabilities in public issues. Follow the
[security policy](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/SECURITY.md) for private reporting instructions.

## License

This project is licensed under the Apache License 2.0. See the
[LICENSE](LICENSE) file for details.
