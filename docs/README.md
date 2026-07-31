# Documentation

These guides separate user and operator workflows from the project overview in
the root README.

## Start here

- New local user: [Installation](installation.md) →
  [Configuration](configuration.md) → [Usage](usage.md)
- Kubernetes operator: [Configuration](configuration.md) →
  [Kubernetes deployment](kubernetes.md)
- Contributor: [Architecture](architecture.md) →
  [Contributing](../CONTRIBUTING.md)

## Guides

| Guide | Use it to |
| --- | --- |
| [Installation](installation.md) | Build from source, run a container, or locate published artifacts. |
| [Configuration](configuration.md) | Define BMC targets and labels, and configure TLS or proxies. |
| [Usage](usage.md) | Run inventory once, start backend sync, or render NICo configuration. |
| [Architecture](architecture.md) | Understand boundaries, data flows, concurrency, and partial failures. |
| [Kubernetes deployment](kubernetes.md) | Install and operate the complete Helm deployment. |

## Component references

The following documentation stays close to the component it describes:

- [Inventory collection notes](../crates/bmc-inventory/README.md) document
  mapping rules, node identity, domain errors, and platform policies.
- [Helm chart reference](../deployments/helm/fleet-intelligence-oob-collector/README.md)
  documents chart values and component-specific deployment details.
- [Local observability stack](../dev/local-observability/README.md) explains the
  development-only Grafana, Loki, Prometheus, and OpenTelemetry stack.
- [Integration tests](../tests/README.md) explains where behavioral tests and
  Redfish fixtures belong.

Project governance remains at the repository root:
[Contributing](../CONTRIBUTING.md), [Security](../SECURITY.md),
[Code of Conduct](../CODE_OF_CONDUCT.md), and [Changelog](../CHANGELOG.md).
