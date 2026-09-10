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
- Opt-in export of Redfish `DiagnosticData` payloads, which carry CPER records,
  through `nicoHardwareHealth.diagnostics.enabled` and the
  `render-nico-config --include-diagnostics` flag. Payload bodies stay opaque;
  decoding is left to downstream consumers. Because NICo negotiates TLS from the
  OTLP endpoint scheme and the generated configuration carries no per-target TLS
  profile, export requires an `https://` endpoint unless
  `nicoHardwareHealth.diagnostics.allowInsecureExport` explicitly permits
  plaintext transport.

### Changed

- The rendered NICo configuration sets the logs collector to `mode = "sse"`
  instead of `"auto"`, so BMC log entries are only streamed over Redfish SSE
  and are no longer polled from LogService when an endpoint has no
  `/EventService`.

### Fixed

- Nothing yet.
