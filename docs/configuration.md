# Configuration

The collector and NICo hardware health share one TOML file. It contains global
labels and one or more BMC targets:

```toml
[labels]
compute_zone = "az51"
node_group = "dev3-dh1"

[[machines]]
ip = "10.0.0.10"
port = 443
mac = "00:11:22:33:44:55"
username = "bmc-user"
password = "bmc-password"
labels = { site = "dev", environment = "development" }
```

Start from `config.example.toml`, keep the resulting file out of source
control, and restrict it to the account running the collector:

```bash
cp config.example.toml config.toml
chmod 600 config.toml
```

Unknown TOML fields are rejected so misspelled settings fail early.

## Top-level fields

| Field | Required | Description |
| --- | --- | --- |
| `labels` | No | Labels applied to every machine. Per-machine labels override values with the same key. |
| `machines` | Yes | One or more `[[machines]]` tables. An empty target list is rejected. |

## Machine fields

| Field | Required | Description |
| --- | --- | --- |
| `ip` | Yes | IPv4 or IPv6 address of the BMC. Host names are not accepted. |
| `port` | No | Redfish port. When omitted, the Redfish client uses its HTTPS default. |
| `mac` | No | Stable BMC identity used to correlate collection results. Must be unique in the file. |
| `username` | Yes | Redfish username. Blank values are rejected. |
| `password` | Yes | Redfish password. Blank values are rejected. |
| `labels` | No | Labels added to this machine or overriding top-level labels. |

If `mac` is omitted, the collector creates a locally administered unicast MAC
from `02:00` and the final four IP bytes. For example, `10.0.0.10` becomes
`02:00:0a:00:00:0a`. For IPv6, the final four bytes of the address are used.
Specify a real stable MAC when one is available; an explicit value always wins.

## Labels

Labels are used by NICo telemetry and accompany inventory uploads. The
collector always adds `oob="true"`.

Label keys must match `[a-zA-Z_][a-zA-Z0-9_]*`. Effective labels are limited to
31 user-defined entries per machine because the collector reserves one entry
for `oob`. Each value is limited to 1,024 bytes.

The following identity labels are reserved by NICo and cannot be configured:

```text
collector_type       endpoint_ip          endpoint_key
endpoint_mac         machine_id           machine_slot_number
machine_tray_index   nvlink_domain_uuid   oob
rack_id              serial_number        switch_id
switch_slot_number   switch_tray_index    system_uuid
```

When effective labels contain `compute_zone` or `node_group`, the inventory
backend sink also exports them as the `computeZone` and `nodeGroup` placement
fields. They remain ordinary NICo labels as well.

## TLS

Redfish requests validate certificates by default. Prefer BMC certificates
trusted by the collector host. For development or legacy BMCs, the
`--insecure-tls` CLI flag disables certificate validation for both inventory
and attestation Redfish requests. The matching Helm value is
`bmcCollector.insecureTls`.

This option affects BMC connections, not the backend TLS policy.

## Proxy environment variables

Redfish and Fleet Intelligence backend requests honor both uppercase and
lowercase standard proxy variables:

- `HTTP_PROXY` for HTTP destinations
- `HTTPS_PROXY` for HTTPS destinations
- `ALL_PROXY` as a fallback for either scheme
- `NO_PROXY` for destinations that must connect directly

HTTP, HTTPS, SOCKS4, and SOCKS5 proxy URLs are supported. Use `socks5h://` when
DNS resolution must occur through the proxy:

```bash
HTTPS_PROXY=socks5h://127.0.0.1:8888 \
  target/debug/bmc-collector inventory \
  --config config.toml \
  --insecure-tls
```

Do not place proxy credentials in the collector TOML or committed Helm values.
For Kubernetes, see the proxy section in
[Kubernetes deployment](kubernetes.md#proxy-configuration).

## Credential handling

The TOML file contains BMC credentials in clear text because Redfish Basic Auth
requires them at runtime. Store the file with restrictive permissions. In
Kubernetes, load the complete file from an existing Secret; the chart does not
accept BMC credentials as values. Store the Fleet Intelligence bootstrap agent
token in a separate Secret or provide it through `FLEETINT_AGENT_TOKEN` for a
local run.
