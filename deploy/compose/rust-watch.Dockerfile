FROM rust:1.95-bookworm

ARG CARGO_WATCH_VERSION=8.5.3

RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/* && \
    cargo install cargo-watch --locked --version "${CARGO_WATCH_VERSION}"

WORKDIR /workspace
