# Fleet Intelligence Out-of-Band Collector

The Fleet Intelligence out-of-band (OOB) collector gathers inventory,
attestation, metrics, and logs from server BMCs over Redfish. It operates
without an in-band host agent and keeps Fleet Intelligence data models isolated
from vendor-specific Redfish representations.

The project provides:

- A Rust CLI for local inventory inspection and continuous backend sync.
- BMC inventory and GPU attestation collection.
- A Helm chart that can also deploy NICo hardware health and an authenticated
  OpenTelemetry gateway.
- Multi-architecture `amd64` and `arm64` container images.

## Quick start

Build the collector and create a local configuration:

```bash
cargo build --workspace
cp config.example.toml config.toml
chmod 600 config.toml
```

Replace the example BMC address and credentials, then inspect inventory:

```bash
target/debug/bmc-collector inventory \
  --config config.toml \
  --format json
```

Use `--insecure-tls` only for BMCs whose certificates cannot be validated.
See [Installation](docs/installation.md) and [Usage](docs/usage.md) for the
container, agent, and Kubernetes workflows.

## Documentation

| Guide | Contents |
| --- | --- |
| [Documentation index](docs/README.md) | Guide map and suggested reading paths |
| [Installation](docs/installation.md) | Source builds, binaries, containers, and prerequisites |
| [Configuration](docs/configuration.md) | BMC targets, labels, credentials, TLS, and proxies |
| [Usage](docs/usage.md) | CLI commands, output, runtime behavior, and logging |
| [Architecture](docs/architecture.md) | Components, data flows, identity, and failure handling |
| [Kubernetes deployment](docs/kubernetes.md) | Helm installation, chart components, and operations |
| [Contributing](CONTRIBUTING.md) | Development workflow, tests, and pull requests |

Component-specific notes live beside their implementation:

- [Inventory collection behavior](crates/bmc-inventory/README.md)
- [Helm chart reference](deployments/helm/fleet-intelligence-oob-collector/README.md)
- [Local observability stack](dev/local-observability/README.md)
- [Integration test layout](tests/README.md)

## Security

The configuration contains BMC credentials. Keep it out of source control,
restrict its file permissions, and use existing Kubernetes Secrets for
deployments. Do not report vulnerabilities in public issues; follow
[SECURITY.md](SECURITY.md).

## License

This project is licensed under Apache-2.0. See [LICENSE](LICENSE).
