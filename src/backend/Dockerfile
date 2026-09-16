# Every Rust service image. Build context: src/backend/.
#
#   cd src/backend && docker build --target analytics -t insight-analytics:dev .
#
# Targets: analytics, authenticator, identity-resolution, insight-v3-core,
# git-cli-proxy, previews.

# ─── toolchain ──────────────────────────────────────────────────────────────
FROM lukemathwalker/cargo-chef:0.1.78-rust-1.98.1-bookworm@sha256:14ac293f096e29d01296479bb2f81ec1fb0d6ff166dc2be399cf79cc79ec4906 AS chef

# protobuf-compiler: grpc-hub -> prost-build. cmake + libcurl headers: the
# authenticator vendors librdkafka, whose OAUTHBEARER path needs curl/curl.h.
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler libprotobuf-dev cmake libcurl4-openssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# ─── dependency recipe ──────────────────────────────────────────────────────
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# ─── workspace dependencies, cached by recipe hash ──────────────────────────
FROM chef AS cooker
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json

# ─── service binaries ───────────────────────────────────────────────────────
FROM cooker AS build-analytics
COPY . .
RUN cargo build --release --locked --bin analytics

FROM cooker AS build-authenticator
COPY . .
RUN cargo build --release --locked --bin authenticator

FROM cooker AS build-identity-resolution
COPY . .
RUN cargo build --release --locked --bin identity-resolution

FROM cooker AS build-insight-v3-core
COPY . .
RUN cargo build --release --locked --bin insight-v3-core

FROM cooker AS build-git-cli-proxy
COPY . .
RUN cargo build --release --locked --bin git-cli-proxy

FROM cooker AS build-previews
COPY . .
RUN cargo build --release --locked --bin previews

# ─── watchexec, the compose auto-reload wrapper ─────────────────────────────
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS watchexec

ARG TARGETARCH
ARG WATCHEXEC_VERSION=2.3.2

# SAFETY: the archive comes from a GitHub release over the network; the build
# fails unless its SHA-256 matches the pinned value for this version and arch.
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in \
      amd64) WX_ARCH=x86_64-unknown-linux-musl; WX_SHA256=2e72e56f722212558ce53d71c13d069f469c8113396da09d8ebf122ce6a655a5 ;; \
      arm64) WX_ARCH=aarch64-unknown-linux-musl; WX_SHA256=4e39391a2b75048cccfb7ab85e53241a8a8c9f88259dac0ee612b13d04e0e7e2 ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    apt-get update; \
    apt-get install -y --no-install-recommends ca-certificates curl xz-utils; \
    curl -fsSL -o /tmp/watchexec.tar.xz "https://github.com/watchexec/watchexec/releases/download/v${WATCHEXEC_VERSION}/watchexec-${WATCHEXEC_VERSION}-${WX_ARCH}.tar.xz"; \
    echo "${WX_SHA256}  /tmp/watchexec.tar.xz" | sha256sum -c -; \
    tar -xJ -C /tmp -f /tmp/watchexec.tar.xz; \
    install -m 0755 "/tmp/watchexec-${WATCHEXEC_VERSION}-${WX_ARCH}/watchexec" /usr/local/bin/watchexec

# ─── runtime bases ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=watchexec /usr/local/bin/watchexec /usr/local/bin/watchexec
COPY --chmod=0755 docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

WORKDIR /app
RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /app
USER 1000

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

# WORKAROUND: git-cli-proxy's blob purge needs `git repack --filter --filter-to`
# (git >= 2.43); bookworm ships 2.39 with no backport, trixie ships 2.47. A
# binary built against bookworm's glibc runs on trixie's newer one.
FROM debian:trixie-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132 AS runtime-git

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates git && \
    rm -rf /var/lib/apt/lists/*

COPY --from=watchexec /usr/local/bin/watchexec /usr/local/bin/watchexec
COPY --chmod=0755 docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

WORKDIR /app
RUN useradd -U -u 1000 -m appuser && chown -R 1000:1000 /app
USER 1000

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

# ─── images ─────────────────────────────────────────────────────────────────
# Gears config is baked at /app/config/; compose bind-mounts over it, k8s
# mounts a ConfigMap at the same path, APP__* env overrides leaf values.

FROM runtime AS analytics
COPY --from=build-analytics --chown=1000:1000 /build/target/release/analytics /app/analytics
COPY --from=build-analytics --chown=1000:1000 /build/services/analytics/config/ /app/config/
EXPOSE 8081
CMD ["/app/analytics", "--", "/app/analytics", "-c", "/app/config/insight.yaml", "run"]

FROM runtime AS authenticator
COPY --from=build-authenticator --chown=1000:1000 /build/target/release/authenticator /app/authenticator
COPY --from=build-authenticator --chown=1000:1000 /build/services/authenticator/config/ /app/config/
EXPOSE 8083
CMD ["/app/authenticator", "--", "/app/authenticator", "-c", "/app/config/insight.yaml", "run"]

FROM runtime AS identity-resolution
COPY --from=build-identity-resolution --chown=1000:1000 /build/target/release/identity-resolution /app/identity-resolution
COPY --from=build-identity-resolution --chown=1000:1000 /build/services/identity-resolution/config/ /app/config/
EXPOSE 8082
CMD ["/app/identity-resolution", "--", "/app/identity-resolution", "-c", "/app/config/insight.yaml"]

FROM runtime AS insight-v3-core
COPY --from=build-insight-v3-core --chown=1000:1000 /build/target/release/insight-v3-core /app/insight-v3-core
COPY --from=build-insight-v3-core --chown=1000:1000 /build/services/insight-v3-core/config/ /app/config/
EXPOSE 8086
CMD ["/app/insight-v3-core", "--", "/app/insight-v3-core", "-c", "/app/config/insight.yaml", "run"]

FROM runtime-git AS git-cli-proxy
COPY --from=build-git-cli-proxy --chown=1000:1000 /build/target/release/git-cli-proxy /app/git-cli-proxy
COPY --from=build-git-cli-proxy --chown=1000:1000 /build/services/git-cli-proxy/config/ /app/config/
EXPOSE 8084
CMD ["/app/git-cli-proxy", "--", "/app/git-cli-proxy", "-c", "/app/config/insight.yaml"]

FROM runtime AS previews
COPY --from=build-previews --chown=1000:1000 /build/target/release/previews /app/previews
COPY --from=build-previews --chown=1000:1000 /build/services/previews/config/ /app/config/
EXPOSE 8085
CMD ["/app/previews", "--", "/app/previews", "-c", "/app/config/insight.yaml"]
