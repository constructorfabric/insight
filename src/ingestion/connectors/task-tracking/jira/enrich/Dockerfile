# syntax=docker/dockerfile:1.7

# Matches the Rust-backend Docker pattern in src/backend/services/* (bookworm
# builder + bookworm-slim runtime). Previously Alpine (musl) → Debian (glibc),
# which risked silent musl/glibc ABI mismatches at runtime.

# ---- Build stage ----
FROM rust:1.92-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies first. Build with --features io (the full binary needs ClickHouse I/O;
# rustc 1.89+ required — rust:1.92 image satisfies this).
COPY Cargo.toml Cargo.lock* ./
RUN mkdir -p src/core src/io \
    && echo "fn main() {}" > src/main.rs \
    && echo "pub mod types; pub mod jira;" > src/core/mod.rs \
    && echo "" > src/core/types.rs \
    && echo "" > src/core/jira.rs \
    && cargo build --release --features io 2>/dev/null || cargo build --release --features io

# Real sources.
COPY src ./src
RUN touch src/main.rs src/core/mod.rs src/core/types.rs src/core/jira.rs
RUN cargo build --release --features io

# ---- Runtime stage ----
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -U -u 1000 appuser

COPY --from=builder /build/target/release/jira-enrich /usr/local/bin/jira-enrich

USER 1000
ENTRYPOINT ["/usr/local/bin/jira-enrich"]
