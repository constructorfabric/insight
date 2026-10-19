#!/usr/bin/env bash
# Simulate deploying this branch onto a WARM installation.
#
# A warm installation's ClickHouse relations were created by an earlier
# release: `CREATE TABLE IF NOT EXISTS` never widens them, so a gold model
# that references a new silver/staging column builds fine on a fresh install
# (the snapshot already carries the new shape) and fails with
# UNKNOWN_IDENTIFIER on every installation that already holds data — unless
# the change ships a numbered migration or a guarded heal. Since #3561 that
# failure lands on the first connector sync after the upgrade rather than on
# the deploy, so this script reproduces both halves:
#
#   1. Install BASE_SHA: run the base tree's own deploy hook
#      (create-warehouse-placeholders + migrations + heals, with dbt pinned
#      from the base tree's pins.env) — what an existing installation's last
#      deploy left behind, plus the bronze a connector would have synced into
#      it (seed_warm_bronze below).
#   2. Deploy the working tree: run this branch's apply-ch-migrations.sh end
#      to end — exactly what the Helm clickhouse-migrate hook runs — and then
#      its build-gold.sh over the warm silver, with dbt pinned from this tree's
#      pins.env. That is the gold half of the first sync, not a whole one: a
#      real sync rebuilds staging and silver in the same dbt run, so a silver
#      model that widens itself through `on_schema_change` widens there and not
#      here. The lane is therefore stricter than the field, never laxer.
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
# Needs python3 (3.11+) and network access to PyPI; each phase gets its own
# venv pinned from its tree's pins.env.
set -euo pipefail

: "${BASE_SHA:?BASE_SHA must be set (the commit playing the installed release)}"
: "${CLICKHOUSE_URL:?}" "${CLICKHOUSE_USER:?}" "${CLICKHOUSE_PASSWORD:?}" "${CLICKHOUSE_DATABASE:?}"

REPO_ROOT="$(git rev-parse --show-toplevel)"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

if ! git -C "$REPO_ROOT" cat-file -e "${BASE_SHA}^{commit}" 2>/dev/null; then
  echo "::error title=warm-upgrade base unavailable::BASE_SHA ${BASE_SHA} is not a reachable commit — not this PR's fault. On a force-pushed or newly created ref there is no before-state to upgrade from."
  exit 1
fi

# Same interpreter preference as bootstrap-db/run-dbt.sh — the pinned dbt
# does not run on newer interpreters.
PYTHON_BIN="$(command -v python3.12 || command -v python3.11 || command -v python3)"

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

echo "=== Step 1: install base $(git -C "$REPO_ROOT" rev-parse --short "$BASE_SHA") (warm state) ==="
git -C "$REPO_ROOT" archive "$BASE_SHA" src/ingestion | tar -x -C "$WORKDIR"
dbt_venv_for "$WORKDIR/src/ingestion/scripts/bootstrap-db/pins.env" "$WORKDIR/base-venv"
if ! PATH="$WORKDIR/base-venv/bin:$PATH" \
    bash "$WORKDIR/src/ingestion/scripts/apply-ch-migrations.sh"; then
  echo "::error title=warm-upgrade base install failed::The BASE deploy (${BASE_SHA}) did not converge — not this PR's fault. Check whether the base tree's apply-ch-migrations.sh is broken on a fresh ClickHouse."
  exit 1
fi

# A base tree from before #3561 built gold inside the hook above; a later one
# leaves it to the sync. Either way the warm state needs the base's gold.
if [[ -f "$WORKDIR/src/ingestion/scripts/build-gold.sh" ]] &&
    ! PATH="$WORKDIR/base-venv/bin:$PATH" \
      bash "$WORKDIR/src/ingestion/scripts/build-gold.sh"; then
  echo "::error title=warm-upgrade base install failed::The BASE gold build (${BASE_SHA}) did not converge — not this PR's fault. Check whether the base tree's build-gold.sh is broken on a fresh ClickHouse."
  exit 1
fi

# A warm installation's git pull requests were settled under the PREVIOUS close
# contract, which preferred `closed_at`: a request CLOSED, reopened and later
# MERGED therefore holds the EARLIER close in `closed_on`. The rows below are
# that state, and the assertion after step 2 is that the upgrade recovered the
# reported close from the SOURCE rather than copying the settled one — the
# difference between a 10-hour merge and a 2-hour one, on data no resync has
# touched. #3362
# No deploy creates bronze — destination-clickhouse does, on a connector's first
# sync — so the warm state a connector would have left is a fixture here. Shape
# follows the destination's append_dedup output (ReplacingMergeTree versioned by
# the extraction stamp, sorted by a non-nullable unique_key); columns are the
# ones the backfill reads, not the connector's full stream.
seed_warm_bronze() {
  echo "=== Creating the bronze a synced warm installation would hold ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  run_ch <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_github;
CREATE DATABASE IF NOT EXISTS bronze_gitlab;
CREATE TABLE IF NOT EXISTS bronze_github.pull_requests
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    state Nullable(String), created_at Nullable(String), closed_at Nullable(String),
    merged_at Nullable(String), _airbyte_raw_id String,
    _airbyte_extracted_at DateTime64(3), _airbyte_meta String, _airbyte_generation_id UInt32
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_requests
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    state Nullable(String), created_at Nullable(String), closed_at Nullable(String),
    merged_at Nullable(String), _airbyte_raw_id String,
    _airbyte_extracted_at DateTime64(3), _airbyte_meta String, _airbyte_generation_id UInt32
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
SQL
}

