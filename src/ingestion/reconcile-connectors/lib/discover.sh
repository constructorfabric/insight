#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-featstatus-reconcile — desired-state discovery
# @cpt-algo:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1
# @cpt-algo:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1
#
# Reads connectors/*/descriptor.yaml and K8s Secrets in namespace `data`
# (label app.kubernetes.io/part-of=insight) to build the "desired state"
# the reconcile engine drives Airbyte toward. Sourced — never executed
# directly. All values are streamed as TSV on stdout so callers can pipe
# through `while IFS=$'\t' read ...`.
#
# Function naming: `disc_*` prefix; lowercase.
# ---------------------------------------------------------------------------

# NOTE: this file is sourced; no top-level `set -euo pipefail` (leaks into
# interactive shells and breaks PROMPT_COMMAND on unset vars).

: "${INSIGHT_NAMESPACE:?INSIGHT_NAMESPACE must be set, e.g. insight}"
: "${CONNECTORS_DIR:?CONNECTORS_DIR must be set, typically src/ingestion/connectors}"

# Resolve project layout relative to this file.
_DISC_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${INGESTION_DIR:=$(cd "${_DISC_LIB_DIR}/../.." && pwd)}"  # RULE-DEFAULTS-OK: derived path, not a config input
: "${SECRET_LABEL_SELECTOR:=app.kubernetes.io/part-of=insight}"  # RULE-DEFAULTS-OK: project-fixed label selector, not operator-tunable

# ---------------------------------------------------------------------------
# disc_load_descriptors
# Walks ${CONNECTORS_DIR}/*/*/descriptor.yaml and emits TSV per descriptor:
#   name<TAB>connector_dir<TAB>version<TAB>type<TAB>cdk_image<TAB>enrich_image<TAB>dbt_select
#     (type = nocode|cdk; cdk_image is the full Docker reference sourced from
#      `descriptor.images.cdk.image` per ADR-0016, empty for nocode or absent;
#      enrich_image is sourced from `descriptor.images.enrich.image`, empty
#      otherwise; dbt_select is the dbt selector passed to the
#      ingestion-pipeline.)
# This is the single bash-side reader of descriptor.yaml — every other lib /
# python helper consumes values via this TSV rather than re-reading the file.
# Skips files missing `name` or `version`; logs a WARN to stderr per skip.
# ---------------------------------------------------------------------------
disc_load_descriptors() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1:inst-ds-descriptor
  local desc
  while IFS= read -r -d '' desc; do
    local connector_dir
    connector_dir="$(dirname "${desc}")"
    python3 - "${desc}" "${connector_dir}" <<'PY'
import sys, yaml
path, connector_dir = sys.argv[1], sys.argv[2]
try:
    with open(path) as f:
        d = yaml.safe_load(f) or {}
except Exception as exc:  # noqa: BLE001
    sys.stderr.write(f"WARN: cannot parse {path}: {exc}\n"); sys.exit(0)
name = d.get("name")
version = d.get("version")
ctype = d.get("type", "nocode")
# Image refs are sourced from the map-style images: block per ADR-0016;
# legacy top-level cdk_image / enrich_image fields no longer exist.
images = d.get("images") or {}
cdk_entry = images.get("cdk") or {}
enrich_entry = images.get("enrich") or {}
cdk_image = (cdk_entry.get("image") or "") if isinstance(cdk_entry, dict) else ""
enrich_image = (enrich_entry.get("image") or "") if isinstance(enrich_entry, dict) else ""
dbt_select = d.get("dbt_select", "") or ""
if not name:
    sys.stderr.write(f"WARN: descriptor missing name, skip: {path}\n"); sys.exit(0)
if version is None:
    sys.stderr.write(f"WARN: descriptor missing version, skip: {path}\n"); sys.exit(0)
print(f"{name}\t{connector_dir}\t{version}\t{ctype}\t{cdk_image}\t{enrich_image}\t{dbt_select}")
PY
  done < <(find "${CONNECTORS_DIR}" -name 'descriptor.yaml' -print0 2>/dev/null)
  # @cpt-end:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1:inst-ds-descriptor
}

# ---------------------------------------------------------------------------
# disc_load_secrets [namespace]
# `kubectl get secret -n NS -l SECRET_LABEL_SELECTOR -o json` and emits TSV:
#   connector_label<TAB>source_id<TAB>secret_name<TAB>cfg_hash
# Secrets without the `insight.cyberfabric.com/connector` annotation are
# skipped with a WARN to stderr (Decision #8: bad/unlabelled → WARN+skip).
# ---------------------------------------------------------------------------
disc_load_secrets() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1:inst-ds-list-secrets
  local namespace="${1:-${INSIGHT_NAMESPACE}}"
  local json
  if ! json="$(kubectl -n "${namespace}" get secret \
        -l "${SECRET_LABEL_SELECTOR}" -o json 2>/dev/null)"; then
    printf 'disc_load_secrets: kubectl get secret failed in ns %s\n' "${namespace}" >&2
    return 1
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1:inst-ds-list-secrets
  # @cpt-begin:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1:inst-ds-loop
  # Canonical hash routine lives in python/extract_secret_loop.py and reuses
  # the same SHA-256 policy as compute_cfg_hash.py — keep them in lockstep.
  printf '%s' "${json}" | python3 "${_DISC_LIB_DIR}/../python/extract_secret_loop.py"
  # @cpt-end:cpt-insightspec-algo-reconcile-discover-secrets-v2:p1:inst-ds-loop
}

