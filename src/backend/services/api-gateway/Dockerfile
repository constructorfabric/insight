# Rebuild marker (2026-06-11): republish so the chart-pinned tag advances to
# an SLSA-attested image — current appVersion predates the provenance
# attestation steps in build-images.yml.
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
COPY plugins/oidc-authn-plugin/Cargo.toml ./plugins/oidc-authn-plugin/Cargo.toml
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml
COPY services/analytics-api/Cargo.toml ./services/analytics-api/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p plugins/oidc-authn-plugin/src && echo "" > plugins/oidc-authn-plugin/src/lib.rs && \
    mkdir -p services/api-gateway/src && echo "fn main() {}" > services/api-gateway/src/main.rs && \
    mkdir -p services/analytics-api/src && echo "fn main() {}" > services/analytics-api/src/main.rs

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

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/insight-api-gateway /app/insight-api-gateway
COPY --from=builder /build/services/api-gateway/config/ /app/config/

RUN mkdir -p /app/data && \
    useradd -U -u 1000 appuser && \
    chown -R 1000:1000 /app
USER 1000

EXPOSE 8080

CMD ["/app/insight-api-gateway", "-c", "/app/config/insight.yaml", "run"]