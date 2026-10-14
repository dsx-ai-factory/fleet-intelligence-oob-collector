# Inventory Collection Notes

This package collects inventory from BMC Redfish endpoints and exports it through
table, JSON, and backend sinks. Each configured target produces a
`NodeInventory` observation containing source metadata, collected resources,
and collection errors.

For the repository-level component and runtime view, see the
[architecture guide](../../docs/architecture.md).

## Pipeline

```text
Target
  -> source::RedfishSource
  -> models::NodeInventory
  -> sink::{TableSink, JsonSink, BackendSink}
```

`RedfishSource` converts Redfish resources directly into the inventory model. It
resolves source metadata with best-effort reverse DNS. When enabled, it also
measures collector-to-BMC latency after the Redfish collection completes.

## Source Metadata

- Each observation is identified by the required configured BMC MAC in
  `source.mac`, normalized to lowercase colon-separated form.
- `InventorySourceInfo.serviceUuid` retains the Redfish service root UUID.
- `InventorySourceInfo.vendor` prefers Redfish service root `Vendor`, then `Product`.
- `InventorySourceInfo.hostName` comes from best-effort reverse DNS. Lookup
  failures do not fail collection.

## Backend Integration

All observations, including target failures, are sent to the backend so it can
record collection status. Batch results correspond by position to submitted
observations. The backend owns identity resolution and returns `nodeUUID` when
resolved. Only accepted observations with a valid returned node UUID are used
for attestation, associated with the configured target through BMC MAC. A
successful response without a node UUID does not enable attestation.

## Partial Collection

Collection is split by inventory domain:

- managers
- systems
- systems.processors
- systems.secureBoot
- chassis
- chassis.pcieDevices
- firmware
- fabrics
- fabrics.switches

A domain failure is recorded in `domainErrors` and does not discard other
domains. For example, a firmware inventory failure should still preserve manager,
system, chassis, and PCIe device data collected from the same target.

A target-level failure, such as being unable to create the Redfish service root,
is recorded as `targetError` and produces an otherwise empty inventory.

## Domain Error Contract

`domainErrors[].domain` is a stable backend write-decision key. Backend ingestion
should key off `domain`, optionally scoped by `resource`; it should not parse
`message` for control flow.

| Domain | Resource | Backend write behavior |
|--------|----------|------------------------|
| `managers` | `null` | Do not write or clear manager data for the target. |
| `systems` | `null` | Do not write or clear system data for the target, including secure boot. |
| `systems.processors` | system ID | Write the system data, but do not replace processors for that system with an empty list. |
| `systems.secureBoot` | system ID | Write the system data, but do not overwrite `secureBootEnabled` for that system. |
| `chassis` | `null` | Do not write or clear chassis data for the target, including PCIe devices. |
| `chassis.pcieDevices` | chassis ID | Write the chassis data, but do not replace PCIe devices for that chassis with an empty list. |
| `firmware` | `null` | Do not write or clear firmware data for the target. |
| `fabrics` | `null` or fabric ID | Preserve the entire previously stored `fabrics` section for the target. |
| `fabrics.switches` | fabric or switch ID | Preserve the entire previously stored `fabrics` section for the target, including all switches. |

Any `fabrics` or `fabrics.switches` error skips replacement of the whole stored
section, regardless of `resource`. Successful fabric and switch results from the
same collection do not update an existing snapshot until a collection completes
without either error. This prevents a partial payload from clearing sibling
fabrics or switches; other inventory domains can still update independently.
On first ingest, when no prior snapshot exists, the backend stores the submitted
partial payload together with its domain errors.

When `targetError` is present, backend ingestion should skip all inventory writes
for that target and record the collection failure instead.

## Fabric and Switch Collection

Fabrics come from the service root `Fabrics` collection, and switches from each
fabric's `Switches` collection. Each fabric and switch is fetched independently,
so one unreadable member is recorded as a scoped domain error without discarding
its siblings. A collection missing `Members` is an error, not an empty list, so
it cannot clear stored fabrics. NVIDIA switch OEM data is decoded through
nv-redfish's typed `Switch::oem_nvidia()`; malformed OEM data keeps the switch's
standard inventory and records a `fabrics.switches` error.

## Network Latency Measurement

`latency[]` carries collector-to-BMC network latency measurements from the
collection run.

