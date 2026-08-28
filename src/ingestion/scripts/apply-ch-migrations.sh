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
#
# Options:
#   --full-refresh   rebuild the selected dbt models from source instead of
#                    appending to them. The deploy Hook never passes it; the
#                    seed's silver step does, because a seed REPLACES the org
#                    and the incremental identity feeders would otherwise
#                    carry the previous roster forward (see below).
set -euo pipefail

FULL_REFRESH=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --full-refresh) FULL_REFRESH=1; shift ;;
    -h|--help)
      awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "${BASH_SOURCE[0]}"
      exit 0 ;;
    *)
      echo "apply-ch-migrations.sh: unknown argument: $1" >&2
      echo "usage: apply-ch-migrations.sh [--full-refresh]" >&2
      exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set (the Insight app database)}"

source "$SCRIPT_DIR/lib/ch-exec.sh"

echo "=== Creating core databases (staging, silver, ${CLICKHOUSE_DATABASE}, presentation) ==="
# `presentation` (#1964): writable namespace for new gold / results / scratch.
run_ch <<SQL
CREATE DATABASE IF NOT EXISTS staging;
CREATE DATABASE IF NOT EXISTS silver;
CREATE DATABASE IF NOT EXISTS ${CLICKHOUSE_DATABASE};
CREATE DATABASE IF NOT EXISTS presentation;
SQL

echo "=== Provisioning presentation access (role + grant-less user) (#1963/#1964) ==="
bash "$SCRIPT_DIR/bootstrap-db/provision-presentation-access.sh"

echo "=== Provisioning grafana access (SELECT-only role + grant-less user) (#2888) ==="
bash "$SCRIPT_DIR/bootstrap-db/provision-grafana-access.sh"

echo "=== Creating bronze/silver placeholders (ADR-0007) ==="
bash "$SCRIPT_DIR/create-bronze-placeholders.sh"

