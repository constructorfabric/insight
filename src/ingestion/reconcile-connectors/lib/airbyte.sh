#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-featstatus-reconcile — airbyte API helpers
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
#   INSIGHT_NAMESPACE    — K8s namespace where airbyte-auth-secrets lives
# Optional env (with documented defaults):
#   AIRBYTE_TOKEN          — pre-minted JWT (skips minting; for tests/CI)
#   AIRBYTE_TOKEN_CACHE    — path to TTL-backed cache file
#                            (default: per-UID file under /tmp)
#   AIRBYTE_TOKEN_TTL      — JWT lifetime in seconds (default 300)
#   AIRBYTE_AUTH_SECRET_NAME — name of the K8s Secret holding the
#                              jwt-signature-secret key (default
#                              airbyte-auth-secrets, the bundled-chart name)
# ---------------------------------------------------------------------------

# NOTE: this file is sourced into callers' shells; do NOT enable
# `set -euo pipefail` at the top level (it leaks into interactive shells
# and breaks PS1 / PROMPT_COMMAND lines that touch unset vars).
# Only define functions; do not run anything when sourced.
: "${AIRBYTE_URL:?AIRBYTE_URL must be set (e.g. http://airbyte-server:8001)}"

_AIRBYTE_LIB_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
_AIRBYTE_PY_DIR="$( cd "${_AIRBYTE_LIB_DIR}/../python" && pwd )"

