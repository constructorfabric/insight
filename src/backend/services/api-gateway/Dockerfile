# Rebuild marker (2026-06-11, take 2): republish so the chart-pinned tag advances to
# an SLSA-attested image — current appVersion predates the provenance
# attestation steps in build-images.yml.
# Take 2: the first publish-chart attempt failed before committing the
# appVersion bumps back to main (chart attest step lacked registry login).
# Multi-stage build for Insight API Gateway
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/api-gateway/Dockerfile -t insight-api-gateway:dev .

# Stage 1: Builder
FROM rust:1.95-bookworm AS builder

# Install protobuf-compiler (required by grpc-hub → prost-build)
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# --- Dependency caching layer ---
# Copy Cargo manifests and create stubs so cargo can cache all dependencies.
COPY Cargo.toml Cargo.lock ./
COPY libs/insight-clickhouse/Cargo.toml ./libs/insight-clickhouse/Cargo.toml
COPY libs/authenticator-sdk/Cargo.toml ./libs/authenticator-sdk/Cargo.toml
COPY plugins/oidc-authn-plugin/Cargo.toml ./plugins/oidc-authn-plugin/Cargo.toml
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml
COPY services/analytics/Cargo.toml ./services/analytics/Cargo.toml
COPY services/authenticator/Cargo.toml ./services/authenticator/Cargo.toml
# fakeidp is a dev/e2e workspace member; cargo must load its manifest to
# resolve the workspace even though this image never builds or ships it.
COPY services/fakeidp/Cargo.toml ./services/fakeidp/Cargo.toml
# routegen is a workspace member (build-time CLI); cargo must load its manifest
# to resolve the workspace even though this image never builds it.
COPY tools/routegen/Cargo.toml ./tools/routegen/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p libs/authenticator-sdk/src && echo "" > libs/authenticator-sdk/src/lib.rs && \
    mkdir -p plugins/oidc-authn-plugin/src && echo "" > plugins/oidc-authn-plugin/src/lib.rs && \
    mkdir -p services/api-gateway/src && echo "fn main() {}" > services/api-gateway/src/main.rs && \
    mkdir -p services/analytics/src && echo "fn main() {}" > services/analytics/src/main.rs && \
    mkdir -p services/authenticator/src && echo "fn main() {}" > services/authenticator/src/main.rs && \
    mkdir -p services/fakeidp/src && echo "fn main() {}" > services/fakeidp/src/main.rs && \
    mkdir -p tools/routegen/src && echo "fn main() {}" > tools/routegen/src/main.rs

# Cache dependencies
RUN cargo build --release --bin insight-api-gateway 2>/dev/null || true

# --- Source layer ---
# Remove stubs and cached binaries, copy real source + config
RUN rm -rf libs/insight-clickhouse/src plugins/oidc-authn-plugin/src services/api-gateway/src \
    target/release/insight-api-gateway target/release/deps/insight_api_gateway* \
    target/release/deps/oidc_authn_plugin* target/release/deps/liboidc_authn_plugin* \
    target/release/deps/insight_clickhouse* target/release/deps/libinsight_clickhouse*

COPY libs/insight-clickhouse/src/ ./libs/insight-clickhouse/src/
COPY plugins/oidc-authn-plugin/src/ ./plugins/oidc-authn-plugin/src/
COPY services/api-gateway/src/ ./services/api-gateway/src/
COPY services/api-gateway/config/ ./services/api-gateway/config/

# Build release binary (deps cached, only our code compiles)
RUN cargo build --release --bin insight-api-gateway

# Stage 2: Runtime
FROM debian:bookworm-slim

ARG TARGETARCH
ARG WATCHEXEC_VERSION=2.3.2

# Runtime deps + watchexec. The watcher is used by the local
# docker-compose dev stack when ENABLE_AUTO_RELOAD=true to restart the
# binary on bind-mount change. Multi-arch via BuildKit's $TARGETARCH.
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

# Shared dev/prod entrypoint. ENABLE_AUTO_RELOAD switches behaviour at
# runtime; see docker-entrypoint.sh for the contract.
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

WORKDIR /app

COPY --from=builder /build/target/release/insight-api-gateway /app/insight-api-gateway
COPY --from=builder /build/services/api-gateway/config/ /app/config/

RUN mkdir -p /app/data && \
    useradd -U -u 1000 -m appuser && \
    chown -R 1000:1000 /app
USER 1000

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
# CMD format: <watched-path> -- <command> [args...]. Compose may override
# `command:` for dev (e.g. swap insight.yaml for no-auth.yaml).
CMD ["/app/insight-api-gateway", "--", "/app/insight-api-gateway", "-c", "/app/config/insight.yaml", "run"]
