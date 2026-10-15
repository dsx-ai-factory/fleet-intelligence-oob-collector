# Inventory Collection Notes

This package collects inventory from BMC Redfish endpoints and exports it through
table, JSON, and backend sinks. Each target produces a `NodeInventory`
observation with collected resources and collection errors.

See the [architecture guide](../../docs/architecture.md) for runtime orchestration
and the [configuration guide](../../docs/configuration.md) for target setup.

## Pipeline

```text
Target
  -> source::RedfishSource
  -> models::NodeInventory
  -> sink::{TableSink, JsonSink, BackendSink}
```

Each observation is identified by its required configured BMC MAC in
`source.mac`, formatted as uppercase colon-separated text. The collector gathers
managers, systems, processors, secure boot, chassis, PCIe devices, firmware,
fabrics, and switches.

A configured target name provides a display label. System hostnames come from
the Redfish ComputerSystem resource.

## Backend Integration

All observations, including collection failures, are sent to the backend.
Batch results correspond to submitted observations by position. The backend
resolves node identity and returns `nodeUUID` for resolved observations.
Attestation runs only for accepted observations with a valid returned node UUID.

## Partial Collection

A domain failure is recorded in `domainErrors` while successfully collected data
remains available. A failure that prevents collection, such as an unavailable
Redfish service root, is recorded in `targetError`.

Fabrics and switches are fetched independently. An unreadable member or malformed
switch OEM data produces a scoped domain error. Missing collection `Members` is
an error rather than an empty result.

## Domain Error Contract

Backend consumers use `domainErrors[].domain` and optional `resource` to decide
which data can be updated. Error messages are descriptive and are not parsed for
control flow.

| Domain | Resource | Backend write behavior |
|--------|----------|------------------------|
| `managers` | `null` | Keep stored manager data. |
| `systems` | `null` | Keep stored system data, including secure boot. |
| `systems.processors` | system ID | Update system data while keeping stored processors. |
| `systems.secureBoot` | system ID | Update system data while keeping stored secure boot state. |
| `chassis` | `null` | Keep stored chassis data, including PCIe devices. |
| `chassis.pcieDevices` | chassis ID | Update chassis data while keeping stored PCIe devices. |
| `firmware` | `null` | Keep stored firmware data. |
| `fabrics` or `fabrics.switches` | any | Keep the entire stored fabrics section. |

On first ingest, the backend stores available partial data and its domain errors.
A `targetError` records collection failure without updating stored inventory.

## Network Latency Measurement

`latency[]` reports TCP connection time to the BMC's Redfish port. Successful
measurements include `minUs` and `medianUs`; use the minimum to compare latency
with less transient queuing noise. The probe excludes TLS, HTTP, and BMC
application processing.

Measurement status is `ok`, `unavailable`, `intercepted`, or `disabled`. A
once-per-run check detects local TCP interception and skips target probes when
present. Disable probing with `--disable-latency-probe` or
`bmcCollector.latencyProbe: false`.

Targets with `targetError` are not probed. Probe failures do not populate
`domainErrors` or `targetError`.

## Output Conventions

- JSON uses schema `inventory.v1alpha1` and camelCase field names. Optional
  Redfish fields remain optional, and enum values use their Redfish spelling.
- Chassis location, processor summaries, and memory summaries retain their nested
  structure. Table output presents selected values without changing the model.
- Resource references use `odataId`. Standard relationships appear in `links`
  as arrays of references; firmware `relatedItem` is separate. Linked resources
  are not fetched solely to populate references.
- Exported errors are sanitized. Reference URLs omit authority and query
  parameters while retaining fragments.
