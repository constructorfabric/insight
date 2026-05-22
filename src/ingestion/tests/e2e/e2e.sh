#!/usr/bin/env bash
# Single-command wrapper for the Bronze-to-API E2E test framework.
#
# Examples:
#   ./e2e.sh test                       # full suite
#   ./e2e.sh test -k people_smoke -v    # one fixture
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
    new)
        # Scaffold a new fixture folder.
        # Usage: ./e2e.sh new <fixture_name> [<bronze_schema>.<table>]
        name="${1:-}"
        if [ -z "$name" ]; then
            echo "usage: $0 new <fixture_name> [<bronze_schema>.<table>]" >&2
            exit 2
        fi
        dir="fixtures/$name"
        if [ -e "$dir" ]; then
            echo "error: $dir already exists" >&2
            exit 1
        fi
        bronze_tbl="${2:-bronze_bamboohr.employees}"
        mkdir -p "$dir/bronze" "$dir/expected"

        cat > "$dir/spec.yaml" <<EOF
spec_version: 1
description: >
  TODO describe what this fixture exercises.

# Look up the UUID from analytics-api seed_metrics migrations OR add a
# custom metric to ../../seed/metrics.yaml and reference it here.
metric_id: REPLACE-WITH-UUID
endpoint: /v1/metrics/{metric_id}/query
method: POST

request_body:
  \$top: 50

# Optional: dbt selector. Omit for view-only metrics that read directly
# from bronze (e.g. \`insight.people\`).
# dbt_selector: +silver_class_<x>+

key_columns:
  - REPLACE_WITH_COLUMN
EOF

        cat > "$dir/bronze/$bronze_tbl.csv" <<EOF
# TODO: replace with real CSV. First row = column names; empty cell = SQL NULL.
# See system.columns for the target table schema:
#   ./e2e.sh shell
#   clickhouse-client --host clickhouse -u insight --password "\$E2E_CH_PASSWORD" \\
#     --query "DESCRIBE $bronze_tbl"
EOF
        # `.csv` with a leading comment is invalid pandas input — rename so
        # fixture-loader doesn't pick this up half-baked. Author removes
        # the `.todo` suffix once the CSV is real.
        mv "$dir/bronze/$bronze_tbl.csv" "$dir/bronze/$bronze_tbl.csv.todo"

        echo "scaffolded $dir/"
        echo ""
        echo "next steps:"
        echo "  1. edit $dir/spec.yaml (set metric_id + key_columns)"
        echo "  2. write $dir/bronze/$bronze_tbl.csv (and any other bronze inputs)"
        echo "     remove the .todo suffix once the CSV is real"
        echo "  3. generate expected/response.csv:  ./e2e.sh test -k $name --update-snapshots"
        echo "  4. inspect the generated expected/response.csv, then commit"
        ;;
    *)
        echo "usage: $0 {build|test|run|shell|up|down|logs|new} [args...]" >&2
        exit 2
        ;;
esac
