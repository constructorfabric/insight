#!/usr/bin/env bash
#
# Install/upgrade Argo Workflows as a standalone Helm release.
#
# Installs the engine for ingestion pipelines.
# WorkflowTemplates (airbyte-sync/dbt-run/ingestion-pipeline) are shipped
# separately via the Insight umbrella chart — see values.ingestion.templates.enabled.
#
# All Insight components share a single namespace (see deploy/README.md).
#
# Environment overrides:
#   INSIGHT_NAMESPACE  (default: insight) — shared by all components
#   ARGO_RELEASE       (default: argo-workflows)
#   ARGO_VERSION       (default: 0.45.16)
#   ARGO_VALUES        (default: deploy/argo/values.yaml)
#   ARGO_RBAC          (default: deploy/argo/rbac.yaml)
#   EXTRA_VALUES_FILE  additional -f file (for prod overrides)
#
# Usage:
#   ./deploy/scripts/install-argo.sh
#   EXTRA_VALUES_FILE=deploy/argo/values-prod.yaml ./deploy/scripts/install-argo.sh
#
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

NAMESPACE="${INSIGHT_NAMESPACE:-insight}"
RELEASE="${ARGO_RELEASE:-argo-workflows}"
VERSION="${ARGO_VERSION:-0.45.16}"
VALUES="${ARGO_VALUES:-deploy/argo/values.yaml}"
RBAC="${ARGO_RBAC:-deploy/argo/rbac.yaml}"
EXTRA="${EXTRA_VALUES_FILE:-}"

log() { printf '\033[36m[install-argo]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[install-argo] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ─── Prerequisites ─────────────────────────────────────────────────────
command -v helm    >/dev/null || die "helm not found"
command -v kubectl >/dev/null || die "kubectl not found"
[[ -f "$VALUES" ]]                || die "values file not found: $VALUES"
[[ -f "$RBAC" ]]                  || die "rbac file not found: $RBAC"
[[ -z "$EXTRA" || -f "$EXTRA" ]]  || die "extra values file not found: $EXTRA"

log "Cluster: $(kubectl config current-context)"
log "Namespace: $NAMESPACE · Release: $RELEASE · Chart: argo/argo-workflows@$VERSION"

# ─── Repo ──────────────────────────────────────────────────────────────
if ! helm repo list 2>/dev/null | grep -q '^argo\s'; then
  log "Adding argo helm repo"
  helm repo add argo https://argoproj.github.io/argo-helm
fi
helm repo update argo >/dev/null

# ─── Pre-create namespace ──────────────────────────────────────────────
kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# ─── Install / upgrade ─────────────────────────────────────────────────
VALUES_ARGS=(-f "$VALUES")
# Automatic dev overlay: dev-up.sh sets DEV_MODE=1 to get auth-off UI.
if [[ "${DEV_MODE:-0}" == "1" && -f "deploy/argo/values-dev.yaml" ]]; then
  VALUES_ARGS+=(-f "deploy/argo/values-dev.yaml")
  log "DEV_MODE=1 — merging deploy/argo/values-dev.yaml (auth-mode=server)"
fi
[[ -n "$EXTRA" ]] && VALUES_ARGS+=(-f "$EXTRA")

log "Running helm upgrade --install"
# `controller.workflowNamespaces` and `controller.instanceID` are set via
# --set so they track the release namespace. The argo-workflows chart
# expects instanceID to be a sub-object (enabled + explicitID), not a
# plain string — hence the dotted keys below.
helm upgrade --install "$RELEASE" argo/argo-workflows \
  --namespace "$NAMESPACE" --create-namespace \
  --version "$VERSION" \
  "${VALUES_ARGS[@]}" \
  --set "controller.workflowNamespaces[0]=$NAMESPACE" \
  --set "controller.instanceID.enabled=true" \
  --set "controller.instanceID.explicitID=$RELEASE-$NAMESPACE" \
  --wait --timeout 5m

# ─── Apply supplemental RBAC ───────────────────────────────────────────
# rbac.yaml ships `${NAMESPACE}` / `${WORKFLOW_SA}` placeholders; we
# substitute them with sed to target the release namespace without
# requiring envsubst to be installed.
log "Applying supplemental RBAC in $NAMESPACE"
sed \
  -e "s|\${NAMESPACE}|$NAMESPACE|g" \
  -e "s|\${WORKFLOW_SA}|argo-workflow|g" \
  "$RBAC" | kubectl apply -f -

# ─── Summary ───────────────────────────────────────────────────────────
cat <<EOF

✓ Argo Workflows installed.

Verify:
  kubectl -n $NAMESPACE get pods
  kubectl -n $NAMESPACE port-forward svc/$RELEASE-server 2746:2746
  # then open http://localhost:2746

Insight WorkflowTemplates will be deployed by the umbrella chart
(ingestion.templates.enabled=true) into the $NAMESPACE namespace.

Next step:
  INSIGHT_NAMESPACE=$NAMESPACE ./deploy/scripts/install-insight.sh

EOF
