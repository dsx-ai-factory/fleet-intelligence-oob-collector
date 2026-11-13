# Configure Linux packages

Configure the DEB/RPM installation through files under `/etc/fleetint-oob/`. The OOB collector, NICo health service, and gateway run as `fleetint-oob`. Follow [Install on Linux](host-installation.md) for the initial setup and service-health checks. For Kubernetes, use the [Helm configuration guide](configuration.md).

## Configuration files and permissions

| File | Purpose | Ownership and mode |
| --- | --- | --- |
| `/etc/fleetint-oob/config.toml` | Shared targets, placement, credential-file path, and NICo controls | `root:fleetint-oob`, `0640` |
| `/etc/fleetint-oob/service.conf` | Backend/token settings and service environment variables | `root:root`, `0600`; systemd reads it before starting each service |
| `/etc/fleetint-oob/credentials/credentials.json` | Operator-created BMC credentials | `root:fleetint-oob`, `0640` |
| `/etc/fleetint-oob/agent-token` | Optional operator-created bootstrap token file | `root:fleetint-oob`, `0640` |
| `/etc/fleetint-oob/gateway.yaml` | OTLP receivers, backend export, and gateway processing | Installed root-owned, `0644` |

Keep `/etc/fleetint-oob/` and its `credentials/` directory owned by `root:fleetint-oob` with mode `0750`. Package hooks grant service access to standard configuration/credential paths during installation and upgrade. Package hooks leave credential symlink targets unchanged. At custom paths or symlink targets, you provide read access for `fleetint-oob`, including access through parent directories. The service sandbox hides home directories and gives services private temporary directories; place persistent service credentials under `/etc`, not a user's home or `/tmp`.

