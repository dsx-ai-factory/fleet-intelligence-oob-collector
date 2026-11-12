# Install on Linux (DEB/RPM)

Install `fleetint-oob` on a Linux host to run the OOB collector, NICo hardware health, and the authenticated telemetry gateway as systemd services. The host needs network access to your BMCs and Fleet Intelligence; it does not need a local GPU or Kubernetes. For Kubernetes deployments, follow [Install on Kubernetes](installation.md).

## Before you start

You need:

- A Linux host running systemd, with administrator access and an `amd64` (x86-64) or `arm64` (AArch64) processor.
- A package manager that accepts DEB or RPM packages, plus CA certificates.
- BMC addresses, their real MAC addresses, and login credentials.
- A Fleet Intelligence bootstrap token from your administrator.
- `curl`, GnuPG (`gpg` and `gpgv`), and GNU coreutils for download verification. DEB signature verification also uses binutils (`ar`) and `awk`; RPM verification uses `rpm`.

Package installation and daemon tests run in Debian 12 and Rocky Linux 9 containers. Those tests do not boot systemd. Validate service startup, DNS, and the service sandbox on a disposable VM running your target distribution before a production rollout.

## Download and verify the package

Choose a published version from [GitHub Releases](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/releases). Set the version without the leading `v`, your architecture, and your package format:

```bash
export OOB_VERSION='<release-version>'
export OOB_ARCH=amd64       # amd64 or arm64
export OOB_FORMAT=deb       # deb or rpm
export OOB_PACKAGE="fleetint-oob_${OOB_VERSION}_${OOB_ARCH}.${OOB_FORMAT}"
export OOB_RELEASE_URL="https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/releases/download/v${OOB_VERSION}"

umask 077
mkdir "fleetint-oob-${OOB_VERSION}-${OOB_ARCH}"
cd "fleetint-oob-${OOB_VERSION}-${OOB_ARCH}"
for asset in "$OOB_PACKAGE" checksums.txt checksums.txt.asc \
  fleet-intelligence.pub.asc verify-linux-package-signature.sh; do
  curl --fail --location --remote-name "$OOB_RELEASE_URL/$asset"
done
```

Authenticate the public key against the Fleet Intelligence signing fingerprint below. Verify the signed checksum manifest before executing the downloaded verifier:

```bash
mkdir -m 0700 verification
fingerprint=$(gpg --batch --homedir "$PWD/verification" \
  --show-keys --with-colons fleet-intelligence.pub.asc |
  awk -F: '$1 == "fpr" { print $10; exit }')
test "$fingerprint" = FE0C8B74CA66357C13BE197DCCE3C963087199B3
```

Continue only if the fingerprint matches. Then run these checks; the `&&` chain stops if verification fails:

```bash
gpg --batch --homedir "$PWD/verification" --dearmor \
  --output verification/fleet-intelligence.gpg fleet-intelligence.pub.asc &&
gpgv --status-fd 1 --keyring "$PWD/verification/fleet-intelligence.gpg" \
  checksums.txt.asc checksums.txt > verification/manifest-status.txt &&
grep -q '^\[GNUPG:\] VALIDSIG FE0C8B74CA66357C13BE197DCCE3C963087199B3 ' \
  verification/manifest-status.txt &&
sha256sum --check --ignore-missing checksums.txt &&
bash ./verify-linux-package-signature.sh "$OOB_PACKAGE" fleet-intelligence.pub.asc
```

The manifest lists packages for both architectures and formats. `--ignore-missing` checks the files you downloaded; the final command requires your selected package to exist and verifies its embedded signature. Proceed with installation only after every check succeeds.

## Install

For DEB systems:

```bash
sudo apt install "./$OOB_PACKAGE"
```

For RPM systems, import the public key you verified, then install:

```bash
sudo rpm --import fleet-intelligence.pub.asc
sudo dnf install "./$OOB_PACKAGE"
```

Installation creates the `fleetint-oob` service account. A fresh installation leaves collection stopped and disabled so you can configure targets and credentials first.

## Configure your first target

The package installs configuration under `/etc/fleetint-oob/`. See [Configure Linux packages](host-configuration.md) for the complete file and setting reference.

### Set BMC credentials

Copy the installed example into the protected credential directory, then edit that copy with your BMC login details:

```bash
sudo install -d -m 0750 -o root -g fleetint-oob /etc/fleetint-oob/credentials
sudo install -m 0640 -o root -g fleetint-oob \
  /usr/share/doc/fleetint-oob/credentials.example.json \
  /etc/fleetint-oob/credentials/credentials.json
sudoedit /etc/fleetint-oob/credentials/credentials.json
```

