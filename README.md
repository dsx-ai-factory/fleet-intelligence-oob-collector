# Fleet Intelligence Out-of-Band Collector

The Fleet Intelligence out-of-band (OOB) collector gathers inventory,
attestation, metrics, and logs from server BMCs over Redfish without requiring
an in-band host agent.

## Documentation

- [Installation](docs/installation.md)
- [Configuration](docs/configuration.md)
- [Usage and behavior](docs/usage.md)
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
