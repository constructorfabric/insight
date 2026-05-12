#!/usr/bin/env bash
set -euo pipefail

# Run only the Silver transformations for Jira on bronze data that's already
# in ClickHouse (no Airbyte sync). Steps:
#   1. dbt run --select tag:jira              — staging models
#   2. tt-enrich-jira-run                     — Rust binary writes task_field_history
#   3. dbt run --select tag:silver,tag:jira+  — silver models downstream of jira (class_task_*)
#
# All ingestion infrastructure parameters (toolbox_image, jira_enrich_image,
# clickhouse_host/port/user, batch_size) come from WorkflowTemplate defaults —
# see charts/insight/templates/ingestion/{dbt-run,tt-enrich-jira-run}.yaml.
#
# Required env:
#   KUBECONFIG          path to the insight cluster kubeconfig
#   INSIGHT_NAMESPACE   release namespace of the umbrella chart
#
# Required args:
#   <tenant>            tenant identifier
# Optional args:
#   <insight_source_id> when set, used directly; otherwise resolved from the
#                       Jira Secret annotations.

: "${KUBECONFIG:?must be set, e.g. export KUBECONFIG=~/.kube/insight.kubeconfig}"
: "${INSIGHT_NAMESPACE:?must be set to the umbrella release namespace, e.g. export INSIGHT_NAMESPACE=insight}"
export KUBECONFIG INSIGHT_NAMESPACE

TENANT="${1:?Usage: $0 <tenant> [<insight_source_id>]}"
SOURCE_ID="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

source "reconcile-connectors/lib/secrets.sh"

# ─── Resolve insight_source_id from Secret annotations ──────────────────
if [[ -z "$SOURCE_ID" ]]; then
  SOURCE_ID=$(resolve_source_id "jira" "$TENANT")
fi
[[ -n "$SOURCE_ID" ]] || {
  echo "ERROR: could not resolve insight_source_id for jira tenant '$TENANT'." >&2
  echo "       Either pass it explicitly as the second argument, or annotate the Jira Secret with all three:" >&2
  echo "         insight.cyberfabric.com/connector=jira" >&2
  echo "         insight.cyberfabric.com/tenant=$TENANT" >&2
  echo "         insight.cyberfabric.com/source-id=<id>" >&2
  exit 1
}

TENANT_DASHED="${TENANT//_/-}"

echo "Running Jira tt-enrich (staging-jira -> enrich -> silver):"
echo "  namespace:         $INSIGHT_NAMESPACE"
echo "  tenant:            $TENANT"
echo "  insight_source_id: $SOURCE_ID"

NAMESPACE="$INSIGHT_NAMESPACE" \
  TENANT="$TENANT" \
  TENANT_DASHED="$TENANT_DASHED" \
  SOURCE_ID="$SOURCE_ID" \
  envsubst < workflows/onetime/tt-enrich-jira.yaml.tpl |
  kubectl create -n "$INSIGHT_NAMESPACE" -f -

echo
echo "Monitor:"
echo "  kubectl -n $INSIGHT_NAMESPACE get workflows -l connector=jira,workflow-kind=tt-enrich --watch"
