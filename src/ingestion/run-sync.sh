#!/usr/bin/env bash
set -euo pipefail

# Submit an ingestion-pipeline Workflow for a single connector + tenant.
#
# Required env:
#   KUBECONFIG          path to the insight cluster kubeconfig
#   INSIGHT_NAMESPACE   release namespace of the umbrella chart
#
# Required args:
#   <connector>         connector descriptor name (matches connectors/*/<name>/descriptor.yaml .name)
#   <tenant_id>         tenant identifier
# Optional args:
#   <insight_source_id> when set, used directly; otherwise resolved from Secret annotations
#                       insight.cyberfabric.com/{connector,tenant,source-id}
#
# Most "infrastructure" parameters of the WorkflowTemplate (toolbox_image,
# airbyte_url, clickhouse_*) come from chart-rendered defaults; this script
# only passes connection-specific inputs. The `jira_enrich_image` parameter
# is REQUIRED with no chart-level default (per ADR-0016) — reconcile reads
# it from `descriptor.images.enrich.image` and passes it via the rendered
# ingestion-pipeline submission.

: "${KUBECONFIG:?must be set, e.g. export KUBECONFIG=~/.kube/insight.kubeconfig}"
: "${INSIGHT_NAMESPACE:?must be set to the umbrella release namespace, e.g. export INSIGHT_NAMESPACE=insight}"
export KUBECONFIG INSIGHT_NAMESPACE

CONNECTOR="${1:?Usage: $0 <connector> <tenant_id> [<insight_source_id>]}"
TENANT="${2:?Usage: $0 <connector> <tenant_id> [<insight_source_id>]}"
SOURCE_ID="${3:-}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

export RECONCILE_DIR="${SCRIPT_DIR}/reconcile-connectors"
# shellcheck source=reconcile-connectors/lib/secrets.sh
source "${RECONCILE_DIR}/lib/secrets.sh"

# ─── Resolve insight_source_id from Secret annotations ──────────────────
if [[ -z "$SOURCE_ID" ]]; then
  SOURCE_ID=$(resolve_source_id "$CONNECTOR" "$TENANT")
fi
[[ -n "$SOURCE_ID" ]] || {
  echo "ERROR: could not resolve insight_source_id for connector '$CONNECTOR' tenant '$TENANT'." >&2
  echo "       Either pass it explicitly as the third argument, or annotate the connector Secret with all three:" >&2
  echo "         insight.cyberfabric.com/connector=$CONNECTOR" >&2
  echo "         insight.cyberfabric.com/tenant=$TENANT" >&2
  echo "         insight.cyberfabric.com/source-id=<id>" >&2
  exit 1
}

# ─── Resolve dbt_select from descriptor.yaml ────────────────────────────
DESC=""
for desc in connectors/*/*/descriptor.yaml; do
  if [[ "$(yq -r '.name' "$desc")" == "$CONNECTOR" ]]; then
    DESC="$desc"
    break
  fi
done
[[ -n "$DESC" ]] || {
  echo "ERROR: no descriptor.yaml found with .name=$CONNECTOR under connectors/" >&2
  exit 1
}
DBT_SELECT="$(yq -r '.dbt_select' "$DESC")"
[[ -n "$DBT_SELECT" && "$DBT_SELECT" != "null" ]] || {
  echo "ERROR: $DESC does not define .dbt_select" >&2
  exit 1
}

# data_source dispatches the pipeline branch; only "jira" triggers the rust enrich path.
DATA_SOURCE="$CONNECTOR"

# dbt_select_staging only fires when data_source=jira.
DBT_SELECT_STAGING=""
if [[ "$DATA_SOURCE" == "jira" ]]; then
  DBT_SELECT_STAGING="tag:jira"
fi

TENANT_DASHED="${TENANT//_/-}"

# Per ADR-0005 / KEY DECISION #1: pass connection_name (not the UUID).
# The airbyte-sync template resolves connection_id at submit time via
# resolve-connection-by-name. Pattern matches reconcile_compute_connection_name.
CONNECTION_NAME="${CONNECTOR}-${SOURCE_ID}-${TENANT}-conn"

echo "Submitting ingestion-pipeline:"
echo "  namespace:          $INSIGHT_NAMESPACE"
echo "  connector:          $CONNECTOR"
echo "  tenant:             $TENANT"
echo "  connection_name:    $CONNECTION_NAME"
echo "  insight_source_id:  $SOURCE_ID"
echo "  data_source:        $DATA_SOURCE"
echo "  dbt_select:         $DBT_SELECT"
[[ -n "$DBT_SELECT_STAGING" ]] && echo "  dbt_select_staging: $DBT_SELECT_STAGING"

NAMESPACE="$INSIGHT_NAMESPACE" \
  CONNECTOR="$CONNECTOR" \
  TENANT="$TENANT" \
  TENANT_DASHED="$TENANT_DASHED" \
  CONNECTION_NAME="$CONNECTION_NAME" \
  SOURCE_ID="$SOURCE_ID" \
  DATA_SOURCE="$DATA_SOURCE" \
  DBT_SELECT="$DBT_SELECT" \
  DBT_SELECT_STAGING="$DBT_SELECT_STAGING" \
  envsubst < workflows/onetime/sync.yaml.tpl |
  kubectl create -n "$INSIGHT_NAMESPACE" -f -

echo
echo "Monitor:"
echo "  kubectl -n $INSIGHT_NAMESPACE get workflows -l connector=$CONNECTOR,tenant=$TENANT --watch"
