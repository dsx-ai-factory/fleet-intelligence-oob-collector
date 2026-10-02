# Pulse container scanning

`policy.json` is an unchanged copy of the ProdSec-approved release policy from
[`pstooling/container-scan-policy-documents`](https://gitlab-master.nvidia.com/pstooling/container-scan-policy-documents/-/blob/main/prodsec_approved_release_policy.json),
revision `a74a5183b35fbd22d466ab4379aa6fdc030a4972`.
It blocks High/Critical vulnerabilities and warns on Low/Medium vulnerabilities.
Pulse also applies centrally maintained global checks and VEX records.

The approved policy retains its upstream identifiers. If the Security PIC instead
requires a custom project policy, use a project-specific bundle ID and unique
mapping IDs. The upstream documentation says self-managed policies do not pass
LaunchAPI release readiness checks. Do not add local exceptions to this approved
copy; follow the ProdSec Exception Tracker process.

## Configure GitHub

Set these repository or organization variables:

| Variable | Value |
| --- | --- |
| `PULSE_RUNNER` | Optional single-label override. Defaults to `[self-hosted, rocky8]`, matching `fleet-intelligence-agent` releases. The pool must be available to this repository and provide Docker, Bash, Python 3, curl, and NVIDIA backend connectivity. |
| `NSPECT_ID` | Optional override for the default program ID `NSPECT-48VE-47RP`. |
| `PULSE_SCANNER_IMAGE` | Optional scanner reference; defaults to `nvcr.io/0898940053369630/pulse/pulse-container-scanner-cli:stable`. Pin a tested version or digest for controlled scanner upgrades. |
| `PULSE_ENABLED` | Set to `true` only after the manual scan and release gate have been verified. Unset or any other value preserves the existing release behavior. |

Set these secrets:

| Secret | Purpose |
| --- | --- |
| `NGC_API_KEY` | NGC Service Key with access to the Pulse namespace (`sds-ngc-pulse-users`). |
| `SSA_CLIENT_ID` | Team SSA client with approved `nspect.verify` and `scan.anchore` scopes for PRP. |
| `SSA_CLIENT_SECRET` | The SSA client secret. Rotate before expiry. |

The scan generates a fresh short-lived SSA token and uses the job's `GITHUB_TOKEN`
to read GHCR. Ensure the runner and repository token can pull both image packages.
Use a runner isolated from untrusted pull request workloads, and clean its workspace
between jobs. The scanner gets a temporary registry config that is removed on exit.
The container runs as root to read the private registry config and write reports
across the bind mounts, independently of the scanner image's default UID. It has
no Docker socket mount; credentials and policy are mounted read-only. The scanner's
built-in Anchore configuration is retained when present under `/home/scanner`.

The default runner selector follows
[`fleet-intelligence-agent` releases](https://github.com/NVIDIA/fleet-intelligence-agent/blob/main/.github/workflows/release.yml).
On inspection, its online `gpu-health-github-runner-2` runner had the
`self-hosted`, `Linux`, `X64`, and `rocky8` labels. The agent's release workflow
uses this pool for builds and signing, but it does not include a Pulse scan.
Therefore its Pulse connectivity and SSA permissions still need verification.

At setup time, no self-hosted runners were available to this repository.
Ask the runner owner to make the pool available through an organization runner
group, or register an equivalent runner here. Matching labels alone does not
grant access to another repository's runner. Restrict the scanning pool to
trusted release/scanning workflows. A manual scan will remain queued until a
matching runner is available.

Public GitHub-hosted runners cannot be allowlisted for Pulse backend access.
If the existing pool cannot reach Pulse, coordinate approved NVIDIA-dedicated
static egress or AWS PrivateLink through `#securitytools-support` before enabling
scanning. An amd64 runner can inspect both target image platforms.

## Verify and enable

1. Run **Pulse container scan** from GitHub Actions using an existing image digest
   and `linux/amd64`, then repeat with `linux/arm64` for each image.
2. Confirm the reports and policy evaluations are present in the uploaded artifacts.
3. Confirm a failing policy evaluation and a backend/authentication failure fail
   the job. Scan exit codes propagate; reports are uploaded even after failure.
4. Set `PULSE_ENABLED=true`. Verify a release candidate scans four build digests
   before either image receives release tags. A failed scan must prevent both
   image publication jobs and therefore Helm chart and documentation publication.

The release workflow scans the collector and NICo images for amd64 and arm64.
It scans already-pushed build digests, so a failed gate leaves those untagged
digests in GHCR. It prevents release tags and downstream publication.
The chart's external OpenTelemetry gateway image is owned by its publishing
project and is outside this repository's image build gate.

Scan reports are retained for 30 days. This workflow does not register containers
or override reports in nSpect. GHCR is absent from the documented nSpect registry
list, so confirm the supported registration path with ProdSec before adding report
override. LaunchAPI readiness additionally requires the appropriate registration
and approval process; using this policy alone does not establish readiness.

The release owner and Security PIC should review upstream policy changes and
replace the checked-in file from a reviewed upstream revision. Record the new
revision here and rerun manual scans before release. Assign recurring scans of
supported releases. `pulse-rescan.yml` scans both `latest` images on both platforms
every Monday at 08:23 UTC when `PULSE_ENABLED=true`, and can be run manually.
Assign an owner to monitor these workflow failures. Older supported releases
require additional scheduled targets or the team's nSpect registration;
release-only scans do not catch vulnerabilities disclosed after publication.

## Local verification

```sh
bash scripts/tests/test-pulse-scan.sh
bash -n scripts/pulse-scan.sh
cargo test --workspace --locked
```

The script contract tests use simulated Docker and SSA responses. Actual backend
connectivity, registry permissions, scanning results, and release gating require
the configured GitHub runner.

Reference: [Pulse scanner setup](https://gitlab-master.nvidia.com/pstooling/pulse-group/pulse-container-scanner#image-scan).
