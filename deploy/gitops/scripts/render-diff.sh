#!/usr/bin/env bash
#
# render-diff.sh — show the engineer (a) the poller commits since the last
# deploy and (b) the diff between the previous render and the current
# render of the chart.
#
# Called by `make diff ENV=…`. Read-only.
#
# Args (positional, all required):
#   $1  ENV
#   $2  NAMESPACE
#   $3  RELEASE
#   $4  CHART (OCI ref, e.g. oci://ghcr.io/constructorfabric/charts/insight)
#   $5  VALUES file
#   $6  RENDER_DIR (typically .deploy)
#   $7  INSIGHT_VERSION (umbrella semver from .insight-version)

set -euo pipefail

ENV="$1"
NAMESPACE="$2"
RELEASE="$3"
CHART="$4"
VALUES="$5"
RENDER_DIR="$6"
INSIGHT_VERSION="${7:-}"

mkdir -p "$RENDER_DIR"
prev="$RENDER_DIR/last-render-${ENV}.yaml"
new="$RENDER_DIR/render-${ENV}-$(date -u +%Y%m%d-%H%M%S).yaml"

echo "=== commits since last deploy ==="
last_tag="$(git tag --list 'deploy-*' --sort=-creatordate | head -n1 || true)"
if [ -n "$last_tag" ]; then
  git log --oneline "${last_tag}..HEAD" -- "$VALUES" "environments/${ENV}/" || true
else
  echo "(no deploy-* tag found; showing the last 10 commits touching this env)"
  git log --oneline -10 -- "$VALUES" "environments/${ENV}/" || true
fi

echo
echo "=== rendering chart ==="
if [ -n "$INSIGHT_VERSION" ]; then
  helm template "$RELEASE" "$CHART" --version "$INSIGHT_VERSION" \
    -n "$NAMESPACE" -f "$VALUES" > "$new"
  echo "rendered $CHART:$INSIGHT_VERSION to $new"
else
  helm template "$RELEASE" "$CHART" -n "$NAMESPACE" -f "$VALUES" > "$new"
  echo "rendered to $new"
fi

if [ -f "$prev" ]; then
  echo
  echo "=== diff vs last render ==="
  if diff -u "$prev" "$new"; then
    echo "(no manifest changes since last render)"
  fi
else
  echo
  echo "(no previous render to diff against; this would be the first deploy)"
fi
