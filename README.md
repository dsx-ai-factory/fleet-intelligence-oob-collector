# Fleet Intelligence Out-of-Band Collector

The Fleet Intelligence out-of-band (OOB) collector gathers inventory,
attestation, metrics, and logs from server BMCs over Redfish without requiring
an in-band host agent. It is designed for environments where hardware
information must be collected independently of the host operating system.

## What it provides

- Inventory collection for systems, managers, chassis, processors, PCIe
  devices, firmware, and secure boot state.
- GPU attestation using backend-issued nonces and Redfish evidence collection.
- Per-BMC concurrency and rate limiting shared by inventory and attestation.
- [NICo hardware health](https://github.com/NVIDIA/infra-controller/tree/main/crates/health)
  from `infra-controller` for metrics and BMC log collection.
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

The OOB collector enrolls with Fleet Intelligence, collects inventory from all
configured BMCs, uploads the inventory batch, and then collects attestation for
nodes with resolved identities. NICo uses the same BMC configuration to emit
hardware metrics and logs through the gateway. Partial inventory failures are
preserved in the output so unavailable Redfish domains do not erase otherwise
valid data.

## Documentation

- [Installation](docs/installation.md)
- [Configuration](docs/configuration.md)
- [Architecture](docs/architecture.md)
- [Contributing](CONTRIBUTING.md)

Component-specific details are available in the
[inventory collection notes](crates/bmc-inventory/README.md),
[Helm chart reference](deployments/helm/fleet-intelligence-oob-collector/README.md),
and [local observability guide](dev/local-observability/README.md).

## Security

Do not report vulnerabilities in public issues. Follow
[SECURITY.md](SECURITY.md) for private reporting instructions.

## License

This project is licensed under Apache-2.0. See [LICENSE](LICENSE).
