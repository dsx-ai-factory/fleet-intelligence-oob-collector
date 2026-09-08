# NMX-C Management Metadata Integration

## Summary

Add one optional NMX-C controller per rack. Targets reference racks with
`rack_id`. Use a pinned `libnmxc` dependency behind a Fleet-owned adapter to
collect the NMX-C version, NVLink domain UUID, domain state, partitions,
members, and GPU-NVSwitch-port topology. NMX-M is out of scope. NMX-C failures
must not stop Redfish inventory, attestation, or other racks.

Collect and push the rack-level NMX fabric snapshot every 5 minutes, as
specified by the PRD.

The prototype covers configuration, collection, validation, JSON/table output,
and NICo UUID enrichment. The scheduled backend sink is the next phase.

## Architecture

```text
[[racks]] + [[targets]]
          |
          +--> libnmxc adapter --> Fleet domain model --> JSON/table/backend
          |
          +--> NICo renderer --> rack/domain labels --> NICo --> OTLP
```

Add `crates/nmxc-inventory` for the adapter, Fleet-owned models, mapping, and
sinks. The root runtime coordinates it independently from `bmc-inventory`.
Use `gateway_id = "fleet-intelligence-oob-collector"` internally.

## Config shape

```toml
[[racks]]
id = "rack-12"

[racks.nmxc]
ip = "192.0.2.12"
port = 9370
# Optional TLS/mTLS file paths:
# ca_file = "/run/secrets/nmxc/rack-12/ca.crt"
# client_cert_file = "/run/secrets/nmxc/rack-12/tls.crt"
# client_key_file = "/run/secrets/nmxc/rack-12/tls.key"
# server_name = "nmxc-rack12.example.internal"

[[targets]]
kind = "compute"
name = "node-01"
rack_id = "rack-12"
ip = "10.85.14.21"
port = 443
mac = "00:11:22:33:44:55"
```

Port `9370` is the default. No TLS fields means plaintext; any TLS field means
TLS. Client certificate and key must be configured together. Rack IDs must be
unique, and a supplied `target.rack_id` must reference an existing rack.
Customers do not configure the domain UUID, chassis serial, or gateway ID.

## gRPC APIs and collected data

| API | Data |
| --- | --- |
| `Hello` | Domain UUID, NMX-C version, capabilities |
| `GetDomainStateInfo` | Control-plane state and controller health |
| `GetDomainProperties` | Fabric node, GPU, switch, port, and partition limits |
| `GetPartitionInfoList` | Partition config, health, GPU UIDs and locations |
| `GetTopologyInfo` | GPUs, NVSwitches, ports, peers, links, and locations |

Call `Hello` first, then make the other read calls. Every response
domain UUID must match `Hello`. Map partition health as:

```text
HEALTHY                         -> ACTIVE
DEGRADED / DEGRADED_BANDWIDTH -> DEGRADED
UNHEALTHY                       -> FAILED
UNKNOWN                         -> collection error
```

Do not call partition mutation APIs. Build topology links by matching each
device/port to its returned peer device/port.

## Sinks

- **JSON:** Full camelCase Fleet domain model for validation and fixtures.
- **Table:** Domain summary; detail mode adds partition rows and collection errors.
- **Backend:** Push the rack record to
  `/v1/racks/{rack_id}/oob/fabric`, authenticated by the existing agent JWT.

The endpoint is rack-scoped. The record includes `rackId`, `domainUUID`,
`nmxcVersion`, domain state, partitions, topology, and partial errors. The
domain UUID remains the NMX-C-authoritative fabric identifier. Evaluate errors
per rack so one failure does not block other racks or OOB workflows.

## NICo config generation

Make `render-nico-config` asynchronous:

1. Group targets by `rack_id`.
2. Call NMX-C `Hello` once per rack.
3. Validate the returned domain UUID.
4. Add both labels to every target in that rack.
5. Render NICo `static_bmc_endpoints` normally.

```toml
[endpoint_sources.static_bmc_endpoints.labels]
rack_id = "rack-12"
nvlink_domain_uuid = "<uuid-from-nmxc>"
oob = "true"
```

NICo attaches these labels to target metrics and logs, and the OTLP gateway
preserves them. Never accept the UUID from customer labels. If UUID discovery
fails, report the rack-level error without stopping Redfish inventory,
attestation, or resolved racks.
