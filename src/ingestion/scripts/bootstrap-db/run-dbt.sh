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
# No `pip show | grep -q`: grep -q exits at first match, pip dies on EPIPE and
# pipefail fails the check for a perfectly good venv (which then gets rm -rf'd).
venv_pin_ok() {
  [[ "$("${VENV_DIR}/bin/pip" show "$1" 2>/dev/null | awk '/^Version:/{print $2}')" == "$2" ]]
}
if [[ ! -x "${DBT_BIN}" ]] || ! venv_pin_ok dbt-core "${DBT_CORE_VERSION}" || ! venv_pin_ok dbt-clickhouse "${DBT_CLICKHOUSE_VERSION}"; then
  PYTHON_BIN="$(command -v python3.12 || command -v python3.11)"
  : "${PYTHON_BIN:?python3.12 or python3.11 is required to run dbt (same major as the toolbox image)}"
  rm -rf "${VENV_DIR}"
  "${PYTHON_BIN}" -m venv "${VENV_DIR}"
  "${VENV_DIR}/bin/pip" install --quiet "dbt-core==${DBT_CORE_VERSION}" "dbt-clickhouse==${DBT_CLICKHOUSE_VERSION}"
fi

PROFILES_DIR="$(mktemp -d)"
trap 'rm -rf "${PROFILES_DIR}"' EXIT

"${VENV_DIR}/bin/python" "${SCRIPT_DIR}/../dbt_profiles.py" --target bootstrap \
  --correlated-subqueries --profiles-dir "${PROFILES_DIR}"

cd "${DBT_DIR}"
"${DBT_BIN}" run --profiles-dir "${PROFILES_DIR}" "$@"
