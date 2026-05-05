#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-feature-reconcile — desired-state discovery
# @cpt-algo:cpt-insightspec-algo-reconcile-discover-secrets:p1
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

set -euo pipefail

# Resolve project layout relative to this file. Callers that need different
# roots can override these vars before sourcing.
_DISC_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
: "${INGESTION_DIR:=$(cd "${_DISC_LIB_DIR}/../.." && pwd)}"
: "${CONNECTORS_DIR:=${INGESTION_DIR}/connectors}"
: "${K8S_NAMESPACE:=data}"
: "${SECRET_LABEL_SELECTOR:=app.kubernetes.io/part-of=insight}"

# ---------------------------------------------------------------------------
# disc_load_descriptors
# Walks ${CONNECTORS_DIR}/*/*/descriptor.yaml and emits TSV per descriptor:
#   name<TAB>connector_dir<TAB>version<TAB>type   (type = nocode|cdk)
# Skips files missing `name` or `version`; logs a WARN to stderr per skip.
# ---------------------------------------------------------------------------
disc_load_descriptors() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-discover-secrets:p1:inst-ds-descriptor
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
if not name:
    sys.stderr.write(f"WARN: descriptor missing name, skip: {path}\n"); sys.exit(0)
if version is None:
    sys.stderr.write(f"WARN: descriptor missing version, skip: {path}\n"); sys.exit(0)
print(f"{name}\t{connector_dir}\t{version}\t{ctype}")
PY
  done < <(find "${CONNECTORS_DIR}" -name 'descriptor.yaml' -print0 2>/dev/null)
  # @cpt-end:cpt-insightspec-algo-reconcile-discover-secrets:p1:inst-ds-descriptor
}

# ---------------------------------------------------------------------------
# disc_load_secrets [namespace]
# `kubectl get secret -n NS -l SECRET_LABEL_SELECTOR -o json` and emits TSV:
#   connector_label<TAB>source_id<TAB>secret_name<TAB>cfg_hash
# Secrets without the `insight.cyberfabric.com/connector` annotation are
# skipped with a WARN to stderr (Decision #8: bad/unlabelled → WARN+skip).
# ---------------------------------------------------------------------------
disc_load_secrets() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-discover-secrets:p1:inst-ds-list-secrets
  local namespace="${1:-${K8S_NAMESPACE}}"
  local json
  if ! json="$(kubectl -n "${namespace}" get secret \
        -l "${SECRET_LABEL_SELECTOR}" -o json 2>/dev/null)"; then
    printf 'disc_load_secrets: kubectl get secret failed in ns %s\n' "${namespace}" >&2
    return 1
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-discover-secrets:p1:inst-ds-list-secrets
  # @cpt-begin:cpt-insightspec-algo-reconcile-discover-secrets:p1:inst-ds-loop
  printf '%s' "${json}" | python3 -c '
import sys, json, hashlib, base64
data = json.load(sys.stdin)
for item in data.get("items", []):
    md = item.get("metadata", {})
    name = md.get("name", "")
    annotations = md.get("annotations", {}) or {}
    connector = annotations.get("insight.cyberfabric.com/connector")
    source_id = annotations.get("insight.cyberfabric.com/source-id")
    if not connector or not source_id:
        sys.stderr.write(f"WARN: secret {name} missing connector/source-id annotation, skip\n")
        continue
    # Canonical hash: keys sorted, base64 values verbatim. Inline impl
    # mirrors disc_compute_cfg_hash so we avoid a kubectl roundtrip per item.
    secret_data = item.get("data", {}) or {}
    canonical = json.dumps(secret_data, sort_keys=True, separators=(",", ":"))
    cfg_hash = hashlib.sha256(canonical.encode()).hexdigest()
    print(f"{connector}\t{source_id}\t{name}\t{cfg_hash}")
'
  # @cpt-end:cpt-insightspec-algo-reconcile-discover-secrets:p1:inst-ds-loop
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
  local namespace="${2:-${K8S_NAMESPACE}}"
  local json
  if ! json="$(kubectl -n "${namespace}" get secret "${secret_name}" -o json 2>/dev/null)"; then
    printf 'disc_compute_cfg_hash: kubectl get secret %s failed in ns %s\n' \
      "${secret_name}" "${namespace}" >&2
    return 1
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-decode
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-canonical
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-sha256
  # @cpt-begin:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-return
  printf '%s' "${json}" | python3 -c '
import sys, json, hashlib
data = json.load(sys.stdin).get("data", {}) or {}
canonical = json.dumps(data, sort_keys=True, separators=(",", ":"))
print(hashlib.sha256(canonical.encode()).hexdigest())
'
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-return
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-sha256
  # @cpt-end:cpt-insightspec-algo-reconcile-compute-cfg-hash:p1:inst-cch-canonical
}

# ---------------------------------------------------------------------------
# disc_match_descriptor_to_secret <connector_name> [namespace]
# Echoes the K8s Secret name whose annotation
# `insight.cyberfabric.com/connector` == <connector_name>. Empty string
# + non-zero exit if no match.
# ---------------------------------------------------------------------------
disc_match_descriptor_to_secret() {
  local connector_name="$1"
  local namespace="${2:-${K8S_NAMESPACE}}"
  local match
  match="$(kubectl -n "${namespace}" get secret \
            -l "${SECRET_LABEL_SELECTOR}" -o json 2>/dev/null \
          | python3 -c '
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
# disc_skip_unlabelled <secret_name> [namespace]
# Returns 0 if the named secret carries the `connector` annotation;
# 1 otherwise. Caller WARNs and skips per Decision #8.
# ---------------------------------------------------------------------------
disc_skip_unlabelled() {
  local secret_name="$1"
  local namespace="${2:-${K8S_NAMESPACE}}"
  local val
  val="$(kubectl -n "${namespace}" get secret "${secret_name}" \
          -o jsonpath='{.metadata.annotations.insight\.cyberfabric\.com/connector}' \
          2>/dev/null || true)"
  [[ -n "${val}" ]]
}
