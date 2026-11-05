# Release Process

## Release ownership

Only authorized NVIDIA employees can create official releases at this time. Publishing official artifacts through NGC requires NVIDIA-managed access and internal security review and promotion. Maintainer status alone does not grant release publishing access. See [GOVERNANCE.md](GOVERNANCE.md) for project roles.

Contributors can propose release fixes, tests, and documentation through the normal [pull request workflow](CONTRIBUTING.md#contribution-workflow). A process for external maintainers to publish official releases is not currently available.

## Publishing setup

Before the first staging run, an authorized maintainer must provision the project's team in the NGC `dsx-ai-factory` staging organization and configure the GitHub environment `ngc-publish`:

| Setting | Purpose |
| --- | --- |
| Environment secret `NGC_API_KEY` | NGC key authorized to publish the collector and NICo images and Helm chart in the staging team. |
| Environment variable `NGC_ORG` | NGC identifier of the private staging organization. The public `nvidia` organization is rejected as a CI publishing destination. |
| Environment variable `NGC_TEAM` | Provisioned staging team name. |
| Environment variable `NGC_PUBLIC_IMAGE_PREFIX` | Approved final public image namespace, in the form `nvcr.io/nvidia/<namespace>`. CI uses it in the packaged chart before review. |

Under the environment's deployment branch rules, allow only the release branches selected by the maintainers, such as `release-*`. Configure any required environment reviewers. API key authentication is used until NGC supports the required OIDC publishing flow. Keep keys in GitHub secrets, never in repository files or workflow inputs.

## Prepare a candidate

1. Select a reviewed commit with passing required CI checks and a semantic version such as `v0.2.0` or `v0.2.0-rc.1`. Create an allowed release branch, such as `release-0.2.0`, at that commit.
2. Run **Stage NGC release** manually on that branch with the `version` input. The workflow records the source commit and uses the version without the leading `v` for image tags, the chart version, and the chart application version. It does not create a source tag or publish a GitHub release.
3. Confirm all jobs succeed and download the release records from the workflow artifacts. Keep them in the internal release record before the workflow's 90-day artifact retention expires.

The [staging workflow](.github/workflows/release.yml):

1. Validates the version, required publishing settings and key, runs workspace tests, and lints the chart.
2. Builds Linux `amd64` and `arm64` collector and NICo hardware health images and publishes them by digest to private NGC staging. It preserves the pinned NICo source revision.
3. Creates versioned multi-platform image indexes, attaches signed SLSA provenance, and retains their digests and attestation bundles. BuildKit provenance and SBOMs remain attached to the platform images. Staging does not update a `latest` tag.
4. Packages a copy of the chart with the configured final public collector and NICo image repositories, adds its SHA-256 checksum and signed provenance, and uploads it with the NGC CLI to the private staging team.
5. Retains the chart package, checksum, provenance, source commit, pinned NICo source revision, and image digests for internal review.

The workflow publishes neither images nor charts to GHCR. The telemetry gateway is released separately; its chart image reference is not changed by this workflow. Source chart defaults remain available for local development; the packaged NGC chart contains the public collector and NICo image references.

## Review and public promotion

1. Test the exact staged images and chart. To test before public promotion, override the collector and NICo image repositories at installation time to use private staging and supply the required pull credentials. Keep the chart package unchanged.
2. Create or update the product version in NVIDIA's internal security review system with the staged artifact references. Complete the required security and compliance checklists and open-source review requirements. Record approval for both image index digests and the chart checksum.
3. Submit the required promotion request through NVIDIA's internal publishing process. Promote the approved collector and NICo images to the configured public namespace and the approved chart to its assigned public NGC Helm repository. Preserve image digests, chart bytes, SBOMs, and provenance; do not rebuild or repackage during promotion.
4. Verify public pulls for both images and the chart. Compare their digests and checksum with the approved release records and verify their signed provenance. Update the installation documentation if the published artifacts require different instructions.
5. Create the version tag at the exact approved collector source commit and publish the GitHub release with release notes and the public NGC artifact links. Confirm the **Publish Fern Docs** workflow succeeds after the GitHub release is published. Staging and source tag creation alone do not publish documentation.

Internal review and public promotion are manual steps outside this repository's staging workflow. Detailed access, review, and promotion instructions and internal reports stay in NVIDIA's internal records. Public release notes can identify the source commit, public artifact references, digests, checksums, and approval completion.

## Verification and failures

Use `docker buildx imagetools inspect <public-image>:<version>` for each promoted image and compare the index digest with the approved value. Download the chart from the public URL, compare its SHA-256 checksum with `chart.sha256`, and check that its collector and NICo image references resolve to the approved images.

Verify the retained chart attestation with `gh attestation verify <chart-package> --bundle chart.sigstore.json --repo dsx-ai-factory/fleet-intelligence-oob-collector`. Verify image attestations with `gh attestation verify oci://<public-image>@<approved-digest> --bundle <retained-image-bundle> --repo dsx-ai-factory/fleet-intelligence-oob-collector`. Retain the verification results with the release record.

A successful staging build does not authorize public publication. Failed or pending checks block promotion. If a correction changes source or artifacts, build a new candidate and repeat testing and approval. Do not rerun staging builds for an approved candidate or replace published tags or artifact versions. After publication, ship corrections as a new version.

If staging fails before approval, retain the logs and diagnose the failing job before retrying. If public image digests or chart checksums differ from the approved records, stop publication and resolve the discrepancy. Report security-sensitive findings through [SECURITY.md](SECURITY.md).
