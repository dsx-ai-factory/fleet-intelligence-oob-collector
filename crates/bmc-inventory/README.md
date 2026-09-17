# Inventory Collection Notes

This package keeps a Fleet Intelligence inventory model between Redfish sources
and output sinks. The goal is to keep Redfish and vendor-specific behavior out of
the product schema where possible, while still documenting the assumptions used
to make one node inventory record from a BMC endpoint.

For the repository-level component and runtime view, see the
[architecture guide](../../docs/architecture.md).

## Pipeline

```text
Target
  -> source::RedfishSource
  -> source::RedfishReport
  -> mapper::RedfishMapper
  -> models::NodeInventory
  -> sink::{TableSink, JsonSink, BackendSink}
```

`RedfishSource` collects raw-ish Redfish data into source structs. `RedfishMapper`
acts as the processing stage: it enriches source metadata with best-effort
reverse DNS and normalizes the report into the backend payload shape.

## Node Identity

- The primary node UUID comes from the selected Redfish `ComputerSystem.UUID`.
- The selected system is the first system with a non-empty BIOS version.
- If no system has a BIOS version, the mapper falls back to the first system in
  the Redfish systems collection.
- If the selected system has a missing, empty, or malformed UUID, the mapper
  derives a stable UUID from the Redfish service root UUID when present, or the
  target address otherwise.
- `NodeInventory.primarySystemId` records the selected system ID used for node
  identity.
- `InventorySourceInfo.serviceUuid` keeps the Redfish service root UUID. It is
  not used as the node UUID.
- `InventorySourceInfo.vendor` prefers Redfish service root `Vendor`, then falls
  back to `Product`.
- `InventorySourceInfo.hostName` is populated from a best-effort reverse DNS
  lookup of the configured BMC IP. A missing PTR record, resolver error, or
  lookup timeout does not fail inventory collection.

The BIOS-based selection is a host-system heuristic. Some BMCs expose auxiliary
systems, such as BMC, DPU, or baseboard systems, in the same collection as the
host. The host system is the one expected to carry BIOS data and the stable node
UUID. The mapper still preserves every collected Redfish manager and system in
`managers[]` and `systems[]`; selection only determines the top-level node UUID
and primary marker.

## Partial Collection

Collection is split by inventory domain:

- managers
- systems
- systems.processors
- systems.secureBoot
- chassis
- chassis.pcieDevices
- firmware

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

When `targetError` is present, backend ingestion should skip all inventory writes
for that target and record the collection failure instead.

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
- Individual-resource `@odata.type` is not exposed by nv-redfish 0.15.1. Recovering
  the original type/version requires dependency support or a separately designed
  raw-response capture path; the collector does not fabricate a schema version.
- The inventory schema remains `inventory.v1alpha1`; legacy aliases are not populated.
- System `processorSummary` and `memorySummary` retain nested counts, capacity,
  processor model/threading, memory mirroring, and their own `status`/conditions.
  They replace `cpuCount`, `cpuCoreCount`, `cpuModel`, and `memoryGib` in JSON;
  table columns remain available.
- Summary status is preserved when supplied, never synthesized. DMTF deprecated
  it in ComputerSystem v1.16 in favor of root status conditions, but older BMCs
  can still provide diagnostic information there.
- Summary metrics links are outside this change; no new resource fetches, OEM
  collection, or inventory schema-version bump are introduced.
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
