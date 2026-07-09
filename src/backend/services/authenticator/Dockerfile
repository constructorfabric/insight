# Multi-stage build for the Insight Authenticator.
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/authenticator/Dockerfile -t insight-authenticator:dev .

# Stage 1: Builder
FROM rust:1.95-bookworm AS builder

# protobuf-compiler is required by grpc-hub -> prost-build.
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# --- Dependency caching layer: copy every workspace member's manifest + a stub
#     source so the dependency graph resolves and deps get cached without the
#     real code. ---
COPY Cargo.toml Cargo.lock ./
COPY libs/insight-clickhouse/Cargo.toml ./libs/insight-clickhouse/Cargo.toml
COPY libs/authenticator-sdk/Cargo.toml ./libs/authenticator-sdk/Cargo.toml
COPY plugins/oidc-authn-plugin/Cargo.toml ./plugins/oidc-authn-plugin/Cargo.toml
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml
COPY services/analytics/Cargo.toml ./services/analytics/Cargo.toml
COPY services/authenticator/Cargo.toml ./services/authenticator/Cargo.toml
COPY services/fakeidp/Cargo.toml ./services/fakeidp/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p libs/authenticator-sdk/src && echo "" > libs/authenticator-sdk/src/lib.rs && \
    mkdir -p plugins/oidc-authn-plugin/src && echo "" > plugins/oidc-authn-plugin/src/lib.rs && \
    mkdir -p services/api-gateway/src && echo "fn main() {}" > services/api-gateway/src/main.rs && \
    mkdir -p services/analytics/src && echo "fn main() {}" > services/analytics/src/main.rs && \
    mkdir -p services/authenticator/src && echo "fn main() {}" > services/authenticator/src/main.rs && \
    mkdir -p services/fakeidp/src && echo "fn main() {}" > services/fakeidp/src/main.rs && \
    echo "" > services/fakeidp/src/lib.rs

RUN cargo build --release --bin authenticator 2>/dev/null || true

# --- Source layer ---
RUN rm -rf libs/authenticator-sdk/src services/authenticator/src \
    target/release/authenticator target/release/deps/authenticator* \
    target/release/deps/libauthenticator*

COPY libs/authenticator-sdk/src/ ./libs/authenticator-sdk/src/
COPY services/authenticator/src/ ./services/authenticator/src/
COPY services/authenticator/config/ ./services/authenticator/config/

RUN cargo build --release --bin authenticator

# Stage 2: Runtime
FROM debian:bookworm-slim

ARG TARGETARCH
ARG WATCHEXEC_VERSION=2.3.2

# Runtime deps + watchexec (auto-reload wrapper in compose). See
# api-gateway/Dockerfile for rationale.
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

COPY --from=builder /build/target/release/authenticator /app/authenticator
# Gears config baked at /app/config/. Compose bind-mounts over it for hot edits;
# k8s mounts a ConfigMap here. Signing keys are NOT baked — they come from a
# mounted Secret (signing_keys_path); compose bind-mounts the dev keys.
COPY --from=builder /build/services/authenticator/config/ /app/config/

RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /app
USER 1000

EXPOSE 8083

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/app/authenticator", "--", "/app/authenticator", "-c", "/app/config/insight.yaml", "run"]
