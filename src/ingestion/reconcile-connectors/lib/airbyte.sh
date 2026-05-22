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
#   AIRBYTE_TOKEN          — pre-supplied bearer token (skips OAuth call; for tests/CI)
#   AIRBYTE_TOKEN_CACHE    — path to TTL-backed cache file
#                            (default: per-UID file under /tmp)
#   AIRBYTE_TOKEN_TTL      — cache window in seconds (default 300)
#   AIRBYTE_AUTH_SECRET_NAME — name of the K8s Secret holding the
#                              instance-admin-client-{id,secret} keys
#                              (default airbyte-auth-secrets, the bundled-chart name)
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
#   3. OAuth2 client_credentials: read instance-admin-client-{id,secret}
#      from K8s Secret (kubectl, RBAC `secrets get` already granted by
#      reconcile-rbac.yaml), POST to /api/v1/applications/token, parse
#      access_token from JSON response, cache to file (mode 600).
#
# This is the single source of truth for "give me a valid Airbyte token".
# The token returned by Airbyte's applications/token endpoint carries the
# role/permission claims required for write operations on
# connector_builder_projects/* (Airbyte 1.8.5+) — a self-minted JWT does
# not. Sensitive values are never logged or echoed in error paths.
# ---------------------------------------------------------------------------
ab_get_token() {
  if [[ -n "${AIRBYTE_TOKEN:-}" ]]; then
    printf '%s' "${AIRBYTE_TOKEN}"
    return 0
  fi
  : "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set (the K8s namespace where Airbyte runs)}"
  local cache="${AIRBYTE_TOKEN_CACHE:-/tmp/insight-airbyte-token-${UID:-$(id -u)}}"  # RULE-DEFAULTS-OK: per-UID tmp cache; mode 600 set below; not a config input
  local ttl="${AIRBYTE_TOKEN_TTL:-300}"  # RULE-DEFAULTS-OK: cache window; Airbyte tokens last hours, this is just our re-fetch cadence
  local secret_name="${AIRBYTE_AUTH_SECRET_NAME:-airbyte-auth-secrets}"  # RULE-DEFAULTS-OK: name fixed by Airbyte Helm chart; override only for non-bundled Airbyte

  # Cache hit — token still fresh enough.
  if [[ -r "$cache" ]]; then
    local mtime now age
    # GNU stat (Linux) uses -c %Y; BSD/macOS uses -f %m. Prefer GNU because
    # `stat -f %m` on GNU is filesystem stat (mountpoint), not mtime — and
    # it succeeds, so the BSD-first ordering would yield an unparseable
    # value and trip set -u inside the arithmetic below.
    if mtime="$(stat -c %Y "$cache" 2>/dev/null || stat -f %m "$cache" 2>/dev/null)"; then
      now="$(date +%s)"
      age=$(( now - mtime ))
      if [[ "$age" -lt $(( ttl - 30 )) ]]; then
        cat "$cache"
        return 0
      fi
    fi
  fi

  # Cache miss — fetch a fresh OAuth2 client_credentials token.
  # RBAC: reconcile-rbac.yaml grants `secrets get/list/watch` on the namespace;
  # locally the user's kubeconfig provides the same.
  local secret_json client_id client_secret
  if ! secret_json="$(kubectl -n "$INSIGHT_NAMESPACE" get secret "$secret_name" -o json 2>/dev/null)"; then
    printf 'ab_get_token: kubectl failed reading secret/%s in ns %s (RBAC? wrong namespace?)\n' \
      "$secret_name" "$INSIGHT_NAMESPACE" >&2
    return 1
  fi
  client_id="$(printf '%s' "$secret_json" | python3 -c 'import sys,json,base64; d=json.load(sys.stdin); print(base64.b64decode(d["data"]["instance-admin-client-id"]).decode())')"
  client_secret="$(printf '%s' "$secret_json" | python3 -c 'import sys,json,base64; d=json.load(sys.stdin); print(base64.b64decode(d["data"]["instance-admin-client-secret"]).decode())')"
  if [[ -z "$client_id" || -z "$client_secret" ]]; then
    printf 'ab_get_token: secret/%s missing instance-admin-client-id or instance-admin-client-secret\n' "$secret_name" >&2
    return 1
  fi

  local body resp token tmp
  body="$(python3 -c 'import sys,json; print(json.dumps({"client_id":sys.argv[1],"client_secret":sys.argv[2],"grant_type":"client_credentials"}))' "$client_id" "$client_secret")"
  if ! resp="$(curl --fail-with-body --silent --show-error \
        -X POST "${AIRBYTE_URL%/}/api/v1/applications/token" \
        -H "Content-Type: application/json" \
        --data-binary "$body" 2>&1)"; then
    printf 'ab_get_token: applications/token failed: %s\n' "$resp" >&2
    return 1
  fi
  token="$(printf '%s' "$resp" | python3 -c 'import sys,json; print(json.load(sys.stdin).get("access_token",""))')"
  if [[ -z "$token" ]]; then
    printf 'ab_get_token: applications/token returned no access_token\n' >&2
    return 1
  fi

  # Atomic write: tmp file in same dir, then mv. Mode 600 throughout.
  # NOTE: a `trap RETURN` from a sourced library would clobber the
  # caller's RETURN trap and keep firing on every later function return.
  # Use explicit cleanup at every return path instead.
  tmp="$(mktemp "${cache}.XXXXXX")" || return 1
  if ! chmod 600 "$tmp"; then rm -f "$tmp"; return 1; fi
  if ! printf '%s' "$token" > "$tmp"; then rm -f "$tmp"; return 1; fi
  if ! mv "$tmp" "$cache"; then rm -f "$tmp"; return 1; fi
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
  # Bail out cleanly when the auth flow can't reach Airbyte. Without
  # this guard every downstream parser (json.load, jq) chokes on an
  # empty curl reply and dumps a Python stacktrace, masking the real
  # cause (port-forward died, server unreachable, expired creds, …).
  if ! token="$(ab_get_token)"; then
    printf 'ab__curl: Airbyte API unavailable (token mint failed). Check AIRBYTE_URL=%s and connectivity.\n' \
      "${AIRBYTE_URL:-<unset>}" >&2
    return 1
  fi
  if [[ -z "${token}" ]]; then
    printf 'ab__curl: Airbyte API unavailable (empty token). Check AIRBYTE_URL=%s and credentials.\n' \
      "${AIRBYTE_URL:-<unset>}" >&2
    return 1
  fi
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
# Per ADR-0016 (supersedes ADR-0011): registers a pre-built CDK image as a
# custom source_definition. docker_repo + image_tag come verbatim from
# descriptor.images.cdk.image (split via split_docker_image_ref.py). Prints
# the new sourceDefinitionId on stdout.
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
# Prints builderProjectId of the project whose top-level `sourceDefinitionId`
# matches; empty if none. (Airbyte's connector_builder_projects/list returns
# sourceDefinitionId on the project record itself, not nested under
# activeDeclarativeManifest.)
# ---------------------------------------------------------------------------
ab_builder_find_by_definition() {
  local workspace_id="$1"
  local definition_id="$2"
  ab_builder_list_projects "${workspace_id}" | python3 -c '
import sys, json
target = sys.argv[1]
for p in json.load(sys.stdin):
    if p.get("sourceDefinitionId") == target:
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
  # Airbyte 1.7+ expects `spec` as a wrapper with documentationUrl /
  # connectionSpecification / advancedAuth, NOT the raw manifest.spec block
  # (which is {type, connection_specification}). Build the wrapper here from
  # snake_case manifest fields. Same shape used by update path below.
  local body
  body=$(python3 -c '
import sys, json
m = json.loads(sys.argv[5])
mspec = m.get("spec", {}) or {}
spec = {
  "documentationUrl": mspec.get("documentation_url", ""),
  "connectionSpecification": mspec.get("connection_specification", {}),
}
if mspec.get("advanced_auth"):
    spec["advancedAuth"] = mspec["advanced_auth"]
print(json.dumps({
  "workspaceId": sys.argv[1],
  "builderProjectId": sys.argv[2],
  "name": sys.argv[3],
  "initialDeclarativeManifest": {
    "description": sys.argv[4],
    "manifest": m,
    "spec": spec,
    "version": 1,
  },
}))
' "${workspace_id}" "${builder_project_id}" "${connector_name}" "${description}" "${manifest_json}")
  ab__curl POST /api/v1/connector_builder_projects/publish "${body}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceDefinitionId",""))'
}

