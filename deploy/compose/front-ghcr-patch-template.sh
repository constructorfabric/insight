#!/bin/sh
# Compose-only patch for the insight-front ghcr image.
#
# The published image's nginx template deliberately has no /api proxy —
# in k8s the cluster ingress routes /api/* straight to the gateway
# pod, so the FE pod never sees those requests. The docker-compose dev
# stack has nothing in front of the FE pod, so /api needs to land
# locally.
#
# Rather than maintain a parallel template (drift risk every time the
# upstream changes), we insert just the /api location block into the
# upstream template at a known marker, then chain to the original
# entrypoint which envsubsts and starts nginx. The patch is idempotent
# so container restarts don't stack copies.
#
# If the upstream template ever drops the marker line, this script is a
# no-op and the symptoms revert to the original "GET /api → 200 HTML,
# POST /api → 405" failure mode — observable, not silent.

set -e

TPL=/etc/nginx/templates/default.conf.template

if [ ! -f "$TPL" ]; then
  echo "WARN: $TPL missing — FE image structure changed; cannot patch." >&2
elif grep -q "location /api/" "$TPL"; then
  echo "front-ghcr-patch: /api proxy already present in template — skipping."
else
  # Insert immediately after the first `include …/security-headers.conf`
  # line (the one inside the server block, before any location blocks).
  # awk lets us touch only the first occurrence; sed -i with multi-line
  # inserts is painful and not portable across busybox/GNU.
  awk '
    /snippets\/security-headers\.conf/ && !done {
      print
      print ""
      print "    # Compose-only /api proxy injected by front-ghcr-patch-template.sh."
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
  ' "$TPL" > "$TPL.new"
  mv "$TPL.new" "$TPL"
  echo "front-ghcr-patch: inserted /api → gateway:8080 into template."
fi

exec /docker-entrypoint.sh "$@"
