# Installation

Choose a source build for development and local inspection, the published
container for standalone packaging, or the Helm chart for a production
Kubernetes deployment.

## Build from source

Prerequisites:

- Git
- The Rust toolchain specified by `rust-toolchain.toml`
- A network path from the collector host to each BMC Redfish endpoint

Install the configured toolchain and build the workspace from the repository
root:

```bash
rustup show
cargo build --workspace
```

The debug binary is written to `target/debug/bmc-collector`. Build an optimized
binary with:

```bash
cargo build --release --bin bmc-collector
```

The optimized binary is written to `target/release/bmc-collector`.

## Build a container

Docker with BuildKit is required:

```bash
make container-build
docker run --rm fleet-intelligence-oob-collector:dev --help
```

The runtime image uses Distroless `static-debian12:nonroot`, runs as numeric
user and group `65532`, and has no shell or package manager. Mount a
configuration file when running it locally:

```bash
docker run --rm \
  --mount type=bind,src="$PWD/config.toml",dst=/config.toml,readonly \
  fleet-intelligence-oob-collector:dev \
  inventory --config /config.toml --format json
```

Add `--insecure-tls` after the inventory options only when required by the BMC
certificate setup.

## Published artifacts

Semantic-version releases publish Linux `amd64` and `arm64` images to:

```text
ghcr.io/nvidia/fleet-intelligence-oob-collector:<version>
```

For example:

```bash
docker pull ghcr.io/nvidia/fleet-intelligence-oob-collector:0.1.0
```

The Kubernetes chart is published as an OCI artifact at:

```text
oci://ghcr.io/nvidia/charts/fleet-intelligence-oob-collector
```

Continue with [Configuration](configuration.md) for the shared TOML format,
[Usage](usage.md) for CLI workflows, or
[Kubernetes deployment](kubernetes.md) for Helm installation.
