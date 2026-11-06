#!/usr/bin/env bash
# Apply the ClickHouse schema migrations and the guarded heals against an
# EXTERNAL ClickHouse, over its HTTP interface via lib/ch-exec.sh (selected by
# CLICKHOUSE_URL). Invoked by the clickhouse-migrate Helm Hook Job
# (post-install,post-upgrade).
#
# Steps:
#   1. Run create-databases.sh — every database this release owns, plus the
#      role/user provisioning that reads them. It is the only creation site:
#      the migrations below and the dbt hooks assume their database stands.
#   2. Refuse a warehouse older than the floor this release upgrades from
#      (lib/warehouse-floor.sh), before any migration or heal runs.
#   3. Apply migrations/*.sql in lexicographic order, then the heals.
#
# A heal stays while an install it has not reached yet can exist; once every
# supported upgrade path has run it, it is retired and its outcome becomes a
# floor fact.
#
# No relation is pre-created. Every staging, silver and gold relation is dbt's,
# built by the sync that feeds it; one no sync has built yet does not exist,
# and analytics answers it as not-ready (#3559).
#
# No dbt: a connector sync rebuilds its own models, gold included, because
# every descriptor's selector ends in `+` (#3561). A deploy therefore leaves
# gold exactly as the last sync built it, and a relation no sync has built yet
# is answered as not-ready rather than as a failure.
#
# Bookkeeping: none — every migration is re-run on every invocation and
# MUST stay idempotent/re-runnable (CREATE OR REPLACE / IF NOT EXISTS). The
# heals already retired: docs/operations/CLICKHOUSE-UPGRADE-FLOOR.md.
#
# Required env (set by the Hook Job from chart values + insight-db-creds):
#   CLICKHOUSE_URL       e.g. http://ch-host:8123  (selects the HTTP backend)
#   CLICKHOUSE_USER, CLICKHOUSE_PASSWORD
#   CLICKHOUSE_DATABASE  the Insight app database
#
# Optional env (topology of the target ClickHouse, epic #2010):
#   CLICKHOUSE_CLUSTER_MODE  true -> the DDL emitted here replicates
#   CLICKHOUSE_CLUSTER_NAME  the cluster the ON CLUSTER clause names
# Both default to a standalone single node.
#
# Options: none beyond --help. Building models is build-gold.sh's job.
set -euo pipefail

while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help)
      awk 'NR>1 && /^#/ {sub(/^# ?/, ""); print; next} NR>1 {exit}' "${BASH_SOURCE[0]}"
      exit 0 ;;
    *)
      echo "apply-ch-migrations.sh: unknown argument: $1" >&2
      echo "usage: apply-ch-migrations.sh" >&2
      exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

: "${CLICKHOUSE_URL:?CLICKHOUSE_URL must be set (e.g. http://ch-host:8123)}"
: "${CLICKHOUSE_DATABASE:?CLICKHOUSE_DATABASE must be set (the Insight app database)}"
# Exported so create-databases.sh, which this script runs as a child, reads the
# same topology this one was given.
export CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE:-false}"
export CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME:-}"

source "$SCRIPT_DIR/lib/ch-topology.sh"
source "$SCRIPT_DIR/lib/retired-relations.sh"
source "$SCRIPT_DIR/lib/warehouse-floor.sh"

bash "$SCRIPT_DIR/create-databases.sh"

echo "=== Verifying the warehouse is at the floor this release upgrades from ==="
ch_assert_floor

# The cohort and coverage relations are dbt VIEWs. A release that found one
# still held as a TABLE renamed it to `<table>__pre_view_backup` so an operator
# could rename it back; the backup goes once the live relation is a View.
# INVARIANT: can fire at the floor: a floor deploy whose gold build failed, or a
# later release's rename before the next sync, leaves the backup behind.
drop_superseded_gold_backup() {
  local db="$1" table="$2" backup="${2}__pre_view_backup" engine
  ch_table_exists "$db" "$backup" || return 0
  engine="$(ch_table_engine "$db" "$table")"
  [[ "$engine" == "View" ]] || return 0
  echo "  ${db}.${backup} (superseded by the view, dropping)"
  run_ch <<SQL
DROP TABLE IF EXISTS ${db}.${backup};
SQL
}

echo "=== Dropping gold identity backups the views superseded ==="
drop_superseded_gold_backup insight metric_entity_cohorts_current
drop_superseded_gold_backup insight identity_resolution_coverage

