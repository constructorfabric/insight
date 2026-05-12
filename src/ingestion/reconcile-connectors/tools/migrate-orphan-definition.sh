#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# migrate-orphan-definition.sh <connector_name>
#
# State-preserving recreate of a single nocode connector whose
# source_definition has no linked builder project (orphan from legacy
# create_custom path). reconcile_definitions WARN+skips orphans now;
# operators run THIS script to safely migrate them.
#
# Flow (idempotent at API level — reusing existing ab_* helpers):
#   1. Resolve the orphan definition (custom: true) by connector_name.
#   2. List sources attached to it; export per-connection state.
#   3. Create new builder project + publish manifest → new_def_id.
#   4. For each source: read its connectionConfiguration, delete (cascades
#      connections), recreate against new_def_id, then recreate each
#      preserved connection on the new source and restore its state.
#   5. Patch reconcile tags on each new connection.
#   6. Delete the old orphan source_definition.
#
# Required env: KUBECONFIG, INSIGHT_NAMESPACE, AIRBYTE_URL,
#               INSIGHT_TENANT_ID, CONNECTORS_DIR.
# Workspace UUID is auto-discovered via ab_workspace_id (ADR-0009).
# Destination is resolved via reconcile_resolve_destination_id (ADR-0012):
# either RECONCILE_DESTINATION_ID is set (legacy override) or the
# RECONCILE_DEST_CLICKHOUSE_* env triplet must be present.
# ---------------------------------------------------------------------------

set -euo pipefail

: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set}"
: "${AIRBYTE_URL:?AIRBYTE_URL must be set}"
: "${INSIGHT_TENANT_ID:?INSIGHT_TENANT_ID must be set}"
: "${CONNECTORS_DIR:?CONNECTORS_DIR must be set (e.g. src/ingestion/connectors)}"

