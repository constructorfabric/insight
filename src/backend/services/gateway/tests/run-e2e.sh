#!/usr/bin/env bash
# End-to-end test for the nginx gateway (NGINX_BFF step-05 scenarios).
#
# Thin wrapper: the compose stack lifecycle and every assertion live in the
# pytest suite (conftest.py + test_gateway.py). This just runs pytest from the
# tests directory; pass extra pytest args through (e.g. `-k revocation -v`).
#
# Requires: docker, openssl, and pytest (`pip install pytest`).
set -euo pipefail

cd "$(dirname "$0")"
exec python3 -m pytest "$@"
