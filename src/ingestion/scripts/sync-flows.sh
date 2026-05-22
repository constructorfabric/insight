#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# KUBECONFIG can be empty when running in-cluster

# Host-side prerequisites — `yq` (and `jq`/PyYAML, transitively) for the
# descriptor read loop below. `envsubst` (gettext) is also used by the
# template render but is assumed present on the host; no auto-install.
# No port-forward needed — this script doesn't hit the Airbyte API.
# See airbyte-toolkit/lib/host-side-prerequisites.sh.
source "./airbyte-toolkit/lib/host-side-prerequisites.sh"
ensure_tooling

WORKFLOWS_DIR="./workflows"
CONNECTORS_DIR="./connectors"
CONNECTIONS_DIR="./connections"

# Shared WorkflowTemplates (airbyte-sync, dbt-run, ingestion-pipeline,
# tt-enrich-jira-run) are owned by the umbrella chart and rendered into
# the release namespace on `helm install` (see charts/insight/templates/
# ingestion/*.yaml, controlled by ingestion.templates.enabled). We skip
# applying them here — they are already in the cluster, and re-applying
# from a long-deleted local copy was a stale leftover from before PR #224.
: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
INSIGHT_NS="${INSIGHT_NAMESPACE}"
if ! kubectl get workflowtemplate -n "$INSIGHT_NS" airbyte-sync >/dev/null 2>&1; then
  echo "ERROR: WorkflowTemplate airbyte-sync not found in namespace '$INSIGHT_NS'." >&2
  echo "       The umbrella chart should have installed it. Check:" >&2
  echo "         helm get values insight -n $INSIGHT_NS | grep ingestion" >&2
  echo "       and ensure ingestion.templates.enabled=true with" >&2
  echo "       ingestion.toolboxImage set (enrich images are sourced from" >&2
  echo "       descriptor.yaml.images.enrich.image per ADR-0016)." >&2
  exit 1
fi
echo "  Found shared WorkflowTemplates in $INSIGHT_NS"

# --- Resolve connection_name from Secret annotations (per ADR-0005) ---
# Per KEY DECISION #1 we now pass connection_name (not the UUID); the
# airbyte-sync init-step resolves the UUID at submit time.
export RECONCILE_DIR="${SCRIPT_DIR}/../reconcile-connectors"
# shellcheck source=../reconcile-connectors/lib/secrets.sh
source "${RECONCILE_DIR}/lib/secrets.sh"

get_connection_name() {
  local tenant="$1" connector="$2"
  local source_id
  source_id="$(resolve_source_id "${connector}" "${tenant}" 2>/dev/null || true)"
  [[ -n "${source_id}" ]] || return 1
  printf '%s-%s-%s-conn' "${connector}" "${source_id}" "${tenant}"
}

# --- Generate and apply CronWorkflows for a tenant ---
sync_tenant() {
  local tenant="$1"
  local tenant_dir="${WORKFLOWS_DIR}/${tenant}"
  mkdir -p "$tenant_dir"

  # Wipe stale generated workflows from prior runs — connectors may have
  # been removed, renamed, or the namespace contract may have changed
  # (e.g. PR #224 dropped the `argo` namespace). Without this cleanup,
  # `kubectl apply -f $tenant_dir/` below will pick up old YAMLs and try
  # to apply them. Path is rooted under WORKFLOWS_DIR/<tenant> so this
  # is safe.
  rm -f "$tenant_dir"/*.yaml

  # Iterate over all connectors with descriptor.yaml
  for descriptor in "${CONNECTORS_DIR}"/*/*/descriptor.yaml; do
    [[ -f "$descriptor" ]] || continue

    local connector schedule dbt_select workflow
    connector=$(yq -r '.name' "$descriptor")
    schedule="$(yq -r '.schedule // "0 2 * * *"' "$descriptor" 2>/dev/null || echo "0 2 * * *")"
    dbt_select="$(yq -r '.dbt_select // "+tag:silver"' "$descriptor" 2>/dev/null || echo "+tag:silver")"
    workflow="$(yq -r '.workflow // "sync"' "$descriptor" 2>/dev/null || echo "sync")"

    # Find the workflow template
    local tpl="${WORKFLOWS_DIR}/schedules/${workflow}.yaml.tpl"
    if [[ ! -f "$tpl" ]]; then
      echo "  SKIP: no template ${tpl} for connector ${connector}"
      continue
    fi

    # Compute connection_name from Secret annotations.
    local connection_name
    connection_name=$(get_connection_name "$tenant" "$connector") || true
    if [[ -z "$connection_name" ]]; then
      echo "  SKIP: no connection_name for ${connector} tenant ${tenant}"
      continue
    fi

    # Generate CronWorkflow
    local output="${tenant_dir}/${connector}-sync.yaml"
    CONNECTOR="$connector" \
    TENANT_ID="$tenant" \
    CONNECTION_NAME="$connection_name" \
    SCHEDULE="$schedule" \
    DBT_SELECT="$dbt_select" \
    NAMESPACE="$INSIGHT_NS" \
      envsubst < "$tpl" > "$output"

    echo "  Generated: ${output}"
  done

  # Apply generated workflows
  if ls "${tenant_dir}"/*.yaml >/dev/null 2>&1; then
    kubectl apply -f "$tenant_dir/"
  fi
}

# --- Main ---
# Tenant list comes from $INSIGHT_TENANT_ID (single tenant) or
# `--tenant <name>` flag, since Airbyte connection.name encodes the
# tenant suffix and we use that as the identifier going forward.
if [[ "${1:-}" == "--all" ]]; then
  tenant="${INSIGHT_TENANT_ID:?--all requires INSIGHT_TENANT_ID env}"
  echo "  Syncing workflows for tenant: $tenant"
  sync_tenant "$tenant"
else
  tenant="${1:?Usage: $0 <tenant_id> | --all}"
  echo "  Syncing workflows for tenant: $tenant"
  sync_tenant "$tenant"
fi
