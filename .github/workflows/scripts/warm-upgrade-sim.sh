#!/usr/bin/env bash
# Simulate deploying this branch onto a WARM installation.
#
# A warm installation's ClickHouse relations were created by an earlier
# release and nothing widens them in place, so a gold model that references a
# new silver/staging column builds fine on a fresh install — where the
# relation is created by the run that first builds it — and fails with
# UNKNOWN_IDENTIFIER on every installation that already holds data, unless the
# change ships a guarded heal. Since #3561 that failure lands on the first
# connector sync after the upgrade rather than on the deploy, so this script
# reproduces both halves:
#
#   1. Install BASE_SHA: run the base tree's own deploy hook (migrations and
#      heals, with dbt pinned from the base tree's pins.env), let the base
#      tree's connectors create their bronze through destination-clickhouse,
#      and play the base's last sync — its whole buildable dbt graph. That is
#      what an existing installation's last deploy and last sync left behind;
#      the fixtures below then add the rows a connector would have synced into
#      it.
#   2. Deploy the working tree: run this branch's apply-ch-migrations.sh end
#      to end — exactly what the Helm clickhouse-migrate hook runs — and then
#      the first sync after it: this tree's whole buildable graph over the
#      warm bronze and silver, with dbt pinned from this tree's pins.env.
#
# Both phases run against the branch's CLICKHOUSE_SERVER_IMAGE: real
# installations upgrade the server before or after the app on their own
# schedule, and one lane cannot hold both positions — new-code-on-new-server
# is the pair every release must support.
#
# A step 2 failure is the forgotten-heal class: see the "Warehouse contract
# changes" section of AGENTS.md.
#
# Required env (same contract as apply-ch-migrations.sh):
#   CLICKHOUSE_URL, CLICKHOUSE_USER, CLICKHOUSE_PASSWORD, CLICKHOUSE_DATABASE
#   BASE_SHA  — commit whose tree plays the already-installed release
# Optional env, the topology (same contract as apply-ch-migrations.sh): both
# deploys, both syncs and the connectors' bronze read it, and the fixtures
# below render their engines for it. Default: a standalone single node.
#   CLICKHOUSE_CLUSTER_MODE, CLICKHOUSE_CLUSTER_NAME
# Optional env, for the destination-clickhouse container that creates bronze:
#   BRONZE_CLICKHOUSE_URL — the URL that container reaches ClickHouse at
#                           (default CLICKHOUSE_URL; 127.0.0.1 is not it)
#   DOCKER_NETWORK        — the network it joins to get there
#   RUNNER_TEMP           — where its bind-mounted inputs live (default /tmp)
#   PYTHON_BIN            — interpreter for the per-phase venvs (default: the
#                           first of python3.12, python3.11, python3)
# Needs python3 (3.11+), docker, jq, yq v4 and network access to PyPI; each
# phase gets its own venv pinned from its tree's pins.env.
set -euo pipefail

: "${BASE_SHA:?BASE_SHA must be set (the commit playing the installed release)}"
: "${CLICKHOUSE_URL:?}" "${CLICKHOUSE_USER:?}" "${CLICKHOUSE_PASSWORD:?}" "${CLICKHOUSE_DATABASE:?}"
export CLICKHOUSE_CLUSTER_MODE="${CLICKHOUSE_CLUSTER_MODE:-false}"
export CLICKHOUSE_CLUSTER_NAME="${CLICKHOUSE_CLUSTER_NAME:-}"

REPO_ROOT="$(git rev-parse --show-toplevel)"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

# The fixtures below spell their engines behind the topology markers this
# tree's hook renders (lib/ch-topology.sh): a bronze relation, in an Atomic
# database, carries the cluster clause; one in a Replicated database must not.
source "$REPO_ROOT/src/ingestion/scripts/lib/ch-topology.sh"

warm_sql() {
  ch_render_topology | run_ch
}

if ! git -C "$REPO_ROOT" cat-file -e "${BASE_SHA}^{commit}" 2>/dev/null; then
  echo "::error title=warm-upgrade base unavailable::BASE_SHA ${BASE_SHA} is not a reachable commit — not this PR's fault. On a force-pushed or newly created ref there is no before-state to upgrade from."
  exit 1
fi

