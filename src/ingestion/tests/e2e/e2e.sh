#!/usr/bin/env bash
# Single-command wrapper for the Bronze-to-API E2E test framework.
#
# The data tier (ClickHouse + MariaDB) comes from the repo-root
# docker-compose.yml — the e2e runner ATTACHES to it on the `insight` network.
# If you already have the dev stack up (`./dev-compose.sh up`) the runner reuses
# it; otherwise the runner brings up clickhouse + mariadb as dependencies.
# Either way the runner builds + spawns its own analytics-api from current
# source.
#
# Examples:
#   ./e2e.sh test                           # full suite
#   ./e2e.sh test -k collab_emails_sent -v  # one test
#   ./e2e.sh shell                          # interactive bash inside the runner
#   ./e2e.sh build                          # rebuild the runner image
#   ./e2e.sh up                             # bring up just CH+MariaDB (host-mode dev)
#   ./e2e.sh down                           # remove the runner (data tier left intact)
#
# The runner image bakes in python+rust+deps so no host setup is required
# beyond Docker. See compose/Dockerfile.runner.

set -euo pipefail

cd "$(dirname "$0")"

# Repo root — exported so the runner override can use it for the build context
# and the /workspace bind-mount.
INSIGHT_REPO_ROOT="$(cd ../../../.. && pwd)"
export INSIGHT_REPO_ROOT

ROOT_COMPOSE="$INSIGHT_REPO_ROOT/docker-compose.yml"
RUNNER_OVERRIDE="compose/docker-compose.runner.yml"

# Credentials + ports come from a committed, test-specific env file (decoupled
# from a developer's personal .env.compose). Its defaults match the root compose
# defaults, so the runner attaches to a default `./dev-compose.sh up` and a
# fresh CI bring-up alike. Override with E2E_ENV_FILE (e.g. point it at your own
# .env.compose if your dev stack uses custom credentials).
ENV_FILE="${E2E_ENV_FILE:-compose/e2e.env}"

# The e2e data tier always uses the LOCAL clickhouse + mariadb (never the
# *_EXTERNAL profiles), so enable both profiles regardless of .env.compose.
COMPOSE=(docker compose
    --env-file "$ENV_FILE"
    -f "$ROOT_COMPOSE"
    -f "$RUNNER_OVERRIDE"
    --profile local-clickhouse
    --profile local-mariadb)

cmd=${1:-test}
shift || true

case "$cmd" in
    build)
        "${COMPOSE[@]}" build runner
        ;;
    test|run)
        # `--rm` removes the runner container on exit; clickhouse + mariadb keep
        # running so a follow-up invocation is fast (no re-init) and a dev stack
        # is left untouched.
        "${COMPOSE[@]}" run --rm runner pytest "$@"
        ;;
    shell)
        "${COMPOSE[@]}" run --rm runner bash
        ;;
    up)
        # Bring up CH+MariaDB only (not the backend/frontend services) — useful
        # when iterating on tests from the host (E2E_RUN_MODE=host).
        "${COMPOSE[@]}" up -d clickhouse mariadb
        ;;
    down)
        # Attach mode: remove ONLY the runner. The data tier belongs to the
        # root stack — tear it down with `./dev-compose.sh down` when you want.
        "${COMPOSE[@]}" rm -sf runner
        ;;
    logs)
        "${COMPOSE[@]}" logs --tail=200 "$@"
        ;;
    *)
        echo "usage: $0 {build|test|run|shell|up|down|logs} [args...]" >&2
        exit 2
        ;;
esac
