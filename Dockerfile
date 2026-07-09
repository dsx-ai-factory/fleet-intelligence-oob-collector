# syntax=docker/dockerfile:1.7

FROM --platform=$TARGETPLATFORM rust:1.95.0-alpine AS builder

WORKDIR /src
RUN apk add --no-cache build-base file
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target \
    cargo build --locked --release --bin bmc-collector && \
    cp target/release/bmc-collector /bmc-collector && \
    strip /bmc-collector && \
    file /bmc-collector | grep -q "statically linked"

FROM gcr.io/distroless/static-debian12:nonroot

ARG VERSION=dev
ARG REVISION=unknown

LABEL org.opencontainers.image.title="Fleet Intelligence OOB Collector" \
      org.opencontainers.image.description="Out-of-band BMC inventory and attestation collector" \
      org.opencontainers.image.source="https://github.com/NVIDIA/fleet-intelligence-oob-collector" \
      org.opencontainers.image.version="$VERSION" \
      org.opencontainers.image.revision="$REVISION" \
      org.opencontainers.image.licenses="Apache-2.0"

COPY --from=builder /bmc-collector /bmc-collector

USER 65532:65532
ENTRYPOINT ["/bmc-collector"]
CMD ["--help"]
