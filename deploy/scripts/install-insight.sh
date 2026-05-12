#!/usr/bin/env bash
#
# Install/upgrade the Insight umbrella chart.
#
# Assumes Airbyte and Argo Workflows are already installed (ingestion
# services will not work otherwise). Run AFTER install-airbyte.sh and
# install-argo.sh, or alongside them.
#
# Environment overrides:
#   INSIGHT_NAMESPACE    (default: insight)
#   INSIGHT_RELEASE      (default: insight)
#   INSIGHT_VERSION      (default: auto — read from Chart.yaml)
#   INSIGHT_VALUES       single extra -f values.yaml (back-compat)
#   INSIGHT_VALUES_FILES colon-separated list of -f values files, applied in order
#   CHART_SOURCE         local | oci   (default: local — path to charts/insight)
#   OCI_REF              OCI reference for the chart (default: oci://ghcr.io/cyberfabric/charts/insight)
#
# Usage:
#   ./deploy/scripts/install-insight.sh
#   INSIGHT_VALUES=deploy/prod-values.yaml ./deploy/scripts/install-insight.sh
#   CHART_SOURCE=oci INSIGHT_VERSION=0.2.0 ./deploy/scripts/install-insight.sh
#
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

NAMESPACE="${INSIGHT_NAMESPACE:-insight}"
RELEASE="${INSIGHT_RELEASE:-insight}"
CHART_SOURCE="${CHART_SOURCE:-local}"
OCI_REF="${OCI_REF:-oci://ghcr.io/cyberfabric/charts/insight}"
EXTRA_VALUES="${INSIGHT_VALUES:-}"
EXTRA_VALUES_FILES="${INSIGHT_VALUES_FILES:-}"

log() { printf '\033[36m[install-insight]\033[0m %s\n' "$*"; }
die() { printf '\033[31m[install-insight] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ─── Resolve chart reference ──────────────────────────────────────────
case "$CHART_SOURCE" in
  local)
    CHART_REF="./charts/insight"
    [[ -f "$CHART_REF/Chart.yaml" ]] || die "local chart not found: $CHART_REF"
    log "Ensuring subchart dependencies"
    helm dependency update "$CHART_REF" >/dev/null
    # Auto-detect version if not set. Prefer `helm show chart` over
    # `grep ^version: + awk` so a stray top-level `version:` block
    # (migration notes, comments, schema docs) doesn't trip us up.
    VERSION="${INSIGHT_VERSION:-$(helm show chart "$CHART_REF" | awk '/^version:/ {print $2; exit}')}"
    VERSION_ARG=()
    ;;
  oci)
    [[ -n "${INSIGHT_VERSION:-}" ]] || die "INSIGHT_VERSION required for CHART_SOURCE=oci"
    VERSION="$INSIGHT_VERSION"
    CHART_REF="$OCI_REF"
    VERSION_ARG=(--version "$VERSION")
    ;;
  *)
    die "unknown CHART_SOURCE: $CHART_SOURCE (expected: local | oci)"
    ;;
esac

# ─── Prerequisites ─────────────────────────────────────────────────────
command -v helm    >/dev/null || die "helm not found"
command -v kubectl >/dev/null || die "kubectl not found"

log "Cluster: $(kubectl config current-context)"
log "Namespace: $NAMESPACE · Release: $RELEASE · Chart: $CHART_REF@$VERSION"

# ─── Pre-flight: dependencies detected in the namespace ───────────────
# Single-namespace model — every dependency Insight needs lives in the
# same namespace. Missing services are warnings, not errors: the umbrella
# chart still installs, but runtime behaviour depends on what is present.
_check_svc() {
  local label="$1" svc="$2" hint="$3"
  if kubectl -n "$NAMESPACE" get svc "$svc" >/dev/null 2>&1; then
    log "Found: $label ($svc)"
  else
    log "WARNING: $label not detected in '$NAMESPACE' ns — $hint"
  fi
}

_check_svc "Airbyte"       "airbyte-airbyte-server-svc" \
  "ingestion workflows will fail. Run: INSIGHT_NAMESPACE=$NAMESPACE ./deploy/scripts/install-airbyte.sh"
_check_svc "Argo Workflows" "argo-workflows-server" \
  "CronWorkflows won't be reconciled. Run: INSIGHT_NAMESPACE=$NAMESPACE ./deploy/scripts/install-argo.sh"

# If the user targets a FRESH cluster (no CH / MariaDB / Redis yet) and
# has `clickhouse.deploy=true` (the default), the umbrella installs
# those itself. If they are set to `deploy: false`, a warning here
# catches missing external dependencies BEFORE helm upgrade runs.
for dep in "$RELEASE-clickhouse" "$RELEASE-mariadb" "$RELEASE-redis-master"; do
  if ! kubectl -n "$NAMESPACE" get svc "$dep" >/dev/null 2>&1; then
    log "Note: $dep not present — umbrella will provision it (if <dep>.deploy=true)."
  fi
