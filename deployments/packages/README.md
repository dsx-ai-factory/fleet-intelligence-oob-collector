# Host package prototype

Build the `fleetint-oob` DEB or RPM package with standalone nFPM. It includes
three systemd services and a target to manage them together:

| Component | Service |
| --- | --- |
| OOB collector | `fleetint-oob-collector.service` |
| NICo hardware health | `fleetint-nico-health.service` |
| Telemetry gateway | `fleetint-otel-gateway.service` |

Use `fleetint-oob.target` to start, stop, or restart the full stack.

## Build

Install the nFPM version pinned in `nfpm.version` using an
[official method](https://nfpm.goreleaser.com/docs/install/), for example:

```bash
npm install -g @goreleaser/nfpm@2.48.0
```

Install Rust through rustup, Go, Git, a C/C++ compiler, Clang, CMake,
`pkg-config`, musl tools, and protobuf tools. On Ubuntu/Debian,
the native development packages are:

```bash
sudo apt-get install build-essential clang cmake pkg-config musl-tools \
  protobuf-compiler libprotobuf-dev
```

The Make targets build the collector with Cargo, build only NICo's
`carbide-health` / `forge-hw-health` binary with Cargo, and build the custom
OpenTelemetry gateway through the agent's pinned collector builder. Rustup
uses each source repository's pinned toolchain and installs the native musl
target. No Docker or Helm is required for building host packages.

Pinned NICo and agent sources are checked out under revision-specific
directories in `dist/package-sources/`. Changing a pinned revision selects a
new checkout; an existing custom checkout must match the requested revision.
NICo uses the same source revision as the release workflow. Build outputs stay
under `target/` and `dist/`; upstream sources are not edited. The gateway's
`sakauth` extension is required for authentication.

```bash
make deb      # DEB only
make rpm      # RPM only
make package  # Both formats
```

The targets first prepare all three binaries and their notices in
`dist/package-input/`, then invoke nFPM using `nfpm/fleetint-oob.yaml`.
Packages go to `dist/packages/`. Build on the target Linux architecture;
cross-compilation is not configured.

Override `VERSION`, `ARCH` (`amd64` or `arm64`), `INPUTDIR`, `OUTPUTDIR`, or
the `NFPM` executable as needed. `NICO_SOURCE` and `AGENT_SOURCE` can point to
existing checkouts at the selected revisions; the Make targets verify their
revisions and do not change those checkouts.
Versions must be valid in both package formats, for example `0.1.0~prototype1`. The package revision is 1.

Both Rust binaries use musl and the gateway uses `CGO_ENABLED=0`. The build
rejects binaries that contain an ELF interpreter or shared-library dependency.
The package therefore does not depend on glibc, libgcc, or OpenSSL. Systemd and
CA certificates are still required on the host. Musl DNS behavior can differ
from glibc, so validate name resolution in the target environment.

## Configure a test VM

Requires systemd. All services run as root. Installation reloads
units without creating accounts or starting collection. Install the package:

```bash
sudo apt install ./dist/packages/fleetint-oob_*.deb
# RPM systems: sudo dnf install ./dist/packages/fleetint-oob-*.rpm
sudo install -m 0600 -o root -g root \
  /usr/share/doc/fleetint-oob/credentials.example.json \
  /etc/fleetint-oob/credentials/credentials.json
```

The package installs `/etc/fleetint-oob/config.toml` with a commented example
target and mode `0600`, preserving edits on upgrade. Uncomment the example and
set a real BMC address and MAC before starting services. An empty target list
is rejected by the collector. See
[configuration](../../docs/configuration.md). Set backend/proxy settings in
`/etc/fleetint-oob/service.conf`, including `FLEETINT_AGENT_TOKEN`. This file is
installed root-owned with mode `0600`. Both services read it directly through
systemd `EnvironmentFile`; no shell wrappers are used.

Set each target's `credentials_profile` to a profile in the shared JSON file,
or provide a per-MAC override. A placeholder example is installed at
`/usr/share/doc/fleetint-oob/credentials.example.json`. Copy it to
`/etc/fleetint-oob/credentials/credentials.json` using the command above, then
replace placeholder values only in that copy. Keep the credentials file owned
by root with mode `0600`; the installed documentation example must contain
placeholders only.
NICo's `ExecStartPre` uses `bmc-collector render-nico-config` to reference
`/etc/fleetint-oob/credentials/credentials.json`. Generated NICo TOML contains
the shared file path and each target's selected profile; credential values stay
in the JSON file.

Set `BACKEND_ENDPOINT` and `ENROLL_ENDPOINT` explicitly when changing the
backend. Environment files do not expand references to other variables.
The host needs access to the BMCs and backend, but no local GPU.

```bash
sudo systemctl enable --now fleetint-oob.target
systemctl status fleetint-oob-collector.service fleetint-nico-health.service \
  fleetint-otel-gateway.service
```

The target groups services; inspect each service to verify health. NICo renders
its private configuration under `/run` before starting. Telemetry listeners
bind to loopback. The collector and gateway use the same
`FLEETINT_AGENT_TOKEN` environment variable from `service.conf`.

## Upgrade and remove

Packages preserve configuration and do not automatically restart services on
upgrade. Restart the target after upgrading or changing targets, tokens,
or shared settings. The collector handles SIGINT and SIGTERM during active
collection: it cancels collection, drains completed inventory awaiting upload,
and finishes an in-flight upload before stopping. Its systemd stop timeout is
300 seconds to accommodate the bounded inventory queue and HTTP retries.

Keep `FLEETINT_INTERVAL_SECONDS` set in `service.conf` (default: `86400`).
Missing or invalid command-line settings are not retried. Other startup failures,
including enrollment failures, restart after 10 seconds using the host's default
systemd start-limit policy. The package does not set a stricter start limit.

```bash
sudo systemctl restart fleetint-oob.target
sudo journalctl -u fleetint-oob-collector.service

sudo systemctl disable --now fleetint-oob.target
sudo apt remove fleetint-oob
# RPM systems: sudo dnf remove fleetint-oob
```

Package-owned configuration is preserved on removal; Debian purge removes
package-owned configuration. Operator-created BMC credential files remain.
Delete unused credentials explicitly. Backend data remains.

Debian hooks use `deb-systemd-invoke` for stopping services and
`deb-systemd-helper` to preserve enablement on removal/reinstallation and clean
up on purge, following the [debhelper convention](https://manpages.debian.org/bookworm/debhelper/dh_installsystemd.1.en.html).
RPM hooks use explicit systemd commands in nFPM scriptlets, following the
[systemd packaging lifecycle](https://www.freedesktop.org/software/systemd/man/latest/daemon.html).
The prototype does not automatically enable/start services or restart them on
upgrade. Service-management failures do not fail package transactions.

Run `make packages-test-hooks packages-test-build` for isolated lifecycle and
build regression tests. They use mocked service/compiler commands and local
Git fixtures; they never manage host services or fetch remote sources.

Package checks run with `make packages-check-deb` or `make packages-check-rpm`
using the distribution's inspection tools. They never install packages.
Real service startup, upgrade/rollback, and shutdown during collection still
need validation in a disposable VM before production use. Signing and release
publishing are outside this prototype.
