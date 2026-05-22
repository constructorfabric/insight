#!/usr/bin/env bash
set -euo pipefail

# Run only the Silver transformations for Jira on bronze data that's already
# in ClickHouse (no Airbyte sync). Steps:
#   1. dbt run --select tag:jira              — staging models
#   2. tt-enrich-jira-run                     — Rust binary writes task_field_history
#   3. dbt run --select tag:silver,tag:jira+  — silver models downstream of jira (class_task_*)
#
# Infrastructure parameters (toolbox_image, clickhouse_*, batch_size) come
# from WorkflowTemplate defaults baked at helm-install time — see
# charts/insight/templates/ingestion/{dbt-run,tt-enrich-jira-run}.yaml. The
# enrich image comes from the jira descriptor's images.enrich.image field
# (per ADR-0016); this script reads it and passes it to the workflow as
# jira_enrich_image.
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

# Resolve enrich image from the jira descriptor. Fail fast on a missing
# value rather than rendering an empty `image:` field that Argo would
# reject with an obscure container-creation error.
JIRA_DESCRIPTOR="$SCRIPT_DIR/connectors/task-tracking/jira/descriptor.yaml"
if ! JIRA_ENRICH_IMAGE="$(python3 \
      "$SCRIPT_DIR/reconcile-connectors/python/parse_descriptor.py" \
      --descriptor "$JIRA_DESCRIPTOR" --field images.enrich.image)" \
   || [[ -z "$JIRA_ENRICH_IMAGE" ]]; then
  echo "ERROR: descriptor.yaml.images.enrich.image missing or empty at $JIRA_DESCRIPTOR" >&2
  echo "       Set it before running tt-enrich." >&2
  exit 1
fi

echo "Running Jira tt-enrich (staging-jira -> enrich -> silver):"
echo "  namespace:         $INSIGHT_NAMESPACE"
echo "  tenant:            $TENANT"
echo "  insight_source_id: $SOURCE_ID"
echo "  jira_enrich_image: $JIRA_ENRICH_IMAGE"

NAMESPACE="$INSIGHT_NAMESPACE" \
  TENANT="$TENANT" \
  TENANT_DASHED="$TENANT_DASHED" \
  SOURCE_ID="$SOURCE_ID" \
  JIRA_ENRICH_IMAGE="$JIRA_ENRICH_IMAGE" \
  envsubst < workflows/onetime/tt-enrich-jira.yaml.tpl |
  kubectl create -n "$INSIGHT_NAMESPACE" -f -

echo
echo "Monitor:"
echo "  kubectl -n $INSIGHT_NAMESPACE get workflows -l connector=jira,workflow-kind=tt-enrich --watch"
