# Contributing to Fleet Intelligence Out Of Band Collector

Thank you for your interest in contributing to Fleet Intelligence Out Of Band
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

## Development Setup

Install the Rust toolchain configured by this repository:

```bash
rustup toolchain install
```

Build the workspace:

```bash
cargo build
```

Run tests:

```bash
cargo test
```

Run formatting and lint checks before opening a pull request:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Running the Collector

Copy the example configuration, replace its example values, and print BMC
inventory locally:

```bash
cp config.example.toml config.toml
chmod 600 config.toml
cargo run -- inventory --config config.toml
```

Use `--insecure-tls` only when the test BMC certificate cannot be validated.

Run the long-running collector mode and submit to a backend:

```bash
cargo run -- run --config config.toml \
  --backend-url https://api.example.com \
  --agent-token "$FLEETINT_AGENT_TOKEN"
```

Do not commit real BMC credentials, customer data, private backend URLs, or raw
hardware dumps. The repository ignores local `.config`, `.env`, and
`redfish-raw*` files by default.

See the [README](README.md) for architecture, configuration, usage, and
installation guides.

## Pull Requests

When opening a pull request:

- Describe the user-visible behavior change.
- Link related issues.
- Include tests for new behavior or explain why tests are not practical.
- Update the relevant guide under `docs/` or component documentation when
  commands, configuration, data models, deployment behavior, or output formats
  change.
- Keep unrelated refactors out of feature and bug fix PRs.

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
