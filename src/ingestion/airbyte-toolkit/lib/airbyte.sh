#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-feature-reconcile — airbyte API helpers
#
# High-level helpers that wrap the Airbyte Public API (v1 under
# /api/public/v1) plus the legacy private API (under /api/v1) for the few
# endpoints not yet exposed in public (state get/create_or_update,
# connector_builder_projects, connection_definitions). Sourced by
# discover.sh / adopt.sh / reconcile.sh — never executed standalone.
#
# Conventions:
#   - Bash 4+ required (assoc arrays in callers); shebang for editor support.
#   - Strict mode is set so `bash lib/airbyte.sh` syntax-checks cleanly,
#     but every entry point checks BASH_SOURCE so re-sourcing doesn't trip
#     callers that already enabled strict mode.
#   - All HTTP calls use `curl --fail-with-body --silent --show-error` so
#     4xx/5xx bodies surface to stderr but the bearer token never does.
#   - JSON payloads are passed via heredocs to avoid shell-quoting bugs.
#   - All functions use lowercase names with the `ab_` prefix.
#   - Sensitive values (token, secret config) MUST NOT be echoed.
#
# Required env (set by callers via lib/env.sh-equivalent or run-init):
#   AIRBYTE_URL          — base URL, e.g. http://airbyte-server:8001
#   AIRBYTE_TOKEN_FILE   — file holding bearer JWT (default
#                          /var/run/secrets/airbyte/token)
# ---------------------------------------------------------------------------

set -euo pipefail

# Only define functions; do not run anything when sourced.

# Absolute base URL for Airbyte API. Callers set AIRBYTE_URL.
: "${AIRBYTE_URL:=http://localhost:8001}"
: "${AIRBYTE_TOKEN_FILE:=/var/run/secrets/airbyte/token}"

# ---------------------------------------------------------------------------
# ab_get_token — print bearer token to stdout.
# Reads from AIRBYTE_TOKEN_FILE if it exists; otherwise echoes
# AIRBYTE_TOKEN env var (set by env-resolver). Never logs the value.
# ---------------------------------------------------------------------------
ab_get_token() {
  if [[ -n "${AIRBYTE_TOKEN:-}" ]]; then
    printf '%s' "${AIRBYTE_TOKEN}"
    return 0
  fi
  if [[ -r "${AIRBYTE_TOKEN_FILE}" ]]; then
    # shellcheck disable=SC2002
    cat "${AIRBYTE_TOKEN_FILE}"
    return 0
  fi
  printf 'ab_get_token: no token (AIRBYTE_TOKEN unset and %s unreadable)\n' "${AIRBYTE_TOKEN_FILE}" >&2
  return 1
}

# ---------------------------------------------------------------------------
# ab__curl — internal helper. Wraps curl with auth + JSON content type.
# Args: METHOD PATH [BODY_JSON_OR_EMPTY]
# Echoes response body on stdout. Token never appears in argv.
# ---------------------------------------------------------------------------
ab__curl() {
  local method="$1"
  local path="$2"
  local body="${3:-}"
  local token
  token="$(ab_get_token)"
  local url="${AIRBYTE_URL%/}${path}"
  if [[ -n "${body}" ]]; then
    printf '%s' "${body}" \
      | curl --fail-with-body --silent --show-error \
          -X "${method}" \
          -H "Authorization: Bearer ${token}" \
          -H "Content-Type: application/json" \
          --data-binary @- \
          "${url}"
  else
    curl --fail-with-body --silent --show-error \
      -X "${method}" \
      -H "Authorization: Bearer ${token}" \
      -H "Content-Type: application/json" \
      "${url}"
  fi
}

# ---------------------------------------------------------------------------
# ab_workspace_id — return the single workspace id; assert exactly one.
# ---------------------------------------------------------------------------
ab_workspace_id() {
  local resp
  resp="$(ab__curl POST /api/v1/workspaces/list_by_organization_id \
    '{"organizationId":"00000000-0000-0000-0000-000000000000"}')"
  printf '%s' "${resp}" | python3 -c '
import sys, json
ws = json.load(sys.stdin).get("workspaces", [])
if len(ws) != 1:
    sys.stderr.write(f"ab_workspace_id: expected 1 workspace, got {len(ws)}\n")
    sys.exit(1)
print(ws[0]["workspaceId"])
'
}

# ---------------------------------------------------------------------------
# ab_list_definitions <workspace_id>
# Returns JSON array of source_definitions for the workspace.
# ---------------------------------------------------------------------------
ab_list_definitions() {
  local workspace_id="$1"
  local body
  body=$(printf '{"workspaceId":"%s"}' "${workspace_id}")
  ab__curl POST /api/v1/source_definitions/list_for_workspace "${body}" \
    | python3 -c 'import sys,json;d=json.load(sys.stdin);print(json.dumps(d.get("sourceDefinitions",[])))'
}