# ---------------------------------------------------------------------------
# ab_builder_update_active_manifest <workspace_id> <source_definition_id> \
#                                   <description> <manifest_yaml_path>
# POST /api/v1/declarative_source_definitions/create_manifest. Adds a new
# manifest version (current+1) for the given source_definition and sets it
# active. The endpoint /connector_builder_projects/update_active_manifest
# only flips an existing version pointer; for content updates Airbyte 1.7+
# routes everything through declarative_source_definitions/create_manifest.
# Caller passes the source_definition_id (NOT the builder_project_id);
# function reads current activeDeclarativeManifestVersion to compute the
# next integer.
# ---------------------------------------------------------------------------
ab_builder_update_active_manifest() {
  local workspace_id="$1"
  local source_definition_id="$2"
  local description="$3"
  local manifest_path="$4"
  [[ -f "${manifest_path}" ]] || {
    printf 'ab_builder_update_active_manifest: manifest not found: %s\n' "${manifest_path}" >&2
    return 1
  }
  local manifest_json
  manifest_json="$(python3 "${_AIRBYTE_PY_DIR}/load_connector_manifest.py" "${manifest_path}")" || return 1

  # Look up current active version → next = current + 1.
  local current_version
  current_version="$(ab__curl POST /api/v1/connector_builder_projects/list \
        "$(printf '{"workspaceId":"%s"}' "${workspace_id}")" \
      | python3 -c '
import sys, json
target = sys.argv[1]
for p in json.load(sys.stdin).get("projects", []):
    if p.get("sourceDefinitionId") == target:
        print(int(p.get("activeDeclarativeManifestVersion") or 0)); break
else:
    print(0)
' "${source_definition_id}")"
  local next_version=$((current_version + 1))

  local body
  body=$(python3 -c '
import sys, json
m = json.loads(sys.argv[4])
mspec = m.get("spec", {}) or {}
spec = {
  "documentationUrl": mspec.get("documentation_url", ""),
  "connectionSpecification": mspec.get("connection_specification", {}),
}
if mspec.get("advanced_auth"):
    spec["advancedAuth"] = mspec["advanced_auth"]
print(json.dumps({
  "workspaceId": sys.argv[1],
  "sourceDefinitionId": sys.argv[2],
  "setAsActiveManifest": True,
  "declarativeManifest": {
    "description": sys.argv[3],
    "manifest": m,
    "spec": spec,
    "version": int(sys.argv[5]),
  },
}))
' "${workspace_id}" "${source_definition_id}" "${description}" "${manifest_json}" "${next_version}")
  ab__curl POST /api/v1/declarative_source_definitions/create_manifest "${body}"
}

# ---------------------------------------------------------------------------
# ab_get_definition_description <definition_id>
# Returns the active declarativeManifest.description (used as semantic
# version) of a nocode source_definition. Empty if non-declarative or no
# manifests exist. Airbyte 1.7+: this lives on
# declarative_source_definitions/list_manifests, not on
# source_definitions/get (which has no manifest field).
# ---------------------------------------------------------------------------
ab_get_definition_description() {
  local definition_id="$1"
  local workspace_id
  workspace_id="$(ab_workspace_id)"
  local body
  body=$(printf '{"workspaceId":"%s","sourceDefinitionId":"%s"}' \
    "${workspace_id}" "${definition_id}")
  ab__curl POST /api/v1/declarative_source_definitions/list_manifests "${body}" \
    | python3 -c '
import sys, json
data = json.load(sys.stdin)
for v in data.get("manifestVersions", []):
    if v.get("isActive"):
        print(v.get("description", "")); break
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
  # source_definitions/update requires workspaceId in the body — Airbyte
  # 1.7+ returns a 500 NPE ("getWorkspaceId(...) must not be null")
  # without it. Resolve the single workspace once.
  local workspace_id
  workspace_id="$(ab_workspace_id)" || return 1
  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId":        sys.argv[1],
  "sourceDefinitionId": sys.argv[2],
  "dockerImageTag":     sys.argv[3],
}))
' "${workspace_id}" "${definition_id}" "${tag}")
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
# ab_get_connection <connection_id>
# POST /api/v1/connections/get — returns the full connection object.
# Used by ab_update_connection_sync_catalog to fetch the existing body so
# only sync_catalog is overwritten.
# ---------------------------------------------------------------------------
ab_get_connection() {
  local connection_id="$1"
  local body
  body=$(printf '{"connectionId":"%s"}' "${connection_id}")
  ab__curl POST /api/v1/connections/get "${body}"
}

