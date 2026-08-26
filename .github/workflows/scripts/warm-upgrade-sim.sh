#!/usr/bin/env bash
# Simulate deploying this branch onto a WARM installation.
#
# A warm installation's ClickHouse relations were created by an earlier
# release: `CREATE TABLE IF NOT EXISTS` never widens them, and the deploy
# hook builds only `tag:gold`, so a gold model that references a new
# silver/staging column deploys fine on a fresh install (the snapshot
# already carries the new shape) and fails with UNKNOWN_IDENTIFIER on every
# installation that already holds data — unless the change ships a numbered
# migration or a guarded heal. This script reproduces that warm deploy:
#
#   1. Install BASE_SHA: run the base tree's own deploy hook
#      (create-bronze-placeholders + migrations + heals + base gold build,
#      with dbt pinned from the base tree's pins.env) — what an existing
#      installation's last deploy left behind.
#   2. Deploy the working tree: run this branch's apply-ch-migrations.sh
#      end to end with dbt pinned from this tree's pins.env — exactly what
#      the Helm clickhouse-migrate hook runs.
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

echo "=== Step 2: deploy the working tree onto the warm state ==="
dbt_venv_for "$REPO_ROOT/src/ingestion/scripts/bootstrap-db/pins.env" "$WORKDIR/branch-venv"
if ! PATH="$WORKDIR/branch-venv/bin:$PATH" \
    bash "$REPO_ROOT/src/ingestion/scripts/apply-ch-migrations.sh"; then
  echo "::error title=warm upgrade fails — forgotten migration or heal::This tree's deploy hook fails against relations an earlier release created. A gold model reads a silver/staging column that only exists in the new snapshot: ship the companion ALTER — a numbered migration in src/ingestion/scripts/migrations/ for silver.class_*, a guarded heal in src/ingestion/scripts/apply-ch-migrations.sh for staging.* — per the Warehouse contract changes rules in AGENTS.md."
  exit 1
fi

echo "=== Warm upgrade OK: every relation the gold build reads is covered by a migration or heal ==="