# ---------------------------------------------------------------------------
# disc_compute_cfg_hash <secret_name> [namespace]
# Fetches the named secret's `.data` map and prints the canonical sha256
# hex hash. Hash policy: keys sorted lexicographically, base64 values
# verbatim, no whitespace (matches disc_load_secrets inline form so that
# a per-secret recompute is byte-identical to the bulk form).
# ---------------------------------------------------------------------------
disc_compute_cfg_hash() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-decode
  local secret_name="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  local data_json
  if ! data_json="$(kubectl -n "${namespace}" get secret "${secret_name}" \
        -o jsonpath='{.data}' 2>/dev/null)"; then
    printf 'disc_compute_cfg_hash: kubectl get secret %s failed in ns %s\n' \
      "${secret_name}" "${namespace}" >&2
    return 1
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-decode
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-canonical
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-sha256
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-return
  [[ -n "${data_json}" && "${data_json}" != "null" ]] || data_json='{}'
  printf '%s' "${data_json}" | python3 "${_DISC_LIB_DIR}/../python/compute_cfg_hash.py"
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-return
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-sha256
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-canonical
}

# ---------------------------------------------------------------------------
# disc_match_descriptor_to_secret <connector_name> [namespace]
# Echoes the K8s Secret name whose annotation
# `insight.cyberfabric.com/connector` == <connector_name>.
# Exit:
#   0  match found; name on stdout.
#   1  no Secret matches (genuine "missing"; safe for cascade-delete).
#   2  kubectl/API failure (transient). Caller MUST NOT treat as
#      "missing" — destructive cascade-delete callers gate on this.
# ---------------------------------------------------------------------------
disc_match_descriptor_to_secret() {
  local connector_name="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  # Run kubectl out-of-pipe so its rc is observable. Otherwise a
  # transient API failure ($? from inside `cmd | python`) gets masked
  # and the caller's cascade-delete fires on a healthy secret.
  local list_json kubectl_rc
  list_json="$(kubectl -n "${namespace}" get secret \
                -l "${SECRET_LABEL_SELECTOR}" -o json 2>/dev/null)"
  kubectl_rc=$?
  if [[ ${kubectl_rc} -ne 0 ]]; then
    printf 'disc_match_descriptor_to_secret: kubectl failed listing secrets in ns %s (rc=%d)\n' \
      "${namespace}" "${kubectl_rc}" >&2
    return 2
  fi
  local match
  match="$(printf '%s' "${list_json}" | python3 -c '
import sys, json
target = sys.argv[1]
data = json.load(sys.stdin)
for it in data.get("items", []):
    md = it.get("metadata", {})
    annos = md.get("annotations", {}) or {}
    if annos.get("insight.cyberfabric.com/connector") == target:
        print(md.get("name", "")); sys.exit(0)
sys.exit(1)
' "${connector_name}")" || { printf '' ; return 1; }
  printf '%s' "${match}"
}

# ---------------------------------------------------------------------------
# disc_required_fields_for_connector <connector_name>
# Reads `secret.required_fields` from the connector's descriptor.yaml via
# python/parse_descriptor.py. Prints one field name per line on stdout.
# Exit: 0 found, 1 not found (field absent in descriptor).
# ---------------------------------------------------------------------------
disc_required_fields_for_connector() {
  local connector="$1"
  # Resolve descriptor by glob — connectors live under an area directory
  # (e.g. ${CONNECTORS_DIR}/collaboration/m365/), so the slug alone doesn't
  # uniquely locate the descriptor.
  local desc_glob desc_path
  # shellcheck disable=SC2206
  desc_glob=("${CONNECTORS_DIR}"/*/"${connector}"/descriptor.yaml)
  desc_path="${desc_glob[0]}"
  [[ -f "${desc_path}" ]] || return 1
  python3 "${_DISC_LIB_DIR}/../python/parse_descriptor.py" \
    --descriptor "${desc_path}" \
    --field secret.required_fields
}

# ---------------------------------------------------------------------------
# disc_skip_unlabelled <secret_name> [namespace]
# Returns 0 if the named secret carries the `connector` annotation;
# 1 otherwise. Caller WARNs and skips per Decision #8.
# ---------------------------------------------------------------------------
disc_skip_unlabelled() {
  local secret_name="$1"
  local namespace="${2:-${INSIGHT_NAMESPACE}}"
  local val
  val="$(kubectl -n "${namespace}" get secret "${secret_name}" \
          -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/connector}' \
          2>/dev/null || true)"
  [[ -n "${val}" ]]
}
