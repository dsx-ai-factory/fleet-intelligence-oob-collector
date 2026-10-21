# Verify host-package releases

Debian releases include signed `fleetint-oob` packages for AMD64 and ARM64,
a signed checksum manifest, the public signing key, and a package verifier.
The package contains the OOB collector, NICo hardware health, and the OTLP
gateway. For installation instructions, see
[host packaging](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/packages/README.md).

## Verify a downloaded release

Download the DEB, `fleet-intelligence.pub.asc`, `checksums.txt`,
`checksums.txt.asc` and `verify-linux-package-signature.sh` from the same release.
Run verification from the download directory using a fresh keyring:

```bash
(
  set -eu
  key_home=$(mktemp -d)
  trap 'rm -rf "$key_home"' EXIT
  fingerprint=$(gpg --batch --homedir "$key_home" --show-keys --with-colons fleet-intelligence.pub.asc | awk -F: '$1 == "fpr" {print $10; exit}')
  test "$fingerprint" = FE0C8B74CA66357C13BE197DCCE3C963087199B3
  gpg --batch --homedir "$key_home" --import fleet-intelligence.pub.asc
  gpg --batch --homedir "$key_home" --verify checksums.txt.asc checksums.txt
  sha256sum -c --ignore-missing checksums.txt
  chmod 0755 verify-linux-package-signature.sh
  ./verify-linux-package-signature.sh fleetint-oob_VERSION_amd64.deb fleet-intelligence.pub.asc
)
```

Replace `VERSION` and architecture with the downloaded package name. The signed
SHA-256 manifest authenticates complete package bytes; the package verifier
also checks the embedded builder signature and its complete payload inventory.