seed_git_close_time_warm_state() {
  echo "=== Seeding the pre-contract git pull-request state ==="
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  run_ch <<'SQL'
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
CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_requests
(
    unique_key Nullable(String), tenant_id Nullable(String), source_id Nullable(String),
    state Nullable(String), created_at Nullable(String), closed_at Nullable(String),
    merged_at Nullable(String), _airbyte_raw_id String,
    _airbyte_extracted_at DateTime64(3), _airbyte_meta String, _airbyte_generation_id UInt32
)
ENGINE = MergeTree ORDER BY _airbyte_raw_id;
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
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
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
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
  run_ch <<'SQL'
CREATE DATABASE IF NOT EXISTS bronze_jira;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), summary Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_keys AS bronze_jira.jira_issue;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_history
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), items Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_comments
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), body Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;
CREATE TABLE IF NOT EXISTS bronze_jira.jira_worklogs
(
    unique_key String, tenant_id Nullable(String), source_id Nullable(String),
    id_readable Nullable(String), jira_id Nullable(String), issueId Nullable(String),
    _airbyte_raw_id String, _airbyte_extracted_at DateTime64(3)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at) ORDER BY unique_key;

-- Issue 101 under both keys it has had: the current one, and the one it was
-- moved away from. Only the old-key row maps WARM-OLD-7 to an id, and it is
-- keyed by that old key — which is what the second heal rebuilds.
INSERT INTO bronze_jira.jira_issue
    (unique_key, tenant_id, source_id, id_readable, jira_id, summary, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-WARM-1', 'warm', 'jira-1', 'WARM-1', '101', 'moved issue', 'raw-i1', '2026-01-02 00:00:00'),
    ('warm-jira-WARM-OLD-7', 'warm', 'jira-1', 'WARM-OLD-7', '101', 'moved issue', 'raw-i0', '2026-01-01 00:00:00');
INSERT INTO bronze_jira.jira_issue_keys
    (unique_key, tenant_id, source_id, id_readable, jira_id, summary, _airbyte_raw_id, _airbyte_extracted_at)
VALUES
    ('warm-jira-WARM-2', 'warm', 'jira-1', 'WARM-2', '102', 'other issue', 'raw-k1', '2026-01-02 00:00:00');

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
  source "$REPO_ROOT/src/ingestion/scripts/lib/ch-exec.sh"
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

seed_warm_bronze
seed_git_close_time_warm_state
seed_jira_issue_identity_warm_state

echo "=== Step 2: deploy the working tree onto the warm state ==="
dbt_venv_for "$REPO_ROOT/src/ingestion/scripts/bootstrap-db/pins.env" "$WORKDIR/branch-venv"
if ! PATH="$WORKDIR/branch-venv/bin:$PATH" \
    bash "$REPO_ROOT/src/ingestion/scripts/apply-ch-migrations.sh"; then
  echo "::error title=warm upgrade fails — the deploy hook does not converge::This tree's migrations or heals fail against relations an earlier release created. Every statement in that hook must be idempotent and guarded — per the Warehouse contract changes rules in AGENTS.md."
  exit 1
fi

assert_git_close_time_recovered
assert_jira_issue_identity_recovered

echo "=== Step 3: build gold over the warm silver, as the first sync would ==="
if ! PATH="$WORKDIR/branch-venv/bin:$PATH" \
    bash "$REPO_ROOT/src/ingestion/scripts/build-gold.sh"; then
  echo "::error title=first sync after the upgrade fails — forgotten migration or heal::A gold model reads a silver/staging column that only exists in the new snapshot, so the first connector sync after this release fails on every installation that already holds data. Ship the companion ALTER — a numbered migration in src/ingestion/scripts/migrations/ for silver.class_*, a guarded heal in src/ingestion/scripts/apply-ch-migrations.sh for staging.* — per the Warehouse contract changes rules in AGENTS.md."
  exit 1
fi

echo "=== Warm upgrade OK: every relation the gold build reads is covered by a migration or heal ==="