if [[ $# -ne 1 || -z "${1:-}" ]]; then
  printf 'Usage: %s <connector_name>\n' "$0" >&2
  exit 2
fi
CONNECTOR_NAME="$1"

SCRIPT_DIR="$( cd "$(dirname "${BASH_SOURCE[0]}")" && pwd )"
LIB_DIR="$( cd "${SCRIPT_DIR}/../lib" && pwd )"
PY_DIR="$( cd "${SCRIPT_DIR}/../python" && pwd )"

# shellcheck source=../lib/log.sh
source "${LIB_DIR}/log.sh"
# shellcheck source=../lib/airbyte.sh
source "${LIB_DIR}/airbyte.sh"
# shellcheck source=../lib/discover.sh
source "${LIB_DIR}/discover.sh"
# reconcile.sh provides reconcile_resolve_destination_id (and the helpers
# it calls); pulling it in here keeps migrate-orphan in lockstep with the
# main loop's resolution policy.
# shellcheck source=../lib/reconcile.sh
source "${LIB_DIR}/reconcile.sh"

WORKSPACE_ID="$(ab_workspace_id)"
RECONCILE_DESTINATION_ID="$(reconcile_resolve_destination_id "${CONNECTOR_NAME}")"

# 1. Resolve orphan definition by name + custom:true.
DEFS_JSON="$(ab_list_definitions "${WORKSPACE_ID}")"
OLD_DEF_ID="$(printf '%s' "${DEFS_JSON}" | python3 -c '
import sys, json
target = sys.argv[1]
for d in json.load(sys.stdin):
    if d.get("name") == target and d.get("custom") is True:
        print(d.get("sourceDefinitionId", "")); break
' "${CONNECTOR_NAME}")"

if [[ -z "${OLD_DEF_ID}" ]]; then
  printf 'no custom definition found for connector=%s — nothing to migrate\n' \
    "${CONNECTOR_NAME}" >&2
  exit 1
fi

BUILDER_ID="$(ab_builder_find_by_definition "${WORKSPACE_ID}" "${OLD_DEF_ID}")"
if [[ -n "${BUILDER_ID}" ]]; then
  printf 'definition %s already has builder project %s — not orphan; aborting\n' \
    "${OLD_DEF_ID}" "${BUILDER_ID}" >&2
  exit 1
fi

MANIFEST_PATH="${CONNECTORS_DIR}/${CONNECTOR_NAME}/connector.yaml"
if [[ ! -f "${MANIFEST_PATH}" ]]; then
  printf 'no connector.yaml at %s\n' "${MANIFEST_PATH}" >&2
  exit 1
fi

# 2. List sources attached to old definition; snapshot config + connections + state.
SOURCES_JSON="$(ab_list_sources "${WORKSPACE_ID}")"
CONNS_JSON="$(ab_list_connections "${WORKSPACE_ID}")"

WORKDIR="$(mktemp -d -t orphan-mig.XXXXXX)"
trap 'rm -rf "${WORKDIR}"' EXIT

printf '%s' "${SOURCES_JSON}" | python3 -c '
import sys, json
target = sys.argv[1]
for s in json.load(sys.stdin):
    if s.get("sourceDefinitionId") == target:
        print(json.dumps(s))
' "${OLD_DEF_ID}" > "${WORKDIR}/old_sources.ndjson"

SRC_COUNT="$(wc -l < "${WORKDIR}/old_sources.ndjson" | tr -d ' ')"
printf 'migrating connector=%s old_def=%s sources=%s\n' \
  "${CONNECTOR_NAME}" "${OLD_DEF_ID}" "${SRC_COUNT}" >&2

# Snapshot per-source connections + state.
: > "${WORKDIR}/conn_states.ndjson"
while IFS= read -r src_line; do
  [[ -n "${src_line}" ]] || continue
  src_id="$(printf '%s' "${src_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["sourceId"])')"
  src_name="$(printf '%s' "${src_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["name"])')"
  printf '%s' "${CONNS_JSON}" | python3 -c '
import sys, json
target_src = sys.argv[1]
for c in json.load(sys.stdin):
    if c.get("sourceId") == target_src:
        print(json.dumps(c))
' "${src_id}" > "${WORKDIR}/conns_${src_id}.ndjson"
  while IFS= read -r conn_line; do
    [[ -n "${conn_line}" ]] || continue
    conn_id="$(printf '%s' "${conn_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["connectionId"])')"
    # Fail fast on state export errors — silently substituting `{}` would
    # let the migration recreate connections without their cursors and
    # silently restart streams from scratch (data loss / duplicates).
    if ! state_blob="$(ab_get_state "${conn_id}")"; then
      printf 'ab_get_state failed for connection %s — aborting migration to preserve cursors\n' \
        "${conn_id}" >&2
      exit 1
    fi
    python3 -c '
import sys, json
src_name = sys.argv[1]
conn = json.loads(sys.argv[2])
state = json.loads(sys.argv[3])
print(json.dumps({"src_name": src_name, "conn": conn, "state": state}))
' "${src_name}" "${conn_line}" "${state_blob}" >> "${WORKDIR}/conn_states.ndjson"
  done < "${WORKDIR}/conns_${src_id}.ndjson"
done < "${WORKDIR}/old_sources.ndjson"

# 3. Create new builder project + publish.
DESC_VERSION="$(python3 "${PY_DIR}/parse_descriptor.py" \
  --descriptor "${CONNECTORS_DIR}/${CONNECTOR_NAME}/descriptor.yaml" \
  --field version 2>/dev/null || printf 'migrated')"
NEW_BUILDER_ID="$(ab_builder_create_with_manifest \
  "${WORKSPACE_ID}" "${CONNECTOR_NAME}" "${MANIFEST_PATH}")"
[[ -n "${NEW_BUILDER_ID}" ]] || { printf 'builder create failed\n' >&2; exit 1; }
NEW_DEF_ID="$(ab_builder_publish \
  "${WORKSPACE_ID}" "${NEW_BUILDER_ID}" "${CONNECTOR_NAME}" \
  "${DESC_VERSION}" "${MANIFEST_PATH}")"
[[ -n "${NEW_DEF_ID}" ]] || { printf 'builder publish failed\n' >&2; exit 1; }
printf 'created new builder=%s new_def=%s\n' "${NEW_BUILDER_ID}" "${NEW_DEF_ID}" >&2

# 4. Per source: CREATE the replacement first under a temporary name
#    (Airbyte forbids two sources with the same name in one workspace),
#    then we'll rename + delete the old one only after the new resource
#    is fully provisioned. Old source is kept until the very last step
#    (#6) so a mid-flow failure leaves the cluster in a recoverable
#    state instead of losing connections + cursors.
declare -A NEW_SOURCE_BY_NAME=()
declare -A OLD_SOURCE_ID_BY_NAME=()
declare -A NEW_SOURCE_TMP_NAME_BY_NAME=()
SRC_MIGRATED=0
while IFS= read -r src_line; do
  [[ -n "${src_line}" ]] || continue
  old_src_id="$(printf '%s' "${src_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["sourceId"])')"
  src_name="$(printf '%s' "${src_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["name"])')"
  cfg_json="$(printf '%s' "${src_line}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin).get("connectionConfiguration",{})))')"
  tmp_name="${src_name}-migrating-$$"
  if ! new_src_json="$(ab_create_source "${WORKSPACE_ID}" "${NEW_DEF_ID}" \
        "${tmp_name}" "${cfg_json}")"; then
    printf '  ERROR: ab_create_source failed for %s; old source %s NOT touched, aborting\n' \
      "${src_name}" "${old_src_id}" >&2
    exit 1
  fi
  new_src_id="$(printf '%s' "${new_src_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("sourceId",""))')"
  if [[ -z "${new_src_id}" ]]; then
    printf '  ERROR: empty sourceId from ab_create_source for %s; old source %s NOT touched, aborting\n' \
      "${src_name}" "${old_src_id}" >&2
    exit 1
  fi
  NEW_SOURCE_BY_NAME["${src_name}"]="${new_src_id}"
  OLD_SOURCE_ID_BY_NAME["${src_name}"]="${old_src_id}"
  NEW_SOURCE_TMP_NAME_BY_NAME["${src_name}"]="${tmp_name}"
  SRC_MIGRATED=$((SRC_MIGRATED + 1))
  printf '  source replacement created (under tmp name): %s old=%s new=%s\n' \
    "${src_name}" "${old_src_id}" "${new_src_id}" >&2
