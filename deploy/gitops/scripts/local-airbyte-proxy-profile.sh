#!/usr/bin/env bash
set -euo pipefail
: "${ENV:?ENV is required}"
: "${NS_APP:?NS_APP is required}"
[[ "$NS_APP" =~ ^[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?$ ]] || { echo 'invalid NS_APP' >&2; exit 1; }
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bash "$ROOT/scripts/local-harness-guard.sh" "$NS_APP"
export NS_APP
envsubst "\${NS_APP}" < "$ROOT/environments/local/airbyte-proxy-values.yaml.tpl"
