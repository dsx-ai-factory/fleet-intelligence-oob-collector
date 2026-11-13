# syntax=docker/dockerfile:1.7
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
ARG NFPM_VERSION=2.48.0
FROM ghcr.io/goreleaser/nfpm:v${NFPM_VERSION} AS nfpm
FROM golang:1.26.6-bookworm AS go
FROM rust:1.96.0-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends build-essential binutils ca-certificates git clang cmake musl-tools pkg-config protobuf-compiler libprotobuf-dev rpm && rm -rf /var/lib/apt/lists/*
COPY --from=nfpm /usr/bin/nfpm /usr/local/bin/nfpm
COPY --from=go /usr/local/go /usr/local/go
ENV PATH=/usr/local/go/bin:$PATH
RUN rustup toolchain install 1.95.0 --profile minimal --no-self-update && \
    target="$(rustc -vV | sed -n 's/^host: //p' | sed 's/-gnu$/-musl/')" && \
    rustup target add --toolchain 1.95.0 "$target"
WORKDIR /src
ARG NICO_SOURCE_REF
# Cache the upstream-pinned toolchain before copying local source changes.
RUN --mount=type=cache,target=/src/dist/package-sources,sharing=locked \
    : "${NICO_SOURCE_REF:?Build with make debian-package to supply the release source pin}" && \
    source="/src/dist/package-sources/infra-controller-$NICO_SOURCE_REF" && \
    if [ ! -d "$source/.git" ]; then \
        git init "$source" && git -C "$source" remote add origin https://github.com/dsx-ai-factory/infra-controller.git && \
        git -C "$source" fetch --depth 1 origin "$NICO_SOURCE_REF" && git -C "$source" checkout --detach FETCH_HEAD; \
    fi && \
    test "$(git -C "$source" rev-parse HEAD)" = "$NICO_SOURCE_REF" && \
    target="$(rustc -vV | sed -n 's/^host: //p' | sed 's/-gnu$/-musl/')" && \
    cd "$source" && rustup target add "$target"
COPY . .
ARG PACKAGE_VERSION
ARG PACKAGE_FORMAT=deb
ARG SOURCE_DATE_EPOCH=0
ENV SOURCE_DATE_EPOCH=$SOURCE_DATE_EPOCH
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/src/dist/package-sources,sharing=locked \
    --mount=type=cache,target=/src/dist/package-build,sharing=locked \
    --mount=type=cache,target=/root/go/pkg/mod,sharing=locked \
    --mount=type=cache,target=/root/.cache/go-build \
    : "${NICO_SOURCE_REF:?Build with make debian-package to supply the release source pin}" && \
    version="${PACKAGE_VERSION:-$(cargo pkgid -p fleet-intelligence-oob | sed 's/.*[#@]//')}" && \
    case "$PACKAGE_FORMAT" in deb|rpm) ;; *) echo 'Unsupported package format' >&2; exit 1;; esac && \
    package_version=$(printf '%s' "$version" | sed 's/-/~/') && \
    make packages-test-linking "$PACKAGE_FORMAT" "packages-check-$PACKAGE_FORMAT" NICO_REF="$NICO_SOURCE_REF" VERSION="$package_version" OUTPUTDIR=/out && \
    set -- /out/*."$PACKAGE_FORMAT" && test "$#" = 1 && \
    mv "$1" "/out/fleetint-oob_${version}_$(dpkg --print-architecture).$PACKAGE_FORMAT" && \
    (cd /out && sha256sum ./*."$PACKAGE_FORMAT" > checksums.txt)
FROM scratch AS export
COPY --from=build /out/ /
