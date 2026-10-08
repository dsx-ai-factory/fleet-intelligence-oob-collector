# Contributing to Fleet Intelligence Out-of-Band Collector

Thank you for your interest in contributing to Fleet Intelligence Out-of-Band
Collector. This project is a Rust workspace for collecting Fleet Intelligence
data from server BMC interfaces through out-of-band management paths.

## Ways to Contribute

You can help by:

- Reporting bugs in BMC inventory collection, Redfish mapping, CLI behavior, or backend submission.
- Requesting improvements for platform coverage, output formats, collector domains, or operational workflows.
- Improving documentation, tests, or examples.
- Implementing fixes or features discussed in GitHub issues.

Do not report security vulnerabilities through GitHub issues. Follow
[SECURITY.md](SECURITY.md) for private vulnerability reporting.

## Before You Start

For non-trivial changes, open an issue first so maintainers can discuss the
problem, scope, and expected design. This is especially important for changes to
Fleet Intelligence data models, backend API behavior, Redfish traversal, or
collector runtime semantics.

## Contribution Workflow

1. Fork this repository on GitHub.
2. Create a branch from `main` named `<type>/<short-dash-separated-name>`, for example `git checkout -b feat/my-feature`.
3. Commit your changes with a DCO sign-off, for example `git commit -s -m "feat(cli): add my feature"`.
4. Push the branch to your fork, for example `git push origin feat/my-feature`.
5. Open a pull request against `main`.

Report bugs and request features through
[GitHub Issues](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/issues).

## Development Setup

Install the Rust toolchain configured by this repository:

```bash
rustup toolchain install
```

Build the workspace:

```bash
cargo build --workspace
```

Run tests:

```bash
cargo test --workspace
```

Code style follows standard Rust formatting. Run formatting and lint checks,
and add tests for new functionality, before opening a pull request:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Running the Collector

Copy the example configuration, replace its example values, and print BMC
inventory locally:

```bash
cp config.example.toml .config
chmod 600 .config
cargo run -- inventory --config .config
```

Use `--insecure-tls` only when the test BMC certificate cannot be validated.

Run the long-running collector mode and submit to a backend:

```bash
: "${FLEETINT_AGENT_TOKEN:?Set FLEETINT_AGENT_TOKEN before continuing}"
cargo run -- run --config .config \
  --backend-url https://api.example.com
```

The CLI reads the bootstrap token from `FLEETINT_AGENT_TOKEN`, keeping it out of
the process arguments.

Do not commit real BMC credentials, customer data, private backend URLs, or raw
hardware dumps. The repository ignores the documented local configuration and
environment files by default.

See the [README](README.md) for installation, configuration, and architecture
guides.

## Pull Requests

When opening a pull request:

- Describe the user-visible behavior change.
- Link related issues.
- Include tests for new behavior or explain why tests are not practical.
- Update the relevant guide under `docs/` or component documentation when
  commands, configuration, data models, deployment behavior, or output formats
  change.
- Keep unrelated refactors out of feature and bug fix PRs.

## Commit and Pull Request Conventions

Use the same format for commit subjects and pull request titles:

```text
<type>(<scope>): <description>
```

- Allowed types: `feat`, `fix`, `docs`, `test`, `refactor`, `perf`, `chore`, `revert`.
- Scope is optional and lowercase, for example `cli`, `inventory`, or `deps`.
- Write a concise, imperative description, for example `fix(inventory): handle missing chassis`.
- For breaking changes, add `!` after the type/scope, for example `feat(cli)!: remove deprecated flag`, and explain the impact and migration in the PR body.
- Sign off every commit with `git commit -s`, using your real name and a valid email address, to comply with the DCO.
- Name branches `<type>/<short-dash-separated-name>`, for example `fix/missing-chassis` or `docs/configuration-guide`. Use one of the allowed types above.
- Target `main` unless maintainers specify a release branch.

PR descriptions must explain the behavior change, link related GitHub issues
(use `Closes #123` when appropriate), report validation and remaining limitations,
and highlight breaking changes. Add relevant tests and documentation, follow the
PR template, and ensure CI passes. PR titles describe the final user-visible
result and may appear in release notes.

Maintainers should **squash merge** each focused PR, using the PR title as the
final commit subject. Contributors may keep separate review commits; they do not
need to squash before review. Preserve all contributors' DCO `Signed-off-by:`
trailers in the final squash commit and verify the final message before merging.

### PR title validation

The **Contribution conventions / PR title** GitHub Actions check validates PR
titles when a PR is opened, edited, or updated. Maintainers should require this
check in branch protection and configure squash merging to use the PR title as
the final commit subject. Local commit-format hooks are optional; individual
commit messages do not become target-branch subjects when squash merging.
Contributors must still sign off every commit, and maintainers must preserve
those sign-offs in the final squash commit.

## Release Process

Pushing a semantic version tag such as `v0.1.0` runs the release validation,
publishes `amd64` and `arm64` collector and NICo images, and publishes the Helm
chart with the matching version. Pre-release tags such as `v0.2.0-rc.1` are
supported and do not update the collector image's `latest` tag.

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

To sign your work and agree to the DCO, you must add a sign-off to every git
commit. This is done by using the `-s` flag when committing:

```bash
git commit -s -m "Your commit message"
```

This appends a line that looks like:

```text
Signed-off-by: Your Name <your.email@example.com>
```

You must use your real name and a valid email address. Anonymous contributions
or contributions under pseudonyms are not accepted.

If you forget to add the sign-off to a commit, you can amend it:

```bash
git commit --amend --signoff
```

For more information about the DCO, see https://developercertificate.org/.

## License

By contributing, you agree that your contributions will be licensed under the
[Apache License 2.0](LICENSE).
