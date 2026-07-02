#!/usr/bin/env bash
# Single-command wrapper for the Bronze-to-API E2E test stack.
#
# Reuses the ROOT docker-compose.yml (clickhouse + mariadb — single version
# SSOT) via the docker-compose.e2e.yml overlay, under an ISOLATED `insight-e2e`
# compose project (own volumes/containers/network — a running `./dev-compose.sh
# up` dev stack is left untouched).
#
# Three phases (see docker-compose.e2e.yml):
#   1. up       clickhouse + mariadb  → healthy
#   2. migrate  e2e-migrate           → the REAL apply-ch-migrations.sh (core DBs,
#                                        bronze placeholders, gold views)
#   3. test     e2e-runner pytest     → seed-once bronze + dbt + enrich + gold
#                                        rebind + MV refresh, then assert
#
# Examples:
#   ./e2e.sh test                          # up + migrate + full suite
#   ./e2e.sh test -k collab_emails_sent -v # one test
#   ./e2e.sh build                         # (re)build the runner image
#   ./e2e.sh up                            # just DBs + migrations (host-mode dev)
#   ./e2e.sh shell                         # interactive bash inside the runner
#   ./e2e.sh down                          # stop + wipe the e2e project

set -euo pipefail

cd "$(dirname "$0")"

# Repo root, 4 levels up from src/ingestion/tests/e2e — exported so the overlay's
# ${INSIGHT_REPO_ROOT} (bind-mount + build context) resolves.
INSIGHT_REPO_ROOT="$(cd ../../../.. && pwd)"
export INSIGHT_REPO_ROOT

PROJECT=insight-e2e
COMPOSE_FILES=(-f "$INSIGHT_REPO_ROOT/docker-compose.yml" -f "$INSIGHT_REPO_ROOT/docker-compose.e2e.yml")

# Optional extra overlays (space-separated), resolved relative to the repo root.
# CI injects docker-compose.e2e.cache.yml here to enable the gha build cache;
# locally it stays empty so builds don't require ACTIONS_* tokens.
if [ -n "${E2E_COMPOSE_OVERLAYS:-}" ]; then
    for overlay in ${E2E_COMPOSE_OVERLAYS}; do
        COMPOSE_FILES+=(-f "$INSIGHT_REPO_ROOT/$overlay")
    done
fi

# DB services live behind the root stack's local-* profiles; e2e services behind
# `e2e`. Naming services explicitly on `up`/`run` keeps the dev stack's no-profile
# services (redis, redpanda, backends) out of scope.
PROFILES=(--profile local-clickhouse --profile local-mariadb --profile e2e)

# Per-host credentials for the e2e stack. Written once and reused so a warm
# ClickHouse volume (re-running `test` without `down`) keeps a matching password.
ENV_FILE="$INSIGHT_REPO_ROOT/.env.e2e"
if [ ! -f "$ENV_FILE" ]; then
    cat <<EOF > "$ENV_FILE"
# Auto-generated per-host credentials for the E2E stack. NOT committed.
CLICKHOUSE_DB=insight
CLICKHOUSE_USER=insight
CLICKHOUSE_PASSWORD=$(openssl rand -hex 12)
MARIADB_USER=insight
MARIADB_PASSWORD=$(openssl rand -hex 12)
MARIADB_ROOT_PASSWORD=$(openssl rand -hex 12)
EOF
    echo "wrote $ENV_FILE (random per-host credentials)"
fi

dc() {
    docker compose --project-directory "$INSIGHT_REPO_ROOT" --env-file "$ENV_FILE" \
        -p "$PROJECT" "${COMPOSE_FILES[@]}" "${PROFILES[@]}" "$@"
}

# The runner image is shared by e2e-runner AND e2e-migrate (which has no build:
# of its own), so it must exist AND match the current Dockerfile before either
# runs. `docker compose build` is a fast layer-cache check when nothing changed
# and — crucially — rebuilds when the Dockerfile/sources moved (a plain
# "is the image present?" test would silently keep a stale image, e.g. one built
# before curl was added). CI primes the layer cache in a dedicated build step, so
# this stays fast there too.
ensure_built() { dc build e2e-runner; }

up_dbs()      { dc up -d --wait clickhouse mariadb; }   # phase 1
migrate_step() { dc run --rm e2e-migrate; }             # phase 2 (assumes built)

cmd=${1:-test}
shift || true

case "$cmd" in
    build)
        dc build e2e-runner
        ;;
    test|run)
        ensure_built
        up_dbs
        migrate_step
        dc run --rm e2e-runner pytest "$@"   # phase 3
        ;;
    up)
        ensure_built
        up_dbs
        migrate_step
        echo "e2e stack up + migrated. Run tests with: ./e2e.sh test"
        ;;
    migrate)
        ensure_built
        up_dbs
        migrate_step
        ;;
    shell)
        ensure_built
        dc run --rm e2e-runner bash
        ;;
    down)
        dc down -v --remove-orphans
        ;;
    logs)
        dc logs --tail=200 "$@"
        ;;
    gates)
        # Analyse the catalog a prior `./e2e.sh test` collected — pure file
        # analysis inside the runner image (--no-deps: no DB, no second compose).
        if [ ! -f .artifacts/catalog_metrics.json ]; then
            echo "no .artifacts/catalog_metrics.json — run './e2e.sh test' first (it collects the catalog)" >&2
            exit 2
        fi
        ensure_built
        dc run --rm --no-deps -T e2e-runner \
            python3 lib/metric_coverage.py --universe-file .artifacts/catalog_metrics.json
        ;;
    *)
        echo "usage: $0 {build|test|run|up|migrate|shell|down|logs|gates} [args...]" >&2
        exit 2
        ;;
esac
