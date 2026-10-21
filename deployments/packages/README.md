# Host package prototype

Build the `fleetint-oob` DEB or RPM package with standalone nFPM. It includes three systemd services and a target to manage them together:

| Component | Service |
| --- | --- |
| OOB collector | `fleetint-oob-collector.service` |
| NICo hardware health | `fleetint-nico-health.service` |
| Telemetry gateway | `fleetint-otel-gateway.service` |

Use `fleetint-oob.target` to start, stop, or restart the full stack.

## Build

Install the nFPM version pinned in `nfpm.version` using an [official installation method](https://nfpm.goreleaser.com/docs/install/). Confirm that `nfpm --version` matches the pinned version before building.

Install Rust through rustup, Go, Git, a C/C++ compiler, Clang, CMake, `pkg-config`, binutils (`readelf`), musl tools, and protobuf tools. On Ubuntu/Debian, the native development packages are:

```bash
sudo apt-get install build-essential clang cmake pkg-config binutils musl-tools \
  protobuf-compiler libprotobuf-dev
```

The Make targets build the collector with Cargo, build only NICo's `carbide-health` / `forge-hw-health` binary with Cargo, and build the custom OpenTelemetry gateway through the agent's pinned collector builder. Rustup uses each source repository's pinned toolchain and installs the native musl target. No Docker or Helm is required for building host packages.

Pinned NICo and agent sources are checked out under revision-specific directories in `dist/package-sources/`. Changing a pinned revision selects a new checkout; an existing custom checkout must match the requested revision. NICo uses the same source revision as the release workflow. Build outputs stay under `target/` and `dist/`; upstream sources are not edited. The gateway's `sakauth` extension is required for authentication.

```bash
make deb      # DEB only
make rpm      # RPM only
make package  # Both formats
```

The targets first prepare all three binaries and their notices in `dist/package-input/`, then invoke nFPM using `nfpm/fleetint-oob.yaml`. Packages go to `dist/packages/`. Build on the target Linux architecture; cross-compilation is not configured.

Override `VERSION`, `ARCH` (`amd64` or `arm64`), `INPUTDIR`, `OUTPUTDIR`, or the `NFPM` executable as needed. `NICO_SOURCE` and `AGENT_SOURCE` can point to existing checkouts at the selected revisions; the Make targets verify their revisions and do not change those checkouts. Versions must be valid in both package formats, for example `0.1.0~prototype1`. The package revision is 1.

Both Rust binaries use Rust's self-contained musl linker defaults; `musl-gcc`
is used for their C dependencies. The gateway uses `CGO_ENABLED=0`.
`make packages-test-linking` builds and runs a small Rust executable with the
package toolchain and checks it for ELF interpreter and library dependencies.
The Docker package build runs this check before building the components. The build
rejects binaries that contain an ELF interpreter or shared-library dependency.
The package therefore does not depend on glibc, libgcc, or OpenSSL. Systemd and
CA certificates are still required on the host. Musl DNS behavior can differ
from glibc, so validate name resolution in the target environment.

## Configure a test VM

Requires systemd. All services run as the dedicated `fleetint-oob` system account with an empty
capability set, a read-only system filesystem, and private temporary directories.
Running as non-root limits file access, but does not stop a process from using
setuid programs or writing to paths its account owns. The systemd restrictions
block privilege gains and limit filesystem, temporary-file, and device access.
Installation creates the `fleetint-oob` service account and reloads units without
starting collection. Install the package:

```bash
sudo apt install ./dist/packages/fleetint-oob_*.deb
# RPM systems: sudo dnf install ./dist/packages/fleetint-oob-*.rpm
sudo install -m 0640 -o root -g fleetint-oob \
  /usr/share/doc/fleetint-oob/credentials.example.json \
  /etc/fleetint-oob/credentials/credentials.json
```

The package installs `/etc/fleetint-oob/config.toml` with a commented example
target and mode `0640`, preserving edits on upgrade. Uncomment the example and
set a real BMC address and MAC before starting services. An empty target list
is rejected by the collector. See
[configuration](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/docs/configuration.md). Set backend/proxy settings in
`/etc/fleetint-oob/service.conf`. Prefer `FLEETINT_AGENT_TOKEN_FILE` pointing to
a root-owned, group-readable `0640` token file such as
`/etc/fleetint-oob/agent-token`. Existing `FLEETINT_AGENT_TOKEN` settings continue
to work; an explicitly configured token file takes precedence and read failures
stop startup visibly. `service.conf` is
installed root-owned with mode `0600`. Systemd reads it, and small shell entry points load the token before
executing the collector or gateway. Tokens never appear in process arguments.
NICo does not load `service.conf`; proxy and logging variables in that file
apply to the collector and gateway only.

Set each target's `credentials_profile` to a profile in the shared JSON file,
or provide a per-MAC override. A placeholder example is installed at
`/usr/share/doc/fleetint-oob/credentials.example.json`. Copy it to
`/etc/fleetint-oob/credentials/credentials.json` using the command above, then
replace placeholder values only in that copy. Keep the credentials file owned
by root, readable by group `fleetint-oob`, with mode `0640`; the installed documentation example must contain
placeholders only.
NICo's `run-nico` launcher calls the Rust `bmc-collector render-nico-config`
command on every start, using `/etc/fleetint-oob/config.toml` as the source for
BMC addresses, MACs, labels, and credential profiles. The generated NICo TOML
references the same credentials JSON file as the collector; credential values
stay in that file. This needs no separate TOML parser or Python dependency.
Do not edit the generated file under `/run`; it is replaced on the next start.

Configure NICo with flat snake_case fields under `[nico_health]` in `/etc/fleetint-oob/config.toml`. See [NICo collection controls](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/docs/configuration.md#nico-collection-controls) for supported fields and defaults. Run `sudo systemctl restart fleetint-nico-health.service` after editing; the launcher regenerates NICo's configuration before starting it.

The packaged `service.conf` supplies component reporting metadata through `FLEETINT_NICO_ENABLED`, `FLEETINT_NICO_TELEMETRY_DESTINATION`, and `FLEETINT_OTEL_GATEWAY_ENABLED`. Set enablement to `false` when excluding a component, and set the NICo destination to `disabled` when excluding NICo or `external` when using your own gateway. Restart the collector after changing this file.

Set `BACKEND_ENDPOINT` and `ENROLL_ENDPOINT` explicitly when changing the backend. Environment files do not expand references to other variables. The host needs access to the BMCs and backend, but no local GPU. `FLEETINT_BACKEND_URL` is the collector API origin; `BACKEND_ENDPOINT` is the gateway telemetry base URL (including `/api/v1/health`), and `ENROLL_ENDPOINT` is its enrollment URL (including `/api/v1/health/enroll`).

```bash
sudo systemctl enable --now fleetint-oob.target
systemctl status fleetint-oob-collector.service fleetint-nico-health.service \
  fleetint-otel-gateway.service
```

The target groups services; inspect each service to verify health. The
`run-service` launcher loads token files before executing the collector or
gateway. The separate `run-nico` launcher creates a fresh configuration directory
under `/run` on each start and removes it if rendering fails. Keeping rendering
and execution in one script passes the fresh path directly to NICo without
sharing state between `ExecStartPre` and `ExecStart`. Telemetry listeners
bind to loopback. The collector and gateway use the same token source.
[Sysusers](https://www.freedesktop.org/software/systemd/man/latest/systemd-sysusers.html)
creates the service account.
[Tmpfiles](https://www.freedesktop.org/software/systemd/man/latest/tmpfiles.d.html)
grants that account read access
to the existing standard configuration and credential files during installation
and upgrades, while retaining root ownership. The rules are stored in
`/usr/lib/fleetint-oob/permissions.conf` and invoked explicitly by package hooks;
they do not reset operator permissions at boot. Symlinked credentials are not
followed for permission changes. Credentials at custom paths must already be readable by
`fleetint-oob`. Configuration changes remain operator-owned.

## Upgrade and remove

Packages preserve configuration. An upgrade restarts only services that were
already active; stopped services remain stopped. The collector drains inventory
uploads before exiting. Restart the target after changing targets, tokens, or
shared settings. The collector handles SIGINT and SIGTERM during active
collection: it cancels collection, drains completed inventory awaiting upload,
and finishes an in-flight upload before stopping. Its systemd stop timeout is
300 seconds to accommodate the bounded inventory queue and HTTP retries.

Keep `FLEETINT_INTERVAL_SECONDS` set in `service.conf` (default: `86400`).
Command-line parse errors, missing tokens, and token-file read failures are not
retried. Other startup failures,
including enrollment failures, restart after 10 seconds using the host's default
systemd start-limit policy. The package does not set a stricter start limit.

```bash
sudo systemctl restart fleetint-oob.target
sudo journalctl -u fleetint-oob-collector.service

sudo systemctl disable --now fleetint-oob.target
sudo apt remove fleetint-oob
# RPM systems: sudo dnf remove fleetint-oob
```

Package-owned configuration is preserved on removal; Debian purge removes package-owned configuration. Operator-created BMC credential files remain. Delete unused credentials explicitly. Backend data remains.

Debian hooks use `deb-systemd-invoke` for stopping services and
`deb-systemd-helper` to preserve enablement on removal/reinstallation and clean
up on purge, following the [debhelper convention](https://manpages.debian.org/bookworm/debhelper/dh_installsystemd.1.en.html).
Debian alternate-root installs validate the target's passwd/group records
without requiring executable tools inside the target.
RPM hooks use explicit systemd commands in nFPM scriptlets, following the
[systemd packaging lifecycle](https://www.freedesktop.org/software/systemd/man/latest/daemon.html).
Fresh installations do not enable or start collection. Upgrades restart only
active services after installing their new binaries. Service-management failures
emit warnings while allowing package transactions to complete. Account creation
and credential-permission failures stop installation.

Run `make packages-test-hooks packages-test-build` for isolated lifecycle and build regression tests. They use mocked service/compiler commands and local Git fixtures; they never manage host services or fetch remote sources.

Package checks run with `make packages-check-deb` or `make packages-check-rpm`
using the distribution's inspection tools. They never install packages.
Real service startup, upgrade/rollback, and shutdown during collection still
need validation in a disposable VM before production use. To verify the signature of a downloaded Debian release, see [verify host-package releases](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/docs/host-package-release.md).

## Docker validation

`make debian-test` builds the same pinned native sources and nFPM manifest in
Docker, installs the actual package in Debian 12, starts all three daemons under
the service account, verifies enrollment and authenticated gateway export against
a loopback fixture, and checks configuration preservation, upgrades, removal,
and purge. The shell tests also cover environment/file token selection, active-only
restarts, visible failure reporting, and permission migration without following
credential symlinks. No customer backend or BMC is contacted.

`make package-snapshot` writes an unsigned snapshot to `dist/snapshot/`.
`make debian-package PACKAGE_VERSION=1.2.3-rc.1` overrides the release version.
The package metadata uses Debian's `~` prerelease ordering; exported filenames
retain the requested version. Docker builds and tests do not boot systemd as
PID 1. Validate service sandboxing and musl DNS on disposable target VMs before
production rollout. RPM lifecycle hooks retain isolated shell coverage; a real
RPM install and upgrade still require validation on a target distribution.
