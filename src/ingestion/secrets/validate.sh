#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# @cpt:cpt-insightspec-feature-reconcile — secrets validate
# @cpt-algo:cpt-insightspec-algo-reconcile-validate-secrets:p2
#
# Read-only audit of the K8s Secrets that feed Airbyte sources. For each
# `secrets/connectors/<connector>.yaml.example` we check:
#   - corresponding cluster Secret exists (in ns `data`)
#   - has the required label `app.kubernetes.io/part-of=insight`
#   - has the required `insight.cyberfabric.com/connector` annotation
#   - has every key declared in the `.example` `stringData`
#   - if a OnePasswordItem CR exists with the same name, its labels +
#     annotations match the child Secret's. Per Decision #9 the operator
#     copies labels but NOT custom annotations, so the source of truth
#     for annotations is the CR — we report drift on each side.
#
# Exit codes:
#   0 — every connector PASS or only WARNings
#   1 — infrastructure error (kubectl not configured, etc.)
#   2 — at least one ERROR
#
# Usage:
#   ./validate.sh                # validate every connector with .yaml.example
#   ./validate.sh --connector m365
# ---------------------------------------------------------------------------

set -euo pipefail

_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_INGESTION_DIR="$(cd "${_SCRIPT_DIR}/.." && pwd)"

# shellcheck source=../airbyte-toolkit/lib/discover.sh
source "${_INGESTION_DIR}/airbyte-toolkit/lib/discover.sh"

EXAMPLES_DIR="${_SCRIPT_DIR}/connectors"
NAMESPACE="${K8S_NAMESPACE:-data}"

# Globals updated by valsec_check_*; valsec_main reads them at exit.
_VALSEC_ERRORS=0
_VALSEC_WARNINGS=0

# ---------------------------------------------------------------------------
# valsec_report <connector_name> <status> <message>
# Single line per finding. Status one of: PASS | WARN | ERROR | INFO.
# ---------------------------------------------------------------------------
valsec_report() {
  local connector_name="$1"
  local status="$2"
  local message="$3"
  printf '[%s] %-10s %s\n' "${status}" "${connector_name}" "${message}"
  case "${status}" in
    ERROR) _VALSEC_ERRORS=$((_VALSEC_ERRORS + 1)) ;;
    WARN)  _VALSEC_WARNINGS=$((_VALSEC_WARNINGS + 1)) ;;
  esac
}

# ---------------------------------------------------------------------------
# valsec_check_secret_vs_example <connector_name>
# Compares stringData keys of `<connector>.yaml.example` to the live
# Secret's data keys. Values are NOT compared. Reports missing keys and
# extra keys; missing required label / annotation; missing Secret entirely.
# ---------------------------------------------------------------------------
valsec_check_secret_vs_example() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-parse
  local connector_name="$1"
  local example_path="${EXAMPLES_DIR}/${connector_name}.yaml.example"
  if [[ ! -f "${example_path}" ]]; then
    valsec_report "${connector_name}" "ERROR" "no example at ${example_path}"
    return
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-parse

  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-find-secret
  local expected_name
  expected_name="$(python3 -c '
import sys, yaml
with open(sys.argv[1]) as f:
    d = yaml.safe_load(f) or {}
print((d.get("metadata") or {}).get("name", ""))
' "${example_path}")"
  if [[ -z "${expected_name}" ]]; then
    valsec_report "${connector_name}" "ERROR" "example metadata.name missing"
    return
  fi
  local secret_json
  if ! secret_json="$(kubectl -n "${NAMESPACE}" get secret "${expected_name}" \
                       -o json 2>/dev/null)"; then
    # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-warn-missing
    valsec_report "${connector_name}" "WARN" \
      "Secret ${expected_name} not deployed in ns ${NAMESPACE}"
    # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-warn-missing
    return
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-find-secret

  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-if-key-missing
  # Optional-fields contract: example may declare
  #   metadata.annotations['insight.cyberfabric.com/optional-fields']
  # as a comma-separated list of stringData keys that may be omitted in
  # the live Secret without triggering ERROR (treated as INFO if absent).
  local diff_report
  diff_report="$(python3 -c '
