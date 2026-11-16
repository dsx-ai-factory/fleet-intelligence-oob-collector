# Discover targets through NICo REST

Use NICo REST to discover the BMC addresses and MAC addresses of compute hosts in one site. The collector then queries those BMCs directly for inventory and attestation using your shared BMC credentials. NICo inventory and attestation records are not imported.

## Prepare access

You need an HTTPS NICo REST endpoint trusted by the collector's system CA store, an organization and site ID, and a bearer access token with the `carbide` scope and provider/site permissions to read machine BMC metadata. Confirm the exact permissions with your NICo administrator. The token file contains an access token, not an SSA client secret; arrange token refresh outside the collector.

The collector also needs network access to the discovered BMCs and their credentials in the [shared credentials file](credentials.md). NICo REST authentication and BMC authentication use separate credentials.

## Select the source

Create a TOML configuration with shared collection settings and an `endpoint_source` section. Omit `[[targets]]` when you select NICo.

```toml
compute_zone = "us-west-dc1"
node_group = "training"
labels = { site = "dc1" }

[credentials]
file = "credentials.json"

[endpoint_source]
type = "nico"
api_url = "https://nico.example.com/v2"
org = "example-org"
site_id = "00000000-0000-0000-0000-000000000001"
token_file = "nico-access-token"
api_name = "nico"
credentials_profile = "bmc"
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
| `credentials_profile` | Optional shared BMC credential profile; per-MAC overrides also apply |
| `timeout_seconds` | Whole discovery deadline, including both attempts and all pages; default `30`, allowed `1`–`3600` |
| `max_stale_seconds` | Maximum age of a last-good discovery used by `run` during NICo outages; default `108000` (30 hours), allowed `1`–`172800` (48 hours) |

Relative token and credential paths resolve beside the TOML file. Protect both files from other users and replace the token file atomically when rotating it. The collector rereads the token on each discovery. See [credential rotation and ownership](credentials.md#rotation-and-ownership) for BMC credential handling.

NICo machine labels are merged with your shared labels using the same validation as static targets. Machine labels named `compute_zone` and `node_group` provide per-machine placement overrides; other labels remain user labels. Global zone/group values are defaults. The source supplies the machine hostname when available. NICo rack slot/tray placement is not mapped to Fleet placement automatically. It lists compute hosts; switches and power shelves require other sources. NICo REST does not expose a BMC port in this response, so discovered targets use the default Redfish HTTPS port, `443`.

## Verify discovery and collect

Use the installed binary to list discovered endpoints without reading BMC passwords or querying hardware:

```bash
bmc-collector discover --config /private/path/config.toml
```

From a source checkout, run `cargo run --locked --bin bmc-collector -- discover --config /private/path/config.toml`.

The JSON output contains the machine reference, IP, MAC, name, kind, labels, placement overrides, and credential profile reference, before shared collection normalization. The command always checks live discovery and does not use cached endpoints. Treat the output as infrastructure information.

To collect inventory with the same configuration:

```bash
cargo run --locked --bin bmc-collector -- inventory --config /private/path/config.toml
```

The `run` command also accepts this configuration; see [running the collector](../CONTRIBUTING.md#running-the-collector). Discovery runs when the collection cycle loads its configuration. Inventory and attestation share the resolved target list for that cycle. There is no separate discovery refresh loop.

## Deployment support

### Linux packages: keep collector and health configs separate

**Keep `/etc/fleetint-oob/config.toml` static when the bundled NICo health service is enabled.** Setting `type = "nico"` in that shared file prevents the health configuration from being rendered. The renderer rejects that configuration before writing output or starting NICo.

Put the discovery configuration above in a separate, protected file and point only the collector at it:

```bash
sudo install -m 0640 -o root -g fleetint-oob /dev/null /etc/fleetint-oob/nico-discovery.toml
sudoedit /etc/fleetint-oob/nico-discovery.toml /etc/fleetint-oob/service.conf
```

In `service.conf`, set:

```ini
FLEETINT_COLLECTOR_CONFIG=/etc/fleetint-oob/nico-discovery.toml
```

Keep static `[[targets]]` in `/etc/fleetint-oob/config.toml` for NICo hardware-health metrics and logs. Both configurations can reference the same BMC credential file. The health target list is maintained separately; API target changes do not update it. Ensure `fleetint-oob` can read the discovery config, token file, credentials, and their parent directories. Verify before restarting:

```bash
sudo -u fleetint-oob /usr/bin/bmc-collector discover --config /etc/fleetint-oob/nico-discovery.toml
sudo systemctl restart fleetint-oob-collector.service
```

### Kubernetes

The Helm chart accepts static `inventory.targets` only; it does not configure the NICo source or mount a REST token. Kubernetes NICo discovery support requires follow-up chart work. Retain the existing static chart configuration until that support is available.

The collector uses direct Redfish connections to BMCs. BMC proxy transport and SSA token acquisition are separate integrations.

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

The long-running `run` command retains the last successfully resolved, credential-free endpoint listing in memory. If the same NICo source becomes unavailable because of a token-file read failure, HTTP `401`/`403`/`429`, server error, connection failure, or timeout, it can reuse that list until `max_stale_seconds` has elapsed since the successful discovery. Warnings report the cache age and target count. Reusing a list does not extend its lifetime, and BMC credentials are reloaded from their file for each cycle.

The default 30-hour bound allows a failed discovery at the default daily collection interval. Choose a shorter limit when address reuse or rapid machine turnover makes stale membership unacceptable. Missing-machine filters apply to live discovery; a cached list cannot detect changes made during an outage and cannot guarantee an IP still belongs to the same BMC. The age check is made before a collection cycle starts.

No cache is written to disk. After a restart, or after the bound expires, collection waits for successful discovery. At startup with unavailable discovery, the process stays running and retries every 30 seconds, while honoring shutdown. Later cycles without a usable cache retry discovery after at most 30 seconds. Configuration errors, explicit empty results, invalid labels/credentials, and identity conflicts do not trigger fallback. A new authoritative result invalidates old membership, even if shared validation then fails. `inventory` and `discover` always require fresh discovery.

## Current limits and validation

All host machines use the existing `compute` target kind; this is not an SKU role classification for storage/cache hosts. One optional credential profile is the site default, and shared per-MAC overrides handle exceptions. SKU/vendor-based profile selection and physical rack-slot mapping require follow-up work.

Request shape and failures are covered by local HTTP fixtures against the checked-in REST contract. Live NICo permissions, deployed `nico` versus `carbide` routing, and pagination compatibility still require an environment-specific POC before production use.
