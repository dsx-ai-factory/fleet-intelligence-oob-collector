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

After publishing the artifacts, push the matching release tag from the commit used by the **Release** workflow. Stable `vMAJOR.MINOR.PATCH` tag pushes automatically run **Publish Fern Docs**, which builds versioned documentation from release tags. Prerelease tags do not trigger docs publishing.