import sys, json, yaml
example_path = sys.argv[1]
secret = json.load(sys.stdin)
with open(example_path) as f:
    expected = yaml.safe_load(f) or {}
expected_keys = set((expected.get("stringData") or {}).keys())
expected_labels = set((expected.get("metadata", {}).get("labels") or {}).keys())
example_annos = expected.get("metadata", {}).get("annotations") or {}
optional_csv = example_annos.get("insight.cyberfabric.com/optional-fields", "") or ""
optional_keys = {k.strip() for k in optional_csv.split(",") if k.strip()}
# The annotation itself is meta — never expected on the live Secret.
expected_annos = set(example_annos.keys()) - {"insight.cyberfabric.com/optional-fields"}
actual_keys = set((secret.get("data") or {}).keys())
actual_labels = set((secret.get("metadata", {}).get("labels") or {}).keys())
actual_annos = set((secret.get("metadata", {}).get("annotations") or {}).keys())
required_keys = expected_keys - optional_keys
print(json.dumps({
  "missing_keys": sorted(required_keys - actual_keys),
  "missing_optional_keys": sorted(optional_keys - actual_keys),
  "extra_keys": sorted(actual_keys - expected_keys),
  "missing_labels": sorted(expected_labels - actual_labels),
  "missing_annotations": sorted(expected_annos - actual_annos),
}))
' "${example_path}" <<<"${secret_json}")"
  local findings
  findings="$(python3 -c '
import sys, json
diff = json.loads(sys.argv[1])
errs = []
infos = []
warns = []
if diff["missing_keys"]:
    errs.append(("missing keys", diff["missing_keys"]))
if diff["missing_labels"]:
    errs.append(("missing labels", diff["missing_labels"]))
if diff["missing_annotations"]:
    errs.append(("missing annotations", diff["missing_annotations"]))
if diff["missing_optional_keys"]:
    infos.append(("optional keys absent", diff["missing_optional_keys"]))
if diff["extra_keys"]:
    warns.append(("extra keys not in example", diff["extra_keys"]))
for label, vals in errs:
    print(f"ERROR\t{label}: {vals}")
for label, vals in warns:
    print(f"WARN\t{label}: {vals}")
for label, vals in infos:
    print(f"INFO\t{label}: {vals}")
if not errs and not warns and not infos:
    print("PASS\tkeys/labels/annotations match example")
' "${diff_report}")"
  while IFS=$'\t' read -r status message; do
    [[ -n "${status}" ]] || continue
    valsec_report "${connector_name}" "${status}" "${message}"
  done <<<"${findings}"
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-if-key-missing
}

# ---------------------------------------------------------------------------
# valsec_check_op_item_vs_secret <connector_name>
# When a OnePasswordItem CR exists for the connector's Secret, compares
# CR labels and annotations to the child Secret. Per Decision #9 the
# 1Password operator copies labels but NOT annotations, so ANY difference
# in annotations is reported (CR is source of truth) and label drift is
# reported as ERROR.
# ---------------------------------------------------------------------------
valsec_check_op_item_vs_secret() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-find-cr
  local connector_name="$1"
  local example_path="${EXAMPLES_DIR}/${connector_name}.yaml.example"
  [[ -f "${example_path}" ]] || return 0
  local expected_name
  expected_name="$(python3 -c '
import sys, yaml
with open(sys.argv[1]) as f:
    d = yaml.safe_load(f) or {}
print((d.get("metadata") or {}).get("name", ""))
' "${example_path}")"
  [[ -n "${expected_name}" ]] || return 0
  local cr_json
  if ! cr_json="$(kubectl -n "${NAMESPACE}" get onepassworditem "${expected_name}" \
                   -o json 2>/dev/null)"; then
    valsec_report "${connector_name}" "INFO" \
      "no OnePasswordItem CR (Secret managed manually)"
    return 0
  fi
  local secret_json
  if ! secret_json="$(kubectl -n "${NAMESPACE}" get secret "${expected_name}" \
                       -o json 2>/dev/null)"; then
    valsec_report "${connector_name}" "WARN" \
      "OnePasswordItem CR present but child Secret missing (operator drift?)"
    return 0
  fi
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-find-cr

  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-if-drift
  local cr_findings
  cr_findings="$(python3 -c '