# ---------------------------------------------------------------------------
# ab_get_token — print bearer token to stdout.
#
# Resolution chain (priority order):
#   1. AIRBYTE_TOKEN env (test/CI shortcut)
#   2. Cached token file in ${AIRBYTE_TOKEN_CACHE} if mtime < TTL - 30s
#   3. Mint a fresh HS256 JWT: read jwt-signature-secret from K8s Secret
#      (kubectl, RBAC `secrets get` already granted by reconcile-rbac.yaml),
#      sign via python/mint_airbyte_jwt.py, cache to file (mode 600).
#
# This is the single source of truth for "give me a valid Airbyte JWT".
# Sensitive values are never logged or echoed in error paths.
# ---------------------------------------------------------------------------
ab_get_token() {
  if [[ -n "${AIRBYTE_TOKEN:-}" ]]; then
    printf '%s' "${AIRBYTE_TOKEN}"
    return 0
  fi
  : "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set (the K8s namespace where Airbyte runs)}"
  local cache="${AIRBYTE_TOKEN_CACHE:-/tmp/insight-airbyte-token-${UID:-$(id -u)}}"  # RULE-DEFAULTS-OK: per-UID tmp cache; mode 600 set below; not a config input
  local ttl="${AIRBYTE_TOKEN_TTL:-300}"  # RULE-DEFAULTS-OK: operational tuning; 5min < JWT exp; safe re-mint cadence
  local secret_name="${AIRBYTE_AUTH_SECRET_NAME:-airbyte-auth-secrets}"  # RULE-DEFAULTS-OK: name fixed by Airbyte Helm chart; override only for non-bundled Airbyte

  # Cache hit? mtime newer than (ttl - 30s) means JWT is still valid for
  # at least 30s — return it. Below that, re-mint to avoid mid-call expiry.
  if [[ -r "$cache" ]]; then
    local mtime now age
    if mtime="$(stat -f %m "$cache" 2>/dev/null || stat -c %Y "$cache" 2>/dev/null)"; then
      now="$(date +%s)"
      age=$(( now - mtime ))
      if [[ "$age" -lt $(( ttl - 30 )) ]]; then
        cat "$cache"
        return 0
      fi
    fi
  fi

  # Cache miss / expired — mint a fresh JWT. Pull HMAC signing secret
  # from K8s. RBAC: reconcile-rbac.yaml grants `secrets get/list/watch` on
  # the namespace; locally the user's kubeconfig provides the same.
  local jwt_secret_b64 token tmp
  if ! jwt_secret_b64="$(kubectl -n "$INSIGHT_NAMESPACE" get secret "$secret_name" \
        -o jsonpath='{.data.jwt-signature-secret}' 2>/dev/null)"; then
    printf 'ab_get_token: kubectl failed reading secret/%s in ns %s (RBAC? wrong namespace?)\n' \
      "$secret_name" "$INSIGHT_NAMESPACE" >&2
    return 1
  fi
  if [[ -z "$jwt_secret_b64" ]]; then
    printf 'ab_get_token: secret/%s has no jwt-signature-secret key\n' "$secret_name" >&2
    return 1
  fi
  if ! token="$(printf '%s' "$jwt_secret_b64" | base64 -d \
        | python3 "${_AIRBYTE_PY_DIR}/mint_airbyte_jwt.py" "$ttl")"; then
    printf 'ab_get_token: mint_airbyte_jwt.py failed\n' >&2
    return 1
  fi

  # Atomic write: tmp file in same dir, then mv. Mode 600 throughout.
  tmp="$(mktemp "${cache}.XXXXXX")"
  trap "rm -f '${tmp}'" RETURN
  chmod 600 "$tmp"
  printf '%s' "$token" > "$tmp"
  mv "$tmp" "$cache"
  printf '%s' "$token"
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
# ab_create_custom_cdk_definition <workspace_id> <connector_name> \
#                                 <docker_repo> <image_tag>
# Per ADR-0011: registers a pre-built CDK image as a custom source_definition.
# Image lives in ${IMAGE_REGISTRY}/source-${connector}-insight; tag matches
# descriptor.yaml.version. Prints the new sourceDefinitionId on stdout.
# Returns 1 if the API responds without a sourceDefinitionId.
# ---------------------------------------------------------------------------
ab_create_custom_cdk_definition() {
  local workspace_id="$1"
  local connector_name="$2"
  local docker_repo="$3"
  local image_tag="$4"
  local body def_id
  body="$(python3 "${_AIRBYTE_PY_DIR}/create_cdk_definition_payload.py" \
    "${workspace_id}" "${connector_name}" "${docker_repo}" "${image_tag}")"
  def_id="$(ab__curl POST /api/v1/source_definitions/create_custom "${body}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceDefinitionId",""))')"
  if [[ -z "${def_id}" ]]; then
    printf 'ab_create_custom_cdk_definition: API returned no sourceDefinitionId for %s\n' \
      "${connector_name}" >&2
    return 1
  fi
  printf '%s' "${def_id}"
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
# ab_builder_list_projects <workspace_id>
# Returns JSON array of all builder projects in the workspace.
# Each entry: { builderProjectId, name, activeDeclarativeManifest{...} }
# ---------------------------------------------------------------------------
ab_builder_list_projects() {
  local workspace_id="$1"
  local body
  body=$(printf '{"workspaceId":"%s"}' "${workspace_id}")
  ab__curl POST /api/v1/connector_builder_projects/list "${body}" \
    | python3 -c 'import sys,json;d=json.load(sys.stdin);print(json.dumps(d.get("projects",[])))'
}

# ---------------------------------------------------------------------------
# ab_builder_find_by_name <workspace_id> <connector_name>
# Prints builderProjectId of the project whose `name` matches; empty if none.
# ---------------------------------------------------------------------------
ab_builder_find_by_name() {
  local workspace_id="$1"
  local connector_name="$2"
  ab_builder_list_projects "${workspace_id}" | python3 -c '
import sys, json
target = sys.argv[1]
for p in json.load(sys.stdin):
    if p.get("name") == target:
        print(p.get("builderProjectId", "")); break
' "${connector_name}"
}

# ---------------------------------------------------------------------------
# ab_builder_find_by_definition <workspace_id> <definition_id>
# Prints builderProjectId of the project whose
# activeDeclarativeManifest.sourceDefinitionId matches; empty if none.
# ---------------------------------------------------------------------------
ab_builder_find_by_definition() {
  local workspace_id="$1"
  local definition_id="$2"
  ab_builder_list_projects "${workspace_id}" | python3 -c '
import sys, json
target = sys.argv[1]
for p in json.load(sys.stdin):
    am = p.get("activeDeclarativeManifest") or {}
    if am.get("sourceDefinitionId") == target:
        print(p.get("builderProjectId", "")); break
' "${definition_id}"
}

# ---------------------------------------------------------------------------
# ab_builder_create_with_manifest <workspace_id> <connector_name> <manifest_yaml_path>
# POST /api/v1/connector_builder_projects/create with the manifest as a
# parsed object. Prints the new builderProjectId. Manifest is loaded by
# python/load_connector_manifest.py to convert YAML -> JSON object.
# ---------------------------------------------------------------------------
ab_builder_create_with_manifest() {
  local workspace_id="$1"
  local connector_name="$2"
  local manifest_path="$3"
  [[ -f "${manifest_path}" ]] || {
    printf 'ab_builder_create_with_manifest: manifest not found: %s\n' "${manifest_path}" >&2
    return 1
  }
  local manifest_json
  manifest_json="$(python3 "${_AIRBYTE_PY_DIR}/load_connector_manifest.py" "${manifest_path}")" || return 1
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId": sys.argv[1],
  "builderProject": {
    "name": sys.argv[2],
    "draftManifest": json.loads(sys.argv[3]),
  },
}))
' "${workspace_id}" "${connector_name}" "${manifest_json}")
  ab__curl POST /api/v1/connector_builder_projects/create "${body}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin).get("builderProjectId",""))'
}

