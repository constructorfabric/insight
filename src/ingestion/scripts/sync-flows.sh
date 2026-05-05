#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

# KUBECONFIG can be empty when running in-cluster

WORKFLOWS_DIR="./workflows"
CONNECTORS_DIR="./connectors"
CONNECTIONS_DIR="./connections"

# Always apply shared WorkflowTemplates first
echo "  Applying WorkflowTemplates..."
kubectl apply -f "${WORKFLOWS_DIR}/templates/"

# --- Get connection_id from Airbyte (authoritative state) ---
export TOOLKIT_DIR="${SCRIPT_DIR}/../airbyte-toolkit"
# shellcheck source=../airbyte-toolkit/lib/airbyte.sh
source "${TOOLKIT_DIR}/lib/airbyte.sh"

_AB_WS_ID="$(ab_workspace_id)"
_AB_CONN_CACHE="$(ab_list_connections "${_AB_WS_ID}")"

get_connection_id() {
  local tenant="$1" connector="$2"
  printf '%s' "${_AB_CONN_CACHE}" | python3 -c '
import sys, json
connector, tenant = sys.argv[1], sys.argv[2]
for c in json.load(sys.stdin):
    name = c.get("name", "")
    if name.startswith(f"{connector}-") and tenant in name:
        print(c.get("connectionId", "")); sys.exit(0)
sys.exit(1)
' "${connector}" "${tenant}"
}

# --- Generate and apply CronWorkflows for a tenant ---
sync_tenant() {
  local tenant="$1"
  local tenant_dir="${WORKFLOWS_DIR}/${tenant}"
  mkdir -p "$tenant_dir"

  # Iterate over all connectors with descriptor.yaml
  for descriptor in "${CONNECTORS_DIR}"/*/*/descriptor.yaml; do
    [[ -f "$descriptor" ]] || continue

    local connector schedule dbt_select workflow
    connector=$(yq -r '.name' "$descriptor")
    schedule=$(yq -r '.schedule' "$descriptor" 2>/dev/null | grep -v null || echo "0 2 * * *")
    dbt_select=$(yq -r '.dbt_select' "$descriptor" 2>/dev/null | grep -v null || echo "+tag:silver")
    workflow=$(yq -r '.workflow' "$descriptor" 2>/dev/null | grep -v null || echo "sync")

    # Find the workflow template
    local tpl="${WORKFLOWS_DIR}/schedules/${workflow}.yaml.tpl"
    if [[ ! -f "$tpl" ]]; then
      echo "  SKIP: no template ${tpl} for connector ${connector}"
      continue
    fi

    # Get connection_id from state
    local connection_id
    connection_id=$(get_connection_id "$tenant" "$connector") || true
    if [[ -z "$connection_id" ]]; then
      echo "  SKIP: no connection_id for ${connector} tenant ${tenant}"
      continue
    fi

    # Generate CronWorkflow
    local output="${tenant_dir}/${connector}-sync.yaml"
    CONNECTOR="$connector" \
    TENANT_ID="$tenant" \
    CONNECTION_ID="$connection_id" \
    SCHEDULE="$schedule" \
    DBT_SELECT="$dbt_select" \
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
