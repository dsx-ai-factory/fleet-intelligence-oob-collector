# Release Process

## Release ownership

Only authorized NVIDIA employees can create official releases at this time. Publishing official artifacts through NGC requires NVIDIA-managed access and internal review and promotion steps. Maintainer status alone does not grant release publishing access. See [GOVERNANCE.md](GOVERNANCE.md) for project roles.

Contributors can propose release fixes, tests, and documentation through the normal [pull request workflow](CONTRIBUTING.md#contribution-workflow). A process for external maintainers to publish official releases is not currently available.

## Current release automation

The [release workflow](.github/workflows/release.yml) runs when a semantic version tag such as `v0.1.0` is pushed. An authorized NVIDIA release maintainer coordinates the release and verifies its validation and publication jobs.

The workflow:

1. Validates the release version, runs workspace tests, and lints the Helm chart.
2. Builds and publishes Linux `amd64` and `arm64` collector and NICo hardware health images to GHCR.
3. Packages and publishes the Helm chart to GHCR with a matching chart version and application version.
4. Publishes versioned documentation after chart publication succeeds.

Pre-release tags such as `v0.2.0-rc.1` are supported and do not update the collector image's `latest` tag. See the [installation guide](docs/installation.md) for the currently documented artifact locations.

## NGC publication

NGC is the required distribution channel for official container images and Helm charts. The current release workflow publishes to GHCR; NGC publication and promotion are not yet automated by this repository.

For NGC publication, an authorized NVIDIA release maintainer publishes artifacts to private staging, completes the required security and compliance reviews, and arranges promotion to the public catalog. Before announcing an NGC release, the maintainer verifies public image and chart access and ensures the installation guide references the published artifacts.

Detailed access, review, and promotion instructions are maintained internally. This public document describes release ownership and the publication stages without including internal service addresses or credentials.
