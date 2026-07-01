#!/usr/bin/env bash
# Single-command wrapper for the Bronze-to-API E2E test framework.
#
# Examples:
#   ./e2e.sh test                       # full suite
#   ./e2e.sh test -k collab_emails_sent -v  # one test
#   ./e2e.sh shell                      # interactive bash inside the runner
#   ./e2e.sh build                      # rebuild the runner image
#   ./e2e.sh down                       # stop containers, clear volumes
#
# The runner image bakes in python+rust+deps so no host setup is required
# beyond Docker. See compose/Dockerfile.runner.

set -euo pipefail

cd "$(dirname "$0")"

# Resolve repo root once and export it so compose can use it for the runner's
# build context (which sits 4 levels up from compose/).
INSIGHT_REPO_ROOT="$(cd ../../../.. && pwd)"
export INSIGHT_REPO_ROOT

COMPOSE_FILES=(-f compose/docker-compose.yml -f compose/docker-compose.runner.yml)

# Optional extra compose overlays, space-separated, resolved relative to this
# script's dir. CI injects compose/docker-compose.cache.yml here to enable the
# gha build cache; locally it stays empty so builds don't require ACTIONS_*.
if [ -n "${E2E_COMPOSE_OVERLAYS:-}" ]; then
    for overlay in ${E2E_COMPOSE_OVERLAYS}; do
        COMPOSE_FILES+=(-f "$overlay")
    done
fi

ENV_FILE=compose/.env

# Generate a .env if one is not present — every session needs a password.
if [ ! -f "$ENV_FILE" ]; then
    cat <<EOF > "$ENV_FILE"
CLICKHOUSE_DB=insight
CLICKHOUSE_USER=insight
CLICKHOUSE_PASSWORD=$(openssl rand -hex 12)
MARIADB_DATABASE=analytics
MARIADB_USER=insight
MARIADB_PASSWORD=$(openssl rand -hex 12)
MARIADB_ROOT_PASSWORD=$(openssl rand -hex 12)
EOF
    echo "wrote $ENV_FILE (random per-host credentials)"
fi

cmd=${1:-test}
shift || true

case "$cmd" in
    build)
        # Builds the runner image; its `additional_contexts` pull each connector's
        # enrich binary from that connector's own build-only service (compiled FROM
        # ITS OWN Dockerfile) and bake it in via COPY --from. No docker-in-docker.
        docker compose "${COMPOSE_FILES[@]}" build runner
        ;;
    test|run)
        # `--rm` removes the runner container on exit; clickhouse + mariadb keep
        # running so a follow-up `test` invocation is fast (no re-init).
        docker compose "${COMPOSE_FILES[@]}" run --rm runner pytest "$@"
        ;;
    shell)
        docker compose "${COMPOSE_FILES[@]}" run --rm runner bash
        ;;
    up)
        # Bring up CH+MariaDB without launching the runner — useful when
        # iterating on tests from outside Docker.
        docker compose "${COMPOSE_FILES[@]}" up -d clickhouse mariadb
        ;;
    down)
        docker compose "${COMPOSE_FILES[@]}" down -v
        ;;
    logs)
        docker compose "${COMPOSE_FILES[@]}" logs --tail=200 "$@"
        ;;
    *)
        echo "usage: $0 {build|test|run|shell|up|down|logs} [args...]" >&2
        exit 2
        ;;
esac
