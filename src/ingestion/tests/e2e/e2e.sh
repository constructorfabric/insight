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
        # Builds the runner image; its `service:` additional_contexts pull each
        # component binary from that component's own build-only service
        # (compiled FROM ITS OWN Dockerfile). No docker-in-docker.
        docker compose "${COMPOSE_FILES[@]}" build runner
        ;;
    test|run)
        # `--rm` removes the runner container on exit; clickhouse + mariadb keep
        # running so a follow-up `test` invocation is fast (no re-init).
        # The norebuild overlay strips the runner's build section — compose
        # v2.36-desktop chokes on resolving `service:` build contexts during
        # `run`, and no build is wanted here anyway.
        docker compose "${COMPOSE_FILES[@]}" -f compose/docker-compose.norebuild.yml run --rm runner pytest "$@"
        ;;
    shell)
        docker compose "${COMPOSE_FILES[@]}" -f compose/docker-compose.norebuild.yml run --rm runner bash
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
    gates)
        # Run the metric coverage gate against the inputs a prior
        # `./e2e.sh test metrics/` collected into .artifacts/ — pure file
        # analysis inside the runner image (no DB via --no-deps, no second
        # compose). A `-k` subset run under-fills the ledger and will fail it.
        #
        # The api and identity endpoint gates used to live here too. They
        # retired with the HTTP contract suites they measured: those contracts
        # are asserted against a deployed stand now, and their gate is
        # `tests/lib/insight_stand/coverage.py`, run by `e2e-stand.yml` and by
        # `./dev-compose.sh test-stand test`.
        which=${1:-all}
        case "$which" in
            all|metrics) ;;
            *)
                echo "usage: $0 gates [metrics]" >&2
                echo "  (the api/identity endpoint gates moved to tests/stand — see" >&2
                echo "   tests/lib/insight_stand/coverage.py)" >&2
                exit 2
                ;;
        esac
        if [ ! -f .artifacts/metric_definitions.json ]; then
            echo "no .artifacts/metric_definitions.json — run './e2e.sh test metrics/' first" >&2
            exit 2
        fi
        run=(docker compose "${COMPOSE_FILES[@]}" -f compose/docker-compose.norebuild.yml run --rm --no-deps -T runner)
        rc=0
        echo "── metric coverage (gate) ──"
        "${run[@]}" python3 lib/metric_coverage.py --universe-file .artifacts/metric_definitions.json || rc=1
        exit "$rc"
        ;;
    *)
        echo "usage: $0 {build|test|run|shell|up|down|logs|gates [metrics]} [args...]" >&2
        exit 2
        ;;
esac
