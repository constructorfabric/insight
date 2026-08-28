# Multi-stage build for Insight Previews (epic #1981).
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/previews/Dockerfile -t insight-previews:dev .

# Stage 1: Builder
FROM rust:1.95-bookworm AS builder

# Install protobuf-compiler (required by grpc-hub → prost-build)
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# --- Dependency caching layer ---
# Every workspace member's manifest must be present for cargo to resolve the
# workspace, even the ones this image never builds or ships.
COPY Cargo.toml Cargo.lock ./
COPY libs/insight-clickhouse/Cargo.toml ./libs/insight-clickhouse/Cargo.toml
COPY libs/authenticator-sdk/Cargo.toml ./libs/authenticator-sdk/Cargo.toml
COPY libs/insight-migration/Cargo.toml ./libs/insight-migration/Cargo.toml
COPY services/analytics/Cargo.toml ./services/analytics/Cargo.toml
COPY services/authenticator/Cargo.toml ./services/authenticator/Cargo.toml
COPY services/identity-resolution/Cargo.toml ./services/identity-resolution/Cargo.toml
COPY services/git-cli-proxy/Cargo.toml ./services/git-cli-proxy/Cargo.toml
COPY services/previews/Cargo.toml ./services/previews/Cargo.toml
COPY tools/routegen/Cargo.toml ./tools/routegen/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p libs/authenticator-sdk/src && echo "" > libs/authenticator-sdk/src/lib.rs && \
    mkdir -p libs/insight-migration/src && echo "" > libs/insight-migration/src/lib.rs && \
    mkdir -p services/analytics/src && echo "fn main() {}" > services/analytics/src/main.rs && \
    mkdir -p services/authenticator/src && echo "fn main() {}" > services/authenticator/src/main.rs && \
    mkdir -p services/identity-resolution/src && echo "fn main() {}" > services/identity-resolution/src/main.rs && \
    mkdir -p services/git-cli-proxy/src && echo "fn main() {}" > services/git-cli-proxy/src/main.rs && \
    mkdir -p services/previews/src && echo "fn main() {}" > services/previews/src/main.rs && \
    mkdir -p tools/routegen/src && echo "fn main() {}" > tools/routegen/src/main.rs

RUN cargo build --release --bin previews 2>/dev/null || true

# --- Source layer ---
RUN rm -rf services/previews/src \
    target/release/previews target/release/deps/previews*

COPY services/previews/src/ ./services/previews/src/
COPY services/previews/config/ ./services/previews/config/

RUN cargo build --release --bin previews

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

COPY --from=builder /build/target/release/previews /app/previews
# Gears config baked at /app/config/. Compose bind-mounts over it for hot
# edits; k8s mounts a ConfigMap at the same path. Env (APP__*) overrides
# the deployment-specific leaf values (route_host).
COPY --from=builder /build/services/previews/config/ /app/config/

RUN useradd -U -u 1000 -m appuser && \
    chown -R 1000:1000 /app
USER 1000

EXPOSE 8085

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/app/previews", "--", "/app/previews", "-c", "/app/config/insight.yaml"]