- Method `tcp_connect` times TCP connection establishment to the target IP and
  Redfish port (443 unless configured). The stream is closed without sending
  TLS or HTTP, so the duration is roughly one round trip and excludes BMC
  application processing.
- Each round makes five attempts, 100 ms apart, with a 2 s timeout, under one
  dispatcher admission for the target's inventory workflow.
- `status: ok` requires at least three successful attempts and reports `minUs`
  and `medianUs`. Consumers should compare the minimum, which filters transient
  queuing delay.
- `status: unavailable` means too few attempts succeeded. It carries a sanitized
  `error` and no latency values; it is a missing measurement, not a high one.
- Once per collection run, the collector connects to `192.0.2.1:443` (RFC 5737
  TEST-NET-1). A successful connect means a local component, such as a service
  mesh sidecar, terminates outbound TCP. Every target is then reported with
  `status: intercepted` and no probe traffic is sent.
- `status: disabled` means the probe is turned off with
  `--disable-latency-probe` or `bmcCollector.latencyProbe: false`; no probe
  traffic is sent.
- Targets whose Redfish collection fails at the target level (`targetError`)
  are not probed.
- Probe failures never populate `domainErrors` or `targetError`.

## PCIe Device Traversal

Collect PCIe devices from each chassis’s advertised `PCIeDevices` collection.

## Output Conventions

- Chassis location preserves nested `partLocation`, `placement`, and
  `partLocationContext` rather than flattening selected fields together.
  Location type, ordinal, orientation/reference, rack-offset units, facility name,
  and additional placement information are retained when supplied.
- Empty location subobjects remain present. Table output uses reported rack-offset
  units and does not assume EIA rack units when the BMC omits units.
- Location covers hardware placement, not geographic/postal/contact fields.
- Collected resources preserve their Redfish `Name` and optional `Description`
  as `name` and `description`. The product model exposes Redfish resource
  references as the camelCase `odataId` field; the wire-level `@odata.id` key
  remains confined to Redfish parsing. Resource IDs and names are not synthesized
  or normalized.
- Individual-resource `@odata.type` is not exposed by nv-redfish 0.18.1. Recovering
  the original type/version requires dependency support or a separately designed
  raw-response capture path; the collector does not fabricate a schema version.
- The inventory schema is `inventory.v1alpha1`.
- System `processorSummary` and `memorySummary` retain nested counts, capacity,
  processor model/threading, memory mirroring, and their own `status`/conditions.
  The table sink presents selected summary values as columns.
- Summary status is included when supplied and is not synthesized.
- Summary metrics links do not trigger additional resource fetches.
- Redfish enum values retain their canonical spelling (for example `BMC`, `On`,
  `CPU`, and `SingleFunction`), rather than being converted to snake_case.
  Unknown values discarded by the dependency parser remain unavailable and are
  represented as null, not a synthetic `UnsupportedValue` string.

- API JSON fields are camelCase.
- Standard Redfish relationships use one stable shape in `links`: each canonical
  relationship name maps to an array of `{"odataId": "..."}` references. A
  singular Redfish relationship becomes a one-element array. Array order,
  duplicates, and empty arrays are preserved. Resources with no relationships
  emit `links: {}`.
- Firmware `relatedItem` is retained separately from `links`.
- Link targets omit URL authority (including credentials) and query parameters;
  fragments are preserved because they can identify subobjects. Linked resources
  are not fetched solely to populate references.
- The collector handles acquisition and safe transport; the backend owns
  reconciliation and product-specific interpretation.
- Optional Redfish fields stay optional in the inventory model rather than being
  filled with synthetic values.
- `domainErrors` describes recoverable domain-level collection failures.
- `targetError` describes a failure that prevents a meaningful node inventory.
- The table sink is presentation-only; it should not introduce schema semantics
  that are absent from `models::NodeInventory`.

## Adding A New Platform Assumption

When adding or changing a platform assumption:

1. Add a fixture or unit test showing the Redfish shape that requires it.
2. Keep the heuristic scoped to the narrowest vendor/product/chassis signature.
3. Prefer skipping uncertain optional domains over producing duplicate or
   incorrectly associated inventory.
4. Record recoverable collection failures in `domainErrors`.
5. Update this README with the new assumption and why it exists.
