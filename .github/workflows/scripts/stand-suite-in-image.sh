#!/usr/bin/env bash
set -euo pipefail

: "${UI_TESTS_IMAGE:?is required}"
: "${GITHUB_WORKSPACE:?is required}"
: "${INSIGHT_STAND_BASE_URL:?is required}"
: "${INSIGHT_STAND_PERSONA_PASSWORD:?is required}"
: "${INSIGHT_STAND_MANIFEST:?is required}"
: "${INSIGHT_STAND_ARTIFACT_DIR:?is required}"
if [[ ! -f "$INSIGHT_STAND_MANIFEST" ]]; then
  echo '::error::stand manifest was not found; check INSIGHT_STAND_MANIFEST' >&2
  exit 1
fi

mkdir -p "$INSIGHT_STAND_ARTIFACT_DIR" "$GITHUB_WORKSPACE/test-results"

run_args=(
  --pull never
  --rm
  --init
  --ipc=host
  --network host
  --user "$(id -u):$(id -g)"
  -e INSIGHT_STAND_BASE_URL
  -e INSIGHT_STAND_PERSONA_PASSWORD
  -v "$INSIGHT_STAND_MANIFEST:/stand/manifest.json:ro"
  -e INSIGHT_STAND_MANIFEST=/stand/manifest.json
  -v "$GITHUB_WORKSPACE/tests:/workspace/tests:ro"
  -v "$INSIGHT_STAND_ARTIFACT_DIR:/stand/artifacts"
  -e INSIGHT_STAND_ARTIFACT_DIR=/stand/artifacts
  -v "$GITHUB_WORKSPACE/test-results:/workspace/test-results"
  -w /workspace
  -e HOME=/tmp
  -e XDG_CACHE_HOME=/tmp/.cache
  -e UV_PYTHON_INSTALL_DIR=/tmp/uv-python
  -e UV_PROJECT_ENVIRONMENT=/tmp/stand-tests
  -e PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
)

realm=deploy/compose/keycloak/realm-insight.generated.json
if [[ -f "$GITHUB_WORKSPACE/$realm" ]]; then
  run_args+=(-v "$GITHUB_WORKSPACE/$realm:/workspace/$realm:ro")
fi

exec docker run "${run_args[@]}" "$UI_TESTS_IMAGE" sh -ceu '
  python -m pip install --user --no-cache-dir "uv==0.12.0"
  export PATH="$HOME/.local/bin:$PATH"
  uv sync --project /workspace/tests --frozen --no-dev --no-install-project
  export PYTHONPATH="/workspace/tests/lib${PYTHONPATH:+:$PYTHONPATH}"
  uv run --project /workspace/tests --no-sync pytest "$@"
' sh "$@"
