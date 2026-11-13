# Install on Linux (DEB/RPM)

Install `fleetint-oob` to run the OOB collector, NICo hardware health, and the telemetry gateway as systemd services. You need administrator access, network connectivity to your BMCs and Fleet Intelligence, BMC addresses and real MAC addresses, BMC credentials, and a Fleet Intelligence collector token. Use a DEB or RPM Linux host with systemd and an `amd64` (x86-64) or `arm64` (AArch64) processor. For Kubernetes, follow [Install on Kubernetes](installation.md).

## Download the package

Open [GitHub Releases](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/releases), choose a release, and download the package for your distribution and processor:

| Distribution | Release asset |
| --- | --- |
| Debian or Ubuntu | `fleetint-oob_<version>_<arch>.deb` |
| Rocky Linux or another RPM-based distribution | `fleetint-oob_<version>_<arch>.rpm` |

Use `amd64` for x86-64 or `arm64` for AArch64. For RPM installation, also download `fleet-intelligence.pub.asc` from the same release.

## Install

Run these commands from the download directory, replacing the package filename with the one you downloaded.

For DEB systems:

```bash
sudo apt install "./fleetint-oob_<version>_<arch>.deb"
```

For RPM systems, import the package signing key and install:

```bash
sudo rpm --import ./fleet-intelligence.pub.asc
sudo dnf install "./fleetint-oob_<version>_<arch>.rpm"
```

Installation creates the `fleetint-oob` service account and configuration directory. Services stay stopped until you configure your targets and start the stack.

## Configure your first target

### Set BMC credentials

Copy the example to the protected credential directory, then edit it with your BMC login details:

```bash
sudo install -d -m 0750 -o root -g fleetint-oob /etc/fleetint-oob/credentials
sudo install -m 0640 -o root -g fleetint-oob \
  /usr/share/doc/fleetint-oob/credentials.example.json \
  /etc/fleetint-oob/credentials/credentials.json
sudoedit /etc/fleetint-oob/credentials/credentials.json
```

Fill in the `bmc` credential profile used by the target below. Keep passwords in this JSON file.

### Set the target list

Edit the shared target configuration:

```bash
sudoedit /etc/fleetint-oob/config.toml
```

Replace the commented example with your target, using its real BMC IP and MAC address. Use the compute zone and node group from your Fleet Intelligence account:

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

Add one `[[targets]]` entry per BMC. The collector and NICo use this shared target list and credential file.

### Set the collector token

Create a protected file containing only your collector token:

```bash
sudo install -m 0640 -o root -g fleetint-oob /dev/null /etc/fleetint-oob/collector-token
sudoedit /etc/fleetint-oob/collector-token
sudoedit /etc/fleetint-oob/service.conf
```

Set this line in `service.conf`:

```ini
FLEETINT_COLLECTOR_TOKEN_FILE=/etc/fleetint-oob/collector-token
```

The collector and gateway use the same token. The packaged backend addresses point to Fleet Intelligence. To use another endpoint or change collection settings, see [Configure Linux packages](host-configuration.md).

## Start and verify collection

Start the services and enable them at boot:

```bash
sudo systemctl enable --now fleetint-oob.target
systemctl is-active fleetint-oob-collector.service \
  fleetint-nico-health.service fleetint-otel-gateway.service
```

All three services should show `active`. Check their logs:

```bash
sudo journalctl -u fleetint-oob-collector.service \
  -u fleetint-nico-health.service -u fleetint-otel-gateway.service \
  -n 100 --no-pager
```

Check for successful inventory collection and uploads, and address any NICo or gateway errors. Confirm fresh inventory and telemetry for your target in Fleet Intelligence.

For troubleshooting and collection settings, use [Configure Linux packages](host-configuration.md#troubleshooting).

## Disconnect a BMC or target

To stop collecting from a BMC, remove its `[[targets]]` entry from `/etc/fleetint-oob/config.toml`:

```bash
sudoedit /etc/fleetint-oob/config.toml
```

If other targets remain, restart the stack and check that collection continues for them:

```bash
sudo systemctl restart fleetint-oob.target
sudo journalctl -u fleetint-oob-collector.service -n 100 --no-pager
```

If you removed the last target, stop and disable the services using the first command in [Uninstall](#uninstall) instead of restarting with an empty target list. You can leave the package installed. Disconnecting a target keeps its existing data in Fleet Intelligence; uploads already in progress can still arrive.

## Uninstall

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