# Same interpreter preference as bootstrap-db/run-dbt.sh — the pinned dbt
# does not run on newer interpreters.
PYTHON_BIN="${PYTHON_BIN:-$(command -v python3.12 || command -v python3.11 || command -v python3)}"

# One venv per phase, pinned from that phase's own pins.env — the base
# deploy ran the base toolbox, the branch deploy runs the branch toolbox.
dbt_venv_for() {
  local pins="$1" venv="$2"
  local core clickhouse
  core="$(grep '^DBT_CORE_VERSION=' "$pins" | cut -d= -f2)"
  clickhouse="$(grep '^DBT_CLICKHOUSE_VERSION=' "$pins" | cut -d= -f2)"
  "$PYTHON_BIN" -m venv "$venv"
  "$venv/bin/pip" install --quiet \
    "dbt-core==${core}" "dbt-clickhouse==${clickhouse}" pyyaml
}

# Whether a tree syncs the way the product does since #3834: the whole graph
# less the models whose bronze no connector has written. Such a tree creates
# no silver at deploy time either, so bronze has to be real for it.
syncs_the_buildable_graph() {
  [[ -f "$1/src/ingestion/dbt/macros/unbuildable_models.sql" ]]
}

# Bronze as a connector's first sync creates it — destination-clickhouse
# applying the connector's discovered catalogue — for the connectors the
# fixtures below cover, using the tree's own connectors and image pins.
create_connector_bronze() {
  local tree="$1" url="${BRONZE_CLICKHOUSE_URL:-$CLICKHOUSE_URL}" protocol hostport port
  protocol="${url%%://*}"
  hostport="${url#*://}"; hostport="${hostport%%/*}"
  if [[ "$hostport" == *:* ]]; then
    port="${hostport##*:}"
  elif [[ "$protocol" == https ]]; then
    port=443
  else
    port=80
  fi
  local pins="$tree/src/ingestion/scripts/bootstrap-db/pins.env"
  echo "=== Creating bronze for github, gitlab, jira and youtrack through destination-clickhouse ==="
  CLICKHOUSE_PROTOCOL="$protocol" \
  CLICKHOUSE_HOST="${hostport%%:*}" \
  CLICKHOUSE_PORT="$port" \
  DESTINATION_CLICKHOUSE_IMAGE="$(grep '^DESTINATION_CLICKHOUSE_IMAGE=' "$pins" | cut -d= -f2)" \
  SOURCE_DECLARATIVE_MANIFEST_IMAGE="$(grep '^SOURCE_DECLARATIVE_MANIFEST_IMAGE=' "$pins" | cut -d= -f2)" \
    bash "$tree/src/ingestion/scripts/bootstrap-db/seed-connectors.sh" \
      "$tree/src/ingestion/scripts/bootstrap-db/connectors-config.yaml" github gitlab jira youtrack
}

# A sync as the Argo dbt-run step runs it (the shape of the cluster rig's
# dbt_build): the tree's profile, the whole graph, the unbuildable models
# excluded. A tree from before #3834 built gold alone over a snapshot-created
# silver; its own build-gold.sh still plays that.
build_graph() {
  local tree="$1" venv="$2" label="$3"
  local profiles="$WORKDIR/profiles-$label" target="$WORKDIR/dbt-$label"
  local path="$venv/bin:$PATH" project="$tree/src/ingestion/dbt" unbuildable excluded=()
  if ! syncs_the_buildable_graph "$tree"; then
    PATH="$path" bash "$tree/src/ingestion/scripts/build-gold.sh"
    return
  fi
  mkdir -p "$profiles"
  PATH="$path" python3 "$tree/src/ingestion/scripts/dbt_profiles.py" --target migrate \
    --profiles-dir "$profiles" --correlated-subqueries
  unbuildable="$(cd "$project" && PATH="$path" dbt --quiet run-operation list_unbuildable_models \
    --profiles-dir "$profiles" --target-path "$target" --log-path "$target/logs")"
  read -r -a excluded <<< "$unbuildable"
  echo "=== dbt run over the buildable graph, excluding ${#excluded[@]} unbuildable model(s) ==="
  (cd "$project" && PATH="$path" dbt --no-use-colors run --profiles-dir "$profiles" \
    --target-path "$target" --log-path "$target/logs" ${excluded[@]+--exclude "${excluded[@]}"})
}

