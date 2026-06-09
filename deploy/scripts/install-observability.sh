#!/usr/bin/env bash
#
# Install/upgrade the BUNDLED observability stack (LGTM, logs first):
#   Loki (store) → Alloy (DaemonSet collector) → Grafana (UI).
#
# This is the reference recipe for `observability.mode: bundled`. Installs
# of Insight that bring their OWN observability skip this entirely and set
# the umbrella's `observability.mode: external` (point services at the
# customer's OTLP endpoint) or `none` (host cluster scrapes stdout).
#
# Services ALWAYS emit structured JSON to stdout regardless — that is the
# product contract; this stack only collects it.
#
# All Insight components share a single namespace by default (see
# deploy/README.md). The cluster (gitops) path runs the same charts from
# infra/insight-gitops:system/{loki,alloy,grafana} into insight-infra.
#
# Version source of truth: infra/insight-gitops Makefile pins
# (LOKI_VERSION / ALLOY_VERSION / GRAFANA_VERSION). The defaults below are
# PLACEHOLDERS — verify with `helm search repo grafana/<chart> --versions`.
#
# Environment overrides:
#   INSIGHT_NAMESPACE  (default: insight) — shared by all components
#   LOKI_RELEASE       (default: loki)
#   ALLOY_RELEASE      (default: alloy)
#   GRAFANA_RELEASE    (default: grafana)
#   LOKI_VERSION       (default: 6.30.1)
#   ALLOY_VERSION      (default: 1.4.0)
#   GRAFANA_VERSION    (default: 10.0.0)
#   *_VALUES           (default: deploy/observability/<svc>-values.yaml)
#
# Usage:
#   ./deploy/scripts/install-observability.sh
#
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

NAMESPACE="${INSIGHT_NAMESPACE:-insight}"

LOKI_RELEASE="${LOKI_RELEASE:-loki}"
ALLOY_RELEASE="${ALLOY_RELEASE:-alloy}"
GRAFANA_RELEASE="${GRAFANA_RELEASE:-grafana}"

LOKI_VERSION="${LOKI_VERSION:-6.30.1}"
ALLOY_VERSION="${ALLOY_VERSION:-1.4.0}"
GRAFANA_VERSION="${GRAFANA_VERSION:-10.0.0}"

LOKI_VALUES="${LOKI_VALUES:-deploy/observability/loki-values.yaml}"
ALLOY_VALUES="${ALLOY_VALUES:-deploy/observability/alloy-values.yaml}"
GRAFANA_VALUES="${GRAFANA_VALUES:-deploy/observability/grafana-values.yaml}"

log() { printf '\033[36m[install-observability]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[install-observability] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ─── Prerequisites ─────────────────────────────────────────────────────
command -v helm    >/dev/null || die "helm not found"
command -v kubectl >/dev/null || die "kubectl not found"
for f in "$LOKI_VALUES" "$ALLOY_VALUES" "$GRAFANA_VALUES"; do
  [[ -f "$f" ]] || die "values file not found: $f"
done

log "Cluster: $(kubectl config current-context)"
log "Namespace: $NAMESPACE"
log "Charts: grafana/loki@$LOKI_VERSION · grafana/alloy@$ALLOY_VERSION · grafana/grafana@$GRAFANA_VERSION"

# ─── Repo ──────────────────────────────────────────────────────────────
if ! helm repo list 2>/dev/null | grep -q '^grafana\s'; then
  log "Adding grafana helm repo"
  helm repo add grafana https://grafana.github.io/helm-charts
fi
helm repo update grafana >/dev/null

# ─── Pre-create namespace ──────────────────────────────────────────────
kubectl create namespace "$NAMESPACE" --dry-run=client -o yaml | kubectl apply -f - >/dev/null

# ${NAMESPACE} substitution for the values that bake an in-cluster endpoint
# (Alloy's Loki push URL, Grafana's Loki datasource). Same sed pattern as
# deploy/argo/rbac.yaml — avoids requiring envsubst.
render() { sed -e "s|\${NAMESPACE}|$NAMESPACE|g" "$1"; }
ALLOY_RENDERED="$(mktemp)"; GRAFANA_RENDERED="$(mktemp)"
trap 'rm -f "$ALLOY_RENDERED" "$GRAFANA_RENDERED"' EXIT
render "$ALLOY_VALUES"   > "$ALLOY_RENDERED"
render "$GRAFANA_VALUES" > "$GRAFANA_RENDERED"

# ─── Install / upgrade (order matters: Loki first) ─────────────────────
log "Installing Loki ($LOKI_RELEASE)"
helm upgrade --install "$LOKI_RELEASE" grafana/loki \
  --namespace "$NAMESPACE" --create-namespace \
  --version "$LOKI_VERSION" \
  -f "$LOKI_VALUES" \
  --wait --timeout 10m

log "Installing Alloy ($ALLOY_RELEASE)"
helm upgrade --install "$ALLOY_RELEASE" grafana/alloy \
  --namespace "$NAMESPACE" --create-namespace \
  --version "$ALLOY_VERSION" \
  -f "$ALLOY_RENDERED" \
  --wait --timeout 5m

log "Installing Grafana ($GRAFANA_RELEASE)"
helm upgrade --install "$GRAFANA_RELEASE" grafana/grafana \
  --namespace "$NAMESPACE" --create-namespace \
  --version "$GRAFANA_VERSION" \
  -f "$GRAFANA_RENDERED" \
  --wait --timeout 5m

# ─── Summary ───────────────────────────────────────────────────────────
cat <<EOF

✓ Observability stack (Loki + Alloy + Grafana) installed in $NAMESPACE.

Open Grafana:
  kubectl -n $NAMESPACE port-forward svc/$GRAFANA_RELEASE 3000:80
  # then http://localhost:3000  (admin pw:)
  kubectl -n $NAMESPACE get secret $GRAFANA_RELEASE -o jsonpath='{.data.admin-password}' | base64 -d; echo

First logs (Explore → Loki):
  {namespace="airbyte"}                 # raw Airbyte sync logs
  {component=~".*dbt.*"}                 # dbt run output
  {component="reconcile-loop"}           # reconcile ticks

Point Insight services at this stack — set in the umbrella values:
  observability:
    mode: bundled
    otlp:
      endpoint: http://$ALLOY_RELEASE.$NAMESPACE.svc.cluster.local:4317

EOF
