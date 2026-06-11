# Rebuild marker (2026-06-11): republish so the chart-pinned tag advances to
# an SLSA-attested image — current appVersion predates the provenance
# attestation steps in build-images.yml.
# Multi-stage build for Insight Analytics API
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/analytics-api/Dockerfile -t insight-analytics-api:dev .

# Stage 1: Builder
FROM rust:1.95-bookworm AS builder

WORKDIR /build

# --- Dependency caching layer ---
COPY Cargo.toml Cargo.lock ./
COPY libs/insight-clickhouse/Cargo.toml ./libs/insight-clickhouse/Cargo.toml
COPY plugins/oidc-authn-plugin/Cargo.toml ./plugins/oidc-authn-plugin/Cargo.toml
COPY services/api-gateway/Cargo.toml ./services/api-gateway/Cargo.toml
COPY services/analytics-api/Cargo.toml ./services/analytics-api/Cargo.toml

RUN mkdir -p libs/insight-clickhouse/src && echo "" > libs/insight-clickhouse/src/lib.rs && \
    mkdir -p plugins/oidc-authn-plugin/src && echo "" > plugins/oidc-authn-plugin/src/lib.rs && \
    mkdir -p services/api-gateway/src && echo "fn main() {}" > services/api-gateway/src/main.rs && \
    mkdir -p services/analytics-api/src && echo "fn main() {}" > services/analytics-api/src/main.rs

RUN cargo build --release --bin analytics-api 2>/dev/null || true

# --- Source layer ---
RUN rm -rf libs/insight-clickhouse/src services/analytics-api/src \
    target/release/analytics-api target/release/deps/analytics_api* \
    target/release/deps/insight_clickhouse* target/release/deps/libinsight_clickhouse*

COPY libs/insight-clickhouse/src/ ./libs/insight-clickhouse/src/
COPY services/analytics-api/src/ ./services/analytics-api/src/

RUN cargo build --release --bin analytics-api

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/analytics-api /app/analytics-api

RUN useradd -U -u 1000 appuser && \
    chown -R 1000:1000 /app
USER 1000

EXPOSE 8081

CMD ["/app/analytics-api"]