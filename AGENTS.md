# Agent Guidance

This file applies to the entire repository. Keep changes focused and use the
linked documents as the source of truth instead of duplicating their details.

## Source of Truth

- Start with [README.md](README.md) for the project purpose and user-facing
  documentation map.
- Follow [CONTRIBUTING.md](CONTRIBUTING.md) for setup, branch names, commit and
  pull request conventions, DCO sign-off, and the issue-first workflow.
- Read [docs/architecture.md](docs/architecture.md) before changing runtime
  orchestration, collection flow, identity, batching, or backend behavior.
- Read [docs/configuration.md](docs/configuration.md) before changing TOML,
  environment variables, CLI defaults, Helm values, or credential handling.
- Read [crates/bmc-inventory/README.md](crates/bmc-inventory/README.md) before
  changing inventory identity, partial-failure behavior, or backend write
  semantics.
- Use [tests/README.md](tests/README.md) for test placement and
  [SECURITY.md](SECURITY.md) for vulnerability reporting.

## Project Structure

- `src/`: CLI, runtime orchestration, enrollment, and backend session lifecycle.
- `crates/backend-client/`: shared backend HTTP policy, typed errors, and redacted credential primitives.
- `crates/bmc-dispatch/`: shared per-BMC admission, rate limiting, and fairness.
- `crates/bmc-inventory/`: configuration, Redfish inventory, models, and sinks.
- `crates/bmc-attestation/`: nonce, evidence collection, and attestation upload.
- `crates/bmc-error/`: sanitized error and URI types used in exported models.
- `deployments/helm/`: Kubernetes chart, schema, templates, and CI values.
- `docs/` and `fern/`: source documentation and Fern publication configuration.
- `.github/workflows/`: required CI, documentation, and release automation.

## Permitted Work

Agents may make focused changes to Rust code, tests, documentation, Helm files,
and CI files when those files are directly required by the task. Add or update
tests with every behavior change, and update the relevant documentation when a
command, configuration field, data model, deployment behavior, or output
contract changes.

Ask before adding or upgrading dependencies; changing public JSON or backend
API contracts; changing node identity or `domainErrors` semantics; removing
legacy configuration support; changing CLI or Helm defaults; or altering
release permissions and publication behavior.

## Out of Scope

- Do not modify NICo or `NVIDIA/infra-controller` source from this repository.
- Do not push directly to `main`, merge pull requests, create tags or releases,
  bypass required checks, or weaken branch protection.
- Do not weaken TLS verification, workload security contexts, credential file
  handling, URL sanitization, or least-privilege workflow permissions merely to
  make a test pass.
- Do not perform live BMC, customer-backend, registry, or production-cluster
  operations unless the task explicitly authorizes them.

## Required Invariants

- Configure targets through `[[targets]]` and credentials through a separate
  shared JSON file. Resolve all credential values from its profiles and
  per-MAC overrides.
- Route inventory and attestation Redfish work through `bmc-dispatch`; do not
  create a separate path that bypasses shared per-BMC limits.
- Preserve successfully collected inventory when one Redfish domain fails.
  `domainErrors[].domain` is a backend control-flow key, and consumers must not
  parse its human-readable `message`.
- Identify inventory observations by the required configured BMC MAC; use the
  configured BMC MAC for attestation nonce requests and uploaded records.
- Use `SanitizedErrorMessage` and `SanitizedUriReference` for externally visible
  Redfish-derived errors and references.
- Keep dependency versions in the workspace tables in `Cargo.toml`; update
  `Cargo.lock` with Cargo rather than editing it manually.

## Coding and Test Conventions

- Use the pinned Rust toolchain in `rust-toolchain.toml`, Rust 2024 idioms,
  `rustfmt`, and warning-free `clippy`.
- Keep unit tests beside the module they cover. Put public-API, CLI, and fixture
  integration tests under `tests/` or the relevant crate's `tests/` directory.
- Use thread-safe synchronization for state shared with Tokio tasks or HTTP
  handlers. Tests must remain safe under concurrent execution.
- Log operational failures with `tracing` without logging tokens, credentials,
  raw BMC responses, or unsanitized URLs.
- Prefer the existing source, model, sink, dispatcher, and backend-session
  abstractions over parallel implementations.

Preferred exported error field:

```rust
pub target_error: Option<SanitizedErrorMessage>
```

Do not introduce a raw externally visible error field:

```rust
pub target_error: Option<String>
```

## Verification

Run the affected package first for fast feedback, for example:

```bash
cargo test -p bmc-inventory --locked
```

Before submitting a Rust change, run the full repository checks:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
```

For Helm, container, or packaging changes, also run:

```bash
make chart-lint
make chart-template
make container-build
```

For documentation changes, use the Fern version pinned in
`fern/fern.config.json` and run:

```bash
fern check
fern docs md check
lychee --offline --no-progress 'docs/**/*.md'
```

If a required tool or environment is unavailable, report which command was not
run and the remaining validation risk.

## Credentials and Sensitive Data

Never commit credentials, passwords, API keys, bootstrap or runtime tokens,
private backend URLs, customer data, raw hardware dumps, local `.config` files,
or environment files. Helm deployments must use credential file paths backed by
Kubernetes Secrets; do not place inline BMC credentials in a ConfigMap or chart
values. Use placeholders in tests and documentation.