# ---------------------------------------------------------------------------
# ab_update_connection_sync_catalog <connection_id> <sync_catalog_json>
# POST /api/v1/connections/update with the existing connection body but the
# new sync_catalog merged in. Per ADR-0015: called on every republish to
# pick up new streams/fields the connector advertises. State (per-stream
# cursors) is preserved by Airbyte across `connections/update` since state
# is keyed on (connectionId, streamName) — only sync_catalog shape changes,
# not the connection identity.
# ---------------------------------------------------------------------------
ab_update_connection_sync_catalog() {
  local connection_id="$1"
  local sync_catalog_json="$2"
  local current body
  if ! current="$(ab_get_connection "${connection_id}")"; then
    return 1
  fi
  body=$(SYNC_CATALOG_ENV="${sync_catalog_json}" python3 -c '
import sys, os, json
conn = json.load(sys.stdin)
conn["syncCatalog"] = json.loads(os.environ["SYNC_CATALOG_ENV"])
# /connections/update accepts a subset; keep only the fields Airbyte
# requires for the update call. Forwarding the full response body works on
# current versions but pruning to the documented update schema is safer.
allowed = {
  "connectionId", "syncCatalog", "schedule", "scheduleData", "scheduleType",
  "status", "name", "namespaceDefinition", "namespaceFormat", "prefix",
  "operationIds", "resourceRequirements", "sourceCatalogId",
  "geography", "notifySchemaChanges", "notifySchemaChangesByEmail",
  "nonBreakingChangesPreference", "backfillPreference", "tags",
}
payload = {k: v for k, v in conn.items() if k in allowed}
print(json.dumps(payload))
' <<<"${current}")
  ab__curl POST /api/v1/connections/update "${body}"
}

# ---------------------------------------------------------------------------
# ab_destination_definition_id_by_name <name>
# Looks up a built-in destination_definition by name (e.g. "Clickhouse",
# "Postgres"). Returns the UUID or empty + non-zero exit if not found.
# Reconcile uses this once at install time to find the Clickhouse
# connector definition before creating the Bronze destination.
# ---------------------------------------------------------------------------
ab_destination_definition_id_by_name() {
  local target="$1"
  local workspace_id
  workspace_id="$(ab_workspace_id)"
  ab__curl POST /api/v1/destination_definitions/list_for_workspace \
    "$(printf '{"workspaceId":"%s"}' "${workspace_id}")" \
    | python3 -c '
import sys, json
target = sys.argv[1].lower()
data = json.load(sys.stdin)
for d in data.get("destinationDefinitions", []):
    if (d.get("name") or "").lower() == target:
        print(d["destinationDefinitionId"]); sys.exit(0)
sys.exit(1)
' "${target}"
}

# ---------------------------------------------------------------------------
# ab_ensure_destination <name> <definition_id> <config_json>
# Idempotent: list destinations, find by name, return its destinationId.
# If absent — create with the given destinationDefinitionId + connection
# configuration; return the new destinationId. Used by reconcile to own
# the Bronze ClickHouse destination on a fresh Airbyte instance so the
# operator never sees a UUID.
# ---------------------------------------------------------------------------
ab_ensure_destination() {
  local name="$1"
  local definition_id="$2"
  local config_json="$3"
  local workspace_id
  workspace_id="$(ab_workspace_id)"

  local list_resp existing_id
  list_resp="$(ab__curl POST /api/v1/destinations/list \
    "$(printf '{"workspaceId":"%s"}' "${workspace_id}")")"
  existing_id="$(printf '%s' "${list_resp}" | python3 -c '
import sys, json
target = sys.argv[1]
for d in json.load(sys.stdin).get("destinations", []):
    if d.get("name") == target:
        print(d["destinationId"]); sys.exit(0)
' "${name}")"
  if [[ -n "${existing_id}" ]]; then
    printf '%s' "${existing_id}"
    return 0
  fi

  local body
  body=$(python3 -c '
import sys, json
print(json.dumps({
  "workspaceId": sys.argv[1],
  "destinationDefinitionId": sys.argv[2],
  "name": sys.argv[3],
  "connectionConfiguration": json.loads(sys.argv[4]),
}))
' "${workspace_id}" "${definition_id}" "${name}" "${config_json}")
  ab__curl POST /api/v1/destinations/create "${body}" \
    | python3 -c 'import sys,json;print(json.load(sys.stdin)["destinationId"])'
}

# ---------------------------------------------------------------------------
# ab_resolve_tags <workspace_id> <names_json>
# names_json: JSON array of strings, e.g. '["insight","cfg-hash:abc"]'.
# Resolves each name to an existing Tag in the workspace; for any name
# not found, creates a new Tag (color is fixed; reconcile owns these
# tags). Echoes a JSON array of Tag objects on stdout, suitable for
# `connections/create.tags` and the PATCH body for tag updates.
# ---------------------------------------------------------------------------
ab_resolve_tags() {
  local workspace_id="$1"
  local names_json="$2"
  [[ -n "${names_json}" && "${names_json}" != "null" ]] || names_json='[]'
  # Iterate names in shell so we reuse ab__curl's auth/token plumbing instead
  # of duplicating it in python (avoids AIRBYTE_TOKEN_FILE assumption).
  local existing
  existing="$(ab__curl POST /api/v1/tags/list \
              "$(printf '{"workspaceId":"%s"}' "${workspace_id}")")"
  local names_lines
  names_lines="$(printf '%s' "${names_json}" \
    | python3 -c 'import sys, json; [print(n) for n in json.load(sys.stdin)]')"

  local out_json='[]'
  while IFS= read -r tag_name; do
    [[ -n "${tag_name}" ]] || continue
    # Try existing first.
    local existing_tag
    existing_tag="$(printf '%s' "${existing}" | python3 -c '
import sys, json
target = sys.argv[1]
data = json.load(sys.stdin)
if isinstance(data, dict):
    data = data.get("tags", data.get("data", []))
for t in (data or []):
    if t.get("name") == target:
        print(json.dumps(t)); break
' "${tag_name}")"
    if [[ -z "${existing_tag}" ]]; then
      local create_body created
      create_body="$(printf '{"workspaceId":"%s","name":"%s","color":"888888"}' \
                       "${workspace_id}" "${tag_name}")"
      created="$(ab__curl POST /api/v1/tags/create "${create_body}")" || return 1
      existing_tag="$(printf '%s' "${created}" | python3 -c '
import sys, json
d = json.load(sys.stdin)
if isinstance(d, dict) and "tagId" not in d and "tag" in d:
    d = d["tag"]
print(json.dumps(d))')"
    fi
    out_json="$(python3 -c '
import sys, json
arr = json.loads(sys.argv[1])
arr.append(json.loads(sys.argv[2]))
print(json.dumps(arr))
' "${out_json}" "${existing_tag}")"
  done <<<"${names_lines}"
  printf '%s' "${out_json}"
}

# ---------------------------------------------------------------------------
# ab_create_connection <workspace_id> <source_id> <destination_id> <name> \
#                      <schedule_json> <tags_json> [sync_catalog_json]
# POST /api/v1/connections/create — private v1 schema requires Tag objects
# on `tags`. tags_json is a JSON array of Tag objects (with tagId, name,
# workspaceId, color). Use ab_resolve_tags to turn a string array of tag
# names into the right shape before calling this.
# schedule_json: e.g. '{"scheduleType":"manual"}' or
#                '{"scheduleType":"cron","cronExpression":"0 2 * * *"}'.
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
  local sync_catalog_json="${7:-}"
  local namespace_format="${8:-}"
  [[ -n "${sync_catalog_json}" ]] || sync_catalog_json='{"streams":[]}'
  [[ -n "${tags_json}" && "${tags_json}" != "null" ]] || tags_json='[]'
  # schedule_json is the Airbyte 1.7+ schedule shape: flat object with
  # `scheduleType` (always) and `scheduleData` (for cron). Reconcile
  # passes `{"scheduleType":"manual"}` because Argo CronWorkflow is the
  # sole sync scheduler; the JSON is spliced onto the top of the payload
  # so the keys land where ConnectionCreate expects them.
  #
  # namespace_format is the literal `namespaceFormat` value (e.g.
  # `bronze_m365`); when non-empty we set namespaceDefinition=customformat
  # so each connector lands in its own ClickHouse schema. Empty leaves
  # Airbyte's defaults (which fall back to the destination's `database`
  # config — usually `default` — landing every connector's streams in
  # one undifferentiated schema).
  local body
  body=$(python3 -c '
import sys, json
payload = {
  "workspaceId":   sys.argv[1],
  "sourceId":      sys.argv[2],
  "destinationId": sys.argv[3],
  "name":          sys.argv[4],
  "tags":          json.loads(sys.argv[6]),
  "syncCatalog":   json.loads(sys.argv[7]),
  "status":        "active",
}
schedule = json.loads(sys.argv[5]) or {}
payload.update(schedule)
ns_fmt = sys.argv[8]
if ns_fmt:
    payload["namespaceDefinition"] = "customformat"
    payload["namespaceFormat"]     = ns_fmt
print(json.dumps(payload))
' "${workspace_id}" "${source_id}" "${destination_id}" "${name}" \
  "${schedule_json}" "${tags_json}" "${sync_catalog_json}" "${namespace_format}")
  ab__curl POST /api/v1/connections/create "${body}"
}

# ---------------------------------------------------------------------------
# ab_patch_connection_tags <connection_id> <tags_json>
# PATCH /api/public/v1/connections/{id} — updates only the tags field.
# tags_json: JSON array of Tag objects (use ab_resolve_tags upstream).
# ---------------------------------------------------------------------------
ab_patch_connection_tags() {
  local connection_id="$1"
  local tags_json="$2"
  [[ -n "${tags_json}" && "${tags_json}" != "null" ]] || tags_json='[]'
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
# state_json: the FULL state object as returned by ab_get_state. The
# endpoint requires a top-level envelope `{connectionId, connectionState}`
# — injecting connectionId *inside* the state object instead leaves the
# wrapper missing and the API either rejects the body or silently
# restores empty state, breaking cursor preservation across recreate.
# ---------------------------------------------------------------------------
ab_create_or_update_state() {
  local connection_id="$1"
  local state_json="$2"
  local body
  body=$(python3 -c '
import sys, json
payload = {
    "connectionId": sys.argv[1],
    "connectionState": json.loads(sys.argv[2]),
}
print(json.dumps(payload))
' "${connection_id}" "${state_json}")
  ab__curl POST /api/v1/state/create_or_update "${body}"
}
