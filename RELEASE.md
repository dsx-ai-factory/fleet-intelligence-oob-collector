# Release Process

Maintainers publish release artifacts through the **Release** workflow using a semantic version such as `v0.2.0` or `v0.2.0-rc.1`. Image tags and chart versions omit the leading `v`. Published versions are immutable.

| Artifact | Location |
| --- | --- |
| Collector image | `nvcr.io/nvidia/fleet-intelligence/oob-collector` |
| NICo hardware health image | `nvcr.io/nvidia/fleet-intelligence/nico-hardware-health` |
| Gateway image | `nvcr.io/nvidia/fleet-intelligence/otel-gateway` |
| DEB/RPM packages (`amd64`, `arm64`) | GitHub release assets |
| Signed checksums, public key, package verifier | GitHub release assets |
| Helm chart | `fleet-intelligence-oob-collector` |
| Helm repository | `https://helm.ngc.nvidia.com/nvidia/fleet-intelligence` |

New images and charts are published only to NGC. The gateway image is provided by Fleet Intelligence Agent. See [Installation](docs/installation.md) for installation and verification commands.

Push the release tag from the reviewed release commit to automatically start **Release**, then approve the pending environment jobs. To run it manually, select that same tag and version:

```bash
gh workflow run release.yml --ref v0.2.0 -f version=v0.2.0
```

Publishing images and charts requires a tag ref matching the version input and the `ngc-publish` GitHub environment. Configure that environment to allow only release tags, require reviewer approval with self-review and administrator bypass disabled, and store `NGC_API_KEY` only as an environment secret. Restrict release tag creation, updates, and deletion to authorized maintainers through a tag ruleset.

After the workflow succeeds, verify that the public NGC images and chart for that version are downloadable and match the built image digests and chart checksum. Verify the host-package assets in the draft GitHub release reserved by the workflow, then publish that existing draft. The draft must contain four packages (DEB and RPM for `amd64` and `arm64`), `checksums.txt`, `checksums.txt.asc`, `fleet-intelligence.pub.asc`, and `verify-linux-package-signature.sh`. Publishing a stable GitHub release automatically runs **Publish Fern Docs**, which builds versioned documentation from release tags. Tag pushes and draft releases do not publish docs. Mark RC releases as prereleases; they do not publish docs.

## Host-package signing and retries

`scripts/package-release.cjs` handles GitHub draft reservation and asset uploads. `scripts/package-signing.sh release <dist-directory> <version>` signs the packages and checksum manifest, verifies the returned signatures, and assembles the release files. Its `packages` and `manifest` commands support signing those artifacts individually. `scripts/verify-package-release.sh` checks a complete release; `scripts/verify-linux-package-signature.sh` is also shipped as the standalone package verifier.

The workflow builds and tests both package formats on native amd64 and arm64 runners, signs them on a Linux x64 self-hosted runner, verifies the signed packages and checksum manifest, and uploads them to the reserved draft. Signing does not use the `ngc-publish` environment; configure `NVSEC_SSA_CLIENT_ID` and `NVSEC_SSA_CLIENT_SECRET` as repository secrets accessible to the signing job. The runner requires `nvsec`, `gpg`, `gpgv`, `ar`, `dpkg-deb`, `rpm`, `sha256sum`, `md5sum`, and `sha1sum`.

The default signing scopes and job types are in `.github/workflows/release.yml`. Override them with `NVSEC_LINUX_PACKAGE_SSA_SCOPE`, `NVSEC_LINUX_PACKAGE_JOB_TYPE`, `NVSEC_CHECKSUM_SSA_SCOPE`, and `NVSEC_CHECKSUM_JOB_TYPE` repository variables if needed; `NSPECT_ID` is optional. The committed public key must match fingerprint `FE0C8B74CA66357C13BE197DCCE3C963087199B3`.

To recover an interrupted upload, rerun only the failed jobs in the original workflow run. This reuses its original signed artifact, retained for seven days. Starting a new run rebuilds and signs again and can produce different bytes; existing release assets are immutable. If the artifact has expired, recover the original signed files before proceeding.

Download the complete draft asset set into an empty directory. Verify the public-key fingerprint against the value above, verify the detached signature over `checksums.txt`, and check every checksum before installing. From the matching source tag, run the combined verification (requires Debian and RPM inspection tools):

```bash
scripts/verify-package-release.sh /path/to/downloaded-assets
```

For installation and required host configuration, see the [host packaging guide](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/packages/README.md).