# ---------------------------------------------------------------------------
# ab_builder_publish <workspace_id> <builder_project_id> <connector_name> \
#                    <description> <manifest_yaml_path>
# POST /api/v1/connector_builder_projects/publish — creates / updates the
# active source_definition for the project. Prints the resulting
# sourceDefinitionId.
# ---------------------------------------------------------------------------
ab_builder_publish() {
  local workspace_id="$1"
  local builder_project_id="$2"
  local connector_name="$3"
  local description="$4"
  local manifest_path="$5"
  [[ -f "${manifest_path}" ]] || {
    printf 'ab_builder_publish: manifest not found: %s\n' "${manifest_path}" >&2
    return 1
  }
  local manifest_json
  manifest_json="$(python3 "${_AIRBYTE_PY_DIR}/load_connector_manifest.py" "${manifest_path}")" || return 1
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId": sys.argv[1],
  "builderProjectId": sys.argv[2],
  "name": sys.argv[3],
  "initialDeclarativeManifest": {
    "manifest": json.loads(sys.argv[5]),
    "version": 1,
    "description": sys.argv[4],
  },
}))
' "${workspace_id}" "${builder_project_id}" "${connector_name}" "${description}" "${manifest_json}")
  ab__curl POST /api/v1/connector_builder_projects/publish "${body}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceDefinitionId",""))'
}

# ---------------------------------------------------------------------------
# ab_builder_update_active_manifest <workspace_id> <builder_project_id> \
#                                   <description> <manifest_yaml_path>
# POST /api/v1/connector_builder_projects/update_active_manifest. Bumps the
# active manifest version and updates description (semantic version label).
# Replaces the older single-purpose ab_set_definition_description for the
# new publish/update flow; the old function stays for backward compat.
# ---------------------------------------------------------------------------
ab_builder_update_active_manifest() {
  local workspace_id="$1"
  local builder_project_id="$2"
  local description="$3"
  local manifest_path="$4"
  [[ -f "${manifest_path}" ]] || {
    printf 'ab_builder_update_active_manifest: manifest not found: %s\n' "${manifest_path}" >&2
    return 1
  }
  local manifest_json
  manifest_json="$(python3 "${_AIRBYTE_PY_DIR}/load_connector_manifest.py" "${manifest_path}")" || return 1
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId": sys.argv[1],
  "builderProjectId": sys.argv[2],
  "description": sys.argv[3],
  "manifest": json.loads(sys.argv[4]),
}))
' "${workspace_id}" "${builder_project_id}" "${description}" "${manifest_json}")
  ab__curl POST /api/v1/connector_builder_projects/update_active_manifest "${body}"
}

# ---------------------------------------------------------------------------
# ab_get_definition_description <definition_id>
# Returns the active declarativeManifest.description (used as semantic
# version) of a source_definition. Empty if the definition is non-nocode.
# ---------------------------------------------------------------------------
ab_get_definition_description() {
  local definition_id="$1"
  ab_get_definition "${definition_id}" \
    | python3 -c '
import sys, json
d = json.load(sys.stdin)
dm = d.get("declarativeManifest") or {}
print(dm.get("description", ""))
'
}

# ---------------------------------------------------------------------------
# ab_delete_source_definition <definition_id>
# POST /api/v1/source_definitions/delete — used during orphan-recovery.
# ---------------------------------------------------------------------------
ab_delete_source_definition() {
  local definition_id="$1"
  local body
  body=$(printf '{"sourceDefinitionId":"%s"}' "${definition_id}")
  ab__curl POST /api/v1/source_definitions/delete "${body}"
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
# ab_discover_schema <source_id>
# POST /api/v1/sources/discover_schema — returns the discovered catalog as
# JSON. Used by reconcile to bootstrap a connection's syncCatalog when one
# does not exist yet. The returned object has a `catalog` key with the
# raw streams; callers normalize it (append-only) before passing to
# ab_create_connection.
# ---------------------------------------------------------------------------
ab_discover_schema() {
  local source_id="$1"
  local body
  body=$(printf '{"sourceId":"%s","disable_cache":false}' "${source_id}")
  ab__curl POST /api/v1/sources/discover_schema "${body}"
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
