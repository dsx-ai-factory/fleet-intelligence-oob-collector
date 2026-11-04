# Contributing to Fleet Intelligence Out-of-Band Collector

Thank you for your interest in contributing to Fleet Intelligence Out-of-Band Collector. This project is a Rust workspace for collecting Fleet Intelligence data from server BMC interfaces through out-of-band management paths.

Participation in this project is governed by the [Code of Conduct](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/CODE_OF_CONDUCT.md).

## Ways to Contribute

You can help by:

- Reporting bugs in BMC inventory collection, Redfish mapping, CLI behavior, or backend submission.
- Requesting improvements for platform coverage, output formats, collector domains, or operational workflows.
- Improving documentation, tests, or examples.
- Implementing fixes or features discussed in GitHub issues.

Do not report security vulnerabilities through GitHub issues. Follow [SECURITY.md](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/SECURITY.md) for private vulnerability reporting.

## Before You Start

For non-trivial changes, open an issue first so maintainers can discuss the problem, scope, and expected design. This is especially important for changes to Fleet Intelligence data models, backend API behavior, Redfish traversal, or collector runtime semantics.

## Contribution Workflow

1. Fork this repository on GitHub.
2. Create a branch from `main` named `<type>/<short-dash-separated-name>`, for example `git checkout -b feat/my-feature`.
3. Commit your changes with a DCO sign-off, for example `git commit -s -m "feat(cli): add my feature"`.
4. Push the branch to your fork, for example `git push origin feat/my-feature`.
5. Open a pull request against `main`.

Report bugs and request features through [GitHub Issues](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/issues).

## Development Setup