The package preserves modified configuration on upgrade. Use `sudoedit` to edit root-owned files. See [rotation and ownership](credentials.md#rotation-and-ownership) for credential reload behavior and restart requirements.

## Targets and placement

A minimal shared configuration is:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"
labels = { site = "dc1" }

[credentials]
file = "/etc/fleetint-oob/credentials/credentials.json"

[[targets]]
kind = "compute"
name = "node-01"
ip = "192.0.2.10"
mac = "02:00:00:00:00:01"
credentials_profile = "bmc"
labels = { rack = "rack-12" }
```

Replace the example values before starting services. Both the collector and NICo read this file.

| TOML field | Required | Description |
| --- | --- | --- |
| `credentials.file` | Yes | Path to the shared JSON credentials file. Relative paths resolve beside `config.toml`. |
| `targets` | Yes | One or more `[[targets]]` entries. |
| `targets[].ip` | Yes | BMC IPv4/IPv6 address, without scheme or port. The TOML name is `ip`; Helm uses `address`. |
| `targets[].mac` | Yes | Real BMC MAC address; duplicate MACs are rejected. |
| `targets[].credentials_profile` | Usually | Name of a credential profile. A per-MAC override can provide credentials instead. |
| `targets[].kind` | No | `compute`, `nvswitch`, `power_shelf`, or `unknown` (default). |
| `targets[].name` | No | Inventory display name and NICo `machine_name` label. |
| `targets[].port` | No | Redfish port; omission uses HTTPS port 443. |
| `compute_zone`, `node_group` | No | Defaults for placement; use names from your Fleet Intelligence account. |
| `targets[].compute_zone`, `targets[].node_group` | No | Override placement for one target. |
| `labels`, `targets[].labels` | No | Shared and per-target telemetry labels; a target value overrides the shared value. |

The shared rules for [target identity](configuration.md#target-identity), [placement](configuration.md#placement), and [label limits](configuration.md#labels) apply to TOML too. TOML uses snake_case field names. Keep passwords in the JSON credentials file.

## BMC credentials

For a target with `credentials_profile = "bmc"`, the credentials file can contain:

```json
{
  "profiles": {
    "bmc": {
      "username": "replace-with-bmc-user",
      "password": "replace-with-bmc-password"
    }
  }
}
```

Use the same file for all targets, with profiles and per-MAC overrides as needed. See the shared [credentials format and resolution rules](configuration.md#bmc-credentials-file) for the full reference. NICo's generated configuration points to this file; it does not copy passwords into its TOML.

## Fleet Intelligence connection

Set these variables in `service.conf`:

| Variable | Packaged default | Purpose |
| --- | --- | --- |
| `FLEETINT_BACKEND_URL` | `https://data.fleet-intelligence.nvidia.com` | Collector API origin, without an API path. |
| `BACKEND_ENDPOINT` | `https://data.fleet-intelligence.nvidia.com/api/v1/health` | Gateway telemetry base; exports use `/metrics` and `/logs` beneath it. |
| `ENROLL_ENDPOINT` | `https://data.fleet-intelligence.nvidia.com/api/v1/health/enroll` | Gateway enrollment URL. |
| `FLEETINT_AGENT_TOKEN_FILE` | Empty | Protected bootstrap-token file used by the collector and gateway. |
| `FLEETINT_AGENT_TOKEN` | Empty | Alternative inline environment-file token. Prefer the token file. |

When using another backend, set all three URL variables explicitly. Systemd environment files do not expand variable references, so `BACKEND_ENDPOINT=${FLEETINT_BACKEND_URL}/api/v1/health` does not construct a URL.

The token file takes precedence over `FLEETINT_AGENT_TOKEN`. An explicitly configured file that is unreadable or empty stops collector/gateway startup; it does not fall back to the inline token. The launchers load the token into the process environment so it stays out of command-line arguments. Follow [rotation and ownership](credentials.md#rotation-and-ownership) when changing these credentials.

## Collection settings

Set collector options in `/etc/fleetint-oob/service.conf`. The packaged services use Helm's collection and log-level defaults; you can change them without editing systemd commands.

| Variable in `service.conf` | Default | Helm equivalent | Effect |
| --- | --- | --- | --- |
| `FLEETINT_INTERVAL_SECONDS` | `86400` | `bmcCollector.intervalSeconds` | Seconds between collection cycles. `0` is treated as one second by the CLI. |
| `FLEETINT_TARGET_CONCURRENCY` | `64` | `bmcCollector.targetConcurrency` | Targets collected concurrently. Must be positive. |
| `FLEETINT_BMC_MAX_IN_FLIGHT` | `2` | `bmcCollector.redfish.maxInFlight` | Concurrent Redfish requests per BMC. Must be positive. |
| `FLEETINT_BMC_REQUESTS_PER_SECOND` | `10` | `bmcCollector.redfish.requestsPerSecond` | Per-BMC request rate and burst capacity. Must be positive. |
| `FLEETINT_TIMEOUT_SECONDS` | `30` | `bmcCollector.redfish.timeoutSeconds` | Redfish request timeout. Must be positive. |
| `FLEETINT_BMC_INSECURE_TLS` | `true` | `bmcCollector.redfish.insecureTls` | Skip BMC certificate verification. See [proxy and TLS](#proxy-and-tls). |
| `FLEETINT_DISABLE_LATENCY_PROBE` | `false` | Inverse of `bmcCollector.latencyProbe.enabled` | Set to `true` to disable collector TCP latency probes. |
| `RUST_LOG` | `info` | `bmcCollector.logLevel` | Collector log level. |
| `FLEETINT_NICO_LOG_LEVEL` | `warn` | `nicoHardwareHealth.logLevel` | NICo log level: `trace`, `debug`, `info`, `warn`, `error`, or `off`. |
| `FLEETINT_OTEL_GATEWAY_LOG_LEVEL` | `warn` | `otelGateway.logLevel` | Gateway log level: `debug`, `info`, `warn`, or `error`. |

Boolean settings accept `true` or `false`. Missing or empty launcher settings use their packaged defaults, including when an existing `service.conf` lacks a newly added variable. Invalid values stop startup with a configuration error. For example, to reduce BMC load and enable certificate verification:

```ini
FLEETINT_TARGET_CONCURRENCY=16
FLEETINT_BMC_MAX_IN_FLIGHT=1
FLEETINT_BMC_REQUESTS_PER_SECOND=5
FLEETINT_BMC_INSECURE_TLS=false
```

Restart the affected services as described in [apply changes](#apply-changes). Host file paths, loopback listeners, gateway memory limits, and systemd service enablement remain host settings; Kubernetes image, resource, and scheduling options apply to Helm deployments.

## NICo health collection

Add optional snake_case controls under `[nico_health]` in `config.toml`, before any `[[targets]]` entries. For example:

```toml
[nico_health]
sensor_fetch_interval_seconds = 120
bmc_request_concurrency = 2
leak_detector_poll_interval_seconds = 120
logs_mode = "auto"
logs_auto_retry_sse_after_downgrade = true
logs_sse_max_backoff_seconds = 60
```

The full [NICo collection controls](configuration.md#nico-collection-controls) table lists TOML names, defaults, and validation shared with Helm. Timing fields preserve a collector's enabled state; set its corresponding `*_enabled` field when enabling an otherwise disabled collector.

NICo regenerates its private configuration from `config.toml` every time its service starts. Restart NICo after changing these controls. Files under `/run/fleetint-nico-health/` are temporary and replaced on startup; edit the source TOML under `/etc`.

Resource-status settings live in `service.conf`:

| Variable | Default | Effect |
| --- | --- | --- |
| `FLEETINT_NICO_RESOURCE_STATUS_ENABLED` | `true` | Emit periodic System, Manager, Chassis, and FirmwareInventory status OTLP logs. Accepts `true` or `false`. |
| `FLEETINT_NICO_RESOURCE_STATUS_FIRMWARE_POLL_INTERVAL_SECONDS` | `1800` | FirmwareInventory status polling interval in seconds; must be positive. |

System and Chassis status follow discovery cadence; Manager status follows manager cadence. Enabling resource-status processing forces firmware collection on at the environment-variable cadence. When it is disabled, `[nico_health].firmware_enabled` and `firmware_refresh_interval_seconds` control firmware collection.

## Gateway and local listeners

| Variable in `service.conf` | Default | Purpose |
| --- | --- | --- |
| `FLEETINT_OTLP_ENDPOINT` | `http://127.0.0.1:4317` | NICo's metrics/logs destination. |
| `FLEETINT_METRICS_ENDPOINT` | `127.0.0.1:9009` | NICo's metrics and health listener. |

The packaged `gateway.yaml` listens on loopback: OTLP gRPC at `127.0.0.1:4317`, OTLP HTTP at `127.0.0.1:4318`, and gateway health at `127.0.0.1:13133`. NICo's Prometheus endpoint is `http://127.0.0.1:9009/metrics`. Keep these local listeners private to the host.

Edit `gateway.yaml` when changing the gateway's own listener addresses or processing/export configuration. Set its log level through `FLEETINT_OTEL_GATEWAY_LOG_LEVEL` in `service.conf`; this takes precedence over the log level in `gateway.yaml`, including in preserved local configurations. Changing `FLEETINT_OTLP_ENDPOINT` changes NICo's destination only; it does not change a gateway listener. The gateway uses the same bootstrap token as the collector but enrolls separately.

### Component reporting

Systemd reads `service.conf` before starting each service and passes its values through the process environment. The collector launcher turns the collection settings into CLI arguments, and the collector includes supported configuration fields in its inventory uploads under `collector.config`.

| Report section | Included settings |
| --- | --- |
| `collector.config.runtime` | Collection interval, target concurrency, per-BMC concurrency, request rate and burst capacity, timeout, BMC certificate-verification bypass, latency probes, and collector log level. |
| `collector.config.nicoHealth` | Component enablement, log level, telemetry destination, and explicitly configured `[nico_health]` controls in camelCase. |
| `collector.config.otelGateway` | Component enablement and log level. |

The runtime report describes the effective collector arguments. Component log levels are reported when their environment variables are present; keep `FLEETINT_NICO_LOG_LEVEL` and `FLEETINT_OTEL_GATEWAY_LOG_LEVEL` explicitly set in `service.conf` for complete reports. Only supported, non-sensitive settings are included; tokens and passwords are excluded. Restart affected services after changing their environment settings so the running configuration and subsequent reports use the new values.

These environment variables describe the deployment in the collector's backend report:

| Variable | Packaged default |
| --- | --- |
| `FLEETINT_NICO_ENABLED` | `true` |
| `FLEETINT_NICO_TELEMETRY_DESTINATION` | `bundledGateway` |
| `FLEETINT_OTEL_GATEWAY_ENABLED` | `true` |

Changing reporting metadata does not start or stop a service. When omitting a component, configure service startup separately and update this metadata to match. Use `external` as NICo's telemetry destination when sending to your own gateway, or `disabled` when omitting NICo. The default `fleetint-oob.target` requests all three services, so excluding a component also requires adapting that systemd arrangement.

## Proxy and TLS

Set `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY` in `service.conf` when needed. Keep loopback traffic outside the proxy and add the BMC addresses or subnets that should be reached directly, for example:

```ini
HTTPS_PROXY=http://proxy.example.com:3128
NO_PROXY=localhost,127.0.0.1,192.0.2.10,192.0.2.11
```

The packaged collector uses HTTPS and skips BMC certificate verification by default, matching Helm and allowing collection from BMCs with self-signed certificates. To verify BMC certificates, set `FLEETINT_BMC_INSECURE_TLS=false` in `service.conf` and ensure the host trusts the BMC certificates. Restart the collector after changing this setting.

When running the collector CLI directly, certificate verification is enabled unless you pass `--insecure-tls`. NICo's BMC certificate handling is separate and does not verify BMC certificates. Backend TLS verification remains enabled for the collector and gateway.

## Apply changes

Use `sudo systemctl restart fleetint-oob.target` after changing shared configuration or environment settings. For changes limited to NICo controls, restart only NICo with `sudo systemctl restart fleetint-nico-health.service`. Check the individual service states and logs afterward, as described in [installation verification](host-installation.md#start-and-verify-collection).

For password and token rotation, follow [rotation and ownership](credentials.md#rotation-and-ownership). Changes to systemd unit files or drop-ins also require `sudo systemctl daemon-reload` before restarting the affected services.

## Troubleshooting

| Problem | What to check |
| --- | --- |
| Service stops with exit status `2` | Invalid settings or an unreadable token file. Check the service log, correct the setting, and restart the affected service. |
| Collector repeatedly fails to enroll | Bootstrap token, backend addresses, host connectivity, proxy, and backend certificate trust. |
| Inventory cannot read a BMC | Target address/MAC, credential profile or MAC override, and BMC certificate trust if you enabled verification. See [proxy and TLS](host-configuration.md#proxy-and-tls). |
| NICo fails before starting | TOML syntax, a readable credentials file, and valid `[nico_health]` settings. |
| Services run but telemetry is absent | BMC credentials, NICo logs, the OTLP destination, all gateway URLs, and gateway enrollment/export errors. |

Optional local health checks are available at `http://127.0.0.1:9009/livez` for NICo and `http://127.0.0.1:13133/` for the gateway. Confirm fresh inventory and telemetry in Fleet Intelligence to verify collection and export.

## Upgrade

Download the new package from [GitHub Releases](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/releases), then follow the [installation commands](host-installation.md#install) with the new file. Modified package configuration is preserved. Review package-manager prompts and merge any new configuration defaults into your local settings as needed.

An upgrade restarts services that were already active and leaves stopped services stopped. Review each service after the transaction. Service-management failures are reported as warnings; a completed package transaction alone does not prove that a restart succeeded.

## Remove a target

Before deleting a machine in Fleet Intelligence, remove its `[[targets]]` entry from `/etc/fleetint-oob/config.toml` and restart the stack:

```bash
sudo systemctl restart fleetint-oob.target
```

Confirm that collection continues for the remaining targets. A few uploads already in progress can still arrive. If you remove the last target, stop or uninstall the stack instead of starting it with an empty target list. Remove unused per-MAC credential overrides only after nothing else uses them. Existing backend data remains.

## Stop or uninstall

Stop collection and disable startup:

```bash
sudo systemctl disable --now fleetint-oob.target \
  fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service
```

Stopping the collector can take up to 300 seconds while it finishes pending inventory uploads. Remove the package with your package manager:

For DEB systems:

```bash
sudo apt remove fleetint-oob
```

For RPM systems:

```bash
sudo dnf remove fleetint-oob
```

Debian removal retains package configuration; `sudo apt purge fleetint-oob` removes package-owned configuration. RPM removal saves modified package configuration as `.rpmsave` files. Operator-created credential/token files and the service account remain. Retire credentials through their issuing systems, then delete unused local copies explicitly. Removing the package does not delete Fleet Intelligence data.
