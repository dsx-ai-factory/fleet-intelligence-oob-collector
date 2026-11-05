# Release Process

## Release ownership

Only authorized NVIDIA employees can create official releases at this time. Official publication through NGC requires NVIDIA-managed access and internal security review and promotion. Maintainer status alone does not grant release publishing access. See [GOVERNANCE.md](GOVERNANCE.md) for project roles.

Contributors can propose release fixes, tests, and documentation through the normal [pull request workflow](CONTRIBUTING.md#contribution-workflow). A process for external maintainers to publish official releases is not currently available.

## Release artifacts

Use the `fleet-intelligence` team in the NGC `dsx-ai-factory` staging organization. Public artifacts are distributed through the `nvidia/fleet-intelligence` namespace:

| Artifact | Public name or location |
| --- | --- |
| Collector image | `nvcr.io/nvidia/fleet-intelligence/oob-collector` |
| NICo hardware health image | `nvcr.io/nvidia/fleet-intelligence/nico-hardware-health` |
| Shared telemetry gateway image | `nvcr.io/nvidia/fleet-intelligence/otel-gateway` |
| Helm chart name | `fleet-intelligence-oob` |
| Helm repository | `https://helm.ngc.nvidia.com/nvidia/fleet-intelligence` |

NGC is the distribution channel for collector images and the chart. The `fleet-intelligence-oob` chart deploys the collector, NICo hardware health, and the shared telemetry gateway. The Fleet Intelligence Agent repository builds and publishes the gateway image.

Use semantic versions such as `v0.1.0` or `v0.2.0-rc.1`. Image tags, chart versions, and chart application versions omit the leading `v`. Release images support Linux `amd64` and `arm64`.

## Publishing access

The authorized release maintainer needs membership in the NGC `dsx-ai-factory` staging organization and publishing access to its `fleet-intelligence` team. Configure the GitHub environment `ngc-publish` with the environment secret `NGC_API_KEY`, environment variable `NGC_ORG` containing the private staging organization identifier, selected release branch rules such as `release-*`, and any required reviewers. The staging team is fixed to `fleet-intelligence`; CI rejects `nvidia` as the staging organization. API key authentication is used until the required OIDC publishing support is available.

Keep keys in GitHub secrets and internal operational records. Detailed access, security review, and promotion instructions remain in NVIDIA's internal records.

## Stage a candidate

Run **Stage NGC release** manually on the reviewed release branch with the final version, such as `v0.2.0` or `v0.2.0-rc.1`. The workflow runs workspace tests and chart lint, builds both image architectures, creates versioned multi-platform image indexes, and uploads the images and chart to private NGC staging. It retains image digests, signed provenance, the chart checksum, and source revisions as review artifacts for 90 days; save them to the internal release record before they expire.

Staging does not publish to GHCR, update a `latest` tag, create a source tag, or publish documentation. The **Publish Fern Docs** workflow runs when the GitHub release is published after approval and public promotion. The shared gateway version selected in the chart must also be available in public NGC before installation.

## Prepare and review artifacts

1. Select a reviewed source commit with passing CI checks and the final release version. Build the collector and NICo images and the chart through CI.
2. Upload the images to private staging at `nvcr.io/<staging-org-id>/fleet-intelligence/oob-collector` and `nvcr.io/<staging-org-id>/fleet-intelligence/nico-hardware-health`. Upload the chart under `<staging-org-id>/fleet-intelligence/fleet-intelligence-oob`.
3. Record the collector and NICo source revisions, image digests, chart checksum, SBOMs, and signed provenance. The chart must use its final public image references before review; staging tests can override image repositories at installation time without modifying the package.
4. Test those exact staged artifacts and complete NVIDIA's required security and compliance checklists and open-source review requirements. Record approval for the image digests and chart checksum. A successful build alone does not authorize public publication.

## Promote and verify a release

1. Submit the internal publishing request to promote the approved artifacts to the public NGC locations above. Preserve image digests, chart bytes, SBOMs, and provenance; do not rebuild or repackage during promotion.
2. Verify public image and chart access, compare digests and the chart checksum with the approved records, and verify signed provenance.
3. Tag the approved source commit, publish the release notes and public NGC links, and verify versioned documentation publication. See the [installation guide](docs/installation.md) for customer installation commands.

Failed or pending checks block promotion. Changes to an artifact require renewed testing and approval. After publication, ship corrections as a new version rather than replacing published tags or artifacts. Report security-sensitive findings through [SECURITY.md](SECURITY.md).