done < "${WORKDIR}/old_sources.ndjson"

# 5. Per preserved connection: recreate on matching new source + restore state.
CONN_MIGRATED=0
while IFS= read -r snap_line; do
  [[ -n "${snap_line}" ]] || continue
  src_name="$(printf '%s' "${snap_line}" | python3 -c 'import sys,json;print(json.load(sys.stdin)["src_name"])')"
  new_src_id="${NEW_SOURCE_BY_NAME[${src_name}]:-}"
  if [[ -z "${new_src_id}" ]]; then
    printf '  WARN: no new source for %s — skip connection\n' "${src_name}" >&2
    continue
  fi
  conn_json="$(printf '%s' "${snap_line}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["conn"]))')"
  state_json="$(printf '%s' "${snap_line}" | python3 -c 'import sys,json;print(json.dumps(json.load(sys.stdin)["state"]))')"
  conn_name="$(printf '%s' "${conn_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("name",""))')"
  schedule_json="$(printf '%s' "${conn_json}" | python3 -c '
import sys, json
c = json.load(sys.stdin)
sched = c.get("schedule")
if isinstance(sched, dict) and sched:
    print(json.dumps(sched))
else:
    print(json.dumps({"scheduleType":"manual"}))
')"
  sync_catalog="$(printf '%s' "${conn_json}" | python3 -c '
import sys, json
c = json.load(sys.stdin)
print(json.dumps(c.get("syncCatalog") or {"streams":[]}))
')"
  cfg_hash="$(printf '%s' "${conn_json}" | python3 -c '
import sys, json
c = json.load(sys.stdin)
for t in c.get("tags") or []:
    name = t.get("name", t) if isinstance(t, dict) else t
    if isinstance(name, str) and name.startswith("cfg-hash:"):
        print(name.split(":",1)[1]); break
')"
  tags_json="$(python3 -c 'import sys, json; h=sys.argv[1]; print(json.dumps(["insight"] + ([f"cfg-hash:{h}"] if h else [])))' "${cfg_hash}")"
  new_conn_json="$(ab_create_connection "${WORKSPACE_ID}" "${new_src_id}" \
    "${RECONCILE_DESTINATION_ID}" "${conn_name}" "${schedule_json}" \
    "${tags_json}" "${sync_catalog}")"
  new_conn_id="$(printf '%s' "${new_conn_json}" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("connectionId",""))')"
  if [[ -n "${new_conn_id}" && "${state_json}" != '{}' ]]; then
    ab_create_or_update_state "${new_conn_id}" "${state_json}" >/dev/null
  fi
  if [[ -n "${new_conn_id}" ]]; then
    ab_patch_connection_tags "${new_conn_id}" "${tags_json}" >/dev/null
  fi
  CONN_MIGRATED=$((CONN_MIGRATED + 1))
  printf '  connection migrated: %s new=%s\n' "${conn_name}" "${new_conn_id}" >&2
done < "${WORKDIR}/conn_states.ndjson"

# 6. Cut over: delete old sources (cascades the now-orphan old
#    connections), then rename the temp-named replacements to their
#    canonical names. This is the only destructive step; everything
#    above creates new resources without touching the originals.
for src_name in "${!OLD_SOURCE_ID_BY_NAME[@]}"; do
  old_src_id="${OLD_SOURCE_ID_BY_NAME[${src_name}]}"
  new_src_id="${NEW_SOURCE_BY_NAME[${src_name}]}"
  ab_delete_source "${old_src_id}" >/dev/null
  # `ab_update_source` requires a connectionConfiguration; for a rename
  # we want to PATCH only `name`. Use partial_update endpoint directly.
  ab__curl POST /api/v1/sources/partial_update \
    "$(python3 -c '
import sys, json
print(json.dumps({"sourceId": sys.argv[1], "name": sys.argv[2]}))
' "${new_src_id}" "${src_name}")" >/dev/null
  printf '  cutover: deleted old %s (%s), renamed new %s -> %s\n' \
    "${src_name}" "${old_src_id}" "${new_src_id}" "${src_name}" >&2
done

# 7. Drop the old orphan definition (no sources reference it now).
ab_delete_source_definition "${OLD_DEF_ID}" >/dev/null
printf 'deleted old orphan definition: %s\n' "${OLD_DEF_ID}" >&2

# 8. Summary.
printf 'migration done: connector=%s sources_migrated=%s connections_migrated=%s state_restored=yes\n' \
  "${CONNECTOR_NAME}" "${SRC_MIGRATED}" "${CONN_MIGRATED}"
