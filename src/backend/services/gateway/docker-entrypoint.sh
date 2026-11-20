#!/bin/sh
set -eu

ROUTES="${GATEWAY_ROUTES_FILE:-/etc/gateway/routes.yaml}"
OUT="${GATEWAY_NGINX_CONF:-/tmp/nginx.conf}"

# With `-p /tmp` (writable prefix for a read-only rootfs) nginx opens its default
# error log at <prefix>/logs/error.log during early startup, before it reads our
# `error_log /dev/stderr`. Create it so that first open does not alert.
mkdir -p /tmp/logs

if [ ! -f "$ROUTES" ]; then
    echo "gateway: routes file not found at $ROUTES (mount it via ConfigMap/compose)" >&2
    exit 1
fi

set -- --routes "$ROUTES" -o "$OUT"
[ -n "${GATEWAY_AUTHENTICATOR_URL:-}" ] && set -- "$@" --authenticator-url "$GATEWAY_AUTHENTICATOR_URL"
[ -n "${GATEWAY_FRONT_URL:-}" ]         && set -- "$@" --front-url "$GATEWAY_FRONT_URL"
[ -n "${GATEWAY_MCP_PUBLIC_URL:-}" ]    && set -- "$@" --mcp-public-url "$GATEWAY_MCP_PUBLIC_URL"
[ -n "${GATEWAY_LISTEN:-}" ]            && set -- "$@" --listen "$GATEWAY_LISTEN"
[ -n "${GATEWAY_JWT_CACHE_SIZE:-}" ]    && set -- "$@" --jwt-cache-size "$GATEWAY_JWT_CACHE_SIZE"
[ -n "${GATEWAY_AUTHZ_PATH:-}" ]        && set -- "$@" --authz-path "$GATEWAY_AUTHZ_PATH"
[ -n "${GATEWAY_AUTHZ_CONNECT_TIMEOUT_MS:-}" ] && set -- "$@" --authz-connect-timeout-ms "$GATEWAY_AUTHZ_CONNECT_TIMEOUT_MS"
[ -n "${GATEWAY_AUTHZ_READ_TIMEOUT_MS:-}" ]    && set -- "$@" --authz-read-timeout-ms "$GATEWAY_AUTHZ_READ_TIMEOUT_MS"

if [ -n "${GATEWAY_SET_REAL_IP_FROM:-}" ]; then
    OLD_IFS=$IFS; IFS=','
    for cidr in $GATEWAY_SET_REAL_IP_FROM; do
        [ -n "$cidr" ] && set -- "$@" --set-real-ip-from "$cidr"
    done
    IFS=$OLD_IFS
fi

echo "gateway: generating $OUT from $ROUTES"
routegen "$@"

openresty -p /tmp -c "$OUT" -t

if [ -n "${GATEWAY_VALIDATE_ONLY:-}" ]; then
    echo "gateway: validate-only, config is valid"
    exit 0
fi

exec openresty -p /tmp -c "$OUT" -g 'daemon off;'
