# Insight gateway -- OpenResty edge (gateway DESIGN 3.15; ADR-0001 Option A).
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/gateway/Dockerfile -t insight-gateway:dev .
#
# The image ships the Lua module + the routegen binary + an entrypoint. It bakes
# NO config: routes.yaml is mounted at runtime (ConfigMap in k8s, bind-mount in
# compose) and compiled to nginx.conf on startup, with deployment settings from
# the environment (DD-GW-02). Single source of truth, nothing committed.

# Stage 1: build the routegen CLI. rust:alpine so the musl binary runs on the
# openresty:alpine runtime without a glibc mismatch. The stage runs on the build
# platform; a different TARGETARCH links with rust-lld against the musl CRT the
# target's rust-std ships, as routegen has no C dependencies.
FROM --platform=$BUILDPLATFORM rust:1.98.1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS routegen
ARG TARGETARCH
ARG BUILDARCH
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY . .
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) triple=x86_64-unknown-linux-musl ;; \
      arm64) triple=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH=${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    if [ "${TARGETARCH}" != "${BUILDARCH}" ]; then \
      rustup target add "${triple}"; \
      export "CARGO_TARGET_$(echo "${triple}" | tr 'a-z-' 'A-Z_')_LINKER=rust-lld"; \
    fi; \
    cargo build --release -p routegen --target "${triple}"; \
    install -D "target/${triple}/release/routegen" /build/out/routegen

# Stage 2: OpenResty runtime.
FROM openresty/openresty:1.31.1.1-2-alpine@sha256:5f621903212ed7194a6bd31de7a76c1cfbe92e7b7a6fdb0f2b38626e97bd38c4

# Runtime deps not bundled in the base image:
#   - lua-resty-http (ADR-0001): the access-phase exchange cosocket.
#   - lua-resty-uuid + libuuid (>= 2.41 for uuid_generate_time_v7): UUIDv7
#     correlation ids. Alpine ships only libuuid.so.1, but the FFI loader wants
#     libuuid.so, so symlink it.
# opm needs perl + curl to fetch the modules; those are build-only, dropped after.
# The base digest is pinned, so the upgrade layer replays from cache until its
# parent changes. Bump the epoch to re-run it when a fix lands in Alpine without
# the base digest moving.
ARG SECURITY_EPOCH=2026-10-29
RUN echo "security epoch ${SECURITY_EPOCH}" \
    && apk upgrade --no-cache \
    && apk add --no-cache libuuid \
    && apk add --no-cache --virtual .opm-deps perl curl \
    && opm get ledgetech/lua-resty-http \
    && opm get bungle/lua-resty-uuid \
    && apk del .opm-deps \
    && ln -sf /usr/lib/libuuid.so.1 /usr/lib/libuuid.so

# The static Lua module (access-phase exchange, error shaping) + the routegen
# binary + the startup entrypoint.
COPY services/gateway/lua/ /etc/nginx/lua/
COPY --from=routegen /build/out/routegen /usr/local/bin/routegen
COPY services/gateway/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Default to non-root, matching the k8s pod securityContext (runAsUser 1000) so
# a bare `docker run` / compose without `user:` is not root either. The entrypoint
# only writes /tmp (config, pid, temp paths) and reads the read-only routes mount.
USER 1000

EXPOSE 8080

STOPSIGNAL SIGQUIT
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

# CI rebuild marker -- first publish of this image (fix/ci-publish-gateway-authenticator).
