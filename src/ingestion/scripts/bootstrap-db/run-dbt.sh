#!/usr/bin/env bash
set -euo pipefail

: "${CLICKHOUSE_HOST:?CLICKHOUSE_HOST must be set}"
: "${CLICKHOUSE_PORT:?CLICKHOUSE_PORT must be set}"
: "${CLICKHOUSE_PROTOCOL:?CLICKHOUSE_PROTOCOL must be set (http or https)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DBT_DIR="$(cd "${SCRIPT_DIR}/../../dbt" && pwd)"

set -a
source "${SCRIPT_DIR}/pins.env"
set +a

VENV_DIR="${SCRIPT_DIR}/.venv"
DBT_BIN="${VENV_DIR}/bin/dbt"
if [[ ! -x "${DBT_BIN}" ]] || ! "${VENV_DIR}/bin/pip" show dbt-clickhouse 2>/dev/null | grep -q "Version: ${DBT_CLICKHOUSE_VERSION}"; then
  PYTHON_BIN="$(command -v python3.12 || command -v python3.11)"
  : "${PYTHON_BIN:?python3.12 or python3.11 is required to run dbt (same major as the toolbox image)}"
  rm -rf "${VENV_DIR}"
  "${PYTHON_BIN}" -m venv "${VENV_DIR}"
  "${VENV_DIR}/bin/pip" install --quiet "dbt-core==${DBT_CORE_VERSION}" "dbt-clickhouse==${DBT_CLICKHOUSE_VERSION}"
fi

PROFILES_DIR="$(mktemp -d)"
trap 'rm -rf "${PROFILES_DIR}"' EXIT

if [[ "${CLICKHOUSE_PROTOCOL}" == "https" ]]; then
  SECURE=true
else
  SECURE=false
fi

cat > "${PROFILES_DIR}/profiles.yml" <<EOF
ingestion:
  target: bootstrap
  outputs:
    bootstrap:
      type: clickhouse
      host: ${CLICKHOUSE_HOST}
      port: ${CLICKHOUSE_PORT}
      schema: silver
      user: ${CLICKHOUSE_USER}
      password: "{{ env_var('CLICKHOUSE_PASSWORD') }}"
      secure: ${SECURE}
      send_receive_timeout: 1500
      query_limit: 0
      connect_timeout: 30
      settings:
        # Correlated subqueries (LEFT ANTI JOIN in the identity seed models)
        # are gated behind this experimental flag on CH 25.7. A model-level
        # config() setting does NOT reach the SELECT plan in dbt-clickhouse, so
        # it must be set at profile level. Kept in parity with prod/test/e2e.
        allow_experimental_correlated_subqueries: 1
EOF

cd "${DBT_DIR}"
"${DBT_BIN}" run --profiles-dir "${PROFILES_DIR}" "$@"
