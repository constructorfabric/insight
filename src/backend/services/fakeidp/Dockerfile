# Multi-stage build for the fake IdP (dev/e2e ONLY).
#
# This image must NEVER be published to a production registry or referenced by
# a production chart — fakeidp signs tokens with a throwaway keypair generated
# at startup and exposes `/_control/*` test hooks. It exists so `docker compose up`
# and CI can drive the authenticator's real OIDC code path. See
# services/fakeidp/README.md and cf/NGINX_BFF.md §10 G6.
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/fakeidp/Dockerfile -t insight-fakeidp:dev .

# Stage 1: Builder
FROM rust:1.95-bookworm AS builder

WORKDIR /build

# --- Dependency caching layer ---
# fakeidp lives in the src/backend workspace, so cargo must be able to load
# every member's manifest. We copy just the manifests + stub sources so the
# dependency graph resolves and fakeidp's deps get cached without dragging in
# the other services' real code.
COPY Cargo.toml Cargo.lock ./
COPY libs/insight-clickhouse/Cargo.toml ./libs/insight-clickhouse/Cargo.toml
COPY libs/authenticator-sdk/Cargo.toml ./libs/authenticator-sdk/Cargo.toml
COPY plugins/oidc-authn-plugin/Cargo.toml ./plugins/oidc-authn-plugin/Cargo.toml
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml
COPY services/analytics/Cargo.toml ./services/analytics/Cargo.toml
COPY services/authenticator/Cargo.toml ./services/authenticator/Cargo.toml
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
    echo "" > services/fakeidp/src/lib.rs && \
    mkdir -p tools/routegen/src && echo "fn main() {}" > tools/routegen/src/main.rs

RUN cargo build --release --bin fakeidp 2>/dev/null || true

# --- Source layer ---
RUN rm -rf services/fakeidp/src \
    target/release/fakeidp target/release/deps/fakeidp* target/release/deps/libfakeidp*

# The default users.yaml is baked into the binary via include_str!, so the
# source tree must carry it at compile time. The signing keypair is generated
# at startup — nothing to copy.
COPY services/fakeidp/src/ ./services/fakeidp/src/
COPY services/fakeidp/users.yaml ./services/fakeidp/users.yaml

RUN cargo build --release --bin fakeidp

# Stage 2: Runtime
FROM debian:bookworm-slim

# ca-certificates so the back-channel control hook can POST over TLS if the RP
# endpoint is https.
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/fakeidp /app/fakeidp
# Also drop users.yaml alongside the binary for reference / optional override
# via FAKEIDP_USERS (the binary bakes a copy, so this is not required to run).
COPY --from=builder /build/services/fakeidp/users.yaml /app/users.yaml

RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /app
USER 1000

EXPOSE 8084

ENTRYPOINT ["/app/fakeidp"]