echo "=== Step 1: install base $(git -C "$REPO_ROOT" rev-parse --short "$BASE_SHA") (warm state) ==="
git -C "$REPO_ROOT" archive "$BASE_SHA" src/ingestion | tar -x -C "$WORKDIR"
dbt_venv_for "$WORKDIR/src/ingestion/scripts/bootstrap-db/pins.env" "$WORKDIR/base-venv"
if ! PATH="$WORKDIR/base-venv/bin:$PATH" \
    bash "$WORKDIR/src/ingestion/scripts/apply-ch-migrations.sh"; then
  echo "::error title=warm-upgrade base install failed::The BASE deploy (${BASE_SHA}) did not converge — not this PR's fault. Check whether the base tree's apply-ch-migrations.sh is broken on a fresh ClickHouse."
  exit 1
fi

if syncs_the_buildable_graph "$WORKDIR" && ! create_connector_bronze "$WORKDIR"; then
  echo "::error title=warm-upgrade base install failed::The BASE connectors (${BASE_SHA}) did not create their bronze — not this PR's fault. Check whether the base tree's seed-connectors.sh is broken against a fresh ClickHouse."
  exit 1
fi

# A base tree from before #3561 built gold inside the hook above; a later one
# leaves it to the sync. Either way the warm state needs the base's silver and
# gold, built under the base contract.
if [[ -f "$WORKDIR/src/ingestion/scripts/build-gold.sh" ]] &&
    ! build_graph "$WORKDIR" "$WORKDIR/base-venv" base; then
  echo "::error title=warm-upgrade base install failed::The BASE sync (${BASE_SHA}) — dbt run over its buildable graph, or build-gold.sh on an older tree — did not converge — not this PR's fault. Check whether the base tree's dbt project is broken on a fresh ClickHouse."
  exit 1
fi

# A warm installation's git pull requests were settled under the PREVIOUS close
# contract, which preferred `closed_at`: a request CLOSED, reopened and later
# MERGED therefore holds the EARLIER close in `closed_on`. The rows below are
# that state, and the assertion after step 2 is that the upgrade recovered the
# reported close from the SOURCE rather than copying the settled one — the
# difference between a 10-hour merge and a 2-hour one, on data no resync has
# touched. #3362
# The bronze tables are real above where the base syncs the buildable graph,
# and these CREATEs are no-ops on them; a base from before that built silver
# from a snapshot and gets the destination's append_dedup shape here
# (ReplacingMergeTree versioned by the extraction stamp, sorted by a
# non-nullable unique_key), with the columns the backfill reads.
seed_warm_bronze() {
  echo "=== Creating the bronze a synced warm installation would hold ==="
  warm_sql <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_github /*@@ON_CLUSTER@@*/;
CREATE DATABASE IF NOT EXISTS bronze_gitlab /*@@ON_CLUSTER@@*/;
CREATE TABLE IF NOT EXISTS bronze_github.pull_requests /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    state Nullable(String), created_at Nullable(String), closed_at Nullable(String),
    merged_at Nullable(String), _airbyte_raw_id String,
    _airbyte_extracted_at DateTime64(3), _airbyte_meta String, _airbyte_generation_id UInt32
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_requests /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    state Nullable(String), created_at Nullable(String), closed_at Nullable(String),
    merged_at Nullable(String), _airbyte_raw_id String,
    _airbyte_extracted_at DateTime64(3), _airbyte_meta String, _airbyte_generation_id UInt32
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
}

