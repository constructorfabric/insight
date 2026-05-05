#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# Build a CDK connector: Docker image → push to registry → Airbyte definition
# register/update.
#
# Usage:
#   ./airbyte-toolkit/build-connector.sh <connector_path>
#   ./airbyte-toolkit/build-connector.sh git/bitbucket-cloud
#   IMAGE_TAG=v0.2.0 ./airbyte-toolkit/build-connector.sh git/bitbucket-cloud --push
#
# Env:
#   IMAGE_TAG        image tag (default: local)
#   IMAGE_REGISTRY   registry prefix (e.g. ghcr.io/cyberfabric); empty = local-only
#
# For nocode connectors (connector.yaml), use airbyte-toolkit/register.sh instead.
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

source ./airbyte-toolkit/lib/state.sh

CONNECTOR="${1:?Usage: $0 <connector_path>  (e.g. git/github)}"
CONNECTOR_DIR="./connectors/${CONNECTOR}"
DESCRIPTOR="${CONNECTOR_DIR}/descriptor.yaml"
DOCKERFILE="${CONNECTOR_DIR}/Dockerfile"

# --- Validate ---
[[ -f "$DESCRIPTOR" ]] || { echo "ERROR: no descriptor at ${DESCRIPTOR}" >&2; exit 1; }
[[ -f "$DOCKERFILE" ]] || { echo "ERROR: no Dockerfile at ${DOCKERFILE}" >&2; exit 1; }

CONNECTOR_NAME=$(yq -r '.name' "$DESCRIPTOR")
CONNECTOR_TYPE=$(yq -r '.type' "$DESCRIPTOR")

if [[ "$CONNECTOR_TYPE" != "cdk" ]]; then
  echo "ERROR: ${CONNECTOR_NAME} is type '${CONNECTOR_TYPE}', not 'cdk'. Use airbyte-toolkit/register.sh for nocode connectors." >&2
  exit 1
fi

IMAGE_BASE="source-${CONNECTOR_NAME}-insight"
IMAGE_TAG="${IMAGE_TAG:-local}"
IMAGE_REGISTRY="${IMAGE_REGISTRY:-}"

PUSH=0
for arg in "${@:2}"; do
  case "$arg" in
    --push) PUSH=1 ;;
  esac
done

if [[ -n "$IMAGE_REGISTRY" ]]; then
  IMAGE="${IMAGE_REGISTRY}/${IMAGE_BASE}:${IMAGE_TAG}"
else
  IMAGE="${IMAGE_BASE}:${IMAGE_TAG}"
fi

echo "=== Building CDK connector: ${CONNECTOR_NAME} ==="
echo "  Image: ${IMAGE}"

# --- Step 1: Build Docker image ---
echo "  Building Docker image..."
docker build -t "$IMAGE" -f "$DOCKERFILE" "$CONNECTOR_DIR"

# --- Step 2: Push to registry or load into Kind ---
CLUSTER_NAME="${CLUSTER_NAME:-insight}"
if [[ "$PUSH" -eq 1 ]]; then
  echo "  Pushing to registry..."
  docker push "$IMAGE"
elif command -v kind &>/dev/null && kind get clusters 2>/dev/null | grep -q "^${CLUSTER_NAME}$"; then
  echo "  Loading into Kind cluster '${CLUSTER_NAME}' (local dev)..."
  kind load docker-image "$IMAGE" --name "$CLUSTER_NAME"
fi

# --- Step 3: Register/update Airbyte source definition ---
if [[ -z "${AIRBYTE_TOKEN:-}" ]]; then
  source ./airbyte-toolkit/lib/env.sh
fi

EXISTING_DEF_ID=$(state_get "definitions.${CONNECTOR_NAME}.id")

# For Airbyte registration, use the full image path (with registry) if available
DOCKER_REPO="${IMAGE_REGISTRY:+${IMAGE_REGISTRY}/}${IMAGE_BASE}"

DEF_ID=$(python3 - "$AIRBYTE_API" "$AIRBYTE_TOKEN" "$WORKSPACE_ID" \
  "$CONNECTOR_NAME" "$DOCKER_REPO" "$IMAGE_TAG" "$EXISTING_DEF_ID" <<'PYTHON'
import sys, json, urllib.request, urllib.error

airbyte_url, token, workspace_id, name, docker_repo, docker_tag, existing_def_id = sys.argv[1:8]
headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}

def api(path, data):
    body = json.dumps(data).encode()
    req = urllib.request.Request(f"{airbyte_url}{path}", data=body, headers=headers, method="POST")
    try:
        return json.loads(urllib.request.urlopen(req).read())
    except urllib.error.HTTPError as e:
        print(f"API {e.code}: {e.read().decode()[:200]}", file=sys.stderr)
        return None

# Try update if definition exists
if existing_def_id:
    result = api("/api/v1/source_definitions/update", {
        "sourceDefinitionId": existing_def_id,
        "dockerImageTag": docker_tag,
    })
    if result and "sourceDefinitionId" in result:
        print(existing_def_id, end="")
        print(f"  Definition updated: {existing_def_id}", file=sys.stderr)
        sys.exit(0)
    print("  Update failed, creating new definition...", file=sys.stderr)

# Create new custom definition
result = api("/api/v1/source_definitions/create_custom", {
    "workspaceId": workspace_id,
    "sourceDefinition": {
        "name": name,
        "dockerRepository": docker_repo,
        "dockerImageTag": docker_tag,
        "documentationUrl": f"https://docs.example.com/connectors/{name}",
    }
})
if result and "sourceDefinitionId" in result:
    def_id = result["sourceDefinitionId"]
    print(def_id, end="")
    print(f"  Definition created: {def_id}", file=sys.stderr)
else:
    print("ERROR: could not register definition", file=sys.stderr)
    sys.exit(1)
PYTHON
)

if [[ -n "$DEF_ID" ]]; then
  state_set "definitions.${CONNECTOR_NAME}.id" "$DEF_ID"
  echo "  State saved: definitions.${CONNECTOR_NAME}.id = ${DEF_ID}"
fi

echo ""
echo "=== Done: ${CONNECTOR_NAME} ==="
echo "  Image:      ${IMAGE}"
echo "  Definition: ${DEF_ID:-unknown}"
echo ""
echo "  Next: ./airbyte-toolkit/connect.sh <tenant>"