# ---------------------------------------------------------------------------
# ab_get_definition <definition_id>
# Returns single source_definition JSON.
# ---------------------------------------------------------------------------
ab_get_definition() {
  local definition_id="$1"
  local body
  body=$(printf '{"sourceDefinitionId":"%s"}' "${definition_id}")
  ab__curl POST /api/v1/source_definitions/get "${body}"
}

# ---------------------------------------------------------------------------
# ab_set_definition_description <definition_id> <description>
# For nocode declarative connectors: re-publish the active manifest with
# `description` set to the descriptor.yaml.version. Caller must already
# know the builderProjectId; if not provided we look it up via
# connector_builder_projects/list.
# Args: definition_id description [builder_project_id]
# ---------------------------------------------------------------------------
ab_set_definition_description() {
  local definition_id="$1"
  local description="$2"
  local builder_project_id="${3:-}"
  if [[ -z "${builder_project_id}" ]]; then
    local workspace_id
    workspace_id="$(ab_workspace_id)"
    local list
    list="$(ab__curl POST /api/v1/connector_builder_projects/list \
      "$(printf '{"workspaceId":"%s"}' "${workspace_id}")")"
    builder_project_id="$(printf '%s' "${list}" | python3 -c '
import sys, json
target = sys.argv[1]
data = json.load(sys.stdin)
for p in data.get("projects", []):
    am = p.get("activeDeclarativeManifest") or {}
    if am.get("sourceDefinitionId") == target:
        print(p["builderProjectId"]); break
' "${definition_id}")"
  fi
  if [[ -z "${builder_project_id}" ]]; then
    printf 'ab_set_definition_description: no builder project for definition %s\n' "${definition_id}" >&2
    return 1
  fi
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId": sys.argv[1],
  "builderProjectId": sys.argv[2],
  "description": sys.argv[3],
}))
' "$(ab_workspace_id)" "${builder_project_id}" "${description}")
  ab__curl POST /api/v1/connector_builder_projects/update_active_manifest "${body}"
}

# ---------------------------------------------------------------------------
# ab_set_definition_image_tag <definition_id> <tag>
# For CDK connectors: update dockerImageTag on the source definition.
# ---------------------------------------------------------------------------
ab_set_definition_image_tag() {
  local definition_id="$1"
  local tag="$2"
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "sourceDefinitionId": sys.argv[1],
  "dockerImageTag": sys.argv[2],
}))
' "${definition_id}" "${tag}")
  ab__curl POST /api/v1/source_definitions/update "${body}"
}

# ---------------------------------------------------------------------------
# ab_list_sources <workspace_id>
# Returns JSON array of sources.
# ---------------------------------------------------------------------------
ab_list_sources() {
  local workspace_id="$1"
  local body
  body=$(printf '{"workspaceId":"%s"}' "${workspace_id}")
  ab__curl POST /api/v1/sources/list "${body}" \
    | python3 -c 'import sys,json;d=json.load(sys.stdin);print(json.dumps(d.get("sources",[])))'
}

# ---------------------------------------------------------------------------
# ab_create_source <workspace_id> <definition_id> <name> <config_json>
# POST /api/v1/sources/create. config_json is a JSON object string.
# Returns the created source JSON.
# ---------------------------------------------------------------------------
ab_create_source() {
  local workspace_id="$1"
  local definition_id="$2"
  local name="$3"
  local config_json="$4"
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId": sys.argv[1],
  "sourceDefinitionId": sys.argv[2],
  "name": sys.argv[3],
  "connectionConfiguration": json.loads(sys.argv[4]),
}))
' "${workspace_id}" "${definition_id}" "${name}" "${config_json}")
  ab__curl POST /api/v1/sources/create "${body}"
}

