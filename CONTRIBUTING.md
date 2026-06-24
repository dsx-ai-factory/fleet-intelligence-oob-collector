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

Print BMC inventory locally:

```bash
cargo run -- inventory --config /path/to/.config --insecure-tls
```

Run the long-running collector mode and submit to a backend:

```bash
cargo run -- run --config /path/to/.config --insecure-tls \
  --backend-url https://api.example.com \
  --agent-token "$FLEETINT_AGENT_TOKEN"
```

Do not commit real BMC credentials, customer data, private backend URLs, or raw
hardware dumps. The repository ignores local `.config`, `.env`, and
`redfish-raw*` files by default.

## Pull Requests

When opening a pull request:

- Describe the user-visible behavior change.
- Link related issues.
- Include tests for new behavior or explain why tests are not practical.
- Update README or crate documentation when commands, configuration, data models, or output formats change.
- Keep unrelated refactors out of feature and bug fix PRs.

## Commit Sign-Off

This project is licensed under Apache-2.0. Contributions should include a
Developer Certificate of Origin sign-off:

```text
Signed-off-by: Your Name <your.email@example.com>
```

Use `git commit -s` to add the sign-off automatically.