Use the `bmc` profile for the example target below. Keep passwords in the credential file; keep the installed documentation example unchanged.

### Set the target list

Edit `/etc/fleetint-oob/config.toml` and replace its commented example with a real target. For example:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"

[credentials]
file = "/etc/fleetint-oob/credentials/credentials.json"

[[targets]]
kind = "compute"
name = "node-01"
ip = "192.0.2.10"
mac = "02:00:00:00:00:01"
credentials_profile = "bmc"
```

Replace the address, MAC, and placement with your actual values. At least one target is required. The collector and NICo use the same target list and credential file.

### Set the bootstrap token

Create a protected token file and edit it to contain only your bootstrap token:

```bash
sudo install -m 0640 -o root -g fleetint-oob /dev/null /etc/fleetint-oob/agent-token
sudoedit /etc/fleetint-oob/agent-token
sudoedit /etc/fleetint-oob/service.conf
```

Set this line in `service.conf`:

```ini
FLEETINT_AGENT_TOKEN_FILE=/etc/fleetint-oob/agent-token
```

The collector and gateway read the same token file. The packaged backend addresses already point to production. For another endpoint, update [all three backend settings](host-configuration.md#fleet-intelligence-connection).

## Start and verify collection

```bash
sudo systemctl enable --now fleetint-oob.target
systemctl is-active fleetint-oob-collector.service \
  fleetint-nico-health.service fleetint-otel-gateway.service
sudo journalctl -u fleetint-oob-collector.service -n 100 --no-pager
sudo journalctl -u fleetint-nico-health.service -n 100 --no-pager
sudo journalctl -u fleetint-otel-gateway.service -n 100 --no-pager
```

All three services should be active. An active `fleetint-oob.target` alone does not establish service health. Check the collector logs for successful inventory uploads, NICo logs for startup or BMC errors, and gateway logs for enrollment/export errors. Confirm fresh inventory and telemetry for your target in Fleet Intelligence.

Optional local health checks are available at `http://127.0.0.1:9009/livez` for NICo and `http://127.0.0.1:13133/` for the gateway. A healthy process does not by itself prove that BMC collection or backend export succeeds.

| Problem | What to check |
| --- | --- |
| Collector or gateway stops with exit status `2` | Token path, readability by `fleetint-oob`, and any command-line parse error. Correct the setting and restart the affected service. |
| Collector repeatedly fails to enroll | Bootstrap token, backend addresses, host connectivity, proxy, and backend certificate trust. |
| Inventory cannot read a BMC | Target address/MAC, credential profile or MAC override, and BMC certificate trust if you enabled verification. See [proxy and TLS](host-configuration.md#proxy-and-tls). |
| NICo fails before starting | TOML syntax, a readable credentials file, and valid `[nico_health]` settings. |
| Services run but telemetry is absent | BMC credentials, NICo logs, the OTLP destination, all gateway URLs, and gateway enrollment/export errors. |

## Upgrade

Download and verify the new package using the steps above, then run the same `apt install` or `dnf install` command with the new file. Modified package configuration is preserved. Review package-manager prompts and merge any new configuration defaults into your local settings as needed.

An upgrade restarts services that were already active and leaves stopped services stopped. Review each service after the transaction. Service-management failures are reported as warnings; a completed package transaction alone does not prove that a restart succeeded.

## Remove a target

Before deleting a machine in Fleet Intelligence, remove its `[[targets]]` entry from `/etc/fleetint-oob/config.toml` and restart the stack:

```bash
sudo systemctl restart fleetint-oob.target
```

Confirm that collection continues for the remaining targets. A few uploads already in progress can still arrive. If you remove the last target, stop or uninstall the stack instead of starting it with an empty target list. Remove unused per-MAC credential overrides only after nothing else uses them. Existing backend data remains.

## Stop or uninstall

Stop collection and disable startup:

```bash
sudo systemctl disable --now fleetint-oob.target \
  fleetint-oob-collector.service fleetint-nico-health.service fleetint-otel-gateway.service
```

Stopping the collector can take up to 300 seconds while it finishes pending inventory uploads. Remove the package with your package manager:

For DEB systems:

```bash
sudo apt remove fleetint-oob
```

For RPM systems:

```bash
sudo dnf remove fleetint-oob
```

Debian removal retains package configuration; `sudo apt purge fleetint-oob` removes package-owned configuration. RPM removal saves modified package configuration as `.rpmsave` files. Operator-created credential/token files and the service account remain. Retire credentials through their issuing systems, then delete unused local copies explicitly. Removing the package does not delete Fleet Intelligence data.