done

# Argo CRD guard. The umbrella's `ingestion.templates.enabled: true`
# default emits `WorkflowTemplate` resources. Disable that emission when
# either:
#   • SKIP_ARGO=1 (operator declared intent — even if CRDs persist from
#     a prior partial install, we honour the skip instead of silently
#     emitting WorkflowTemplates that no controller is going to
#     reconcile — issue #275 B6); OR
#   • the `workflowtemplates.argoproj.io` CRD is absent (helm render
#     would fail with `no matches for kind "WorkflowTemplate"`).
ARGO_CRD_DISABLE_ARGS=()
if [[ "${SKIP_ARGO:-0}" == "1" ]]; then
  log "SKIP_ARGO=1 → auto-disabling ingestion.templates.enabled"
  ARGO_CRD_DISABLE_ARGS=(--set ingestion.templates.enabled=false)
elif ! kubectl get crd workflowtemplates.argoproj.io >/dev/null 2>&1; then
  log "WARNING: workflowtemplates.argoproj.io CRD missing — auto-disabling ingestion.templates.enabled"
  log "         Run install-argo.sh to register CRDs, then re-run install-insight.sh to enable templates."
  ARGO_CRD_DISABLE_ARGS=(--set ingestion.templates.enabled=false)
fi

# ─── Install / upgrade ─────────────────────────────────────────────────
VALUES_ARGS=()
if [[ -n "$EXTRA_VALUES_FILES" ]]; then
  # Colon-separated list — apply in order so later files override earlier.
  IFS=':' read -ra _FILES <<< "$EXTRA_VALUES_FILES"
  for _f in "${_FILES[@]}"; do
    [[ -f "$_f" ]] || die "values file not found: $_f"
    VALUES_ARGS+=(-f "$_f")
  done
fi
[[ -n "$EXTRA_VALUES" ]] && VALUES_ARGS+=(-f "$EXTRA_VALUES")

# HELM_EXTRA_ARGS: caller-supplied passthrough (e.g. --set flags).
# Split on whitespace — caller is responsible for not embedding
# whitespace inside individual arg values. To pass quoted arguments
# safely, set HELM_EXTRA_ARGS_FILE to a path with one arg per line:
#   HELM_EXTRA_ARGS_FILE=/tmp/args ./install-insight.sh
EXTRA_ARGS=()
if [[ -n "${HELM_EXTRA_ARGS_FILE:-}" ]]; then
  [[ -f "$HELM_EXTRA_ARGS_FILE" ]] || die "HELM_EXTRA_ARGS_FILE not found: $HELM_EXTRA_ARGS_FILE"
  while IFS= read -r line; do
    [[ -z "$line" || "$line" =~ ^[[:space:]]*# ]] && continue
    EXTRA_ARGS+=("$line")
  done < "$HELM_EXTRA_ARGS_FILE"
elif [[ -n "${HELM_EXTRA_ARGS:-}" ]]; then
  # shellcheck disable=SC2206
  EXTRA_ARGS=($HELM_EXTRA_ARGS)
fi

log "Running helm upgrade --install"
# `${array[@]+"${array[@]}"}` (the "expand only if set" idiom) lets us
# pass these optional arrays under `set -u` without the macOS-default
# bash 3.2 firing `unbound variable` on empty `"${array[@]}"`. Bash 4.4+
# (Linux, brew bash) treats `"${empty[@]}"` as zero words and would not
# need this dance, but the canonical installer must work on Apple
# Silicon's stock /bin/bash (= 3.2).
helm upgrade --install "$RELEASE" "$CHART_REF" \
  --namespace "$NAMESPACE" --create-namespace \
  ${VERSION_ARG[@]+"${VERSION_ARG[@]}"} \
  ${VALUES_ARGS[@]+"${VALUES_ARGS[@]}"} \
  ${ARGO_CRD_DISABLE_ARGS[@]+"${ARGO_CRD_DISABLE_ARGS[@]}"} \
  ${EXTRA_ARGS[@]+"${EXTRA_ARGS[@]}"} \
  --wait --timeout 10m

# ─── Summary ───────────────────────────────────────────────────────────
cat <<EOF

✓ Insight installed.

Verify:
  kubectl -n $NAMESPACE rollout status deploy --timeout=5m

Access:
  kubectl -n $NAMESPACE port-forward svc/$RELEASE-frontend 8080:80
  # then open http://localhost:8080

EOF
