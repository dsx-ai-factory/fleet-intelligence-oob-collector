# Changelog

All notable changes to Fleet Intelligence Out-of-Band Collector should be
documented in this file.

This project follows a human-maintained changelog format. Group entries by
release and use the sections that apply.

## Unreleased

### Added

- Initial changelog.
- Multi-architecture static Distroless container image packaging and GHCR
  release automation.
- OCI Helm chart for deploying the collector with existing credential Secrets
  and hardened pod defaults.
- Processor UUID and serial-number collection from Redfish inventory.
- NVLink fabric and NVSwitch inventory: fabrics, their switches, and NVIDIA
  switch OEM data (device and vendor IDs, reference clock, protected PCIe mode,
  isolation mode), with `fabrics` and `fabrics.switches` domain errors.
- Collector-to-BMC network latency measurement: after each inventory collection, the
  collector times TCP connects to the BMC's Redfish port and uploads the round
  minimum and median as a top-level `latency` field. A once-per-run check
  marks measurements `intercepted` when a local component such as a service mesh
  sidecar terminates outbound TCP. Disable with `--disable-latency-probe` or
  the Helm value `bmcCollector.latencyProbe: false`; disabled probes report
  `status: disabled`.

### Changed

- Breaking: inventory observations require an explicitly configured BMC MAC and
  no longer include top-level `uuid` or `primarySystemId`. Reported system UUIDs
  remain unchanged; attestation uses the canonical UUID returned by the backend.
  MACs are validated and normalized, and the IP-derived MAC fallback is removed.

- Upgrade nv-redfish to 0.18.1. NVIDIA GPU processor data is read through the
  typed `Processor::oem_nvidia()` API. Upstream BMC quirk fixes for missing
  member IDs and empty log references also apply to collection.
- Compile only the nv-redfish schema features inventory collection uses instead
  of `std-redfish`, cutting the release rebuild of nv-redfish and its dependents
  from 77 s to 31 s and the release binary from 21.0 MB to 14.9 MB.
- Redfish and attestation requests identify themselves to BMCs with the
  `fleet-intelligence-oob-collector` user agent.
- The rendered NICo configuration sets the logs collector to `mode = "sse"`
  instead of `"auto"`, so BMC log entries are only streamed over Redfish SSE
  and are no longer polled from LogService when an endpoint has no
  `/EventService`.
- The rendered NICo OTLP target sets `include_diagnostics = true`, which exports
  the Redfish `DiagnosticData` payload carrying CPER records along with the
  `redfish.diagnostic_data.*` attributes. Payload bodies stay opaque; decoding is
  left to downstream consumers.

### Fixed

- Nothing yet.
