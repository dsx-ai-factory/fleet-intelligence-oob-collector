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
| `timeout_seconds` | Whole discovery deadline, including all pages; default `30`, allowed `1`–`3600` |

Relative token and credential paths resolve beside the TOML file. Protect both files from other users and replace the token file atomically when rotating it. The collector rereads the token on each discovery. See [credential rotation and ownership](credentials.md#rotation-and-ownership) for BMC credential handling.

Shared labels, zone, group, credential lookup, and target validation follow the same rules as static targets. The source supplies the machine hostname when available. It lists compute hosts; switches and power shelves require other sources. NICo REST does not expose a BMC port in this response, so discovered targets use the default Redfish HTTPS port, `443`.

## Verify discovery and collect

From a source checkout, list the discovered endpoints without reading BMC passwords or querying hardware:

```bash
cargo run --locked -p bmc-inventory --example discover-endpoints -- /private/path/config.toml
```

The JSON output contains source IP, MAC, name, and kind fields, before shared collection normalization. Treat the output as infrastructure information.

To collect inventory with the same configuration:

```bash
cargo run --locked --bin bmc-collector -- inventory --config /private/path/config.toml
```

The `run` command also accepts this configuration; see [running the collector](../CONTRIBUTING.md#running-the-collector). Discovery runs when the collection cycle loads its configuration. Inventory and attestation share the resolved target list for that cycle. There is no separate discovery refresh loop.

## Deployment support

NICo discovery is available through the collector's TOML runtime configuration. The Helm `inventory.targets` schema and the bundled NICo health configuration generator require static targets. The `render-nico-config` command rejects NICo discovery configurations. For NICo hardware-health metrics and logs, retain your existing NICo health deployment; the native packages' bundled health configuration generation also requires static targets.

The collector uses direct Redfish connections to BMCs. BMC proxy transport and SSA token acquisition are separate integrations.

## Troubleshoot discovery

| Problem | What to check |
| --- | --- |
| HTTP `401` or `403` | Token expiry, `carbide` scope, and provider/site permissions |
| HTTP `404` | REST prefix and `api_name` for your deployment |
| Missing BMC metadata | Machine metadata permission and BMC IP/MAC availability |
| Inconsistent pages | Retry after concurrent site membership changes settle |
| No targets | Selected site and host visibility; empty listings are rejected |
| Timeout | Connectivity, CA trust, site size, and the whole-discovery deadline |
| Unknown BMC profile | Shared BMC credentials file; this is separate from the REST access token |

Discovery fails as a whole if any page or required BMC field is missing or invalid. It requests 100 machines per page, allows at most 1,000 pages, and limits each page body to 8 MiB. Pagination checks catch changed totals, missing rows, and repeated machine IDs, but cannot provide an atomic view of a site changing during pagination. A discovery failure aborts that collection cycle instead of publishing a partial target list.
