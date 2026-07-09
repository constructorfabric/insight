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
    gates)
        # Run the coverage gate(s) against the inputs a prior `./e2e.sh test`
        # collected into .artifacts/ — pure file analysis inside the runner
        # image (no DB via --no-deps, no second compose). Each suite's
        # contract coverage is self-contained now: the api/ contract suite
        # alone exercises every spec operation, so `./e2e.sh test api/`
        # followed by `./e2e.sh gates api` is a complete endpoint-gate loop,
        # and `./e2e.sh test metrics/` + `./e2e.sh gates metrics` likewise for
        # the metric gate. A `-k` subset run within either suite still
        # under-fills that suite's ledger and will fail its gate. CI mirrors
        # this split as two independent lanes (e2e-api / e2e-metrics), each
        # uploading its own artifact and feeding only its own gate job (see
        # e2e-bronze-to-api.yml).
        which=${1:-all}
        case "$which" in
            all|metrics|api) ;;
            *)
                echo "usage: $0 gates [api|metrics]" >&2
                exit 2
                ;;
        esac
        if [ "$which" = all ] || [ "$which" = metrics ]; then
            if [ ! -f .artifacts/catalog_metrics.json ]; then
                echo "no .artifacts/catalog_metrics.json — run './e2e.sh test metrics/' first (it collects the metric catalog)" >&2
                exit 2
            fi
        fi
        if [ "$which" = all ] || [ "$which" = api ]; then
            if [ ! -f .artifacts/observed_endpoints.json ]; then
                echo "no .artifacts/observed_endpoints.json — run './e2e.sh test api/' first (it collects the endpoint ledger)" >&2
                exit 2
            fi
        fi
        spec=/workspace/docs/components/backend/analytics/openapi.json
        run=(docker compose "${COMPOSE_FILES[@]}" run --rm --no-deps -T runner)
        rc=0
        if [ "$which" = all ] || [ "$which" = metrics ]; then
            echo "── metric coverage (gate) ──"
            "${run[@]}" python3 lib/metric_coverage.py --universe-file .artifacts/catalog_metrics.json || rc=1
        fi
        if [ "$which" = all ] || [ "$which" = api ]; then
            echo "── api endpoint coverage (gate) ──"
            "${run[@]}" python3 lib/api_coverage.py --observed .artifacts/observed_endpoints.json --spec "$spec" || rc=1
        fi
        exit "$rc"
        ;;
    *)
        echo "usage: $0 {build|test|run|shell|up|down|logs|gates [api|metrics]} [args...]" >&2
        exit 2
        ;;
esac