echo "=== Applying ClickHouse migrations ==="
# A migration that creates a relation spells its engine family behind the
# `/*@@REPLICATED@@*/` marker, which ch_render_topology expands for the
# declared topology: the plain family stands on every replica holding whatever
# rows reached the node that answered.
shopt -s nullglob
for migration in "$SCRIPT_DIR/migrations"/*.sql; do
  echo "  $(basename "$migration")"
  ch_render_topology < "$migration" | run_ch
done

echo "=== Dropping retired databases and relations ==="
# INVARIANT: no release creates these, and each drops only while empty; delete a line once no supported install can hold its relation.
drop_retired_database_if_empty person
for database in bronze_claude_admin bronze_figma bronze_github_copilot bronze_openai bronze_salesforce bronze_workday; do
  drop_retired_database_if_empty "${database}"
done
drop_retired_table_if_empty insight raw_data
drop_retired_table_if_empty identity aliases
# INVARIANT: can fire at the floor: any dropped materialized view leaves its inner table behind.
drop_orphaned_inner_tables insight

echo "=== Healing Jira bronze issue identity ==="
# jira_issue_history / jira_comments / jira_worklogs carry `jira_id` since
# connector 6.1.0; rows written before it name their issue only by
# `id_readable`, the key at fetch time. The id is recovered from the two issue
# streams, read WITHOUT FINAL: an issue's superseded rows may still map a key it
# no longer answers to. The pairs are joined in a SELECT and the table is
# rebuilt from the result, keyed by (tenant_id, source_id, id_readable) because
# one bronze holds every connector instance. Worklogs prefer Jira's own
# `issueId`. Guarded on the rows it would change, so a converged warehouse pays
# one SELECT per table and nothing else.
#
# Not a mutation: it reads a second relation, and where the warehouse
# replicates ClickHouse admits only the row's own columns in an `ALTER …
# UPDATE`, rejecting joinGet, dictGet and every subquery there as
# non-deterministic. A SELECT is written into a new table and swapped in.
#
# Distinct keys, not rows: the physical row count of a ReplacingMergeTree is
# not a quantity to hold a copy against — a background merge collapsing a
# superseded row between two counts would fail a sound rebuild. `count() FINAL`
# would say the same thing but only where the table is replacing; a bronze
# table that is not answers it with ILLEGAL_FINAL.
_jira_key_count() {
  ch_scalar "SELECT uniqExact(unique_key) FROM bronze_jira.$1"
}

# The original is under the `__rebuild` name after an EXCHANGE; put it back and
# leave the rejected copy there for inspection (the next run drops it first).
#
# INVARIANT: `bronze_*` is an Atomic database (the destination creates it), so
# its table DDL carries the cluster clause: the database replays nothing.
# `CREATE TABLE ... AS` copies the engine, Replicated* included, and the
# clause gives every replica the same table uuid, so the copy replicates.
_jira_swap_back() {
  run_ch <<SQL
EXCHANGE TABLES bronze_jira.$1 AND bronze_jira.$1__rebuild $(ch_on_cluster);
SQL
}

# INVARIANT: the copy below takes the source's engine verbatim, so on a cluster
# a plain source (a table from before clusterMode was turned on) would yield a
# plain copy that replicates nothing, and the quorum counts would judge one
# replica. The topology guard refuses such a table before any DDL; this holds
# the same at the copy.
_jira_require_replicated_source() {
  local table="$1" engine
  ch_cluster_mode || return 0
  engine="$(ch_table_engine bronze_jira "${table}")" || {
    echo "  bronze_jira.${table}: could not read its engine — refusing to rebuild" >&2
    return 1
  }
  [[ "${engine}" == Replicated* ]] && return 0

  echo "  bronze_jira.${table}: engine ${engine:-not found on the replica that answered}; a copy of it would not replicate — refusing to rebuild" >&2
  return 1
}

# Rebuild one bronze_jira table from `select_sql` and swap the result in. The
# copy is held between the table's key count before it started and its count
# once it finished, so a key lost and a key invented are both rejected while a
# row a sync committed meanwhile is not; those rows are copied over whole, so
# the rebuild's own filter can drop none of them.
#
# INVARIANT: no statement that can lose rows runs before the rows it depends
# on are verified. The copy is counted against the table before the swap; the
# swapped-in table is counted again before the old one is dropped; any miss
# leaves the original table live and stops the deploy. `run_ch` stops at the
# first failed statement, so a failed copy never reaches the EXCHANGE either
# way.
_jira_swap_in_rebuilt() {
  local table="$1" select_sql="$2"
  local copy="bronze_jira.${table}__rebuild" before after copied live copy_start_ms

  _jira_require_replicated_source "${table}"
  before="$(_jira_key_count "${table}")"
  copy_start_ms="$(ch_scalar "SELECT toUnixTimestamp64Milli(now64(3))")"

  run_ch <<SQL
DROP TABLE IF EXISTS ${copy} $(ch_on_cluster);
CREATE TABLE ${copy} $(ch_on_cluster) AS bronze_jira.${table};
INSERT INTO ${copy}
${select_sql};
SQL

  copied="$(_jira_key_count "${table}__rebuild")"
  after="$(_jira_key_count "${table}")"
  if [[ "${copied}" -lt "${before}" || "${copied}" -gt "${after}" ]]; then
    echo "  bronze_jira.${table}: copy holds ${copied} key(s), expected between ${before} and ${after} — leaving the table as it is" >&2
    run_ch <<SQL
DROP TABLE IF EXISTS ${copy} $(ch_on_cluster);
SQL
    return 1
  fi

  run_ch <<SQL
EXCHANGE TABLES bronze_jira.${table} AND ${copy} $(ch_on_cluster);
SQL
  # From here on the rebuilt table is live and the original sits under the
  # `__rebuild` name. Every step until the DROP is checked explicitly, and any
  # failure swaps the original back: under `set -e` a bare failing statement
  # would end the script with the unverified table still live.
  #
  # The hour of slack covers an extraction stamp older than the write. Only the
  # rows the rebuild did not already take are carried: a row it copied WITH its
  # fill coming back without one would land on the same key with the same
  # version, and the pair would settle on whichever of the two a merge kept.
  if ! run_ch <<SQL
INSERT INTO bronze_jira.${table}
SELECT *
FROM ${copy}
WHERE _airbyte_extracted_at >= fromUnixTimestamp64Milli(${copy_start_ms}) - INTERVAL 1 HOUR
  AND _airbyte_raw_id NOT IN (
    SELECT _airbyte_raw_id
    FROM bronze_jira.${table}
    WHERE _airbyte_extracted_at >= fromUnixTimestamp64Milli(${copy_start_ms}) - INTERVAL 1 HOUR
);
SQL
  then
    echo "  bronze_jira.${table}: carrying the late rows over failed — swapping the original back" >&2
    _jira_swap_back "${table}"
    return 1
  fi

  if ! live="$(_jira_key_count "${table}")" || [[ "${live}" -lt "${before}" ]]; then
    echo "  bronze_jira.${table}: rebuilt table holds ${live:-?} key(s), expected at least ${before} — swapping the original back" >&2
    _jira_swap_back "${table}"
    return 1
  fi

  run_ch <<SQL
DROP TABLE IF EXISTS ${copy} $(ch_on_cluster);
SQL
}

# Counts only the rows the pairs can actually fill: a key no issue stream ever
# delivered stays NULL forever, and counting it would re-run the rebuild on
# every deploy. Worklogs carry Jira's own issueId and count whenever it is set.
_jira_rows_needing_issue_id() {
  local table="$1" own="0"
  [[ "${table}" == "jira_worklogs" ]] && own="issueId IS NOT NULL"
  ch_scalar "$(printf "SELECT count() FROM bronze_jira.%s
          WHERE jira_id IS NULL
            AND (%s
                 OR (id_readable IS NOT NULL AND tenant_id IS NOT NULL AND source_id IS NOT NULL
                     AND (assumeNotNull(tenant_id), assumeNotNull(source_id), assumeNotNull(id_readable)) IN (
                   SELECT assumeNotNull(tenant_id), assumeNotNull(source_id), assumeNotNull(id_readable)
                   FROM bronze_jira.jira_issue
                   WHERE tenant_id IS NOT NULL AND source_id IS NOT NULL AND id_readable IS NOT NULL AND jira_id IS NOT NULL
                   UNION ALL
                   SELECT assumeNotNull(tenant_id), assumeNotNull(source_id), assumeNotNull(id_readable)
                   FROM bronze_jira.jira_issue_keys
                   WHERE tenant_id IS NOT NULL AND source_id IS NOT NULL AND id_readable IS NOT NULL AND jira_id IS NOT NULL)))" \
    "${table}" "${own}")"
}

# One row per (tenant, source, key): a key reused across issues resolves to one
# of its ids, and the aggregate is where that choice is made rather than
# whichever row the join below met first.
_jira_issue_id_pairs() {
  cat <<SQL
    SELECT tenant_id, source_id, id_readable, any(jira_id) AS jira_id
    FROM
    (
        SELECT assumeNotNull(tenant_id) AS tenant_id, assumeNotNull(source_id) AS source_id,
               assumeNotNull(id_readable) AS id_readable, assumeNotNull(jira_id) AS jira_id
        FROM bronze_jira.jira_issue
        WHERE tenant_id IS NOT NULL AND source_id IS NOT NULL AND id_readable IS NOT NULL AND jira_id IS NOT NULL
        UNION ALL
        SELECT assumeNotNull(tenant_id) AS tenant_id, assumeNotNull(source_id) AS source_id,
               assumeNotNull(id_readable) AS id_readable, assumeNotNull(jira_id) AS jira_id
        FROM bronze_jira.jira_issue_keys
        WHERE tenant_id IS NOT NULL AND source_id IS NOT NULL AND id_readable IS NOT NULL AND jira_id IS NOT NULL
    )
    GROUP BY tenant_id, source_id, id_readable
SQL
}

# A worklog names its issue itself (Jira's `issueId`) and needs no key; the
# other two, and a worklog without one, resolve through the pairs and so need
# the full (tenant, source, key) identity. A row that already carries an id
# keeps it, which is what makes the rebuild re-runnable, and `ANY` takes at
# most one match, so the rebuild is row for row whatever the pairs hold.
_jira_substream_issue_id_select() {
  local table="$1" own="s.jira_id"
  [[ "${table}" == "jira_worklogs" ]] && own="COALESCE(s.jira_id, s.issueId)"
  cat <<SQL
SELECT s.* REPLACE (COALESCE(${own}, nullIf(pairs.jira_id, '')) AS jira_id)
FROM bronze_jira.${table} AS s
LEFT ANY JOIN
(
$(_jira_issue_id_pairs)
) AS pairs
ON (assumeNotNull(s.tenant_id), assumeNotNull(s.source_id), assumeNotNull(s.id_readable))
 = (pairs.tenant_id, pairs.source_id, pairs.id_readable)
SQL
}

# INVARIANT: repairs rows jira connectors before 6.1.0 wrote (#3370, 0.7.21); such a row
# turns fillable whenever a sync delivers its key. Delete once no pre-6.1.0 row can stand.
heal_jira_substream_issue_id() {
  local table n
  for table in jira_issue_history jira_comments jira_worklogs; do
    ch_table_exists bronze_jira "${table}" || continue
    n="$(_jira_rows_needing_issue_id "${table}")"
    [[ "${n}" -gt 0 ]] || continue
    echo "  bronze_jira.${table}: ${n} row(s) without jira_id — filling from the issue streams"
    _jira_swap_in_rebuilt "${table}" "$(_jira_substream_issue_id_select "${table}")"
  done
}

# A plain call on purpose: `f || exit 1` would switch `set -e` OFF inside the
# function, so a failed statement in the middle would not stop it — the
# following swap and drop would run against an unverified copy.
heal_jira_substream_issue_id

echo "=== Healing the Jira catalogue first-sync record ==="
source "$SCRIPT_DIR/lib/heal-jira-catalogue-first-seen.sh"
heal_jira_catalogue_first_seen

echo "=== Healing Allure staging test results ==="
# INVARIANT: repairs releases without #3707, #3713 (main before 0.7.108); delete once every CH_FLOOR_RELEASES release runs it.
heal_allure_test_results_staging() {
  ch_table_exists staging allure__test_results || return 0
  echo "  staging.allure__test_results"
  run_ch <<'SQL'
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS category_id Nullable(Int64) AFTER job_run_url;
ALTER TABLE staging.allure__test_results MODIFY COLUMN category_id Nullable(Int64) AFTER job_run_url;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS category_name String AFTER category_id;
ALTER TABLE staging.allure__test_results MODIFY COLUMN category_name String AFTER category_id;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS assignee String AFTER category_name;
ALTER TABLE staging.allure__test_results MODIFY COLUMN assignee String AFTER category_name;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS tested_by String AFTER assignee;
ALTER TABLE staging.allure__test_results MODIFY COLUMN tested_by String AFTER assignee;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS launch_env String DEFAULT '{}' AFTER message;
ALTER TABLE staging.allure__test_results MODIFY COLUMN launch_env String DEFAULT '{}' AFTER message;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS test_result_url String AFTER launch_env;
ALTER TABLE staging.allure__test_results MODIFY COLUMN test_result_url String AFTER launch_env;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS launch_url String AFTER test_result_url;
ALTER TABLE staging.allure__test_results MODIFY COLUMN launch_url String AFTER test_result_url;
ALTER TABLE staging.allure__test_results ADD COLUMN IF NOT EXISTS test_case_url String AFTER launch_url;
ALTER TABLE staging.allure__test_results MODIFY COLUMN test_case_url String AFTER launch_url;
SQL
}

heal_allure_test_results_staging

echo "=== Healing task field-history relations ==="
# INVARIANT: repairs releases without #3498 (main before 0.7.66); delete once every CH_FLOOR_RELEASES release runs it.
run_ch <<'SQL'
DROP TABLE IF EXISTS staging.jira__task_field_history;
SQL

# `event_order` joined the class right after `_seq`, and every arm must carry
# it there before the class unions them — the GitHub arm and the derived
# journal too, because a class build can run before their own models rebuild
# them; the class itself carries it in the same place. Rows already stored get
# an approximation, so the class never folds zeros in; the full refresh the
# major versions dispatch writes the real values. The mutation is issued only
# while a row still matches it: only a row whose approximation is itself 0
# can, and a mutation scans the whole table and waits for every replica.
_EVENT_ORDER_APPROXIMATION="toUnixTimestamp64Milli(event_at) * 1000000
                       + multiIf(event_kind = 'synthetic_initial', 0, event_kind = 'changelog', 1, 2) * 100000
                       + _seq"

# INVARIANT: repairs releases without #3592 (main before 0.7.80); delete once every CH_FLOOR_RELEASES release runs it.
heal_task_field_history_event_order() {
  local db="$1" table="$2" pending
  ch_table_exists "${db}" "${table}" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS event_order Int64 AFTER _seq;
ALTER TABLE ${db}.${table} MODIFY COLUMN event_order Int64 AFTER _seq;
SQL

  pending="$(ch_scalar "SELECT count() FROM ${db}.${table} WHERE event_order = 0 AND ${_EVENT_ORDER_APPROXIMATION} != 0")"
  [[ "${pending}" -gt 0 ]] || return 0
  echo "  ${db}.${table}: ${pending} row(s) without an event order"
  run_ch <<SQL
ALTER TABLE ${db}.${table}
    UPDATE event_order = ${_EVENT_ORDER_APPROXIMATION}
    WHERE event_order = 0 AND ${_EVENT_ORDER_APPROXIMATION} != 0
    SETTINGS mutations_sync = 2;
SQL
}

heal_task_field_history_event_order staging jira__field_history_derived
heal_task_field_history_event_order staging jira__availability_events
heal_task_field_history_event_order staging jira__comment_lifecycle_events
heal_task_field_history_event_order staging jira__worklog_lifecycle_events
heal_task_field_history_event_order staging github__task_field_history

# The class takes the approximation once, on the deploy that adds the column:
# afterwards a 0 there is the model's own value (a sync rewrites those rows
# every run), and approximating it on every deploy would mutate the whole class
# for a value the next sync writes back.
# INVARIANT: repairs releases without #3592 (main before 0.7.80); delete once every CH_FLOOR_RELEASES release runs it.
heal_task_field_history_class_event_order() {
  local present
  ch_table_exists silver class_task_field_history || return 0
  present="$(ch_scalar "SELECT count() FROM system.columns
    WHERE database = 'silver' AND table = 'class_task_field_history' AND name = 'event_order'")"
  if [[ "${present}" -gt 0 ]]; then
    run_ch <<SQL
ALTER TABLE silver.class_task_field_history MODIFY COLUMN event_order Int64 AFTER _seq;
SQL
    return 0
  fi

  echo "  silver.class_task_field_history: adding event_order, approximated for the rows it holds"
  run_ch <<SQL
ALTER TABLE silver.class_task_field_history ADD COLUMN IF NOT EXISTS event_order Int64 AFTER _seq;
ALTER TABLE silver.class_task_field_history
    UPDATE event_order = ${_EVENT_ORDER_APPROXIMATION}
    WHERE event_order = 0
    SETTINGS mutations_sync = 2;
SQL
}

heal_task_field_history_class_event_order

echo "=== Healing collab-meeting contract schema ==="
# Same positional invariant: collab meeting rows gained the account triple
# after data_source (zoom-guest attribution), then the attended meeting keys
# after account_id. Every row written before the columns existed is keyed by
# its email, which the default states, and names no meetings: gold then keeps
# its count under the address until the descriptor's major bump rebuilds the day.
# INVARIANT: repairs releases without #3989, #4021 (main before 0.7.172); delete once every CH_FLOOR_RELEASES release runs it.
heal_collab_meeting_table() {
  local db="$1" table="$2"
  ch_table_exists "$db" "$table" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS account_source_type String DEFAULT 'email' AFTER data_source;
ALTER TABLE ${db}.${table} MODIFY COLUMN account_source_type String DEFAULT 'email' AFTER data_source;
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS account_source_id String AFTER account_source_type;
ALTER TABLE ${db}.${table} MODIFY COLUMN account_source_id String AFTER account_source_type;
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS account_id String AFTER account_source_id;
ALTER TABLE ${db}.${table} MODIFY COLUMN account_id String AFTER account_source_id;
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS attended_meeting_keys Array(String) AFTER account_id;
ALTER TABLE ${db}.${table} MODIFY COLUMN attended_meeting_keys Array(String) AFTER account_id;
SQL
}

heal_collab_meeting_table staging m365__collab_meeting_activity
heal_collab_meeting_table staging zoom__collab_meeting_activity
heal_collab_meeting_table silver class_collab_meeting_activity

echo "=== Healing git pull-request reported close-time column ==="
# Same positional invariant: every projection feeding class_git_pull_requests
# gained closed_on_reported after closed_on — the close time as the source
# stated it, which the duration measures read so a recovered one cannot pose as
# a measurement (#3362). The class carries it in the same place, and must
# before the backfill below reads it. Idempotent.
# INVARIANT: repairs releases without #3363 (main before 0.7.35); delete once every CH_FLOOR_RELEASES release runs it.
heal_git_pr_close_time_reported() {
  local db="$1" table="$2"
  ch_table_exists "${db}" "${table}" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS closed_on_reported Nullable(DateTime) AFTER closed_on;
ALTER TABLE ${db}.${table} MODIFY COLUMN closed_on_reported Nullable(DateTime) AFTER closed_on;
SQL
}

for _git_source in github gitlab bitbucket_cloud; do
  heal_git_pr_close_time_reported staging "${_git_source}__pull_requests"
done
heal_git_pr_close_time_reported silver class_git_pull_requests

echo "=== Backfilling git pull-request reported close time from the sources ==="
# The rows a warm warehouse already holds must be filled from the SOURCE
# timestamps, never from this table's own `closed_on`.
#
# `closed_on` is the SETTLED close, and the contract that settled it changed
# here: both projections used to prefer `closed_at`, so a request CLOSED,
# reopened and later MERGED settled on the earlier close. Copying that into
# `closed_on_reported` would hand every duration measure an interval that ended
# before the merge — the very reading this change corrects — and it would do so
# invisibly, because the value is a real instant the source once reported.
#
# So the reported close is recomputed from bronze under the new contract:
# `merged_at` when the source states one, else `closed_at`. `unique_key` reaches
# bronze unchanged through both projections, which is what makes the lookup a
# key equality rather than a guess. The lookup is a subquery joined in a SELECT
# and the answer is written back as a new row: where the warehouse replicates,
# a mutation may read nothing but the row it rewrites — ClickHouse rejects
# joinGet, dictGet and every subquery inside an `ALTER … UPDATE` as
# non-deterministic — while these relations are keyed and versioned, so a row
# carrying a later `_version` IS the correction. That is how dbt writes them
# too, and every read of them already takes the newest row per key.
#
# Three relations feed it. `bronze_gitlab.merge_requests` is the pre-#3250
# stream: it is absent from the DDL snapshot and from any fresh cluster, and
# holds the GitLab history of an installation that has not yet re-synced on the
# rebuilt connector — which is exactly the installation this backfill is for.
#
# A row bronze cannot answer for falls back only where the STATE itself proves
# what `closed_on` holds. `CLOSED` is such a state: it is terminal and it never
# merged, so the settled close can only be the reported `closed_at`. `MERGED`
# is not — `closed_on` there may be a merge or the stale close this change
# corrects, and guessing is what went wrong. Nor is `OPEN` or `LOCKED`, which
# have no close to report. Those keep NULL.
#
# Bitbucket is excluded throughout: its `closed_on` may be RECOVERED, and
# promoting a recovered time to a reported one is the error this column exists
# to prevent.

# The relations that carry the column, as "<db>|<table>|<eligible rows>". The
# class holds all three connectors in one table, so it alone needs the filter
# that keeps Bitbucket out; the staging projections are per connector already.
_GIT_PR_REPORTED_CLOSE_TARGETS=(
  "silver|class_git_pull_requests|data_source IN ('insight_github', 'insight_gitlab')"
  "staging|github__pull_requests|1"
  "staging|gitlab__pull_requests|1"
)

# One branch of the lookup's UNION: the reported close as the source states it.
#
# `stream_rank` settles which relation wins when a GitLab installation holds
# both the pre-#3250 stream and its replacement. The current stream ranks above
# the legacy one by DECLARATION, not by whichever happened to be extracted
# later — a clock is not a contract.
_git_pr_reported_close_branch() {
  local relation="$1" stream_rank="$2"
  cat <<SQL
    SELECT assumeNotNull(tenant_id) AS tenant_id,
           assumeNotNull(source_id) AS source_id,
           assumeNotNull(unique_key) AS unique_key,
           COALESCE(nullIf(merged_at, ''), nullIf(closed_at, ''), '') AS reported_close,
           toUInt8(${stream_rank}) AS stream_rank,
           _airbyte_extracted_at AS extracted_at
    FROM ${relation}
    WHERE tenant_id IS NOT NULL AND source_id IS NOT NULL AND unique_key IS NOT NULL
SQL
}

# The rows this backfill would CHANGE: the current row per key, still without a
# reported close, that a source or the state itself can answer for. A converged
# warehouse counts zero and writes nothing — the rows that stay NULL for ever
# (a merge bronze cannot answer for, a request still open) must not re-trigger
# it on every deploy. The count and the INSERT read this one expression, so the
# rows written and the rows counted cannot drift apart.
_git_pr_rows_needing_reported_close() {
  local db="$1" table="$2" eligible="$3" lookup="$4"
  cat <<SQL
FROM
(
    SELECT *
    FROM ${db}.${table}
    QUALIFY ROW_NUMBER() OVER (PARTITION BY unique_key ORDER BY _version DESC) = 1
) AS t
LEFT JOIN
(
${lookup}
) AS src
ON (assumeNotNull(t.tenant_id), assumeNotNull(t.source_id), assumeNotNull(t.unique_key))
 = (src.tenant_id, src.source_id, src.unique_key)
WHERE t.closed_on_reported IS NULL
  AND t.unique_key IS NOT NULL AND t.tenant_id IS NOT NULL AND t.source_id IS NOT NULL
  AND (${eligible})
  AND (coalesce(src.reported_close, '') != '' OR (t.state = 'CLOSED' AND t.closed_on IS NOT NULL))
SQL
}

# INVARIANT: repairs releases without #3363 (main before 0.7.35); delete once every CH_FLOOR_RELEASES release runs it.
backfill_git_pr_reported_close() {
  local sources=() source relation rank branches lookup spec db table eligible rows n candidates=0
  # "<relation> <stream_rank>": the current streams outrank the legacy one.
  for source in "bronze_github.pull_requests 1" "bronze_gitlab.pull_requests 1" \
                "bronze_gitlab.merge_requests 0"; do
    relation="${source%% *}"
    ch_table_exists "${relation%%.*}" "${relation##*.}" && sources+=("${source}")
  done
  [[ "${#sources[@]}" -gt 0 ]] || { echo "  no git pull-request bronze to read — skipping"; return 0; }

  branches=""
  for source in "${sources[@]}"; do
    relation="${source%% *}"; rank="${source##* }"
    [[ -n "${branches}" ]] && branches+="    UNION ALL"$'\n'
    branches+="$(_git_pr_reported_close_branch "${relation}" "${rank}")"$'\n'
  done

  # One row per key, so the join below cannot duplicate a pull request. The
  # ordering tuple is total: the current stream outranks the legacy one, a
  # later extraction outranks an earlier one, and the value itself breaks a
  # remaining tie — so the answer is a function of the data alone, not of
  # which part a merge happened to leave behind.
  lookup="$(cat <<SQL
    SELECT tenant_id, source_id, unique_key,
           argMax(reported_close, (stream_rank, extracted_at, reported_close)) AS reported_close
    FROM
    (
${branches})
    GROUP BY tenant_id, source_id, unique_key
SQL
)"

  for spec in "${_GIT_PR_REPORTED_CLOSE_TARGETS[@]}"; do
    IFS='|' read -r db table eligible <<<"${spec}"
    ch_table_exists "${db}" "${table}" || continue
    rows="$(_git_pr_rows_needing_reported_close "${db}" "${table}" "${eligible}" "${lookup}")"
    n="$(ch_scalar "SELECT count() ${rows}")"
    [[ "${n}" -gt 0 ]] || continue
    candidates=$((candidates + n))
    echo "  ${db}.${table}: ${n} row(s)"
    # `_version` is the write clock these relations dedup on, so stamping it
    # now is what makes the rewritten row the one every reader sees.
    run_ch <<SQL
INSERT INTO ${db}.${table}
SELECT t.* REPLACE (
    if(coalesce(src.reported_close, '') != '',
       parseDateTimeBestEffortOrNull(src.reported_close),
       t.closed_on) AS closed_on_reported,
    toUnixTimestamp64Milli(now64(3)) AS _version
)
${rows};
SQL
  done
  [[ "${candidates}" -gt 0 ]] || echo "  every row a source can answer for already carries its reported close"
}

backfill_git_pr_reported_close

echo "=== Healing task relationship ID columns ==="
# Same positional invariant: comments and worklogs gained their issue's
# immutable id after _version, links their source and target ids, in every
# projection and in the class that unions them.
# INVARIANT: repairs releases without #3616 (main before 0.7.84); delete once every CH_FLOOR_RELEASES release runs it.
heal_task_parent_id() {
  local db="$1" table="$2"
  ch_table_exists "${db}" "${table}" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS issue_id Nullable(String) AFTER _version;
ALTER TABLE ${db}.${table} MODIFY COLUMN issue_id Nullable(String) AFTER _version;
SQL
}

# INVARIANT: repairs releases without #3616 (main before 0.7.84); delete once every CH_FLOOR_RELEASES release runs it.
heal_task_link_ids() {
  local db="$1" table="$2"
  ch_table_exists "${db}" "${table}" || return 0
  echo "  ${db}.${table}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS issue_id Nullable(String) AFTER _version;
ALTER TABLE ${db}.${table} ADD COLUMN IF NOT EXISTS target_id Nullable(String) AFTER issue_id;
ALTER TABLE ${db}.${table} MODIFY COLUMN issue_id Nullable(String) AFTER _version;
ALTER TABLE ${db}.${table} MODIFY COLUMN target_id Nullable(String) AFTER issue_id;
SQL
}

for _task_parent_table in jira__task_comments jira__task_worklogs youtrack__task_comments youtrack__task_worklogs; do
  heal_task_parent_id staging "${_task_parent_table}"
done
heal_task_parent_id silver class_task_comments
heal_task_parent_id silver class_task_worklogs

for _task_link_table in github__task_links youtrack__task_links; do
  heal_task_link_ids staging "${_task_link_table}"
done
heal_task_link_ids silver class_task_links

echo "=== Healing focus metrics ==="
# Focus rows record the newest meeting-row version they were derived from, so an
# incremental build re-derives exactly the person-days whose meetings changed;
# rows stored before the column read 0 and are re-derived once. Rows derived
# before the 8 h fallback hold a zero working day, which the model no longer
# writes; the mutation is issued only while such a row remains.
# INVARIANT: repairs releases without #3665, #3741 (main before 0.7.114); delete once every CH_FLOOR_RELEASES release runs it.
heal_focus_metrics() {
  local pending
  ch_table_exists silver class_focus_metrics || return 0
  echo "  silver.class_focus_metrics"
  run_ch <<SQL
ALTER TABLE silver.class_focus_metrics ADD COLUMN IF NOT EXISTS source_version Int64 AFTER _version;
ALTER TABLE silver.class_focus_metrics MODIFY COLUMN source_version Int64 AFTER _version;
SQL

  pending="$(ch_scalar "SELECT count() FROM silver.class_focus_metrics WHERE working_hours_per_day = 0")"
  [[ "${pending}" -gt 0 ]] || return 0
  echo "  silver.class_focus_metrics: ${pending} row(s) without a working day"
  run_ch <<SQL
ALTER TABLE silver.class_focus_metrics
    UPDATE working_hours_per_day = 8.0,
           focus_time_pct        = round(greatest(toFloat64(0), 100.0 - (meeting_hours / 8.0) * 100.0), 2),
           dev_time_h            = round(greatest(toFloat64(0), 8.0 - meeting_hours), 4)
    WHERE working_hours_per_day = 0
    SETTINGS mutations_sync = 2;
SQL
}

heal_focus_metrics

echo "=== Healing column types a fresh install no longer creates ==="
# dbt never retypes a column an earlier run created: an incremental model
# inserts into the relation it finds, so one created before its model's
# expression changed type keeps the old type for good, while a fresh install
# of the same release creates the new one (#4057). Each heal below moves such
# a column to the type the current model creates. The stored values fit: the
# old expression never wrote what the new type cannot hold, and where it wrote
# NULL the current one writes the fallback the narrowing states.

# `<column>` becomes `<type>` where system.columns states another type; a
# column the table lacks is left to the heal that adds it. Only for a change
# every stored value survives, which each caller states.
retype_column() {
  local db="$1" table="$2" column="$3" type="$4" current
  current="$(ch_column_type "${db}" "${table}" "${column}")"
  [[ -n "${current}" && "${current}" != "${type}" ]] || return 0
  echo "  ${db}.${table}.${column}: ${current} -> ${type}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} MODIFY COLUMN ${column} ${type};
SQL
}

# `Nullable(<type>)` becomes `<type>`. ClickHouse refuses the narrowing while a
# row holds NULL, so those rows first take `<fallback>`, the value the current
# model writes where the old one wrote NULL.
retype_column_not_null() {
  local db="$1" table="$2" column="$3" type="$4" fallback="$5" current pending
  current="$(ch_column_type "${db}" "${table}" "${column}")"
  [[ "${current}" == "Nullable(${type})" ]] || return 0
  pending="$(ch_scalar "SELECT count() FROM ${db}.${table} WHERE ${column} IS NULL")"
  if [[ "${pending}" -gt 0 ]]; then
    echo "  ${db}.${table}.${column}: ${pending} NULL row(s) take ${fallback}"
    run_ch <<SQL
ALTER TABLE ${db}.${table} UPDATE ${column} = ${fallback} WHERE ${column} IS NULL SETTINGS mutations_sync = 2;
SQL
  fi
  echo "  ${db}.${table}.${column}: ${current} -> ${type}"
  run_ch <<SQL
ALTER TABLE ${db}.${table} MODIFY COLUMN ${column} ${type};
SQL
}

# The worklog state stopped reading a LEFT JOIN under join_use_nulls, which had
# made `is_deleted` Nullable without ever writing a NULL; a snapshot keeps the
# types its first build took.
# INVARIANT: repairs releases without #3704 (main before 0.7.106); delete once every CH_FLOOR_RELEASES release runs it.
heal_worklog_snapshot_is_deleted() {
  ch_table_exists staging jira__worklog_snapshot || return 0
  retype_column_not_null staging jira__worklog_snapshot is_deleted UInt8 0
}

heal_worklog_snapshot_is_deleted

# The Salesforce arm, which had no fallback time, widened the class union to
# Nullable; the HubSpot arm writes epoch 0 where it has none.
# INVARIANT: repairs releases without #2437 (main before 0.5.124); delete once every CH_FLOOR_RELEASES release runs it.
heal_crm_activities_timestamp() {
  ch_table_exists silver class_crm_activities || return 0
  retype_column_not_null silver class_crm_activities timestamp 'DateTime64(3)' 'toDateTime64(0, 3)'
}

heal_crm_activities_timestamp

# Every staging counter whose class union is Nullable declares it so; a
# relation from before that keeps the plain UInt32 of the branch that created
# it. Widening, no value moves.
# INVARIANT: repairs releases without #2101 (main before 0.5.40); delete once every CH_FLOOR_RELEASES release runs it.
heal_ai_dev_usage_counters() {
  local db="$1" table="$2" column
  shift 2
  ch_table_exists "${db}" "${table}" || return 0
  for column in "$@"; do
    retype_column "${db}" "${table}" "${column}" 'Nullable(UInt32)'
  done
}

heal_ai_dev_usage_counters staging cursor__ai_dev_usage \
  lines_removed total_lines_added total_lines_removed chat_requests
heal_ai_dev_usage_counters staging claude_enterprise__ai_dev_usage \
  lines_removed tool_use_offered tool_use_accepted commits_count pull_requests_count
heal_ai_dev_usage_counters silver class_ai_dev_usage \
  lines_removed total_lines_added total_lines_removed tool_use_offered tool_use_accepted \
  chat_requests commits_count pull_requests_count

# The SharePoint branch passed the bronze JSON number through as
# Decimal(38, 9) and the class union took it; a page count is integral, so
# the cast to Int64 changes no value.
# INVARIANT: repairs releases without #2101 (main before 0.5.40); delete once every CH_FLOOR_RELEASES release runs it.
heal_collab_document_visited_page_count() {
  local db="$1" table="$2"
  ch_table_exists "${db}" "${table}" || return 0
  retype_column "${db}" "${table}" visited_page_count 'Nullable(Int64)'
}

heal_collab_document_visited_page_count staging m365__collab_document_activity_sharepoint
heal_collab_document_visited_page_count silver class_collab_document_activity

# The compose seed once pre-created silver.class_ai_overage from placeholder
# DDL: String identity columns and a UInt64 version, where the model creates
# Nullable(String) and Int64. A ReplacingMergeTree version column cannot
# change sign in place, so a placeholder-shaped class goes only while it is
# empty, for the next sync to create as the model does; one holding rows
# keeps them, with the identity columns widened.
# INVARIANT: repairs the placeholder shape #1597 retired (main before 0.3.2); delete once every CH_FLOOR_RELEASES release runs it.
heal_ai_overage_placeholder_shape() {
  local column version
  ch_table_exists silver class_ai_overage || return 0
  version="$(ch_column_type silver class_ai_overage _version)"
  if [[ "${version}" == "UInt64" ]]; then
    echo "  silver.class_ai_overage: placeholder-shaped (_version ${version}), goes while empty"
    drop_retired_table_if_empty silver class_ai_overage
    ch_table_exists silver class_ai_overage || return 0
  fi
  for column in insight_tenant_id source_id email account_id data_source; do
    retype_column silver class_ai_overage "${column}" 'Nullable(String)'
  done
}

heal_ai_overage_placeholder_shape

echo "=== Removing Zoom participant device identifiers ==="
# Connector 3.0.0 stops collecting these; rows already stored still hold them.
# Dropping a column removes no row. Idempotent: a converged table, or one
# created after 3.0.0, has none of them.
# INVARIANT: repairs releases without #3880 (main before 0.7.142); delete once every CH_FLOOR_RELEASES release runs it.
heal_zoom_participant_device_columns() {
  ch_table_exists bronze_zoom participants || return 0
  echo "  bronze_zoom.participants"
  run_ch <<SQL
ALTER TABLE bronze_zoom.participants
    DROP COLUMN IF EXISTS ip_address,
    DROP COLUMN IF EXISTS internal_ip_addresses,
    DROP COLUMN IF EXISTS location,
    DROP COLUMN IF EXISTS pc_name,
    DROP COLUMN IF EXISTS domain,
    DROP COLUMN IF EXISTS mac_addr,
    DROP COLUMN IF EXISTS harddisk_id,
    DROP COLUMN IF EXISTS leave_reason,
    DROP COLUMN IF EXISTS device_name;
SQL
}

heal_zoom_participant_device_columns

echo "=== ClickHouse migrations complete ==="
