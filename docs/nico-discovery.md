# Discover targets through NICo REST

Use NICo REST to discover the BMC addresses and MAC addresses of compute hosts in one site. In NICo integration mode, the collector collects inventory and attestation through the NICo BMC proxy using its own client certificate. The proxy supplies BMC credentials and enforces authorization and concurrency controls. NICo inventory and attestation records are not imported.

## Prepare access

You need an HTTPS NICo REST endpoint trusted by the collector's system CA store, an organization and site ID, and a bearer access token with the `carbide` scope and provider/site permissions to read machine BMC metadata. Confirm the exact permissions with your NICo administrator. The token file contains an access token, not an SSA client secret; arrange token refresh outside the collector.

Ask your NICo administrator to issue a client certificate for the FI service identity and authorize `GET /redfish/v1/**` and `POST /redfish/v1/ComponentIntegrity/*/Actions/ComponentIntegrity.SPDMGetSignedMeasurements`. Mount the client certificate, private key, and proxy CA bundle as files readable only by the collector. The collector needs network access to NICo REST and the proxy; it requires no BMC password. NICo REST uses a bearer token, while the proxy uses mutual TLS.

## Select the source

Create a TOML configuration with shared collection settings and an `endpoint_source` section. Omit `[[targets]]` when you select NICo.

```toml
compute_zone = "us-west-dc1"
node_group = "training"
labels = { site = "dc1" }

[nico_bmc_proxy]
url = "https://nico-bmc-proxy.forge-system.svc.cluster.local:1079"
ca_file = "proxy-identity/ca.crt"
client_cert_file = "proxy-identity/tls.crt"
client_key_file = "proxy-identity/tls.key"

[endpoint_source]
type = "nico"
api_url = "https://nico.example.com/v2"
org = "example-org"
site_id = "00000000-0000-0000-0000-000000000001"
token_file = "nico-access-token"
api_name = "nico"
timeout_seconds = 30
max_stale_seconds = 108000
```

| Field | Value |
| --- | --- |
| `type` | `nico` |
| `api_url` | HTTPS REST prefix including `/v2`; exclude organization and API-name segments |
| `org` | NICo organization name |
| `site_id` | Site to discover |
| `token_file` | File containing only the bearer access token |
| `api_name` | `nico` by default; use `carbide` for deployments with that REST route |
| `credentials_profile` | Omit in proxy mode; optional shared BMC credential profile for direct connections |
| `timeout_seconds` | Whole discovery deadline, including both attempts and all pages; default `30`, allowed `1`–`3600` |
| `max_stale_seconds` | Maximum age of a last-good discovery used by `run` during NICo outages; default `108000` (30 hours), allowed `1`–`172800` (48 hours) |

Relative token and TLS identity paths resolve beside the TOML file. Replace the token file atomically when rotating it; the collector rereads it on each discovery. Mount TLS identity directories without `subPath` so Kubernetes Secret updates reach the collector. Each target collection attempt loads the current certificate and key. Invalid or unreadable identity files stop that attempt. Proxy CA and hostname verification remain enabled even with `--insecure-tls`.

Proxy mode routes inventory, attestation actions, task polling, and evidence reads through the configured HTTPS proxy. The discovered BMC IP remains the target identity and is sent in the `Forwarded` header. The configured BMC MAC remains the identity used for inventory and backend nonce requests. Direct BMC latency probes are disabled. Omit `[credentials]` and `credentials_profile` in this mode; proxy targets use HTTPS port 443.

For installations that explicitly use direct BMC access, replace `[nico_bmc_proxy]` with `[credentials]` and set `file = "credentials.json"`; see the [shared credentials file](credentials.md).

Configure placement with the shared `compute_zone` and `node_group` settings and add collection labels with the shared `labels` setting. NICo machine labels are ignored, including labels named `compute_zone` and `node_group`. The source supplies the machine hostname when available. NICo rack slot/tray placement is not mapped to Fleet placement automatically. It lists compute hosts; switches and power shelves require other sources. NICo REST does not expose a BMC port in this response, so discovered targets use the default Redfish HTTPS port, `443`.

## Verify discovery and collect

Use the installed binary to list discovered endpoints without reading BMC passwords or querying hardware:

```bash
bmc-collector discover --config /private/path/config.toml
```

From a source checkout, run `cargo run --locked --bin bmc-collector -- discover --config /private/path/config.toml`.

The JSON output contains the machine reference, IP, MAC, name, kind, and credential profile reference, before shared collection normalization. Source labels are empty and source placement fields are null; your shared settings apply during collection. The command always checks live discovery and does not use cached endpoints. Treat the output as infrastructure information.

To collect inventory with the same configuration:

```bash
cargo run --locked --bin bmc-collector -- inventory --config /private/path/config.toml
```