Install Git, [Rustup](https://rustup.rs/), and a C linker for the host platform (for example, the toolchain provided by `build-essential` on Debian or Ubuntu).

Clone the repository:

```bash
git clone https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector.git
cd fleet-intelligence-oob-collector
```

From the repository root, activate and inspect the pinned Rust toolchain (rustup installs it if needed):

```bash
rustup show
```

Build the workspace:

```bash
cargo build --workspace
```

Run tests:

```bash
cargo test --workspace
```

Code style follows standard Rust formatting. Run formatting and lint checks, and add tests for new functionality, before opening a pull request:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Running the Collector

Copy the example configuration, replace its example values, and print BMC inventory locally:

```bash
umask 077
cp config.example.toml config.toml
cp credentials.example.json credentials.json
chmod 600 config.toml credentials.json
```

Before running inventory, edit `config.toml` with your BMC targets and set `credentials.file = "credentials.json"`. Edit `credentials.json` with your BMC credentials and matching profiles or MAC overrides. The relative credential path resolves beside `config.toml`. Keep both files private and outside commits. Then collect inventory:

```bash
cargo run --locked -- inventory
```

Use `--insecure-tls` only when the test BMC certificate cannot be validated.

Run the long-running collector mode and submit to a backend:

```bash
: "${FLEETINT_AGENT_TOKEN:?Set FLEETINT_AGENT_TOKEN before continuing}"
cargo run --locked -- run \
  --backend-url https://api.example.com
```

The CLI reads the bootstrap token from `FLEETINT_AGENT_TOKEN`, keeping it out of the process arguments.

### Collector TOML

In Kubernetes, the Helm chart generates this file from `inventory` values. When running from source, you write it yourself. It has the same fields as the [Helm targets](docs/configuration.md#targets), in snake_case, and follows the same rules for MACs, labels, placement, and credentials:

```toml
compute_zone = "us-west-dc1"
node_group = "gb200-training"
labels = { site = "dc1" }

[credentials]
file = "credentials.json"

[[targets]]
kind = "compute"
name = "node-01"
ip = "192.0.2.10"
port = 443
mac = "00:11:22:33:44:55"
credentials_profile = "bmc"
labels = { rack = "rack-12" }
```

| TOML field | Helm value |
| --- | --- |
| `compute_zone`, `node_group`, `labels` | `inventory.computeZone`, `inventory.nodeGroup`, `inventory.labels` |
| `credentials.file` | Set by the chart to the mounted Secret. A relative path is resolved from the directory containing the TOML file. |
| `targets[].ip` | `inventory.targets[].address` |
| `targets[].credentials_profile` | `inventory.targets[].credentialsProfile` |
| `targets[].compute_zone`, `targets[].node_group` | `inventory.targets[].computeZone`, `inventory.targets[].nodeGroup` |
| `targets[].kind`, `name`, `port`, `mac`, `labels` | Same names under `inventory.targets[]` |

Unlike Helm, the TOML file accepts a MAC with mixed separators. The collector reports MACs as uppercase with colons and doesn't rewrite the file.

Configure NICo with flat fields under `[nico_health]`, using snake_case equivalents of the Helm fields under `nicoHardwareHealth.config`. See [NICo collection controls](docs/configuration.md#nico-collection-controls) for supported fields, defaults, and validation. The shared settings model drives both `render-nico-config` and the camelCase `collector.config.nicoHealth` report. Regenerate the NICo file and restart NICo after editing.

### CLI reference

The executable is `bmc-collector`. Add `--help` to any command to list its options.

| Command | What it does |
| --- | --- |
| `inventory` | Collects once and prints a table. Add `--format json` for JSON or `--detail` for more columns. |
| `run` | Enrolls with Fleet Intelligence, then collects and uploads inventory and attestation on a schedule. |
| `render-nico-config --output nico.toml` | Generates NICo configuration that references the credentials file without copying passwords. The Helm chart runs it in NICo's init container. |

`inventory` and `run` share these options. The matching Helm values are in [collection settings](docs/configuration.md#collection-settings).

| Option | Default | Description |
| --- | --- | --- |
| `--config` | `config.toml` | Path to the collector TOML file. |
| `--target-concurrency` | `64` | Number of BMCs collected at the same time. |
| `--bmc-max-in-flight` | `2` | Maximum simultaneous Redfish requests to one BMC. |
| `--bmc-requests-per-second` | `10` | Maximum request rate to one BMC. |
| `--bmc-burst` | `10` | Requests allowed in a short burst above that rate. Helm sets it equal to the request rate. |
| `--timeout-seconds` | `30` | Redfish request timeout. |
| `--insecure-tls` | Off | Skip BMC certificate verification. |
| `--disable-latency-probe` | Off | Skip network latency measurements. Also set by `FLEETINT_DISABLE_LATENCY_PROBE`. |

`run` also uses:

| Setting | Description |
| --- | --- |
| `FLEETINT_BACKEND_URL` or `--backend-url` | Fleet Intelligence API base address, without a path. |
| `FLEETINT_AGENT_TOKEN` or `--agent-token` | Bootstrap token. Prefer the environment variable. |
| `--interval-seconds` | Wait time after a cycle ends before the next starts. Defaults to `86400`; `0` is treated as one second. |

`RUST_LOG` sets the log level and defaults to `info`.

`render-nico-config` also accepts `--otlp-endpoint` (default `http://localhost:4317`), `--metrics-endpoint` (default `0.0.0.0:9009`), and `--metrics-prefix` (default `fleetint`). These options only configure NICo; they don't start a gateway.

Do not commit real BMC credentials, customer data, private backend URLs, or raw hardware dumps. The repository ignores the documented local configuration and environment files by default.

See the [README](README.md) for installation, configuration, and architecture guides.

## Implementation Notes

These notes describe how the collector works internally. Read them before changing runtime orchestration, collection flow, identity, batching, or backend behavior. The public [Architecture](docs/architecture.md) guide describes the same behavior from an operator's point of view; keep both in sync.

**Crates.**

| Path | Responsibility |
| --- | --- |
| `src/` | Runtime orchestration, enrollment, and backend session lifecycle |
| `crates/backend-client` | Backend URL validation, HTTP policy, enrollment transport, and typed HTTP errors |
| `crates/bmc-dispatch` | Shared per-BMC concurrency, rate limiting, and fair scheduling |
| `crates/bmc-inventory` | Redfish inventory collection, mapping, and output |
| `crates/bmc-attestation` | Nonce retrieval, GPU evidence collection, and upload |

**Inventory pipeline.** Inventory flows source → model → sink. The Redfish source builds Fleet Intelligence models directly and keeps vendor-specific Redfish structures inside the source. Inventory node identity resolution is independent of attestation.

**Configuration snapshot.** Each cycle loads one resolved target and credential snapshot and synchronizes the dispatcher's BMC endpoints before collecting; inventory and attestation share it. Startup validation supplies the first snapshot. During attestation, a target gets at most one credential reload per attempt. The retry pass reloads credentials for the targets it retries.

**Upload queue.** Completed inventories pass through a bounded channel holding at most twice the target concurrency. Inventory item results are matched to submissions by position. Attestation results are matched by normalized BMC MAC, regardless of response order or MAC formatting. A missing or duplicate result leaves only that target unconfirmed, and an unreadable response leaves its batch unconfirmed. Unexpected rows are reported separately, and accepted items are never retried because another item failed. Attestation finishes collection before uploading, which keeps nonce-driven JWT rotation and the authentication retry deterministic. Attestation upload batches are bounded by the target concurrency. Targets that fail as a whole are held for a retry pass about five minutes after the first pass, and only their final results are uploaded; on shutdown, held inventory is uploaded as it was and attestation is not uploaded. A probe round uses one inventory dispatcher admission, and an interception check against an unroutable address runs once per inventory phase.

**Backend transport.** Enrollment, inventory, nonce, and attestation requests share `backend-client`. The session owns the enrollment credentials, the current JWT, and the enrollment HTTP client, and re-enrollment reuses that client. Only a typed HTTP `401` counts as a JWT rejection; a `403`, or an error message that merely contains `401`, does not trigger re-enrollment. Backend requests default to a 30-second timeout with a connect timeout of at most 10 seconds; nonce and attestation requests use `--timeout-seconds`. Enrollment and inventory follow redirects; nonce and attestation requests don't. Enrollment and inventory paths extend the configured base path, while attestation paths replace it, so the backend URL should be an API origin. CLI help shows the token environment variable's name, never its value.

**Sanitized types.** Product-model URI and error fields use transparent sanitized string types with private inner values, so every public construction path applies the same sanitization, including callers that build models without the Redfish source. They serialize as plain strings. Absolute and scheme-relative references keep only their path and fragment; relative paths stay relative with the query removed.

**Helm rollout.** Helm generates only the collector TOML. ConfigMap checksum annotations restart the collector and NICo when targets change, and NICo's init container regenerates its configuration with `render-nico-config` into a pod-local `emptyDir` volume.

## Pull Requests

When opening a pull request:

- Describe the user-visible behavior change.
- Link related issues.
- Include tests for new behavior or explain why tests are not practical.
- Update the relevant guide under `docs/` or component documentation when commands, configuration, data models, deployment behavior, or output formats change.
- Keep unrelated refactors out of feature and bug fix PRs.

## Commit and Pull Request Conventions

Use the same format for commit subjects and pull request titles:

```text
<type>(<scope>): <description>
```

- Allowed types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `revert`.
- Scope is optional and lowercase, for example `cli`, `inventory`, or `deps`.
- Write a concise, imperative description, for example `fix(inventory): handle missing chassis`.
- For breaking changes, add `!` after the type/scope, for example `feat(cli)!: require a target file`, and explain the impact and required operator actions in the PR body.
- Sign off every commit with `git commit -s`, using your real name and a valid email address, to comply with the DCO.
- Name branches `<type>/<short-dash-separated-name>`, for example `fix/missing-chassis` or `docs/configuration-guide`. Use one of the allowed types above.
- Target `main` unless maintainers specify a release branch.

PR descriptions must explain the behavior change, link related GitHub issues (use `Closes #123` when appropriate), report validation and remaining limitations, and highlight breaking changes. Add relevant tests and documentation, follow the PR template, and ensure CI passes. PR titles describe the final user-visible result and may appear in release notes.

Maintainers should **squash merge** each focused PR, using the PR title as the final commit subject. Contributors may keep separate review commits; they do not need to squash before review. Preserve all contributors' DCO `Signed-off-by:` trailers in the final squash commit and verify the final message before merging.

### PR title validation

The **Contribution conventions / PR title** GitHub Actions check validates PR titles when a PR is opened, edited, or updated. Maintainers should require this check in branch protection and configure squash merging to use the PR title as the final commit subject. Local commit-format hooks are optional; individual commit messages do not become target-branch subjects when squash merging. Contributors must sign off every commit, and maintainers must preserve those sign-offs in the final squash commit.

## Release Process

Pushing a semantic version tag such as `v0.1.0` runs the release validation, publishes `amd64` and `arm64` collector and NICo images, and publishes the Helm chart with the matching version. Pre-release tags such as `v0.2.0-rc.1` are supported and do not update the collector image's `latest` tag.

## Developer Certificate of Origin (DCO)

```text
Developer Certificate of Origin
Version 1.1

Copyright (C) 2004, 2006 The Linux Foundation and its contributors.

Everyone is permitted to copy and distribute verbatim copies of this
license document, but changing it is not allowed.

Developer's Certificate of Origin 1.1

By making a contribution to this project, I certify that:

(a) The contribution was created in whole or in part by me and I
    have the right to submit it under the open source license
    indicated in the file; or

(b) The contribution is based upon previous work that, to the best
    of my knowledge, is covered under an appropriate open source
    license and I have the right under that license to submit that
    work with modifications, whether created in whole or in part
    by me, under the same open source license (unless I am
    permitted to submit under a different license), as indicated
    in the file; or

(c) The contribution was provided directly to me by some other
    person who certified (a), (b) or (c) and I have not modified
    it.

(d) I understand and agree that this project and the contribution
    are public and that a record of the contribution (including all
    personal information I submit with it, including my sign-off) is
    maintained indefinitely and may be redistributed consistent with
    this project or the open source license(s) involved.
```

### How to Sign Your Work

To sign your work and agree to the DCO, you must add a sign-off to every git commit. This is done by using the `-s` flag when committing:

```bash
git commit -s -m "Your commit message"
```

This appends a line that looks like:

```text
Signed-off-by: Your Name <your.email@example.com>
```

You must use your real name and a valid email address. Anonymous contributions or contributions under pseudonyms are not accepted.

If you forget to add the sign-off to a commit, you can amend it:

```bash
git commit --amend --signoff
```

For more information about the DCO, see https://developercertificate.org/.

## License

By contributing, you agree that your contributions will be licensed under the [Apache License 2.0](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/LICENSE).
