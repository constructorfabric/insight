#!/usr/bin/env bash
# Pre-create every bronze/silver/insight relation on a fresh cluster by
# applying the committed DDL snapshot in scripts/connectors-ddl/*.sql.
#
# The snapshot is generated in CI (.github/workflows/connectors-ddl.yml) by
# running the full bootstrap-db pipeline — real connector `discover`, the real
# destination-clickhouse connector, real dbt models — against a throwaway
# ClickHouse and dumping SHOW CREATE for everything it produced. The schemas
# therefore always match what a real sync/dbt run would create; the
# hand-written placeholder heredocs this script used to carry (ADR-0007) are
# gone, and with them the drop-before-first-sync caveat: Airbyte and dbt both
# accept these tables as their own.
#
# Apply order: per-connector bronze files first, then silver.sql, then
# insight.sql (gold views read silver/bronze). Statements are separated by
# blank lines in the dump format; views may reference other views, so failed
# statements are retried in additional passes until a pass makes no progress
# (dependency order resolves itself), and only a stuck pass is fatal.
#
# Everything is CREATE ... IF NOT EXISTS / CREATE OR REPLACE VIEW — existing
# relations are never dropped or altered, so re-runs and upgrades are no-ops
# for anything a connector or dbt already owns.
#
# Required env (same contract as apply-ch-migrations.sh, via lib/ch-exec.sh):
#   CLICKHOUSE_URL, CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#
# Generation mode: when this script runs as part of bootstrap-db's snapshot
# REGENERATION (dump-ddl workflow), the connectors-ddl/*.sql on disk are the
# PREVIOUS (stale) snapshot — applying them would leak dropped relations into
# the fresh dump and break convergence. bootstrap-db.sh therefore sets
# BOOTSTRAP_SKIP_SNAPSHOT=1 so this applicator is a no-op during generation;
# the real bronze/silver/gold are built from connectors + dbt + migrations
# instead. It stays fully active on real deploys (fresh-cluster apply).
set -euo pipefail

if [[ "${BOOTSTRAP_SKIP_SNAPSHOT:-}" == "1" ]]; then
  echo "=== Skipping connectors-ddl snapshot apply (BOOTSTRAP_SKIP_SNAPSHOT=1, generation mode) ==="
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DDL_DIR="${SCRIPT_DIR}/connectors-ddl"

source "${SCRIPT_DIR}/lib/ch-exec.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "${WORKDIR}"' EXIT

split_statements() {
  local file="$1" prefix="$2"
  awk -v out="${WORKDIR}/${prefix}" 'BEGIN { RS = "" } { print $0 > sprintf("%s-%04d.sql", out, NR) }' "${file}"
}

echo "=== Applying connectors-ddl snapshot ==="
index=0
for file in "${DDL_DIR}"/*.sql; do
  base="$(basename "${file}" .sql)"
  [[ "${base}" == "silver" || "${base}" == "insight" ]] && continue
  split_statements "${file}" "$(printf '%03d' "${index}")-${base}"
  index=$((index + 1))
done
split_statements "${DDL_DIR}/silver.sql"  "900-silver"
split_statements "${DDL_DIR}/insight.sql" "950-insight"

pass=0
while true; do
  pass=$((pass + 1))
  progress=0
  remaining=0
  for stmt in "${WORKDIR}"/*.sql; do
    [[ -e "${stmt}" ]] || break
    if run_ch < "${stmt}" > /dev/null 2> "${stmt}.err"; then
      rm -f "${stmt}" "${stmt}.err"
      progress=$((progress + 1))
    else
      remaining=$((remaining + 1))
    fi
  done
  echo "  pass ${pass}: applied ${progress}, remaining ${remaining}"
  [[ "${remaining}" -eq 0 ]] && break
  if [[ "${progress}" -eq 0 ]]; then
    echo "ERROR: ${remaining} statement(s) failed with no progress:" >&2
    for err in "${WORKDIR}"/*.err; do
      echo "--- $(basename "${err%.err}")" >&2
      head -n 3 "${err}" >&2
    done
    exit 1
  fi
done

echo "=== connectors-ddl snapshot applied ==="
