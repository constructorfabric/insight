#!/usr/bin/env bash
set -euo pipefail
: "${ENV:?ENV is required}"
: "${KUBE_CTX:?KUBE_CTX is required}"
: "${NS_INFRA:?NS_INFRA is required}"
[[ "$ENV" == local ]] || { echo 'manifest seeding is local-only' >&2; exit 1; }
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
kubectl --context "$KUBE_CTX" -n "$NS_INFRA" exec -i statefulset/airbyte-db -- \
  sh -eu -c "exec psql -X -v ON_ERROR_STOP=1 -U \"\${POSTGRES_USER:?POSTGRES_USER is required}\" -d \"\${POSTGRES_DB:?POSTGRES_DB is required}\"" \
  < "$ROOT/environments/local/airbyte-manifest-versions.sql" >/dev/null
