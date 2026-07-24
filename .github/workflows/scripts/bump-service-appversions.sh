#!/usr/bin/env bash
# Bump each backend service subchart's appVersion to $BUILD_TAG when that
# service's image was (re)built this run: its paths-filter flag is 'true', or
# this is a manual full rebuild (FULL_REBUILD=true).
#
# Shared by the bump-descriptors and publish-chart jobs so both pin the exact
# same set of services in the same way. A service that wasn't built is left
# untouched — it keeps its previous appVersion, which still resolves to an
# image that exists.
#
# Env: BUILD_TAG (required), FULL_REBUILD (default false), and one flag per
# service (ANALYTICS/AUTHENTICATOR/GATEWAY/IDENTITY) carrying the paths-filter
# output ('true' when that service changed).
set -euo pipefail

: "${BUILD_TAG:?BUILD_TAG is required}"
FULL_REBUILD="${FULL_REBUILD:-false}"

# service-flag-env : subchart Chart.yaml
services=(
  "ANALYTICS:src/backend/services/analytics/helm/Chart.yaml"
  "AUTHENTICATOR:src/backend/services/authenticator/helm/Chart.yaml"
  "GATEWAY:src/backend/services/gateway/helm/Chart.yaml"
  "IDENTITY:src/backend/services/identity/helm/Chart.yaml"
)

for entry in "${services[@]}"; do
  flag_name="${entry%%:*}"
  chart="${entry#*:}"
  flag_val="${!flag_name:-false}"
  if [ "$flag_val" = "true" ] || [ "$FULL_REBUILD" = "true" ]; then
    echo "Bumping $chart appVersion -> $BUILD_TAG"
    yq -i ".appVersion = \"$BUILD_TAG\"" "$chart"
  fi
done
