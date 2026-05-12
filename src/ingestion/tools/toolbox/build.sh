#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INGESTION_DIR="$SCRIPT_DIR/../.."
IMAGE_NAME="${TOOLBOX_IMAGE:-insight-toolbox:local}"

PUSH=0
PLATFORM=""
for arg in "$@"; do
  case "$arg" in
    --push)     PUSH=1 ;;
    --platform=*) PLATFORM="${arg#*=}" ;;
    -h|--help)
      cat <<USAGE
Usage: $0 [--push] [--platform=<linux/amd64|linux/arm64>]

Env:
  TOOLBOX_IMAGE   image name/tag (default: insight-toolbox:local)

Examples:
  $0
  TOOLBOX_IMAGE=ghcr.io/cyberfabric/insight-toolbox:latest $0 --push --platform=linux/amd64
USAGE
      exit 0 ;;
  esac
done

echo "Building ${IMAGE_NAME}..."
if [[ -n "$PLATFORM" ]]; then
  # buildx for cross-arch builds (e.g. amd64 image from Apple Silicon)
  if [[ "$PUSH" -eq 1 ]]; then
    docker buildx build --platform "$PLATFORM" -t "$IMAGE_NAME" \
      -f "$SCRIPT_DIR/Dockerfile" "$INGESTION_DIR" --push
    echo "Pushed: ${IMAGE_NAME} (${PLATFORM})"
    exit 0
  else
    docker buildx build --platform "$PLATFORM" -t "$IMAGE_NAME" \
      -f "$SCRIPT_DIR/Dockerfile" "$INGESTION_DIR" --load
  fi
else
  docker build -t "$IMAGE_NAME" -f "$SCRIPT_DIR/Dockerfile" "$INGESTION_DIR"
fi

# Load into Kind cluster if running locally
if command -v kind &>/dev/null && kind get clusters 2>/dev/null | grep -q "^insight$"; then
  echo "Loading into Kind cluster..."
  kind load docker-image "$IMAGE_NAME" --name insight
fi

if [[ "$PUSH" -eq 1 ]]; then
  echo "Pushing ${IMAGE_NAME}..."
  docker push "$IMAGE_NAME"
fi

echo "Done: ${IMAGE_NAME}"
