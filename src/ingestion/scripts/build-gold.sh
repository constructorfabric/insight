#!/usr/bin/env bash
# Build the dbt-owned gold models against an EXTERNAL ClickHouse.
#
# NOT part of a deploy. A connector sync rebuilds its own gold models: every
# descriptor's `dbt_select` ends in `+`, so the Argo dbt-run step walks the
# graph from that connector's silver down through gold, in DAG order, with no
# separate trigger (#3561). The deploy hook (apply-ch-migrations.sh) therefore
# builds nothing — which is what lets silver stop being widened by hand ahead
# of it.
#
# What is left for this script is the paths that write silver directly and have
# no sync to ride on: the sample seed (tools/seed, both its silver step and its
# `gold` subcommand) and the warm-upgrade CI lane, which plays a deploy plus the
# first sync after it to catch a gold model reading a column no migration adds.
#
# Required env (what dbt_profiles.py reads; no SQL is issued from here, so
# lib/ch-exec.sh is not sourced):
#   CLICKHOUSE_URL       e.g. http://ch-host:8123
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#
# Optional env (topology of the target ClickHouse, epic #2010):
#   CLICKHOUSE_CLUSTER_MODE  true -> the DDL emitted here replicates
#   CLICKHOUSE_CLUSTER_NAME  the cluster the ON CLUSTER clause names
# Both default to a standalone single node.
#
# Options:
#   --select <selectors>  space-separated dbt selectors to build instead of
#                         the default `tag:gold`. `tag:sync:always` is appended
#                         to whatever is given, never replaced by it.
#   --full-refresh        rebuild the selected models from source instead of
#                         appending to them. The seed's silver step passes it,
#                         because a seed REPLACES the org and the incremental
#                         identity feeders would otherwise carry the previous
#                         roster forward.
#
# Needs `dbt` on PATH (the toolbox image and the seed image both carry it).
set -euo pipefail

DBT_SELECT_ARG="tag:gold"
FULL_REFRESH=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --select)
      if [[ $# -lt 2 || -z "$2" ]]; then
        echo "build-gold.sh: --select needs a value" >&2
        exit 2
      fi
      DBT_SELECT_ARG="$2"; shift 2 ;;
    --full-refresh) FULL_REFRESH=1; shift ;;
    -h|--help)
      awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "${BASH_SOURCE[0]}"
      exit 0 ;;
    *)
      echo "build-gold.sh: unknown argument: $1" >&2
      echo "usage: build-gold.sh [--select <selectors>] [--full-refresh]" >&2
      exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_USER:?CLICKHOUSE_USER must be set}"
: "${CLICKHOUSE_PASSWORD:?CLICKHOUSE_PASSWORD must be set}"
# Read by the dbt profile below and, through the environment, by the cluster
# vars in dbt_project.yml. Exported so both see the same values.
export CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE:-false}"
export CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME:-}"

read -r -a _dbt_select <<<"$DBT_SELECT_ARG"
# The models no graph walk reaches — the two identity map views and
# `bronze_insert_events`. SAFETY: appended after the override so no caller can
# narrow the selector and leave them at the snapshot's point-in-time bodies.
_dbt_select+=("tag:sync:always")

# INVARIANT: never export DBT_FULL_REFRESH — reconcile-connectors owns that
# name, and env reaches every child.
_dbt_flags=()
if [[ "$FULL_REFRESH" == "1" ]]; then
  _dbt_flags+=(--full-refresh)
fi

echo "=== Building gold models (dbt run --select ${_dbt_select[*]} ${_dbt_flags[*]:-}) ==="
# Idempotent: views are create-or-replace and table-materialized gold models
# rebuild via atomic swap. Table builds are bounded by the models' own
# query_settings (memory, threads, disk spill), so this degrades to a slower
# build rather than a failure on data volume.
DBT_PROFILES_DIR="$(mktemp -d)"
export DBT_PROFILES_DIR
trap 'rm -rf "$DBT_PROFILES_DIR"' EXIT
python3 "$SCRIPT_DIR/dbt_profiles.py" --target migrate --correlated-subqueries \
  --profiles-dir "$DBT_PROFILES_DIR"
(cd "$SCRIPT_DIR/../dbt" && dbt run --profiles-dir "$DBT_PROFILES_DIR" --log-format json --select "${_dbt_select[@]}" ${_dbt_flags[@]+"${_dbt_flags[@]}"})

echo "=== Gold models built ==="
