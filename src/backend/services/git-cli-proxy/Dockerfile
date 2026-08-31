# Multi-stage build for Insight Git CLI Proxy (clone-based git data extraction
# for the nocode git connectors; design: docs/components/connectors/git/git-cli-proxy/).
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/git-cli-proxy/Dockerfile -t insight-git-cli-proxy:dev .

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
COPY libs/insight-http-metrics/Cargo.toml ./libs/insight-http-metrics/Cargo.toml
COPY libs/authenticator-sdk/Cargo.toml ./libs/authenticator-sdk/Cargo.toml
COPY libs/insight-migration/Cargo.toml ./libs/insight-migration/Cargo.toml
COPY services/analytics/Cargo.toml ./services/analytics/Cargo.toml
COPY services/authenticator/Cargo.toml ./services/authenticator/Cargo.toml
COPY services/identity-resolution/Cargo.toml ./services/identity-resolution/Cargo.toml
COPY services/git-cli-proxy/Cargo.toml ./services/git-cli-proxy/Cargo.toml
COPY services/previews/Cargo.toml ./services/previews/Cargo.toml
COPY tools/routegen/Cargo.toml ./tools/routegen/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p libs/insight-http-metrics/src && echo "" > libs/insight-http-metrics/src/lib.rs && \
    mkdir -p libs/authenticator-sdk/src && echo "" > libs/authenticator-sdk/src/lib.rs && \
    mkdir -p libs/insight-migration/src && echo "" > libs/insight-migration/src/lib.rs && \
    mkdir -p services/analytics/src && echo "fn main() {}" > services/analytics/src/main.rs && \
    mkdir -p services/authenticator/src && echo "fn main() {}" > services/authenticator/src/main.rs && \
    mkdir -p services/identity-resolution/src && echo "fn main() {}" > services/identity-resolution/src/main.rs && \
    mkdir -p services/git-cli-proxy/src && echo "fn main() {}" > services/git-cli-proxy/src/main.rs && \
    mkdir -p services/previews/src && echo "fn main() {}" > services/previews/src/main.rs && \
    mkdir -p tools/routegen/src && echo "fn main() {}" > tools/routegen/src/main.rs

RUN cargo build --release --bin git-cli-proxy 2>/dev/null || true

# --- Source layer ---
RUN rm -rf libs/insight-http-metrics/src services/git-cli-proxy/src \
    target/release/deps/insight_http_metrics* target/release/deps/libinsight_http_metrics* \
    target/release/git-cli-proxy target/release/deps/git_cli_proxy*

COPY libs/insight-http-metrics/src/ ./libs/insight-http-metrics/src/
COPY services/git-cli-proxy/src/ ./services/git-cli-proxy/src/
COPY services/git-cli-proxy/config/ ./services/git-cli-proxy/config/

RUN cargo build --release --bin git-cli-proxy

# Stage 2: Runtime
#
# Trixie, where the sibling services use bookworm. `git` is not a utility here,
# it is the implementation: the blob purge needs `repack --filter` together
# with `--filter-to` (design §3.3), which arrived in git 2.42/2.43. Bookworm
# ships 2.39 and has no git in backports at all; trixie ships 2.47. A binary
# built against bookworm's glibc runs on trixie's newer one, so only this
# stage moves.
FROM debian:trixie-slim

ARG TARGETARCH
ARG WATCHEXEC_VERSION=2.3.2

# The service re-runs the purge invocation against a throwaway repository at
# boot and refuses to start if git rejects it, so a base image that regresses
# here fails the deploy rather than quietly filling a volume.
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in \
      amd64) WX_ARCH=x86_64-unknown-linux-musl ;; \
      arm64) WX_ARCH=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates git curl xz-utils; \
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

COPY --from=builder /build/target/release/git-cli-proxy /app/git-cli-proxy
# Gears config baked at /app/config/. Compose bind-mounts over it for hot
# edits; k8s mounts a ConfigMap at the same path. Env (APP__*) overrides
# the deployment-specific leaf values (budgets, data_dir, proxy_token).
COPY --from=builder /build/services/git-cli-proxy/config/ /app/config/

RUN useradd -U -u 1000 -m appuser && \
    chown -R 1000:1000 /app
USER 1000

EXPOSE 8084

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
CMD ["/app/git-cli-proxy", "--", "/app/git-cli-proxy", "-c", "/app/config/insight.yaml"]