The `run` command also accepts this configuration; see [running the collector](../CONTRIBUTING.md#running-the-collector). Discovery runs when the collection cycle loads its configuration. Inventory and attestation share the resolved target list for that cycle. There is no separate discovery refresh loop.

## Deployment support

### Kubernetes: collector only

Enable `nicoIntegration` to deploy the FI collector against the existing site NICo services. This mode omits the bundled NICo hardware-health and OTLP gateway workloads. It leaves the site's NICo hardware-health configuration unchanged. Configure telemetry forwarding from existing site services separately.

Create two Secrets in the collector namespace: one containing a refreshed NICo REST `access-token`, and one containing `ca.crt`, `tls.crt`, and `tls.key` for the proxy client identity. Your platform can manage the identity Secret through cert-manager. Provision and refresh the REST access token outside the collector; the chart does not exchange SSA client secrets.

```yaml
nicoIntegration:
  enabled: true
  apiUrl: https://nico.example.com/v2
  org: example-org
  siteId: 00000000-0000-0000-0000-000000000001
  tokenSecretName: fi-nico-rest-token
  proxyUrl: https://nico-bmc-proxy.forge-system.svc.cluster.local:1079
  identitySecretName: fi-proxy-identity
```

Omit `inventory.targets` and BMC credential configuration. Keep your normal FI backend URL and enrollment Secret settings. The chart mounts the REST token and proxy identity read-only and verifies proxy TLS. See [configuration](configuration.md) for the remaining deployment settings.

### Linux packages

Place the discovery and proxy configuration in a protected file, set `FLEETINT_COLLECTOR_CONFIG` to its path in `/etc/fleetint-oob/service.conf`, and ensure `fleetint-oob` can read the token and identity files. Enable only `fleetint-oob-collector.service` when using the site's existing NICo health deployment. Keep the bundled NICo health and gateway services disabled. The `render-nico-config` command requires a static direct-BMC configuration and rejects proxy mode.

## Troubleshoot discovery

| Problem | What to check |
| --- | --- |
| HTTP `401` or `403` | Token expiry, `carbide` scope, and provider/site permissions |
| HTTP `404` | REST prefix and `api_name` for your deployment |
| Skipped machines | Warning logs name up to 20 machine IDs and give total skipped/usable counts; check onboarding, metadata permissions, and BMC IP/MAC availability |
| Inconsistent pages | Discovery retries the complete listing once within the same deadline; retry later if the site keeps changing |
| No targets | Selected site and host visibility; empty listings are rejected |
| Timeout | Connectivity, CA trust, site size, and the whole-discovery deadline |
| Unknown BMC profile | Shared BMC credentials file; this is separate from the REST access token |

Discovery requests non-missing machines in `Ready`, `InUse`, `Initializing`, `Error`, `Maintenance`, `Reset`, and `Unknown` states; decommissioning and decommissioned machines are excluded. Machines without usable BMC metadata are skipped, with individual warnings for the first 20 and a summary count. If no usable machines remain, discovery fails rather than reusing removed membership. Duplicate BMC MACs or addresses fail discovery and name both machines.

Every page must be fetched successfully. Discovery requests 100 machines per page, allows at most 1,000 pages, and limits each page body to 8 MiB. Changed totals, incomplete pages, or repeated machine IDs trigger one restart from page 1 within the original deadline. These checks cannot provide an atomic view of a changing site.

## Outages and last-good discovery

The long-running `run` command retains the last successfully resolved, credential-free endpoint listing in memory. If the same NICo source becomes unavailable because of a token-file read failure, HTTP `401`/`403`/`429`, server error, connection failure, or timeout, it can reuse that list until `max_stale_seconds` has elapsed since the successful discovery. Warnings report the cache age and target count. Reusing a list does not extend its lifetime, and collection authentication files are reloaded for new attempts.

The default 30-hour bound allows a failed discovery at the default daily collection interval. Choose a shorter limit when address reuse or rapid machine turnover makes stale membership unacceptable. Missing-machine filters apply to live discovery; a cached list cannot detect changes made during an outage and cannot guarantee an IP still belongs to the same BMC. The age check is made before a collection cycle starts.

No cache is written to disk. After a restart, or after the bound expires, collection waits for successful discovery. At startup with unavailable discovery, the process stays running and retries every 30 seconds, while honoring shutdown. Later cycles without a usable cache retry discovery after at most 30 seconds. Configuration errors, explicit empty results, invalid labels/credentials, and identity conflicts do not trigger fallback. A new authoritative result invalidates old membership, even if shared validation then fails. `inventory` and `discover` always require fresh discovery.

## Current limits and validation

All host machines use the existing `compute` target kind; this is not an SKU role classification for storage/cache hosts. Direct connections support a site credential profile and per-MAC overrides. Proxy mode delegates BMC credentials to NICo. Physical rack-slot mapping requires follow-up work.

Request shape and failures are covered by local HTTP fixtures against the checked-in REST contract. A local mTLS proxy fixture covers inventory, nonce-bearing attestation POSTs, task polling, evidence reads, and invalid identity rotation. Verify your deployed REST route and proxy authorization with your administrator before production use.
