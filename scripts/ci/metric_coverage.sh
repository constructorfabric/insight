#!/usr/bin/env bash
# Standalone metric-coverage gate (NOT part of the e2e pytest suite).
#
# Boots MariaDB + analytics-api (scripts/ci/compose.metric-coverage.yml), lets
# analytics-api's migrations seed the `metrics` catalog, then reads the universe
# from GET /v1/metrics and runs the coverage diff in
# src/ingestion/tests/e2e/lib/metric_coverage.py (URL mode — needs only
# pyyaml + httpx on the host). Exit code = gate result.
#
# Env knobs:
#   ANALYTICS_API_IMAGE  use this prebuilt image instead of building (e.g. a
#                        digest published by build-images.yml). Default: build
#                        insight-analytics-api:coverage from src/backend.
#   BUILD_CACHE_FROM     buildx --cache-from spec when building (CI passes
#                        type=gha,scope=analytics-api-amd64 to reuse build-images'
#                        layer cache for a fast incremental compile).
#   ANALYTICS_API_PORT   host port for analytics-api (default 18081).
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

PORT="${ANALYTICS_API_PORT:-18081}"
TENANT="00000000-0000-0000-0000-000000000001"
PROJECT="insight-metric-coverage"
COMPOSE=(docker compose -f scripts/ci/compose.metric-coverage.yml -p "$PROJECT")

# Ephemeral per-run credentials (the DB is torn down at exit).
export MARIADB_ROOT_PASSWORD="$(openssl rand -hex 12)"
export MARIADB_PASSWORD="$(openssl rand -hex 12)"
export ANALYTICS_API_PORT="$PORT"

# Build the analytics-api image from the CURRENT source unless one was provided
# (so the universe reflects the PR's migrations, not a stale published image).
if [ -z "${ANALYTICS_API_IMAGE:-}" ]; then
    export ANALYTICS_API_IMAGE="insight-analytics-api:coverage"
    echo "::group::build analytics-api ($ANALYTICS_API_IMAGE)"
    build=(docker buildx build --load -t "$ANALYTICS_API_IMAGE"
        -f src/backend/services/analytics-api/Dockerfile src/backend)
    [ -n "${BUILD_CACHE_FROM:-}" ] && build+=(--cache-from "$BUILD_CACHE_FROM")
    "${build[@]}"
    echo "::endgroup::"
else
    echo "using provided analytics-api image: $ANALYTICS_API_IMAGE"
fi

cleanup() { "${COMPOSE[@]}" down -v --remove-orphans >/dev/null 2>&1 || true; }
trap cleanup EXIT

echo "::group::start MariaDB + analytics-api"
"${COMPOSE[@]}" up -d
echo "::endgroup::"

echo "waiting for analytics-api /health on :$PORT ..."
ok=
for _ in $(seq 1 90); do
    if curl -fsS -H "X-Insight-Tenant-Id: $TENANT" "http://localhost:${PORT}/health" >/dev/null 2>&1; then
        ok=1
        break
    fi
    sleep 2
done
if [ -z "$ok" ]; then
    echo "analytics-api did not become healthy" >&2
    "${COMPOSE[@]}" logs analytics-api | tail -80 >&2
    exit 1
fi

# URL mode: hit GET /v1/metrics for the universe. Run the module as a plain file
# so it does NOT import the `lib` package (keeps host deps to pyyaml + httpx).
export ANALYTICS_API_URL="http://localhost:${PORT}"
export ANALYTICS_TENANT_ID="$TENANT"
report="$(mktemp)"
set +e
python3 src/ingestion/tests/e2e/lib/metric_coverage.py --md | tee "$report"
rc=${PIPESTATUS[0]}
set -e

# Surface the table in the PR's job summary.
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    cat "$report" >> "$GITHUB_STEP_SUMMARY"
fi

if [ "$rc" -ne 0 ]; then
    echo "::error::metric coverage gate failed — a metric is untested and not in SKIP_LIST (or the skip list is stale). See the report above."
fi
exit "$rc"
