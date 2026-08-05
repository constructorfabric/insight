FROM rust:1.97-bookworm

ARG CARGO_WATCH_VERSION=8.5.3

RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/* && \
    cargo install cargo-watch --locked --version "${CARGO_WATCH_VERSION}"

WORKDIR /workspace

# /target and the cargo caches are named volumes. Docker seeds a fresh volume
# from the image path, ownership included, so these must belong to the runtime
# user before the volume is created.
RUN useradd -U -u 1000 -m appuser \
    && mkdir -p /target /usr/local/cargo/registry /usr/local/cargo/git \
    && chown -R 1000:1000 /target /usr/local/cargo/registry /usr/local/cargo/git
USER 1000
