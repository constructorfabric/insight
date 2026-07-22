# Rebuild marker (2026-06-11, take 2): republish so the chart-pinned tag advances to
# an SLSA-attested image — current appVersion predates the provenance
# attestation steps in build-images.yml.
# Take 2: the first publish-chart attempt failed before committing the
# appVersion bumps back to main (chart attest step lacked registry login).
# Multi-stage build for Insight Analytics API
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/analytics/Dockerfile -t insight-analytics:dev .

# Stage 1: Builder
FROM rust:1.95-bookworm AS builder

# Install protobuf-compiler (required by grpc-hub → prost-build)
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# --- Dependency caching layer ---
COPY Cargo.toml Cargo.lock ./
COPY libs/insight-clickhouse/Cargo.toml ./libs/insight-clickhouse/Cargo.toml
COPY libs/authenticator-sdk/Cargo.toml ./libs/authenticator-sdk/Cargo.toml
COPY services/analytics/Cargo.toml ./services/analytics/Cargo.toml
COPY services/authenticator/Cargo.toml ./services/authenticator/Cargo.toml
# identity-resolution is a workspace member; cargo must load its manifest to
# resolve the workspace even though this image never builds or ships it.
COPY services/identity-resolution/Cargo.toml ./services/identity-resolution/Cargo.toml
# fakeidp is a dev/e2e workspace member; cargo must load its manifest to
# resolve the workspace even though this image never builds or ships it.
COPY services/fakeidp/Cargo.toml ./services/fakeidp/Cargo.toml
# routegen is a workspace member (build-time CLI); cargo must load its manifest
# to resolve the workspace even though this image never builds it.
COPY tools/routegen/Cargo.toml ./tools/routegen/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p libs/authenticator-sdk/src && echo "" > libs/authenticator-sdk/src/lib.rs && \
    mkdir -p services/analytics/src && echo "fn main() {}" > services/analytics/src/main.rs && \
    mkdir -p services/authenticator/src && echo "fn main() {}" > services/authenticator/src/main.rs && \
    mkdir -p services/identity-resolution/src && echo "fn main() {}" > services/identity-resolution/src/main.rs && \
    mkdir -p services/fakeidp/src && echo "fn main() {}" > services/fakeidp/src/main.rs && \
    mkdir -p tools/routegen/src && echo "fn main() {}" > tools/routegen/src/main.rs

RUN cargo build --release --bin analytics 2>/dev/null || true

# --- Source layer ---
RUN rm -rf libs/insight-clickhouse/src services/analytics/src \
    target/release/analytics target/release/deps/analytics* \
    target/release/deps/insight_clickhouse* target/release/deps/libinsight_clickhouse*

COPY libs/insight-clickhouse/src/ ./libs/insight-clickhouse/src/
COPY services/analytics/src/ ./services/analytics/src/
COPY services/analytics/config/ ./services/analytics/config/

RUN cargo build --release --bin analytics

# Stage 2: Runtime
FROM debian:bookworm-slim

ARG TARGETARCH
ARG WATCHEXEC_VERSION=2.3.2

# Runtime deps + watchexec.
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in \
      amd64) WX_ARCH=x86_64-unknown-linux-musl ;; \
      arm64) WX_ARCH=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl xz-utils; \
    curl -fsSL "https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-${WX_ARCH}.tar.xz" \
        | tar -xJ -C /tmp; \
    install -m 0755 "/tmp/watchexec-${WATCHEXEC_VERSION}-${WX_ARCH}/watchexec" /usr/local/bin/watchexec; \
    rm -rf /tmp/watchexec-*; \
    apt-get purge -y curl xz-utils; \
    apt-get autoremove -y; \
    rm -rf /var/lib/apt/lists/*

COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

WORKDIR /app

COPY --from=builder /build/target/release/analytics /app/analytics
# Gears config baked at /app/config/. Compose bind-mounts over it for hot
# edits; k8s mounts a ConfigMap at the same path. Env (APP__*) overrides
# the deployment-specific leaf values (DB/tenant).
COPY --from=builder /build/services/analytics/config/ /app/config/

RUN useradd -U -u 1000 -m appuser && \
    chown -R 1000:1000 /app
USER 1000

EXPOSE 8081

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/app/analytics", "--", "/app/analytics", "-c", "/app/config/insight.yaml", "run"]