import sys, json
cr = json.loads(sys.argv[1])
sec = json.loads(sys.argv[2])
cr_labels = (cr.get("metadata", {}).get("labels") or {})
cr_annos = (cr.get("metadata", {}).get("annotations") or {})
sec_labels = (sec.get("metadata", {}).get("labels") or {})
sec_annos = (sec.get("metadata", {}).get("annotations") or {})
# Filter out system-injected annotations from both sides for fairness.
# `operator.1password.io/*` is auto-added by the 1Password operator on the
# child Secret and is never present on the CR — it is not drift.
SYSTEM_ANNO_PREFIXES = (
    "kubectl.kubernetes.io/",
    "onepassword.com/last-applied",
    "operator.1password.io/",
)
def strip_system(d):
    return {k: v for k, v in d.items()
            if not any(k.startswith(p) for p in SYSTEM_ANNO_PREFIXES)}
cr_annos = strip_system(cr_annos)
sec_annos = strip_system(sec_annos)
findings = []
# Labels: operator should copy them. Drift here is suspicious.
for k in sorted(set(cr_labels) | set(sec_labels)):
    if cr_labels.get(k) != sec_labels.get(k):
        findings.append(("ERROR", f"label drift {k!r}: CR={cr_labels.get(k)!r} Secret={sec_labels.get(k)!r}"))
# Annotations: operator does NOT copy. CR is source of truth; report each
# annotation present on CR but absent (or different) on the Secret as INFO,
# AND report annotations only on the Secret as WARN (manual drift).
for k in sorted(set(cr_annos) - set(sec_annos)):
    findings.append(("INFO", f"annotation only on CR (expected): {k!r}"))
for k in sorted(set(sec_annos) - set(cr_annos)):
    findings.append(("WARN", f"annotation only on Secret (manual edit?): {k!r}"))
for k in sorted(set(cr_annos) & set(sec_annos)):
    if cr_annos.get(k) != sec_annos.get(k):
        findings.append(("WARN", f"annotation value drift {k!r}"))
if not findings:
    findings.append(("PASS", "OnePasswordItem CR ↔ Secret aligned"))
for status, msg in findings:
    print(f"{status}\t{msg}")
' "${cr_json}" "${secret_json}")"
  while IFS=$'\t' read -r status message; do
    [[ -n "${status}" ]] || continue
    valsec_report "${connector_name}" "${status}" "${message}"
  done <<<"${cr_findings}"
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-if-drift
}

# ---------------------------------------------------------------------------
# valsec_main — entrypoint dispatcher.
# ---------------------------------------------------------------------------
valsec_main() {
  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-loop
  local only_connector=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --connector) only_connector="$2"; shift 2 ;;
      --namespace) NAMESPACE="$2"; shift 2 ;;
      -h|--help)
        printf 'Usage: %s [--connector NAME] [--namespace NS]\n' "$0"
        return 0 ;;
      *) printf 'valsec_main: unknown arg %s\n' "$1" >&2; return 1 ;;
    esac
  done

  # Sanity: kubectl reachable.
  if ! kubectl version --client >/dev/null 2>&1; then
    printf 'valsec_main: kubectl not configured\n' >&2
    return 1
  fi

  # Iterate by example file (canonical contract source).
  local example
  for example in "${EXAMPLES_DIR}"/*.yaml.example; do
    [[ -f "${example}" ]] || continue
    local connector_name
    connector_name="$(basename "${example}" .yaml.example)"
    if [[ -n "${only_connector}" && "${connector_name}" != "${only_connector}" ]]; then
      continue
    fi
    valsec_check_secret_vs_example "${connector_name}"
    valsec_check_op_item_vs_secret "${connector_name}"
  done

  printf '\nSummary: errors=%d warnings=%d\n' \
    "${_VALSEC_ERRORS}" "${_VALSEC_WARNINGS}" >&2
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-loop
  # @cpt-begin:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-return
  if (( _VALSEC_ERRORS > 0 )); then
    return 2
  fi
  return 0
  # @cpt-end:cpt-insightspec-algo-reconcile-validate-secrets:p2:inst-vs-return
}

# Only run dispatch when executed (not sourced).
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  valsec_main "$@"
fi