# ---------------------------------------------------------------------------
# ab_update_source <source_id> <config_json> [name]
# POST /api/v1/sources/update — preserves source-id, idempotent.
# ---------------------------------------------------------------------------
ab_update_source() {
  local source_id="$1"
  local config_json="$2"
  local name="${3:-}"
  local body
  body=$(python3 -c '
import sys, json
payload = {
  "sourceId": sys.argv[1],
  "connectionConfiguration": json.loads(sys.argv[2]),
}
if len(sys.argv) > 3 and sys.argv[3]:
    payload["name"] = sys.argv[3]
print(json.dumps(payload))
' "${source_id}" "${config_json}" "${name}")
  ab__curl POST /api/v1/sources/update "${body}"
}

# ---------------------------------------------------------------------------
# ab_delete_source <source_id>
# ---------------------------------------------------------------------------
ab_delete_source() {
  local source_id="$1"
  local body
  body=$(printf '{"sourceId":"%s"}' "${source_id}")
  ab__curl POST /api/v1/sources/delete "${body}"
}

# ---------------------------------------------------------------------------
# ab_list_connections <workspace_id>
# Returns JSON array of connections in workspace.
# ---------------------------------------------------------------------------
ab_list_connections() {
  local workspace_id="$1"
  local body
  body=$(printf '{"workspaceId":"%s"}' "${workspace_id}")
  ab__curl POST /api/v1/connections/list "${body}" \
    | python3 -c 'import sys,json;d=json.load(sys.stdin);print(json.dumps(d.get("connections",[])))'
}

# ---------------------------------------------------------------------------
# ab_create_connection <workspace_id> <source_id> <destination_id> <name> \
#                      <schedule_json> <tags_json> [sync_catalog_json]
# POST /api/v1/connections/create.
# schedule_json: e.g. '{"scheduleType":"manual"}' or
#                '{"scheduleType":"cron","cronExpression":"0 2 * * *"}'.
# tags_json: JSON array of strings, e.g. '["insight","cfg-hash:abc123"]'.
# sync_catalog_json: optional pre-discovered syncCatalog object (else
# caller should call sources/discover_schema beforehand and pass it).
#
# @cpt-constraint:cpt-dataflow-constraint-airbyte-append:p1
# Per cpt-dataflow-constraint-airbyte-append (PR #251 conventions),
# every stream in the supplied syncCatalog MUST set
# destinationSyncMode = "append". Dedup happens in silver via unique_key;
# destination-side append_dedup buffers all records in memory until
# stream COMPLETE, OOMs on large streams, and loses all data on
# mid-stream pod death. Overwrite has the same problem on retries.
# Callers building syncCatalog are responsible for honouring this.
# ---------------------------------------------------------------------------
ab_create_connection() {
  local workspace_id="$1"
  local source_id="$2"
  local destination_id="$3"
  local name="$4"
  local schedule_json="$5"
  local tags_json="$6"
  local sync_catalog_json="${7:-{\"streams\":[]}}"
  local body
  body=$(python3 -c '
import sys, json
payload = {
  "workspaceId": sys.argv[1],
  "sourceId": sys.argv[2],
  "destinationId": sys.argv[3],
  "name": sys.argv[4],
  "schedule": json.loads(sys.argv[5]),
  "tags": json.loads(sys.argv[6]),
  "syncCatalog": json.loads(sys.argv[7]),
  "status": "active",
}
print(json.dumps(payload))
' "${workspace_id}" "${source_id}" "${destination_id}" "${name}" \
  "${schedule_json}" "${tags_json}" "${sync_catalog_json}")
  ab__curl POST /api/v1/connections/create "${body}"
}

# ---------------------------------------------------------------------------
# ab_patch_connection_tags <connection_id> <tags_json>
# PATCH /api/public/v1/connections/{id} — updates only the tags field.
# tags_json: JSON array of strings.
# ---------------------------------------------------------------------------
ab_patch_connection_tags() {
  local connection_id="$1"
  local tags_json="$2"
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({"tags": json.loads(sys.argv[1])}))
' "${tags_json}")
  ab__curl PATCH "/api/public/v1/connections/${connection_id}" "${body}"
}

# ---------------------------------------------------------------------------
# ab_get_state <connection_id>
# POST /api/v1/state/get — returns connection's stored state blob (legacy
# private API; public API does not yet expose state endpoints).
# ---------------------------------------------------------------------------
ab_get_state() {
  local connection_id="$1"
  local body
  body=$(printf '{"connectionId":"%s"}' "${connection_id}")
  ab__curl POST /api/v1/state/get "${body}"
}

# ---------------------------------------------------------------------------
# ab_create_or_update_state <connection_id> <state_json>
# POST /api/v1/state/create_or_update — restores a state blob.
# state_json: the FULL state object as returned by ab_get_state, with the
# connectionId rewritten to the new connection (caller's responsibility).
# ---------------------------------------------------------------------------
ab_create_or_update_state() {
  local connection_id="$1"
  local state_json="$2"
  local body
  body=$(python3 -c '
import sys, json
state = json.loads(sys.argv[2])
state["connectionId"] = sys.argv[1]
print(json.dumps(state))
' "${connection_id}" "${state_json}")
  ab__curl POST /api/v1/state/create_or_update "${body}"
}
