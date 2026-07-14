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
# openresty:alpine runtime without a glibc mismatch.
FROM rust:1.95-alpine AS routegen
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY . .
RUN cargo build --release -p routegen

# Stage 2: OpenResty runtime.
FROM openresty/openresty:1.27.1.2-alpine

# Runtime deps not bundled in the base image:
#   - lua-resty-http (ADR-0001): the access-phase exchange cosocket.
#   - lua-resty-uuid + libuuid (>= 2.41 for uuid_generate_time_v7): UUIDv7
#     correlation ids. Alpine ships only libuuid.so.1, but the FFI loader wants
#     libuuid.so, so symlink it.
# opm needs perl + curl to fetch the modules; those are build-only, dropped after.
RUN apk add --no-cache libuuid \
    && apk add --no-cache --virtual .opm-deps perl curl \
    && opm get ledgetech/lua-resty-http \
    && opm get bungle/lua-resty-uuid \
    && apk del .opm-deps \
    && ln -sf /usr/lib/libuuid.so.1 /usr/lib/libuuid.so

# The static Lua module (access-phase exchange, error shaping) + the routegen
# binary + the startup entrypoint.
COPY services/gateway/lua/ /etc/nginx/lua/
COPY --from=routegen /build/target/release/routegen /usr/local/bin/routegen
COPY services/gateway/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

# Default to non-root, matching the k8s pod securityContext (runAsUser 1000) so
# a bare `docker run` / compose without `user:` is not root either. The entrypoint
# only writes /tmp (config, pid, temp paths) and reads the read-only routes mount.
USER 1000

EXPOSE 8080

STOPSIGNAL SIGQUIT
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