echo "=== Applying ClickHouse migrations ==="
shopt -s nullglob
for migration in "$SCRIPT_DIR/migrations"/*.sql; do
  echo "  $(basename "$migration")"
  run_ch < "$migration"
done

echo "=== Healing AI staging contract schemas ==="
# Physical column order must equal the model's SELECT order (positional
# incremental inserts, positional union). Labels left the contract (they
# derive in gold — macros/ai_labels.sql): DROP converges every table
# state. conversation_count and seat_status are data: ADD/MODIFY pin their
# position. All four contributors in one deploy — a class unions them
# positionally, so healing per-sync mismatches the column counts.
# Guarded (staging tables exist only after the connector's first run);
# idempotent (re-runs are no-ops).
heal_ai_dev_staging() {
  local table="$1"
  ch_table_exists staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} DROP COLUMN IF EXISTS tool_label;
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS conversation_count Nullable(UInt32) AFTER session_count;
ALTER TABLE staging.${table} MODIFY COLUMN conversation_count Nullable(UInt32) AFTER session_count;
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS seat_status Nullable(String) AFTER _version;
ALTER TABLE staging.${table} MODIFY COLUMN seat_status Nullable(String) AFTER _version;
SQL
}

heal_ai_assistant_staging() {
  local table="$1"
  ch_table_exists staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} DROP COLUMN IF EXISTS tool_label;
ALTER TABLE staging.${table} DROP COLUMN IF EXISTS surface_label;
SQL
}

heal_ai_invoice_staging() {
  local table="$1"
  ch_table_exists staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS tier_ref Nullable(String) AFTER tier_label;
ALTER TABLE staging.${table} MODIFY COLUMN tier_ref Nullable(String) AFTER tier_label;
SQL
}

heal_ai_dev_staging cursor__ai_dev_usage
heal_ai_dev_staging claude_enterprise__ai_dev_usage
heal_ai_dev_staging claude_team__ai_dev_usage
heal_ai_dev_staging chatgpt_team__ai_dev_usage
heal_ai_assistant_staging claude_enterprise__ai_assistant_usage
heal_ai_assistant_staging chatgpt_team__ai_assistant_usage
heal_ai_invoice_staging claude_team__ai_invoice

echo "=== Healing CRM staging contract schemas ==="
# The CRM overflow blob left the contract — the connectors carry the
# unabridged record in raw_data — so the column must leave the physical
# tables too, or the positional incremental insert misaligns. The silver
# side drops in migrations/*.sql; staging drops here because these tables
# exist only after the connector's first run. Idempotent.
heal_crm_staging() {
  local table="$1"
  ch_table_exists staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} DROP COLUMN IF EXISTS custom_fields;
SQL
}

for _crm_grain in accounts activities contacts deals users; do
  heal_crm_staging "hubspot__crm_${_crm_grain}"
done

echo "=== Healing collab-chat contract schema ==="
# Same positional invariant: collab chat's direct_and_group_messages
# (#266) was added mid-SELECT without a rebuild. Healed here rather than
# in migrations/*.sql because the AFTER anchors do not exist on the
# minimal gold-view placeholders — heals run only on real tables
# (placeholders are replaced with the real schema at first build).
ch_table_is_real() {
  local db="$1" table="$2"
  ch_table_exists "$db" "$table" || return 1
  local placeholder_count
  placeholder_count="$(
    printf "SELECT count() FROM system.tables WHERE database='%s' AND name='%s' AND comment='INSIGHT_PLACEHOLDER_v1'" "$db" "$table" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  [[ "$placeholder_count" == "0" ]]
}

heal_collab_chat_table() {
  local db="$1" table="$2"
  ch_table_is_real "$db" "$table" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS direct_and_group_messages Nullable(Int64) AFTER group_chat_messages;
ALTER TABLE ${db}.${table} MODIFY COLUMN direct_and_group_messages Nullable(Int64) AFTER group_chat_messages;
SQL
}

heal_collab_chat_table staging m365__collab_chat_activity
heal_collab_chat_table staging slack__collab_chat_activity
heal_collab_chat_table staging zulip_proxy__collab_chat_activity
heal_collab_chat_table silver class_collab_chat_activity

# Same positional invariant: the task-users staging views gained tenant_id
# mid-SELECT (after unique_key) for the task observation attribution, and
# class_task_users inherits its column order from that union. Pre-existing
# tables lack the column; existing rows heal to NULL tenant and converge on
# the next sync (bronze jira_user is full_refresh + overwrite, so every row
# re-emits with a fresh _version). Staging needs no heal — both members are
# views, recreated on every run.
heal_task_users_table() {
  local db="$1" table="$2"
  ch_table_is_real "$db" "$table" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS tenant_id Nullable(String) AFTER unique_key;
ALTER TABLE ${db}.${table} MODIFY COLUMN tenant_id Nullable(String) AFTER unique_key;
SQL
}

heal_task_users_table silver class_task_users

# Same positional invariant, one relation further along: class_task_field_history
# gained `title` after `id_readable` (#2739) so evidence rows can name the work
# item. The connectors-ddl snapshot only CREATEs IF NOT EXISTS, so a warm
# installation keeps the old column list, and the gold build fails resolving
# `fh.title`. Staging needs no heal — github's member is a table rebuilt every
# run, and jira's is altered by the DDL macro that owns it.
heal_task_field_history_table() {
  local db="$1" table="$2"
  ch_table_is_real "$db" "$table" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS title Nullable(String) AFTER id_readable;
ALTER TABLE ${db}.${table} MODIFY COLUMN title Nullable(String) AFTER id_readable;
SQL
}

heal_task_field_history_table silver class_task_field_history

# The column above only makes room for the summary; jira-enrich fills it. That
# binary skips any issue that already has rows, so on a warm installation the
# summary never reaches a Jira issue whose changelog has gone quiet — and a
# closed issue, the kind task metrics count, never moves again. Emptying the
# staging table makes the next enrich run bootstrap every issue from bronze,
# which is the only channel that rewrites those rows.
#
# The window is invisible downstream: this hook builds tag:gold only, silver's
# delete+insert removes just the unique_keys the incoming batch carries (an
# empty Jira arm carries none), and the sync pipeline runs enrich before dbt.
#
# Self-limiting instead of ledger-backed, per this script's re-run contract:
# rows but not one title means the table predates the enrich change, and the
# bronze probe keeps an instance whose issues genuinely carry no summary from
# truncating on every deploy.

# SAFETY: callers use `ch_answer_is_yes ... || return 0`, which disables `set -e`
# for the body — a probe failure (auth/transient, or a column the DDL macro has
# not added yet, since it runs at dbt on-run-start below) reads as "no" and the
# caller skips its heal rather than aborting the deploy.
ch_answer_is_yes() {
  local query="$1" answer
  answer="$(printf '%s' "$query" | _ch_http_query | tr -d '[:space:]')"
  [[ "$answer" == "1" ]]
}

heal_jira_task_history_titles() {
  ch_table_is_real staging jira__task_field_history || return 0
  ch_table_is_real bronze_jira jira_issue || return 0
  ch_answer_is_yes "SELECT count() = 1 FROM system.columns WHERE database='staging' AND table='jira__task_field_history' AND name='title'" || return 0
  ch_answer_is_yes "SELECT count() > 0 AND countIf(title IS NOT NULL) = 0 FROM staging.jira__task_field_history" || return 0
  ch_answer_is_yes "SELECT count() > 0 FROM bronze_jira.jira_issue WHERE JSONExtractString(COALESCE(custom_fields_json, ''), 'summary') != ''" || return 0

  echo "  staging.jira__task_field_history (emptied; next enrich run re-bootstraps it with summaries)"
  run_ch <<SQL
TRUNCATE TABLE staging.jira__task_field_history;
SQL
}

heal_jira_task_history_titles

echo "=== Healing git file-change object id columns ==="
# The file-change object ids arrive at the tail of every projection that feeds
# class_git_file_changes. Pre-existing tables lack them and the positional
# insert misaligns; the silver side heals in migrations/*.sql, the rest heals
# here because these tables exist only after a connector has run.
#
# bronze_github.file_changes is healed for a different reason: the GitHub
# staging model READS the two columns, and nothing else adds them in time.
# create-bronze-placeholders.sh is IF NOT EXISTS so a warm bronze table is
# never altered, and the destination only widens it on the connector's next
# sync — which lands after this deploy's dbt run, leaving the staging model
# (and every git model downstream of it) failing on an unknown identifier
# until then.
#
# Existing rows heal to NULL and carry an oid from the first sync that
# re-collects them. Idempotent.
heal_git_file_change_oids() {
  local db="$1" table="$2" anchor="$3"
  ch_table_is_real "${db}" "${table}" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS pre_image_oid Nullable(String) AFTER ${anchor};
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS post_image_oid Nullable(String) AFTER pre_image_oid;
ALTER TABLE ${db}.${table} MODIFY COLUMN pre_image_oid Nullable(String) AFTER ${anchor};
ALTER TABLE ${db}.${table} MODIFY COLUMN post_image_oid Nullable(String) AFTER pre_image_oid;
SQL
}

# Bronze's tail is patch_truncated; every staging projection ends with
# _airbyte_extracted_at.
heal_git_file_change_oids bronze_github file_changes patch_truncated

for _git_source in github gitlab bitbucket_cloud; do
  heal_git_file_change_oids staging "${_git_source}__file_changes" _airbyte_extracted_at
done

echo "=== Healing git commit patch id column ==="
# Same positional invariant: every projection feeding class_git_commits gained
# patch_id at the tail (commit-content identity for counting an authored
# change once, #2792). The silver side heals in migrations/*.sql; staging
# heals here because these tables exist only after a connector has run.
# Existing rows heal to NULL and carry a patch id from the first sync that
# re-collects them. Idempotent.
heal_git_commit_patch_id() {
  local table="$1"
  ch_table_is_real staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS patch_id Nullable(String) AFTER _airbyte_extracted_at;
ALTER TABLE staging.${table} MODIFY COLUMN patch_id Nullable(String) AFTER _airbyte_extracted_at;
SQL
}

for _git_source in github gitlab bitbucket_cloud; do
  heal_git_commit_patch_id "${_git_source}__commits"
done

echo "=== Healing git pull-request author account column ==="
# Same positional invariant: every projection feeding class_git_pull_requests
# gained author_account_id after author_email (account-first person
# attribution, #2819). The silver side heals in migrations/*.sql; staging
# heals here because these tables exist only after a connector has run.
# Existing rows heal to '' and carry the account id from the first sync that
# re-collects them — the rollout's full refresh backfills the rest. Idempotent.
heal_git_pr_author_account() {
  local table="$1"
  ch_table_is_real staging "${table}" || return 0
  echo "  staging.${table}"
  run_ch <<SQL
ALTER TABLE staging.${table} ADD COLUMN IF NOT EXISTS author_account_id String AFTER author_email;
ALTER TABLE staging.${table} MODIFY COLUMN author_account_id String AFTER author_email;
SQL
}

for _git_source in github gitlab bitbucket_cloud; do
  heal_git_pr_author_account "${_git_source}__pull_requests"
done

echo "=== Healing git repository default-branch column ==="
# class_git_repositories gained `default_branch` at the projection tail; the
# silver side heals in migrations/*.sql, the staging members heal here because
# those tables exist only after their connector has run. Existing rows heal to
# NULL and carry a branch from the next repositories sync. Idempotent.
heal_git_repository_default_branch() {
  local db="$1" table="$2"
  ch_table_is_real "${db}" "${table}" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS default_branch Nullable(String) AFTER _airbyte_extracted_at;
ALTER TABLE ${db}.${table} MODIFY COLUMN default_branch Nullable(String) AFTER _airbyte_extracted_at;
SQL
}

for _git_source in github gitlab bitbucket_cloud; do
  heal_git_repository_default_branch staging "${_git_source}__repositories"
done

echo "=== Healing git size column types ==="
# Every projection feeding the git classes wraps its size columns in
# toNullable(), so the model's type is Nullable(Int64). A table created before
# that keeps the narrower non-null type ClickHouse inferred from the values it
# then held, and dbt aborts an incremental model whose source and target types
# differ — freezing the relation at its last successful build. MODIFY converges
# warm tables; non-null to Nullable is lossless. Type only, no AFTER anchor:
# the columns already sit at their contract positions. Guarded: staging tables
# exist only after the connector's first run. Idempotent.
heal_git_size_column() {
  local table="$1" column="$2"
  ch_table_exists staging "$table" || return 0
  echo "  staging.${table}.${column}"
  run_ch <<SQL
ALTER TABLE staging.${table} MODIFY COLUMN IF EXISTS ${column} Nullable(Int64);
SQL
}

for _git_source in github gitlab bitbucket_cloud; do
  for _size_column in files_changed lines_added lines_removed; do
    heal_git_size_column "${_git_source}__commits" "${_size_column}"
    heal_git_size_column "${_git_source}__pull_requests" "${_size_column}"
  done
  for _size_column in lines_added lines_removed; do
    heal_git_size_column "${_git_source}__file_changes" "${_size_column}"
  done
done

echo "=== Healing jira task id column types (#1743) ==="
# #1892 retyped the jira staging id projections (worklog_id, comment_id)
# from raw bronze Decimal(38,9) to toString(...), but pre-existing
# incremental-append staging tables keep the Decimal column, and the
# positional union with the youtrack String twins fails with NO_COMMON_TYPE,
# blanking Task Delivery / Code Quality. MODIFY converges warm tables (and
# silver targets built from them) to the snapshot's String; Decimal->String
# is lossless. Guarded: staging tables exist only after the first jira sync.
heal_task_id_column() {
  local db="$1" table="$2" column="$3"
  ch_table_exists "$db" "$table" || return 0
  echo "  ${db}.${table}.${column}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} MODIFY COLUMN IF EXISTS ${column} Nullable(String);
SQL
}

heal_task_id_column staging jira__task_worklogs worklog_id
heal_task_id_column staging jira__task_comments comment_id
heal_task_id_column silver class_task_worklogs worklog_id
heal_task_id_column silver class_task_comments comment_id

# The cohort and coverage relations are dbt VIEWs (identity resolves at query
# time). A cluster that predates that holds them as MergeTree TABLEs, and dbt's
# view materialization replaces via CREATE OR REPLACE VIEW, which is not
# guaranteed to replace a table.
#
# SAFETY: renamed, never dropped, and only when the gold build that recreates
# them is about to run. A failed build aborts the script (set -e) with the data
# still in `<table>__pre_view_backup`, which an operator can rename back; the
# backup is dropped only after the build succeeds.
GOLD_VIEW_QUARANTINE=()

quarantine_gold_table_for_view() {
  local db="$1" table="$2"
  local engine
  engine="$(
    printf "SELECT engine FROM system.tables WHERE database='%s' AND name='%s'" "$db" "$table" |
      _ch_http_query |
      tr -d '[:space:]'
  )"
  [[ -n "$engine" && "$engine" != "View" ]] || return 0
  echo "  ${db}.${table} (${engine}, renamed to ${table}__pre_view_backup)"
  run_ch <<SQL
DROP TABLE IF EXISTS ${db}.${table}__pre_view_backup;
RENAME TABLE ${db}.${table} TO ${db}.${table}__pre_view_backup;
SQL
  GOLD_VIEW_QUARANTINE+=("${db}.${table}__pre_view_backup")
}

drop_gold_view_quarantine() {
  local relation
  for relation in "${GOLD_VIEW_QUARANTINE[@]:-}"; do
    [[ -n "$relation" ]] || continue
    echo "  ${relation}"
    run_ch <<SQL
DROP TABLE IF EXISTS ${relation};
SQL
  done
}

# SKIP_DBT_GOLD=1 (set by bootstrap-db snapshot generation) skips this step:
# generation already built every tag:gold model with the pinned dbt venv
# (run-dbt.sh) BEFORE the migrations ran, and re-running here would need a `dbt`
# on PATH — which outside the prod toolbox (local dev, the connectors-ddl CI
# workflow) is absent or the wrong build (dbt-fusion 2.0). Real deploys leave it
# unset and rely on this step to materialise gold at deploy time.
if [[ "${SKIP_DBT_GOLD:-}" == "1" ]]; then
  echo "=== Skipping gold dbt build (SKIP_DBT_GOLD=1; gold pre-built by generation) ==="
else
# DBT_GOLD_SELECT widens the selection (space-separated dbt selectors);
# the seed's silver step adds +identity_inputs, deploys leave it unset.
read -r -a _dbt_select <<<"${DBT_GOLD_SELECT:-tag:gold}"
# SAFETY: appended after the override so no caller can narrow the selector and
# leave the map views at the snapshot's point-in-time bodies.
_dbt_select+=("tag:identity:map")
echo "=== Quarantining gold identity relations still held as tables ==="
quarantine_gold_table_for_view insight metric_entity_cohorts_current
quarantine_gold_table_for_view insight identity_resolution_coverage

# INVARIANT: never export DBT_FULL_REFRESH — reconcile-connectors owns that
# name, and env reaches every child.
_dbt_flags=()
if [[ "$FULL_REFRESH" == "1" ]]; then
  _dbt_flags+=(--full-refresh)
fi
echo "=== Building gold models (dbt run --select ${_dbt_select[*]} ${_dbt_flags[*]:-}) ==="
# Gold views are dbt-owned but must exist at DEPLOY time, not first-sync
# time: the analytics service marks metric definitions schema-error while
# an observation view is missing, which blanks those metrics for every
# frontend request until the first connector sync builds the view (hours
# on a scheduled instance). The placeholders created above guarantee every
# relation the views reference exists, so this run type-checks on a fresh
# cluster — the same guarantee the scoped per-connector dbt runs rely on
# for sideways refs. Idempotent: views are create-or-replace and
# table-materialized gold models rebuild via atomic swap. Table builds
# are bounded by the models' own query_settings (memory, threads, disk
# spill), so this step degrades to a slower build rather than failing
# the deploy on data volume.
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
                # Correlated subqueries (LEFT ANTI JOIN in the identity seed
                # models) are gated behind this experimental flag on CH 25.7.
                # A model-level config() setting does NOT reach the SELECT plan
                # in dbt-clickhouse, so it must be set at profile level. Parity
                # with test/bootstrap.
                "settings": {"allow_experimental_correlated_subqueries": 1},
            }
        },
    }
}
with open(os.path.join(os.environ["DBT_PROFILES_DIR"], "profiles.yml"), "w") as f:
    yaml.safe_dump(profile, f)
PY
(cd "$SCRIPT_DIR/../dbt" && dbt run --profiles-dir "$DBT_PROFILES_DIR" --log-format json --select "${_dbt_select[@]}" ${_dbt_flags[@]+"${_dbt_flags[@]}"})

echo "=== Dropping quarantined pre-view tables (gold build succeeded) ==="
drop_gold_view_quarantine
rm -rf "$DBT_PROFILES_DIR"
fi

echo "=== ClickHouse migrations complete ==="
