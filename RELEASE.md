# Release Process

Maintainers publish release artifacts through the **Release** workflow using a semantic version such as `v0.2.0` or `v0.2.0-rc.1`. Image tags and chart versions omit the leading `v`. Published versions are immutable.

| Artifact | Location |
| --- | --- |
| Collector image | `nvcr.io/nvidia/fleet-intelligence/oob-collector` |
| NICo hardware health image | `nvcr.io/nvidia/fleet-intelligence/nico-hardware-health` |
| Gateway image | `nvcr.io/nvidia/fleet-intelligence/otel-gateway` |
| Helm chart | `fleet-intelligence-oob` |
| Helm repository | `https://helm.ngc.nvidia.com/nvidia/fleet-intelligence` |

New images and charts are published only to NGC. The gateway image is provided by Fleet Intelligence Agent. See [Installation](docs/installation.md) for installation and verification commands.

Push the release tag from the reviewed release commit to automatically start **Release**, then approve the pending environment jobs. To run it manually, select that same tag and version:

```bash
gh workflow run release.yml --ref v0.2.0 -f version=v0.2.0
```

Publishing requires a tag ref matching the version input and the `ngc-publish` GitHub environment. Configure that environment to allow only release tags, require reviewer approval with self-review and administrator bypass disabled, and store `NGC_API_KEY` only as an environment secret. Restrict release tag creation, updates, and deletion to authorized maintainers through a tag ruleset.

Successful stable releases automatically run **Publish Fern Docs** after the images and chart are published. Versioned documentation is built from release tags. Prereleases do not trigger docs publishing.
