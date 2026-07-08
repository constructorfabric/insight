#!/usr/bin/env bash
# Apply the ClickHouse gold-view migrations against an EXTERNAL ClickHouse.
#
# This is the in-cluster, network-mode counterpart to the ClickHouse half
# of scripts/init.sh. init.sh `kubectl exec`s into a bundled CH StatefulSet
# (retired in #1428 when the umbrella stopped bundling L2 infra), so it
# cannot reach an external CH. This script talks to CH over its HTTP
# interface via lib/ch-exec.sh (selected by CLICKHOUSE_URL) and is invoked
# by the clickhouse-migrate Helm Hook Job (post-install,post-upgrade).
#
# Steps (same order and contract as init.sh):
#   1. Create the core databases (staging, silver, app db).
#   2. Run create-bronze-placeholders.sh — minimum-viable bronze/silver
#      stubs so gold-view CREATE VIEW type-checks on a fresh cluster
#      (CH validates referenced tables at parse time). See ADR-0007.
#   3. Apply migrations/*.sql in lexicographic order.
#   4. Build the dbt gold models (tag:gold) so dbt-owned views exist at
#      deploy time instead of after the first connector sync.
#
# Bookkeeping: none — every migration is re-run on every invocation and
# MUST stay idempotent/re-runnable (CREATE OR REPLACE / IF NOT EXISTS).
# This matches the existing init.sh contract (see ingestion DESIGN §migrations).
#
# Required env (set by the Hook Job from chart values + insight-db-creds):
#   CLICKHOUSE_URL       e.g. http://ch-host:8123  (selects the HTTP backend)
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#   CLICKHOUSE_DATABASE  the Insight app database
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set (the Insight app database)}"

source "$SCRIPT_DIR/lib/ch-exec.sh"

echo "=== Creating core databases (staging, silver, ${CLICKHOUSE_DATABASE}) ==="
run_ch <<SQL
CREATE DATABASE IF NOT EXISTS staging;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS ${CLICKHOUSE_DATABASE};
SQL

echo "=== Creating bronze/silver placeholders (ADR-0007) ==="
bash "$SCRIPT_DIR/create-bronze-placeholders.sh"

echo "=== Applying ClickHouse migrations ==="
shopt -s nullglob
for migration in "$SCRIPT_DIR/migrations"/*.sql; do
  echo "  $(basename "$migration")"
  run_ch < "$migration"
done

echo "=== Repairing class-contract labels on AI staging history ==="
# Staging rows ingested before the label columns existed read them as ''
# (String DEFAULT materialized by append_new_columns), and incremental
# models never re-read old rows. Labels are DECLARED CONSTANTS in the
# staging models, so writing the same constants here is byte-identical to
# what a full re-materialization would produce — any later full refresh
# independently converges to the same values and these updates become
# permanent no-ops. Data-bearing contract columns are NOT repairable this
# way; those go through the ADR-0015 major-bump full refresh instead.
# Guarded per table: staging tables do not exist before the connector's
# first dbt run. Idempotent: re-runs match zero rows.
repair_staging_tool_label() {
  local table="$1" label="$2"
  ch_table_exists staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS tool_label String DEFAULT '';
ALTER TABLE staging.${table} UPDATE tool_label = '${label}' WHERE tool_label = '' SETTINGS mutations_sync = 2;
SQL
}

repair_staging_surface_label() {
  local table="$1"
  ch_table_exists staging "${table}" || return 0
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS surface_label String DEFAULT '';
ALTER TABLE staging.${table} UPDATE surface_label = multiIf(
    surface = 'chat', 'Chat',
    surface = 'excel', 'Excel',
    surface = 'powerpoint', 'PowerPoint',
    surface = 'cowork', 'Cowork',
    surface = 'cross', 'Cross',
    surface
) WHERE surface_label = '' SETTINGS mutations_sync = 2;
SQL
}

repair_staging_tool_label cursor__ai_dev_usage "Cursor"
repair_staging_tool_label claude_enterprise__ai_dev_usage "Claude Code"
repair_staging_tool_label claude_team__ai_dev_usage "Claude Code"
repair_staging_tool_label claude_admin__ai_dev_usage "Claude Code"
repair_staging_tool_label copilot__ai_dev_usage "GitHub Copilot"
repair_staging_tool_label chatgpt_team__ai_dev_usage "Codex"
repair_staging_tool_label claude_enterprise__ai_assistant_usage "Claude"
repair_staging_tool_label chatgpt_team__ai_assistant_usage "ChatGPT"
repair_staging_surface_label claude_enterprise__ai_assistant_usage
repair_staging_surface_label chatgpt_team__ai_assistant_usage

echo "=== Building gold models (dbt run --select tag:gold) ==="
# Gold views are dbt-owned but must exist at DEPLOY time, not first-sync
# time: the analytics service marks metric definitions schema-error while
# an observation view is missing, which blanks those metrics for every
# frontend request until the first connector sync builds the view (hours
# on a scheduled instance). The placeholders created above guarantee every
# relation the views reference exists, so this run type-checks on a fresh
# cluster — the same guarantee the scoped per-connector dbt runs rely on
# for sideways refs. Idempotent: view materialization is create-or-replace.
#
# Profile generation mirrors the dbt-run WorkflowTemplate: python3 writes
# profiles.yml from env vars, never interpolating values into YAML text.
DBT_PROFILES_DIR="$(mktemp -d)"
export DBT_PROFILES_DIR
python3 - <<'PY'
import os
from urllib.parse import urlparse

import yaml

url = urlparse(os.environ["CLICKHOUSE_URL"])
profile = {
    "ingestion": {
        "target": "migrate",
        "outputs": {
            "migrate": {
                "type": "clickhouse",
                "host": url.hostname,
                "port": url.port or (8443 if url.scheme == "https" else 8123),
                "schema": "silver",
                "user": os.environ["CLICKHOUSE_USER"],
                "password": os.environ["CLICKHOUSE_PASSWORD"],
                "secure": url.scheme == "https",
                "send_receive_timeout": 1500,
                "query_limit": 0,
                "connect_timeout": 30,
            }
        },
    }
}
with open(os.path.join(os.environ["DBT_PROFILES_DIR"], "profiles.yml"), "w") as f:
    yaml.safe_dump(profile, f)
PY
(cd "$SCRIPT_DIR/../dbt" && dbt run --profiles-dir "$DBT_PROFILES_DIR" --log-format json --select tag:gold)
rm -rf "$DBT_PROFILES_DIR"

echo "=== ClickHouse migrations complete ==="