seed_git_close_time_warm_state() {
  echo "=== Seeding the pre-contract git pull-request state ==="
  warm_sql <<'SQL'
-- TWO generations of the same request, the shape bronze actually holds: the
-- first collection saw it closed, the second saw the merge. The backfill has to
-- answer from the later one, so a lookup that takes whichever row a part merge
-- left behind fails here.
INSERT INTO bronze_github.pull_requests
    (unique_key, tenant_id, source_id, state, created_at, closed_at, merged_at,
     _airbyte_raw_id, _airbyte_extracted_at, _airbyte_meta, _airbyte_generation_id)
VALUES
    ('warm-gh-reopened', 'warm', 'warm', 'closed',
     '2026-01-01T00:00:00Z', '2026-01-01T02:00:00Z', '',
     'warm-raw-1', '2026-01-01 03:00:00', '{}', 0),
    ('warm-gh-reopened', 'warm', 'warm', 'closed',
     '2026-01-01T00:00:00Z', '2026-01-01T02:00:00Z', '2026-01-01T10:00:00Z',
     'warm-raw-2', '2026-01-02 00:00:00', '{}', 0);
-- The pre-#3250 GitLab stream, which only an upgrading installation has, beside
-- the stream that replaced it. Both name the same request and disagree; the
-- current stream has to win by declaration, not by extraction time, so the
-- legacy row here is deliberately the MORE recently extracted of the two.
CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_requests /*@@ON_CLUSTER@@*/
(
    unique_key Nullable(String), tenant_id Nullable(String), source_id Nullable(String),
    state Nullable(String), created_at Nullable(String), closed_at Nullable(String),
    merged_at Nullable(String), _airbyte_raw_id String,
    _airbyte_extracted_at DateTime64(3), _airbyte_meta String, _airbyte_generation_id UInt32
)
ENGINE = /*@@REPLICATED@@*/MergeTree ORDER BY _airbyte_raw_id;
INSERT INTO bronze_gitlab.merge_requests
    (unique_key, tenant_id, source_id, state, created_at, closed_at, merged_at,
     _airbyte_raw_id, _airbyte_extracted_at, _airbyte_meta, _airbyte_generation_id)
VALUES
    ('warm-gl-reopened', 'warm', 'warm', 'closed',
     '2026-01-04T00:00:00Z', '2026-01-04T01:00:00Z', '',
     'warm-raw-legacy', '2026-01-09 00:00:00', '{}', 0);
INSERT INTO bronze_gitlab.pull_requests
    (unique_key, tenant_id, source_id, state, created_at, closed_at, merged_at,
     _airbyte_raw_id, _airbyte_extracted_at, _airbyte_meta, _airbyte_generation_id)
VALUES
    ('warm-gl-reopened', 'warm', 'warm', 'merged',
     '2026-01-04T00:00:00Z', '2026-01-04T01:00:00Z', '2026-01-04T09:00:00Z',
     'warm-raw-new', '2026-01-05 00:00:00', '{}', 0);
INSERT INTO silver.class_git_pull_requests
    (tenant_id, source_id, unique_key, pr_id, state, created_on, closed_on, data_source, _version)
VALUES
    ('warm', 'warm', 'warm-gh-reopened', 9001, 'MERGED',
     '2026-01-01 00:00:00', '2026-01-01 02:00:00', 'insight_github', 1),
    ('warm', 'warm', 'warm-gl-reopened', 9003, 'MERGED',
     '2026-01-04 00:00:00', '2026-01-04 01:00:00', 'insight_gitlab', 1),
    ('warm', 'warm', 'warm-gh-open', 9004, 'OPEN',
     '2026-01-06 00:00:00', NULL, 'insight_github', 1),
    ('warm', 'warm', 'warm-bb-recovered', 9002, 'MERGED',
     '2026-01-05 00:00:00', '2026-01-05 07:00:00', 'insight_bitbucket_cloud', 1);
SQL
}

assert_git_close_time_recovered() {
  echo "=== Asserting the reported close came from the source, not from closed_on ==="
  local got
  local expected="2026-01-01 10:00:00|2026-01-04 09:00:00|1|1"
  # ifNull around each part: concat() propagates a NULL, so an unfilled column
  # would otherwise collapse the whole answer to one '\N' and hide WHICH row
  # the rollout failed to reach.
  # FINAL because the backfill writes the correction as a newer row rather than
  # rewriting the old one in place: the answer asserted here is the one every
  # reader of this relation takes.
  got="$(printf "SELECT concat(
      ifNull(toString(maxIf(closed_on_reported, unique_key = 'warm-gh-reopened')), 'NULL'), '|',
      ifNull(toString(maxIf(closed_on_reported, unique_key = 'warm-gl-reopened')), 'NULL'), '|',
      toString(countIf(unique_key = 'warm-bb-recovered' AND closed_on_reported IS NULL)), '|',
      toString(countIf(unique_key = 'warm-gh-open' AND closed_on_reported IS NULL)))
    FROM silver.class_git_pull_requests FINAL
    WHERE tenant_id = 'warm'" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=warm upgrade left a stale close time::Expected '${expected}' — the reopened-then-merged GitHub request reporting its merge from the NEWER bronze generation, the GitLab one reporting the merge the CURRENT stream states rather than the legacy stream's close, and the Bitbucket and still-open rows reporting nothing — but got '${got}'. The rollout must recompute the reported close from bronze under the merged_at-first contract, not copy the settled closed_on — see #3362."
    exit 1
  fi
  echo "=== Reported close recovered from the source, newest generation and current stream ==="
}

