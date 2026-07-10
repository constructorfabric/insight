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

# lua-resty-http is NOT bundled in the base image (ADR-0001 "More Information").
# opm needs perl + curl to fetch it; they are build-only, so drop them after.
RUN apk add --no-cache --virtual .opm-deps perl curl \
    && opm get ledgetech/lua-resty-http \
    && apk del .opm-deps

# The static Lua module (access-phase exchange, uuid7, error shaping) + the
# routegen binary + the startup entrypoint.
COPY services/gateway/lua/ /etc/nginx/lua/
COPY --from=routegen /build/target/release/routegen /usr/local/bin/routegen
COPY services/gateway/docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 8080

STOPSIGNAL SIGQUIT
ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]
