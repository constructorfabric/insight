#!/bin/sh
# Compose-only patch for the published frontend ghcr image.
#
# The published image's nginx config deliberately has no /api proxy — in k8s
# the cluster ingress routes /api/* straight to the gateway pod, so the FE pod
# never sees those requests. The docker-compose stack has nothing in front of
# the FE container, so /api has to land locally.
#
# Rather than maintain a parallel config (drift risk every time the upstream
# changes), we insert just the /api location block at a known marker. The patch
# is idempotent, so container restarts do not stack copies.
#
# If the upstream config ever drops the marker line, this script is a no-op and
# the symptoms revert to the original "GET /api → 200 HTML, POST /api → 405"
# failure mode — observable, not silent.
#
# This runs as the container's COMMAND, not its entrypoint, and patches the
# SERVED config rather than the template it is copied from. Both details are
# load-bearing and both were learned the hard way:
#
#   * The image runs as `nginx` (uid 101) and ships /etc/nginx/templates
#     root-owned 0755. Patching the template in place worked only while the
#     image ran as root; afterwards it fails with "Permission denied", the
#     container exits, and the sole visible symptom is the gateway reporting
#     "insight-front could not be resolved" — a DNS error for a container that
#     never started. /etc/nginx/conf.d IS writable by that user.
#   * The image's own /docker-entrypoint.sh is NOT the stock nginx one. It does
#     a plain `cp /etc/nginx/templates/default.conf.template
#     /etc/nginx/conf.d/default.conf` and execs its argv — no envsubst, so
#     NGINX_ENVSUBST_TEMPLATE_DIR is not honoured and pointing it at a writable
#     copy of the template achieves nothing.
#
# Running after that entrypoint, on the file it produced, sidesteps both. The
# entrypoint keeps whatever behaviour it gains upstream; we only add a location
# block to the result.

set -e

CONF=/etc/nginx/conf.d/default.conf

if [ ! -f "$CONF" ]; then
  echo "WARN: $CONF missing — FE image structure changed; cannot patch." >&2
elif grep -q "location /api/" "$CONF"; then
  echo "front-ghcr-patch: /api proxy already present — skipping."
elif ! grep -q "snippets/security-headers\.conf" "$CONF"; then
  echo "WARN: marker 'snippets/security-headers.conf' not in $CONF — FE config changed;" >&2
  echo "      /api will be served by the SPA instead of proxied to the gateway." >&2
else
  # Insert immediately after the first `include …/security-headers.conf`
  # line (the one inside the server block, before any location blocks).
  # awk lets us touch only the first occurrence; sed -i with multi-line
  # inserts is painful and not portable across busybox/GNU.
  awk '
    /snippets\/security-headers\.conf/ && !done {
      print
      print ""
      print "    # Compose-only /api proxy injected by front-ghcr-patch-conf.sh."
      print "    # k8s relies on the cluster ingress to route /api → gateway;"
      print "    # compose has no front-proxy so we add the hop here."
      print "    #"
      print "    # 127.0.0.11 is Dockers embedded DNS. We use it via `resolver` +"
      print "    # `set $upstream_apigw` so nginx resolves at request time instead"
      print "    # of startup — without this, the FE container refuses to start"
      print "    # if gateway isnt yet reachable, breaking `up -d` ordering."
      print "    resolver 127.0.0.11 valid=10s;"
      print "    location /api/ {"
      print "        set $upstream_apigw \"gateway:8080\";"
      print "        proxy_pass http://$upstream_apigw;"
      print "        proxy_http_version 1.1;"
      print "        proxy_set_header Host $host;"
      print "        proxy_set_header X-Real-IP $remote_addr;"
      print "        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;"
      print "        proxy_set_header X-Forwarded-Proto $scheme;"
      print "    }"
      done=1
      next
    }
    { print }
  ' "$CONF" > "$CONF.new"
  mv "$CONF.new" "$CONF"
  echo "front-ghcr-patch: inserted /api → gateway:8080 into the served config."
fi

exec "$@"
