#!/bin/sh
set -e

# The SPA is a pure static bundle served behind the nginx `gateway`, which
# fronts `/api/*`, `/auth/*`, and `/` (this container). Authentication is a
# server-side cookie/BFF flow: the browser hits `/auth/login` (gateway ->
# authenticator -> IdP), gets a `__Host-sid` session cookie, and the SPA calls
# `/api/*` and `/auth/me` same-origin with `credentials: 'include'`.

# Place the nginx config. The build ships it under /etc/nginx/templates, so
# copy it into conf.d here.
cp /etc/nginx/templates/default.conf.template /etc/nginx/conf.d/default.conf

exec "$@"
