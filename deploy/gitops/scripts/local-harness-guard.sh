#!/usr/bin/env bash
set -euo pipefail
: "${ENV:?ENV is required}"
: "${KUBE_CTX:?KUBE_CTX is required}"
: "${1:?namespace is required}"
[[ "$ENV" == local ]] || { echo 'harness is local-only; set ENV=local' >&2; exit 1; }
server=$(kubectl config view --minify --context "$KUBE_CTX" -o jsonpath='{.clusters[0].cluster.server}')
python3 - "$server" <<'PY'
import ipaddress
import sys
from urllib.parse import urlsplit


def local_server(value: str) -> bool:
    try:
        url = urlsplit(value)
        host = url.hostname
        if (
            url.scheme not in {"http", "https"}
            or not host
            or url.username is not None
            or url.password is not None
            or url.path not in {"", "/"}
            or url.query
            or url.fragment
            or url.port == 0
        ):
            return False
        if host in {"localhost", "host.docker.internal", "kubernetes.docker.internal"} or host.endswith(".local"):
            return True
        address = ipaddress.ip_address(host)
        return address in ipaddress.ip_network("127.0.0.0/8") or address == ipaddress.IPv6Address("::1")
    except ValueError:
        return False


if not local_server(sys.argv[1]):
    sys.exit("harness refused: context API server must use a loopback address or an allowed local hostname")
PY
