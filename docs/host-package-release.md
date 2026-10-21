# Signed host-package releases

This flow publishes Debian builds of the three-component `fleetint-oob` host
package. For build and installation instructions, see
[host packaging](../deployments/packages/README.md). RPM generation remains
available through the native build; RPM signing/publication is outside this flow.

## Signed release flow

The tag-triggered `.github/workflows/release.yml` workflow uses the shared
Fleet Intelligence identity, with primary key fingerprint:

```text
FE0C 8B74 CA66 357C 13BE 197D CCE3 C963 0871 99B3
```

The public key is checked in at
`deployments/packages/keys/fleet-intelligence.pub.asc`. Private signing keys
remain in NVIDIA 3S; the workflow does not store or export them.

1. Validate the tag and reserve a private GitHub draft. An existing draft or
   published release blocks a full rerun that would replace versioned artifacts.
2. Build and test release-version DEBs on native AMD64 and ARM64 runners.
3. Use NVIDIA 3S to sign each package, then verify its signing identity and
   signed archive-member hashes before replacing the unsigned input.
4. Regenerate package checksums, include the public key and verification tool,
   then sign and verify the final checksum manifest with 3S.
5. Publish the existing multi-platform images and Helm chart after signing
   succeeds. Upload verified DEBs, key, verifier, checksums and signature to the
   private draft. Publish stable DEBs to the OOB Artifactory repository; release
   candidates skip Artifactory.
6. Publish the verified GitHub release and then the versioned documentation.

Configure the same trusted `self-hosted, rocky8` signing runner class used by
the agent. It needs `nvsec`, GPG/GPGV, GNU archive/hash tools and Bash. Set
repository secrets `NVSEC_SSA_CLIENT_ID` and `NVSEC_SSA_CLIENT_SECRET` for the
approved Fleet Intelligence signing identity. The defaults match the agent:

- `NVSEC_LINUX_PACKAGE_SSA_SCOPE`: `SIGNING_LINUX_PACKAGE_FLEET_INTELLIGENCE`
- `NVSEC_LINUX_PACKAGE_JOB_TYPE`: `LINUX_PACKAGE_FLEET_INTELLIGENCE`
- `NVSEC_CHECKSUM_SSA_SCOPE`: `SIGNING_FLEET_INTELLIGENCE`
- `NVSEC_CHECKSUM_JOB_TYPE`: `FLEET_INTELLIGENCE`

The same names can be overridden through repository variables. Set `NSPECT_ID`
when required by the signing service. For stable publication, configure
`ARTIFACTORY_URL`, `ARTIFACTORY_USER` and `ARTIFACTORY_TOKEN` secrets for the OOB
Debian destination. Packages are indexed as distribution `bookworm`, component
`main`, with their native architecture. Use read/deploy permission without
Delete/Overwrite permission to enforce immutability on the server. The uploader
uses HTTPS, refuses redirects and sends a conditional create request.

The signing job checks configuration before any image or chart is published.
Missing tools, credentials, returned signatures or valid signature coverage
fail the job. If a job fails, rerun the failed jobs to retain the reserved draft
and already verified artifacts. GitHub draft assets and existing Artifactory
packages are skipped only when their SHA-256 digests match; differing content
is rejected. Do not move a published tag or replace a released artifact. Use a
new version if an incompatible partial release cannot be resumed.

### Verify a downloaded release

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

### Test release automation locally

```bash
make release-test
```

These tests generate ephemeral OpenPGP test keys, simulate 3S results, and mock
Artifactory and GitHub calls. They exercise successful signing, missing output,
wrong identities, tampered packages and manifests, incomplete signature
coverage, redirects, and idempotent publication. They do not submit production
3S requests or publish artifacts. Live signing and publication require the
configured runner and repository secrets.
