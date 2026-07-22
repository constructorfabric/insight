#!/usr/bin/env bash
# Downstream-verification e2e (NGINX_BFF §6 R1 / §D).
#
# Brings up the full chain — fakeidp + authenticator + gateway + the REAL
# analytics (Rust) and identity (.NET) services + MariaDB/Redis — and asserts
# the five downstream-verification scenarios. Stack lifecycle + assertions live
# in conftest.py + test_downstream.py.
#
# Requires: docker, openssl, pytest, and (for the service-token scenario) PyJWT
# (`pip install pytest pyjwt cryptography`).
set -euo pipefail

cd "$(dirname "$0")"
exec python3 -m pytest "$@"
