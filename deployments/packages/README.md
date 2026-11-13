# Host packages

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

Pinned NICo and agent sources are checked out under revision-specific directories in `dist/package-sources/`. Changing a pinned revision requires reviewing and updating `NOTICE` and selects a new checkout; an existing custom checkout must match the reviewed revision. NICo uses the same source revision as the release workflow. Build outputs stay under `target/` and `dist/`; upstream sources are not edited. The gateway's `sakauth` extension is required for authentication.

```bash
make deb      # DEB only
make rpm      # RPM only
make package  # Both formats
```

The targets first prepare all three binaries in `dist/package-input/`, then invoke nFPM using `nfpm/fleetint-oob.yaml`. Packages go to `dist/packages/`. Build on the target Linux architecture; cross-compilation is not configured.

Override `VERSION`, `ARCH` (`amd64` or `arm64`), `INPUTDIR`, `OUTPUTDIR`, or the `NFPM` executable as needed. `NICO_SOURCE` and `AGENT_SOURCE` can point to existing checkouts at the selected revisions; the Make targets verify their revisions and do not change those checkouts. Versions must be valid in both package formats, for example `0.1.0~prototype1`. The package revision is 1. Effective `NICO_REF` and `AGENT_REF` values, including environment or command-line overrides, must exactly match the full revisions recorded in `NOTICE`. Native package targets and Docker package targets reject mismatches before invoking the packager or Docker.

Both Rust binaries use Rust's self-contained musl linker defaults; `musl-gcc`
is used for their C dependencies. The gateway uses `CGO_ENABLED=0`.
`make packages-test-linking` builds and runs a small Rust executable with the
package toolchain and checks it for ELF interpreter and library dependencies.
The Docker package build runs this check before building the components. The build
rejects binaries that contain an ELF interpreter or shared-library dependency.
The package therefore does not depend on glibc, libgcc, or OpenSSL. Systemd and
CA certificates are still required on the host. Musl DNS behavior can differ
from glibc, so validate name resolution in the target environment.

## Operator documentation

For installation, service configuration, target disconnect, and removal, use [Install on Linux](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/docs/host-installation.md) and [Configure Linux packages](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/docs/host-configuration.md).

## Package validation

Run `make packages-test-hooks packages-test-build` for isolated lifecycle and build regression tests. They use mocked service/compiler commands and local Git fixtures; they never manage host services or fetch remote sources.

Package checks run with `make packages-check-deb` or `make packages-check-rpm`
using the distribution's inspection tools. They never install packages.
Validate systemd startup, service sandboxing, reboot behavior, upgrade/rollback, and shutdown during collection in a disposable VM before production use. See the [release process](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/RELEASE.md) for signed DEB/RPM assets, signing prerequisites, verification, and retry instructions.

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
production rollout. The RPM container tests execute real installation, upgrade, and removal transactions on Rocky Linux 9 while replacing the systemd manager with a stub.

## License notices

The package installs the repository's checked-in `LICENSE` as `copyright` and its checked-in `NOTICE` under `/usr/share/doc/fleetint-oob/`. `NOTICE` covers the collector, pinned NICo health dependencies, and pinned gateway. There are no dependency license directories or build-time notice generation steps.

When changing dependencies or the NICo/agent source pins, review and update the corresponding sections of `NOTICE` in the same change. Run `python3 scripts/check-notice.py` for the offline checks used by CI and release validation: collector Cargo.lock crate/version coverage, license content, component/source revisions, and shared references. Incomplete shared-reference markers are rejected even when another reference uses the same identifier. Dependency headers, URLs, and license-file paths alone do not satisfy coverage. Unrelated build-file edits and lockfile formatting do not require notice updates.

Use the source audit when reviewing attribution from published Rust crates:

1. Fetch the collector and pinned NICo dependencies with `cargo fetch --locked` from their respective source trees. Include dependencies for both supported architectures. The collector lockfile alone does not cover NICo's inventory.
2. Find the Cargo registry source directories under `${CARGO_HOME:-$HOME/.cargo}/registry/src/`. Each supplied directory must contain the published `<name>-<version>` crate directories. Supply each required registry separately:

   ```bash
   python3 scripts/check-notice.py \
     --crate-sources /path/to/first-registry-sources \
     --crate-sources /path/to/second-registry-sources
   ```

3. Review the reported crate names and relative file paths. Copy missing terms into the corresponding dependency entry in `NOTICE`, preserving attribution and license variants, or use a matching shared-license reference. Rerun the audit after editing.

The audit recursively discovers conventional LICENSE, LICENCE, COPYING, UNLICENSE, COPYRIGHT, NOTICE, and AUTHORS filenames and their suffix variants, plus each crate's declared `license-file`. XML schema files are excluded unless explicitly declared as the license file. It checks complete text after normalizing whitespace and expanding shared license references, scoped to each dependency entry. Explicit Apache appendix references can supply common terms. Missing or ambiguous crate directories, mismatching manifest identities, empty files, path-only pointers, and files outside the crate stop the audit with an error. Crates without published license files require manual review against upstream terms; the audit reports those cases rather than claiming complete coverage. Exact text differences also require review, even when the difference is a license URL's scheme.

The source audit reads local files and leaves `NOTICE` unchanged. The default check needs no crate cache; the source audit requires sources for every Rust crate version represented by the NOTICE dependency entries. Review gateway attribution and any unpublished upstream terms separately. The checked-in `NOTICE` remains the source of truth; no generator or fallback directory is required.
