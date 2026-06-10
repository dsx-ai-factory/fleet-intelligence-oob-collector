# Inventory Collection Notes

This package keeps a Fleet Intelligence inventory model between Redfish sources
and output sinks. The goal is to keep Redfish and vendor-specific behavior out of
the product schema where possible, while still documenting the assumptions used
to make one node inventory record from a BMC endpoint.

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
normalizes that report into the backend payload shape.

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
| `systems.secureBoot` | system ID | Write the system data, but do not overwrite `secureBootEnabled` for that system. |
| `chassis` | `null` | Do not write or clear chassis data for the target, including PCIe devices. |
| `chassis.pcieDevices` | chassis ID | Write the chassis data, but do not replace PCIe devices for that chassis with an empty list. |
| `firmware` | `null` | Do not write or clear firmware data for the target. |

When `targetError` is present, backend ingestion should skip all inventory writes
for that target and record the collection failure instead.

## PCIe Device Traversal

PCIe devices are collected from every chassis by default. A small set of known
platform policies narrows traversal where broad collection is known to produce
duplicates or unrelated devices.

Current policies:

- AMI with selected system `DGX`: collect PCIe devices from chassis IDs starting
  with `HGX_GPU_SXM` or `HGX_NVSwitch`.
- AMI, Dell, HPE, Supermicro, and Lenovo: collect PCIe devices only from the
  chassis whose ID matches the selected system ID.
- All other platforms: collect PCIe devices from all chassis.

Add fixture-backed tests before adding another narrowed traversal policy.

## Vendor And Platform Quirks

- Redfish `Vendor` matching is case-insensitive.
- AMI DGX/Viking platforms use special PCIe chassis selection rules.

## Output Conventions

- API JSON fields are camelCase.
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
