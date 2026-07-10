# Insight gateway -- OpenResty edge (gateway DESIGN 3.15; ADR-0001 Option A).
#
# Build context: src/backend/
# Usage: cd src/backend && docker build -f services/gateway/Dockerfile -t insight-gateway:dev .
#
# The Lua module ships in the image (DESIGN 3.15); the nginx.conf is baked as a
# default but is overridden at runtime by the ConfigMap the configurator
# generated in CI (k8s) or by a compose bind-mount.
FROM openresty/openresty:1.27.1.2-alpine

# lua-resty-http is NOT bundled in the base image (ADR-0001 "More Information").
# opm needs perl + curl to fetch it; they are build-only, so drop them after.
RUN apk add --no-cache --virtual .opm-deps perl curl \
    && opm get ledgetech/lua-resty-http \
    && apk del .opm-deps

# The static Lua module (access-phase exchange, uuid7, error shaping). This is
# ALL the image ships (DESIGN 3.15): the nginx.conf is always mounted at runtime
# -- a ConfigMap in k8s, a bind-mount in compose -- never baked. (It could not be
# baked anyway: helm/ is dockerignored, and the generated config carries
# in-cluster upstream names that would not resolve at build time.)
COPY services/gateway/lua/ /etc/nginx/lua/

EXPOSE 8080

STOPSIGNAL SIGQUIT
CMD ["openresty", "-g", "daemon off;"]