# A warm installation's jira bronze was written by a connector older than 6.1.0:
# the substreams name their issue by the key it was fetched under and carry no
# `jira_id`, and the issue streams themselves are keyed by that key, so an issue
# moved between projects exists once per key it ever had. The assertion after
# step 2 is that the upgrade recovered the id — including for a key the issue no
# longer answers to — and collapsed the issue onto it. Same fixture rules as the
# git bronze above: the destination's shape, only the columns the heals read.
seed_jira_issue_identity_warm_state() {
  echo "=== Seeding the pre-6.1.0 jira bronze state ==="
  warm_sql <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_jira /*@@ON_CLUSTER@@*/;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_keys /*@@ON_CLUSTER@@*/ AS bronze_jira.jira_issue;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_history /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), items Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_comments /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), body Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_worklogs /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), issueId Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;

-- Issue 101 under both keys it has had: the current one, and the one it was
-- moved away from. Only the old-key row maps WARM-OLD-7 to an id, and it is
-- keyed by that old key — which is what the second heal rebuilds.
INSERT INTO bronze_jira.jira_issue
    (unique_key, tenant_id, source_id, id_readable, jira_id, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-WARM-1', 'warm', 'jira-1', 'WARM-1', '101', 'raw-i1', '2026-01-02 00:00:00'),
    ('warm-jira-WARM-OLD-7', 'warm', 'jira-1', 'WARM-OLD-7', '101', 'raw-i0', '2026-01-01 00:00:00');
INSERT INTO bronze_jira.jira_issue_keys
    (unique_key, tenant_id, source_id, id_readable, jira_id, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-WARM-2', 'warm', 'jira-1', 'WARM-2', '102', 'raw-k1', '2026-01-02 00:00:00');

INSERT INTO bronze_jira.jira_issue_history
    (unique_key, tenant_id, source_id, id_readable, jira_id, items, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-h-current', 'warm', 'jira-1', 'WARM-1', NULL, '[]', 'raw-h1', '2026-01-03 00:00:00'),
    ('warm-jira-h-moved', 'warm', 'jira-1', 'WARM-OLD-7', NULL, '[]', 'raw-h2', '2026-01-03 00:00:00'),
    ('warm-jira-h-unknown', 'warm', 'jira-1', 'WARM-GONE-9', NULL, '[]', 'raw-h3', '2026-01-03 00:00:00'),
    ('warm-jira-h-known', 'warm', 'jira-1', 'WARM-1', '999', '[]', 'raw-h4', '2026-01-03 00:00:00'),
    -- Extracted just now: a sync landed it minutes before the deploy, which is
    -- the window the rebuild carries rows over from. It must come out of the
    -- deploy filled like the rest, not reinstated as the connector wrote it.
    ('warm-jira-h-recent', 'warm', 'jira-1', 'WARM-1', NULL, '[]', 'raw-h5', now());
INSERT INTO bronze_jira.jira_comments
    (unique_key, tenant_id, source_id, id_readable, jira_id, body, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-c-keys', 'warm', 'jira-1', 'WARM-2', NULL, 'comment', 'raw-c1', '2026-01-03 00:00:00');
INSERT INTO bronze_jira.jira_worklogs
    (unique_key, tenant_id, source_id, id_readable, jira_id, issueId, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-w-own', 'warm', 'jira-1', 'WARM-GONE-9', NULL, '777', 'raw-w1', '2026-01-03 00:00:00'),
    ('warm-jira-w-key', 'warm', 'jira-1', 'WARM-1', NULL, NULL, 'raw-w2', '2026-01-03 00:00:00');
SQL
}

assert_jira_issue_identity_recovered() {
  echo "=== Asserting the issue id was recovered and the issue collapsed onto it ==="
  local got
  local expected="101|101|NULL|999|101|102|777|101|8|1|warm-jira-1-101"
  # The row count is part of the answer: the fill rebuilds the whole table, so a
  # dropped or doubled row is as much a failure as a wrong id. FINAL throughout,
  # because a row the rebuild filled and then took back would otherwise answer
  # twice and the filled copy would hide the one every reader takes.
  got="$(printf "SELECT concat(
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-current'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-moved'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-unknown'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-known'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-h-recent'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-c-keys'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-w-own'), 'NULL'), '|',
      ifNull(maxIf(jira_id, unique_key = 'warm-jira-w-key'), 'NULL'), '|',
      toString(count()), '|',
      toString((SELECT count() FROM bronze_jira.jira_issue)), '|',
      ifNull((SELECT anyLast(unique_key) FROM bronze_jira.jira_issue), 'NULL'))
    FROM
    (
        SELECT unique_key, jira_id FROM bronze_jira.jira_issue_history FINAL
        UNION ALL SELECT unique_key, jira_id FROM bronze_jira.jira_comments FINAL
        UNION ALL SELECT unique_key, jira_id FROM bronze_jira.jira_worklogs FINAL
    )" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=warm upgrade left jira bronze without its issue id::Expected '${expected}' — the two substream rows naming the issue by a key (current and pre-move) carrying its id, the row naming a key no stream delivered staying NULL, the row that already had an id keeping it, the comment resolving through jira_issue_keys, the worklog preferring Jira's own issueId, every row still present, and jira_issue collapsed onto one row keyed by the issue id — but got '${got}'."
    exit 1
  fi
  echo "=== Jira issue id recovered and the moved issue collapsed onto it ==="
}

# Before the catalogue first-sync record kept its history, it was a plain
# MergeTree keyed by source. Bronze has already forgotten the first extraction —
# what it holds now is younger — so the upgrade must carry the recorded instant
# over rather than re-derive it. One source still has bronze rows to name its
# tenant; the other does not, and keeps an empty one.
seed_jira_catalogue_first_seen_warm_state() {
  echo "=== Seeding the pre-history Jira catalogue first-sync record ==="
  warm_sql <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_jira /*@@ON_CLUSTER@@*/;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_fields /*@@ON_CLUSTER@@*/
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
INSERT INTO bronze_jira.jira_fields (unique_key, tenant_id, source_id, _airbyte_raw_id, _airbyte_extracted_at)
VALUES ('warm-jira-1-summary', 'warm', 'jira-1', 'raw-f1', '2026-03-01 00:00:00');

DROP TABLE IF EXISTS staging.jira__catalogue_first_seen;
CREATE TABLE staging.jira__catalogue_first_seen
(
    insight_source_id String, catalogue_first_sync DateTime64(3), recorded_at DateTime64(3)
)
ENGINE = /*@@REPLICATED@@*/MergeTree ORDER BY insight_source_id;
INSERT INTO staging.jira__catalogue_first_seen (insight_source_id, catalogue_first_sync, recorded_at)
VALUES
    ('jira-1', '2026-01-01 00:00:00', '2026-01-01 00:05:00'),
    ('jira-gone', '2026-01-02 00:00:00', '2026-01-02 00:05:00');
SQL
}

assert_jira_catalogue_first_seen_kept() {
  echo "=== Asserting the catalogue first sync survived the upgrade ==="
  local got expected
  expected="$(ch_replicated_prefix)ReplacingMergeTree|-jira-gone,,jira-gone,2026-01-02 00:00:00.000;warm-jira-1,warm,jira-1,2026-01-01 00:00:00.000"
  got="$(printf "SELECT concat(
      (SELECT engine FROM system.tables WHERE database = 'staging' AND name = 'jira__catalogue_first_seen'), '|',
      arrayStringConcat(groupArray(row), ';'))
    FROM
    (
        SELECT concat(unique_key, ',', tenant_id, ',', insight_source_id, ',', toString(catalogue_first_sync)) AS row
        FROM staging.jira__catalogue_first_seen FINAL
        ORDER BY unique_key
    )" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=warm upgrade lost the Jira catalogue first sync::Expected '${expected}' — the record rebuilt as a ReplacingMergeTree keyed by tenant and source, each source keeping the instant it was first recorded with, and the tenant taken from bronze where bronze still names one — but got '${got}'."
    exit 1
  fi
  echo "=== Jira catalogue first sync kept through the upgrade ==="
}

# The base sync built the YouTrack field observations without a tenant column.
# The upgrade adds it beside the key and reads each old row's tenant back from
# its key, against the tenants bronze holds; a row whose key names none keeps
# NULL for the tenant migration.
seed_youtrack_field_observation_warm_state() {
  echo "=== Seeding the pre-tenant YouTrack field observations ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  run_ch <<'SQL'
INSERT INTO bronze_youtrack.youtrack_custom_fields (unique_key, tenant_id, source_id, _airbyte_raw_id, _airbyte_extracted_at)
VALUES ('warm-yt-1-field-1', 'warm', 'yt-1', 'raw-y1', '2026-01-01 00:00:00');
INSERT INTO staging.youtrack__field_observations
    (unique_key, insight_source_id, field_id, field_name, field_type, observed_at, _version)
VALUES
    ('warm-yt-1-field-1-2026-01-01 00:00:00', 'yt-1', 'field-1', 'Priority', 'enum', '2026-01-01 00:00:00', 1),
    ('stray-yt-1-field-2-2026-01-01 00:00:00', 'yt-1', 'field-2', 'State', 'state', '2026-01-01 00:00:00', 1);
SQL
}

assert_youtrack_field_observation_tenants() {
  echo "=== Asserting the field observations carry their tenant ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  local got
  local expected="2|field-1:warm;field-2:NULL"
  got="$(printf "SELECT concat(
      toString((SELECT position FROM system.columns WHERE database = 'staging' AND table = 'youtrack__field_observations' AND name = 'tenant_id')), '|',
      arrayStringConcat(groupArray(concat(field_id, ':', ifNull(tenant_id, 'NULL'))), ';'))
    FROM (SELECT field_id, tenant_id FROM staging.youtrack__field_observations FINAL ORDER BY field_id)" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=warm upgrade left YouTrack field observations without a tenant::Expected '${expected}' — the tenant column right after unique_key, the row keyed by a bronze tenant carrying it, the row keyed by none left NULL — but got '${got}'."
    exit 1
  fi
  echo "=== YouTrack field observations carry their tenant ==="
}

# The base sync built the task class producers without a tenant column. A
# producer table gains it right after unique_key: String when it holds no row,
# Nullable while rows without a tenant wait for the rebuild. A producer view
# cannot be altered, so one without the column is dropped for its own
# connector's run to recreate; a redeploy after that run keeps the recreated one.
seed_task_producer_tenant_warm_state() {
  echo "=== Seeding the pre-tenant task class producers ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  run_ch <<'SQL'
INSERT INTO staging.jira__task_comments (unique_key, insight_source_id, comment_id, _version)
VALUES ('jira-1-comment-1', 'jira-1', '1', 1);
INSERT INTO staging.youtrack__task_issuetypes (unique_key, insight_source_id, issue_type_id, _version)
VALUES ('yt-1-youtrack-type-1', 'yt-1', '1', 1);
SQL
}

assert_task_producer_tenants() {
  echo "=== Asserting the task class producers carry their tenant ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  local got
  local expected="jira__task_comments:2:Nullable(String);youtrack__task_issuetypes:2:Nullable(String);youtrack__task_links:2:String|"
  got="$(printf "SELECT concat(
      (SELECT arrayStringConcat(groupArray(concat(table, ':', toString(position), ':', type)), ';')
       FROM (SELECT table, position, type FROM system.columns
             WHERE database = 'staging' AND name = 'tenant_id'
               AND table IN ('jira__task_comments', 'youtrack__task_issuetypes', 'youtrack__task_links') ORDER BY table)), '|',
      (SELECT arrayStringConcat(groupArray(name), ';')
       FROM (SELECT name FROM system.tables
             WHERE database = 'staging' AND name IN ('github__task_projects', 'jira__task_projects') ORDER BY name)))" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=warm upgrade left a task class producer without a tenant::Expected '${expected}' — tenant_id right after unique_key, String only where no row lacks a tenant, and the producer views without the column dropped — but got '${got}'."
    exit 1
  fi
  echo "=== Task class producers carry their tenant ==="
}

assert_task_producer_views_kept_on_redeploy() {
  echo "=== Redeploying over the first sync: the recreated producer views stay ==="
  if ! PATH="$WORKDIR/branch-venv/bin:$PATH" \
      bash "$REPO_ROOT/src/ingestion/scripts/apply-ch-migrations.sh"; then
    echo "::error title=a redeploy after the first sync fails::This tree's deploy hook does not converge when run again over the relations its own first sync built."
    exit 1
  fi
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  local got
  local expected="github__task_projects;jira__task_projects"
  got="$(printf "SELECT arrayStringConcat(groupArray(name), ';')
    FROM (SELECT name FROM system.tables
          WHERE database = 'staging' AND name IN ('github__task_projects', 'jira__task_projects') ORDER BY name)" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=a redeploy dropped a producer view that carries its tenant::Expected '${expected}' — every producer view the first sync recreated with tenant_id kept — but got '${got}'."
    exit 1
  fi
  echo "=== Producer views that carry their tenant are kept ==="
}

# The base sync built the task classes without a tenant column. A class gains
# it right after unique_key, where every producer's rebuilt SELECT now puts it,
# and stays Nullable while a row lacks a tenant.
seed_task_class_tenant_warm_state() {
  echo "=== Seeding a pre-tenant task class ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  run_ch <<'SQL'
INSERT INTO silver.class_task_comments (unique_key, insight_source_id, comment_id, _version)
VALUES ('jira-1-comment-1', 'jira-1', '1', 1);
SQL
}

assert_task_class_tenant() {
  echo "=== Asserting the task class carries its tenant ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  local got
  local expected="2:Nullable(String)|1"
  got="$(printf "SELECT concat(
      (SELECT concat(toString(position), ':', type) FROM system.columns
       WHERE database = 'silver' AND table = 'class_task_comments' AND name = 'tenant_id'), '|',
      toString((SELECT count() FROM silver.class_task_comments)))" | _ch_http_query | tr -d '\n')"
  if [[ "${got}" != "${expected}" ]]; then
    echo "::error title=warm upgrade left a task class without a tenant::Expected '${expected}' — a Nullable(String) tenant_id right after unique_key, never narrowed while a row lacks a tenant, and the class row kept — but got '${got}'."
    exit 1
  fi
  echo "=== Task class carries its tenant ==="
}

seed_warm_bronze
seed_git_close_time_warm_state
seed_jira_issue_identity_warm_state
seed_jira_catalogue_first_seen_warm_state
seed_youtrack_field_observation_warm_state
seed_task_producer_tenant_warm_state
seed_task_class_tenant_warm_state

echo "=== Step 2: deploy the working tree onto the warm state ==="
dbt_venv_for "$REPO_ROOT/src/ingestion/scripts/bootstrap-db/pins.env" "$WORKDIR/branch-venv"
if ! PATH="$WORKDIR/branch-venv/bin:$PATH" \
    bash "$REPO_ROOT/src/ingestion/scripts/apply-ch-migrations.sh"; then
  echo "::error title=warm upgrade fails — the deploy hook does not converge::This tree's migrations or heals fail against relations an earlier release created. Every statement in that hook must be idempotent and guarded — per the Warehouse contract changes rules in AGENTS.md."
  exit 1
fi

assert_git_close_time_recovered
assert_jira_issue_identity_recovered
assert_jira_catalogue_first_seen_kept
assert_youtrack_field_observation_tenants
assert_task_producer_tenants
assert_task_class_tenant

echo "=== Step 3: the first sync after the upgrade — this tree's buildable graph over the warm warehouse ==="
if ! build_graph "$REPO_ROOT" "$WORKDIR/branch-venv" branch; then
  echo "::error title=first sync after the upgrade fails — forgotten heal::A model reads a bronze/silver/staging column this branch adds but no heal widens an existing relation with, so the first connector sync after this release fails on every installation that already holds data. Ship the companion ALTER — a ch_table_exists-guarded heal in src/ingestion/scripts/apply-ch-migrations.sh, for silver.class_* as for staging.* — per the Warehouse contract changes rules in AGENTS.md."
  exit 1
fi

assert_task_producer_views_kept_on_redeploy

echo "=== Warm upgrade OK: every relation the first sync reads is covered by a migration or heal ==="
