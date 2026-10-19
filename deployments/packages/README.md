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
`pkg-config`, OpenSSL development headers, and protobuf tools. On Ubuntu/Debian,
the native development packages are:

```bash
sudo apt-get install build-essential clang cmake pkg-config libssl-dev \
  protobuf-compiler libprotobuf-dev
```

The Make targets build the collector with Cargo, build only NICo's
`carbide-health` / `forge-hw-health` binary with Cargo, and build the custom
OpenTelemetry gateway through the agent's pinned collector builder. Rustup
uses each source repository's pinned toolchain. No Docker is required.

Pinned NICo and agent sources are checked out under `dist/package-sources/`.
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
existing compatible checkouts; the Make targets do not change those checkouts.
Versions must be valid in both package formats, for example `0.1.0~prototype1`. The package revision is 1.

**Prototype limitation:** use artifacts built for the selected architecture
and distribution baseline. The package declares system dependencies;
add the actual native-library dependencies to the configuration's `depends`
list before distributing dynamically linked binaries. nFPM does not discover
those dependencies.

## Configure a test VM

Requires systemd 247 or newer. All services run as root. Installation reloads
units without creating accounts or starting collection. Install the package:

```bash
sudo apt install ./dist/packages/fleetint-oob_*.deb
# RPM systems: sudo dnf install ./dist/packages/fleetint-oob-*.rpm
sudo install -m 0600 -o root -g root \
  /usr/share/doc/fleetint-oob/config.example.toml \
  /etc/fleetint-oob/config.toml
sudo install -m 0600 -o root -g root /secure/path/to/bmc-username \
  /etc/fleetint-oob/credentials/username
sudo install -m 0600 -o root -g root /secure/path/to/bmc-password \
  /etc/fleetint-oob/credentials/password
```

Edit `config.toml` with the target BMCs and MACs; see
[configuration](../../docs/configuration.md). Set backend/proxy settings in
`/etc/fleetint-oob/service.conf`, including `FLEETINT_AGENT_TOKEN`. This file is
installed root-owned with mode `0600`. Both services read it directly through
systemd `EnvironmentFile`; no shell wrappers are used.

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
or shared settings.

```bash
sudo systemctl restart fleetint-oob.target
sudo journalctl -u fleetint-oob-collector.service

sudo systemctl disable --now fleetint-oob.target
sudo apt remove fleetint-oob
# RPM systems: sudo dnf remove fleetint-oob
```

Operator-created configuration and credentials remain, including on purge. Delete unused credentials explicitly. Backend data remains.

Debian hooks use `deb-systemd-invoke` for stopping services and
`deb-systemd-helper` to preserve enablement on removal/reinstallation and clean
up on purge, following the [debhelper convention](https://manpages.debian.org/bookworm/debhelper/dh_installsystemd.1.en.html).
RPM hooks use explicit systemd commands in nFPM scriptlets, following the
[systemd packaging lifecycle](https://www.freedesktop.org/software/systemd/man/latest/daemon.html).
The prototype does not automatically enable/start services or restart them on
upgrade. Service-management failures do not fail package transactions.

Run `make packages-test-hooks` for isolated lifecycle tests with mocked service
commands; these never manage host services.

Package checks run with `make packages-check-deb` or `make packages-check-rpm`
using the distribution's inspection tools. They never install packages.
Real service startup, upgrade/rollback, and shutdown during collection still
need validation in a disposable VM before production use. Signing and release
publishing are outside this prototype.
